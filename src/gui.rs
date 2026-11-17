//! The deckd window: a clickable copy of the deck, an editor for the selected button, and live status.
//! Edits are saved to deck.toml (debounced); the service thread notices the file change and re-uploads.

use crate::config::{Action, Alt, Button, Config, Face, StateSource, COLS, ROWS, SAMPLE};
use crate::theme::Styles;
use crate::{actions, layout, service};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::theme::Theme;
use eframe::egui::RichText;

mod studio;

const UNDO_LIMIT: usize = 100;
const SAVE_DELAY: Duration = Duration::from_millis(500);
const QUICK_GLYPHS: &[&str] = &[
    "▶", "⏸", "⏭", "⏮", "⏹", "🔇", "🔉", "🔊", "✎", "✂", "⚙", "★", "♪", "⌂", "☀", "⏻", "✉", "⌨", "☎", "⚑", "♥", "✔", "✖", "↻",
];

fn parse_rgb(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten().map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    None,
    Run,
    Open,
    Hotkey,
    Media,
    Page,
    Print,
    Text,
    Delay,
    Macro,
    Focus,
}

/// The step list of the macro in an action slot of a button ("press", "double", "repeat", "tap_hold", "hold<n>").
fn slot_macro<'a>(b: &'a mut Button, slot: &str) -> Option<&'a mut Vec<Action>> {
    let a: Option<&mut Action> = match slot {
        "press" => b.action.as_mut(),
        "double" => b.double.as_mut(),
        "repeat" => b.repeat.as_mut(),
        "tap_hold" => b.tap_hold.as_mut(),
        s => s.strip_prefix("hold").and_then(|n| n.parse::<usize>().ok()).and_then(|n| b.holds.get_mut(n)).map(|h| &mut h.action),
    };
    match a {
        Some(Action::Macro { steps }) => Some(steps),
        _ => None,
    }
}

fn kind_of(a: &Option<Action>) -> Kind {
    match a {
        None => Kind::None,
        Some(Action::Text { .. }) => Kind::Text,
        Some(Action::Delay { .. }) => Kind::Delay,
        Some(Action::Macro { .. }) => Kind::Macro,
        Some(Action::Page { .. }) => Kind::Page,
        Some(Action::Print { .. }) => Kind::Print,
        Some(Action::Run { .. }) => Kind::Run,
        Some(Action::Open { .. }) => Kind::Open,
        Some(Action::Hotkey { .. }) => Kind::Hotkey,
        Some(Action::Media { .. }) => Kind::Media,
        Some(Action::Focus { .. }) => Kind::Focus,
    }
}

struct App {
    path: PathBuf,
    cfg: Config,
    selected: Option<(u8, u8)>,
    svc: service::Ctx,
    dirty_since: Option<Instant>,
    save_error: Option<String>,
    /// Set when deck.toml could not be loaded: saving is blocked so a hand-edited file is never overwritten silently.
    load_error: Option<String>,
    textures: HashMap<String, Option<egui::TextureHandle>>,
    show_log: bool,
    saved_at: Option<Instant>,
    test_msg: Option<(String, (bool, String))>,
    drag: Option<(u8, u8)>,
    /// Cell the dragged button would land on if dropped now.
    drag_hover: Option<(u8, u8)>,
    /// Page the dragged button came from (the window may switch pages while dragging).
    drag_page: u8,
    /// Page tabs as drawn last frame, and which tab the dragged button has been hovering over since when.
    tab_rects: Vec<(u8, Rect)>,
    tab_hover: Option<(u8, Instant)>,
    /// Which action slot is currently recording a hotkey ("press" / "double" / "hold").
    recording: Option<String>,
    /// Page being edited (1-based).
    page: u8,
    styles: Styles,
    /// Editor tab of the selected button.
    tab: Tab,
    /// Undo / redo: whole-config snapshots, one per saved batch of edits.
    undo: Vec<Config>,
    redo: Vec<Config>,
    committed: Config,
    restoring: bool,
    clipboard: Option<Button>,
    show_settings: bool,
    /// Tile size that fits the window (0 = use the style's size).
    fit: f32,
    lib_query: String,
    lib_cat: String,
    /// Library: pin the next action you add to all pages.
    lib_pin: bool,
    /// Whether the sign-in startup entry exists (read once at launch).
    autostart: bool,
    /// A macro's step list open in its own window: (page, key, action slot such as "press" or "hold0").
    macro_pop: Option<(u8, (u8, u8), String)>,
    /// True while that window is being drawn.
    in_macro_window: bool,
    /// Live preview textures keyed by what they show.
    previews: HashMap<String, egui::TextureHandle>,
    bg_tex: Option<(PathBuf, egui::TextureHandle)>,
    toast: Option<(String, Instant)>,
    style_msg: Option<String>,
    /// Font the window currently uses (glyph font, style font): reinstalled when either changes.
    fonts_for: (String, Option<PathBuf>),
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Look,
    Tap,
    Double,
    Hold,
    Repeat,
    TapHold,
    State,
}

/// What a tile's context menu / shortcut asked for (applied after drawing, so we never mutate while iterating).
enum TileCmd {
    Copy((u8, u8)),
    Paste((u8, u8)),
    Duplicate((u8, u8)),
    Clear((u8, u8)),
}

fn valid_cell(c: i32, r: i32) -> bool {
    (0..COLS as i32).contains(&c) && (0..ROWS as i32).contains(&r)
}

/// The wide slot spans (3,2) and (4,2); both map to its cell (3,2).
fn norm_cell(c: i32, r: i32) -> Option<(u8, u8)> {
    valid_cell(c, r).then(|| if (c, r) == (4, 2) { (3, 2) } else { (c as u8, r as u8) })
}

/// Tile width: the wide slot is two tiles plus the gap.
fn tile_width(col: u8, row: u8, tile: f32, gap: f32) -> f32 {
    if (col, row) == (3, 2) { 2.0 * tile + gap } else { tile }
}

fn c32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl App {
    fn tg(&self) -> (f32, f32) {
        let t = &self.styles.theme;
        if self.fit > 0.0 {
            (self.fit, (t.tile_gap * self.fit / t.tile_size).round().max(4.0))
        } else {
            (t.tile_size, t.tile_gap)
        }
    }

    /// Put a copy of `b` on the clipboard (internal, and as TOML text for pasting elsewhere).
    fn copy_button(&mut self, ctx: &egui::Context, cell: (u8, u8)) {
        if let Some(b) = self.cfg.button_at(self.page, cell.0, cell.1).cloned() {
            if let Ok(text) = toml::to_string_pretty(&b) {
                ctx.copy_text(text);
            }
            self.clipboard = Some(b);
            self.toast("Copied");
        }
    }

    fn paste_button(&mut self, cell: (u8, u8)) {
        let Some(mut b) = self.clipboard.clone() else { return };
        b.page = self.page;
        b.pos = format!("{},{}", cell.0, cell.1);
        if b.face.is_firmware() && cell != (3, 2) {
            b.face = Face::Image; // firmware faces only exist on the wide slot
        }
        self.cfg.clear_button(self.page, cell.0, cell.1);
        self.cfg.buttons.push(b);
        self.selected = Some(cell);
        self.changed();
        self.toast("Pasted");
    }

    fn first_empty_cell(&self) -> Option<(u8, u8)> {
        (0..ROWS).flat_map(|r| (0..COLS).map(move |c| (c, r))).filter(|&(c, r)| !(r == 2 && c >= 3)).find(|&(c, r)| self.cfg.button_at(self.page, c, r).is_none())
    }

    fn toast(&mut self, msg: &str) {
        self.toast = Some((msg.to_string(), Instant::now()));
    }

    fn do_undo(&mut self) {
        if self.dirty_since.is_some() {
            self.try_save();
        }
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.cfg, prev));
            self.committed = self.cfg.clone();
            self.restoring = true;
            self.changed();
            self.toast("Undone");
        }
    }

    fn do_redo(&mut self) {
        if self.dirty_since.is_some() {
            self.try_save();
        }
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.cfg, next));
            self.committed = self.cfg.clone();
            self.restoring = true;
            self.changed();
            self.toast("Redone");
        }
    }

    /// Swap the buttons at two cells (either may be empty).
    fn swap_cells(&mut self, a: (u8, u8), b: (u8, u8)) {
        if a == b {
            return;
        }
        let page = self.page;
        for btn in self.cfg.buttons.iter_mut().filter(|b| b.page == page) {
            match btn.cell() {
                Ok(c) if c == a => btn.pos = format!("{},{}", b.0, b.1),
                Ok(c) if c == b => btn.pos = format!("{},{}", a.0, a.1),
                _ => {}
            }
        }
        self.selected = Some(b);
        self.changed();
    }

    /// Move the button at `a` on page `sp` to page `dp`: onto cell `b` (swapping with what is there; the other button goes
    /// to `a` on `sp`), or, with no cell given, onto the first free cell of that page.
    fn move_across_pages(&mut self, sp: u8, a: (u8, u8), dp: u8, b: Option<(u8, u8)>) {
        if sp == dp {
            return;
        }
        let cell_of = |btn: &Button| btn.cell().ok();
        let Some(src) = self.cfg.buttons.iter().position(|x| x.page == sp && cell_of(x) == Some(a)) else {
            return; // nothing own to move (empty key, or only shown there because it is pinned)
        };
        let dest = b.or_else(|| {
            (0..ROWS).flat_map(|r| (0..COLS).map(move |c| (c, r))).filter_map(|(c, r)| norm_cell(c as i32, r as i32)).find(|c| {
                !self.cfg.buttons.iter().any(|x| x.page == dp && cell_of(x) == Some(*c))
            })
        });
        let Some(dest) = dest else {
            self.toast(&format!("Page {dp} is full"));
            return;
        };
        if let Some(other) = self.cfg.buttons.iter().position(|x| x.page == dp && cell_of(x) == Some(dest)) {
            self.cfg.buttons[other].page = sp;
            self.cfg.buttons[other].pos = format!("{},{}", a.0, a.1);
        }
        self.cfg.buttons[src].page = dp;
        self.cfg.buttons[src].pos = format!("{},{}", dest.0, dest.1);
        self.page = dp;
        self.selected = Some(dest);
        self.changed();
    }

    fn changed(&mut self) {
        self.dirty_since = Some(Instant::now());
    }

    fn try_save(&mut self) {
        self.dirty_since = None;
        if self.load_error.is_some() {
            return;
        }
        // Validate the way the service will, so a bad edit never reaches the deck or the file.
        let check = self
            .cfg
            .buttons
            .iter()
            .try_for_each(|b| {
                b.cell()?;
                b.check_face()?;
                [&b.action, &b.double, &b.repeat, &b.tap_hold]
                    .into_iter()
                    .flatten()
                    .chain(b.holds.iter().map(|h| &h.action))
                    .try_for_each(|a| actions::validate(a).map_err(|e| format!("button {:?} (page {}): {e}", b.pos, b.page)))
            })
            .and_then(|_| (1..=self.cfg.page_count()).try_for_each(|p| layout::build_zip(&self.cfg, p).map(|_| ())));
        self.save_error = match check.and_then(|_| self.cfg.save(&self.path)) {
            Ok(()) => {
                self.saved_at = Some(Instant::now());
                if !self.restoring && serde_json::to_string(&self.cfg).ok() != serde_json::to_string(&self.committed).ok() {
                    self.undo.push(self.committed.clone());
                    if self.undo.len() > UNDO_LIMIT {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                }
                self.restoring = false;
                self.committed = self.cfg.clone();
                None
            }
            Err(e) => Some(e),
        };
    }

    /// Milliseconds on the window's animation clock (GIFs play from this).
    fn anim_ms(ctx: &egui::Context) -> u64 {
        (ctx.input(|i| i.time) * 1000.0) as u64
    }

    fn texture(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        // animated GIF: pick the frame for now and schedule the next repaint for when it changes
        if self.cfg.animate_gifs && layout::is_gif(path) {
            let full = self.cfg.base_dir.join(path);
            if let Some((idx, left)) = layout::gif_index_at(&full, Self::anim_ms(ctx)) {
                ctx.request_repaint_after(Duration::from_millis(left.clamp(16, 1000)));
                let key = format!("{path}#{idx}");
                if !self.textures.contains_key(&key) {
                    if self.textures.len() > 600 {
                        self.textures.clear();
                    }
                    let tex = layout::gif_frame_image(&full, idx).map(|img| {
                        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                        ctx.load_texture(key.clone(), ci, egui::TextureOptions::LINEAR)
                    });
                    self.textures.insert(key.clone(), tex);
                }
                if let Some(t) = self.textures.get(&key).cloned().flatten() {
                    return Some(t);
                }
            }
        }
        if !self.textures.contains_key(path) {
            let full = self.cfg.base_dir.join(path);
            let tex = image::open(&full).ok().map(|img| {
                let rgba = img.resize_exact(196, 196, image::imageops::FilterType::Triangle).to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([196, 196], rgba.as_raw());
                ctx.load_texture(path, ci, egui::TextureOptions::LINEAR)
            });
            self.textures.insert(path.to_string(), tex);
        }
        self.textures.get(path).cloned().flatten()
    }

    /// The icon of the program a button starts (shown when the button has no picture or symbol of its own).
    fn app_texture(&mut self, ctx: &egui::Context, b: &Button) -> Option<egui::TextureHandle> {
        if b.icon.is_some() || b.glyph.is_some() || !b.face.is_image() {
            return None;
        }
        let key = format!("app:{}", crate::appicon::target(b)?.display());
        if !self.textures.contains_key(&key) {
            let tex = crate::appicon::image_for(b).map(|img| {
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                ctx.load_texture(key.clone(), ci, egui::TextureOptions::LINEAR)
            });
            self.textures.insert(key.clone(), tex);
        }
        self.textures.get(&key).cloned().flatten()
    }

    fn draw_tile(&mut self, ui: &mut egui::Ui, col: u8, row: u8, pressed: bool, cmds: &mut Vec<TileCmd>) {
        let (tile, gap) = self.tg();
        let th = self.styles.theme.clone();
        let rect = Rect::from_min_size(
            ui.min_rect().min + Vec2::new(col as f32 * (tile + gap), row as f32 * (tile + gap)),
            Vec2::new(tile_width(col, row, tile, gap), tile),
        );
        let resp = ui.interact(rect, ui.scope_id().with(("tile", col, row)), Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
            self.selected = Some((col, row));
        }
        let has_button = self.cfg.button_at(self.page, col, row).is_some();
        let has_clip = self.clipboard.is_some();
        resp.context_menu(|ui| {
            if ui.add_enabled(has_button, egui::Button::new("Copy   Ctrl+C")).clicked() {
                cmds.push(TileCmd::Copy((col, row)));
                ui.close();
            }
            if ui.add_enabled(has_clip, egui::Button::new("Paste   Ctrl+V")).clicked() {
                cmds.push(TileCmd::Paste((col, row)));
                ui.close();
            }
            if ui.add_enabled(has_button, egui::Button::new("Duplicate to next free key   Ctrl+D")).clicked() {
                cmds.push(TileCmd::Duplicate((col, row)));
                ui.close();
            }
            ui.separator();
            if ui.add_enabled(has_button, egui::Button::new("Clear   Del")).clicked() {
                cmds.push(TileCmd::Clear((col, row)));
                ui.close();
            }
        });
        if resp.drag_started() {
            self.drag = Some((col, row));
            self.drag_page = self.page;
            self.tab_hover = None;
        }
        let pointer = ui.ctx().pointer_interact_pos();
        if resp.drag_stopped() {
            self.drag = None;
            self.tab_hover = None;
            let src_page = self.drag_page;
            let on_tab = pointer.and_then(|p| self.tab_rects.iter().find(|(_, r)| r.contains(p)).map(|(n, _)| *n));
            if let Some(n) = on_tab {
                // dropped on a page tab: the button goes to that page (first free key)
                self.move_across_pages(src_page, (col, row), n, None);
            } else if let Some(p) = pointer {
                // drop onto another tile = swap the two buttons (on another page: move it there)
                let origin = rect.min - Vec2::new(col as f32 * (tile + gap), row as f32 * (tile + gap));
                let (tc, tr) = (((p.x - origin.x) / (tile + gap)).floor() as i32, ((p.y - origin.y) / (tile + gap)).floor() as i32);
                if let Some(t) = norm_cell(tc, tr) {
                    if src_page == self.page {
                        self.swap_cells((col, row), t);
                    } else {
                        let page = self.page;
                        self.move_across_pages(src_page, (col, row), page, Some(t));
                    }
                }
            }
        }

        // While dragging over another tile, preview the swap: the two tiles show each other's button.
        let page = self.page;
        let same_page = self.drag_page == page;
        let swapping = match (self.drag, self.drag_hover) {
            (Some(s), Some(h)) if s != h && same_page => Some((s, h)),
            _ => None,
        };
        let shown = match swapping {
            Some((s, h)) if (col, row) == s => h,
            Some((s, h)) if (col, row) == h => s,
            _ => (col, row),
        };
        let own = self.cfg.button_at(page, col, row).cloned();
        let b = self.cfg.button_at(page, shown.0, shown.1).cloned();
        let tex = b.as_ref().and_then(|b| b.icon.clone()).and_then(|i| self.texture(ui.ctx(), &i));
        let own_tex = if shown == (col, row) { tex.clone() } else { own.as_ref().and_then(|b| b.icon.clone()).and_then(|i| self.texture(ui.ctx(), &i)) };
        let is_source = self.drag == Some((col, row)) && same_page;
        let alpha = if swapping.is_some() && shown != (col, row) {
            0.75 // phased preview of where the other button would land
        } else if is_source {
            0.3
        } else {
            1.0
        };
        let app_tex = b.as_ref().and_then(|b| self.app_texture(ui.ctx(), b));
        // live faces (clock, stats, weather) show the real picture that is sent to the deck, updating as it changes
        let live_tex = match b.as_ref() {
            Some(lb) if lb.face.is_live() => self.preview_texture(ui.ctx(), lb, (col, row) == (3, 2), false),
            _ => None,
        };
        if let Some(t) = &live_tex {
            let r = if self.cfg.round_icons { th.tile_rounding } else { 0.0 };
            rounded_image(ui.painter(), t.id(), rect, r, Color32::from_white_alpha((alpha * 255.0) as u8));
        } else {
            paint_tile(ui.painter(), rect, b.as_ref(), tex.as_ref(), app_tex.as_ref(), alpha, th.tile_rounding, th.tile_empty, self.cfg.round_icons);
        }
        if b.as_ref().is_some_and(|b| b.pinned) {
            ui.painter().text(rect.right_top() + Vec2::new(-9.0, 10.0), Align2::CENTER_CENTER, "📌", FontId::proportional(12.0), Color32::WHITE);
        }
        let caption = match b.as_ref().map(|b| b.face) {
            None if (col, row) == (3, 2) => Some("wide slot: firmware clock\n(click to set up a button)"),
            Some(Face::Clock) => Some("firmware clock"),
            Some(Face::Stats) => Some("firmware stats"),
            Some(Face::LiveClock) => Some("live clock"),
            Some(Face::LiveStats) => Some("live stats"),
            Some(Face::LiveWeather) => Some("live weather"),
            Some(Face::ClockWeather) => Some("clock + weather"),
            Some(Face::Everything) => Some("time + date + weather"),
            _ => None,
        };
        if let (Some(c), None) = (caption, &live_tex) {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, c, FontId::proportional(12.0), th.text_dim);
        }

        let is_target = swapping.is_some_and(|(_, h)| h == (col, row) || (is_source && shown != (col, row)))
            || (self.drag.is_some() && !same_page && self.drag_hover == Some((col, row)));
        let selected = self.selected == Some((col, row));
        let stroke = if pressed {
            Stroke::new(3.0, th.press)
        } else if is_target {
            Stroke::new(3.0, th.ok)
        } else if selected {
            Stroke::new(3.0, th.select)
        } else if resp.hovered() {
            Stroke::new(2.0, th.text_dim)
        } else {
            Stroke::new(1.0, th.tile_border)
        };
        ui.painter().rect_stroke(rect, th.tile_rounding, stroke, StrokeKind::Inside);

        // the same rim the deck is showing right now (tap / double / hold step / repeat)
        let rim = self.svc.status.lock().ok().and_then(|s| (s.page == self.page).then(|| s.rims[(row * COLS + col) as usize]).flatten());
        if let (Some(c), true) = (rim, self.cfg.feedback.enabled) {
            let k = tile / 196.0;
            let fb = &self.cfg.feedback;
            let w = (fb.width as f32 * k).max(2.0);
            // centered stroke so the rim's inner edge is rounded too (outer corner matches the tile)
            let r = (th.tile_rounding - w / 2.0).max(w / 2.0);
            ui.painter().rect_stroke(rect.shrink(w / 2.0), r, Stroke::new(w, Color32::from_rgb(c[0], c[1], c[2])), StrokeKind::Middle);
        }

        // translucent ghost that follows the pointer while dragging
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some(p) = pointer {
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
                let painter = ui.ctx().layer_painter(layer);
                let r = Rect::from_center_size(p, Vec2::new(tile_width(col, row, tile, gap), tile));
                // the ghost is the button being dragged, which may come from another page
                let (ghost, ghost_tex) = if same_page {
                    (own.clone(), own_tex.clone())
                } else {
                    let g = self.cfg.button_at(self.drag_page, col, row).cloned();
                    let t = g.as_ref().and_then(|b| b.icon.clone()).and_then(|i| self.texture(ui.ctx(), &i));
                    (g, t)
                };
                let own_app = ghost.as_ref().and_then(|b| self.app_texture(ui.ctx(), b));
                paint_tile(&painter, r, ghost.as_ref(), ghost_tex.as_ref(), own_app.as_ref(), 0.6, th.tile_rounding, th.tile_empty, self.cfg.round_icons);
                painter.rect_stroke(r, th.tile_rounding,Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 255, 255, 150)), StrokeKind::Inside);
            }
        }
    }

    fn deck_panel(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<TileCmd>) {
        let (tile, gap) = self.tg();
        // flash only when the deck is showing the page we are looking at
        let on_page = self.svc.status.lock().ok().is_some_and(|s| s.page == self.page || s.page == 0);
        let pressed = self.svc.status.lock().ok().and_then(|s| s.pressed).filter(|(_, t)| on_page && t.elapsed() < Duration::from_millis(400));
        let size = Vec2::new(COLS as f32 * (tile + gap) - gap, ROWS as f32 * (tile + gap) - gap);
        // centered (or left aligned, per style) in the space we have
        let avail = ui.available_width();
        let indent = if self.styles.theme.deck_left { 0.0 } else { ((avail - size.x) / 2.0).max(0.0) };
        let (full, _) = ui.allocate_exact_size(Vec2::new(avail.max(size.x), size.y), Sense::hover());
        let outer = Rect::from_min_size(full.min + Vec2::new(indent, 0.0), size);
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(outer));
        self.drag_hover = match (self.drag, ui.ctx().pointer_interact_pos()) {
            (Some(_), Some(p)) => {
                let (c, r) = (((p.x - outer.min.x) / (tile + gap)).floor() as i32, ((p.y - outer.min.y) / (tile + gap)).floor() as i32);
                norm_cell(c, r)
            }
            _ => None,
        };
        for row in 0..ROWS {
            for col in 0..COLS {
                if row == 2 && col == 4 {
                    continue; // covered by the wide slot, drawn as the tile at (3,2)
                }
                let flashing = pressed.map(|(i, _)| i == row * COLS + col).unwrap_or(false);
                self.draw_tile(&mut inner, col, row, flashing, cmds);
            }
        }
    }

    /// Editor for one action slot (press / double tap / hold). `slot` identifies it for hotkey recording.
    fn action_editor(&mut self, ui: &mut egui::Ui, slot: &str, action: &mut Option<Action>) {
        self.action_editor_ex(ui, slot, action, false);
    }

    /// `step`: editing one step of a macro (no page switch, no nested macro, a Wait is allowed).
    fn action_editor_ex(&mut self, ui: &mut egui::Ui, slot: &str, action: &mut Option<Action>, step: bool) {
        let bad = Color32::from_rgb(0xff, 0x7b, 0x72);
        let mut kind = kind_of(action);
        let old_kind = kind;
        if !step {
            let kinds: &[(Kind, &str)] = &[(Kind::None, "Nothing"), (Kind::Run, "Program"), (Kind::Open, "Open"), (Kind::Hotkey, "Hotkey"), (Kind::Media, "Media"), (Kind::Text, "Type text"), (Kind::Focus, "Focus window"), (Kind::Macro, "Macro"), (Kind::Page, "Page"), (Kind::Print, "Print")];
            let cur = kinds.iter().find(|(k, _)| *k == kind).map(|(_, n)| *n).unwrap_or("Nothing");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Action").color(self.styles.theme.text_dim));
                egui::ComboBox::from_id_salt(format!("kind_{slot}")).selected_text(cur).width(ui.available_width().max(120.0) - 8.0).show_ui(ui, |ui| {
                    for (k, name) in kinds {
                        ui.selectable_value(&mut kind, *k, *name);
                    }
                });
            });
        }
        if kind != old_kind {
            self.recording = None;
            self.test_msg = None;
            let previous = action.take();
            *action = match kind {
                Kind::None => None,
                Kind::Run => Some(Action::Run { command: String::new(), args: vec![] }),
                Kind::Open => Some(Action::Open { target: String::new() }),
                Kind::Hotkey => Some(Action::Hotkey { keys: String::new() }),
                Kind::Media => Some(Action::Media { key: "play_pause".into() }),
                Kind::Page => Some(Action::Page { to: "next".into() }),
                Kind::Print => Some(Action::Print { text: "hello".into() }),
                Kind::Text => Some(Action::Text { text: String::new() }),
                Kind::Delay => Some(Action::Delay { ms: 250 }),
                Kind::Focus => Some(Action::Focus { window: String::new() }),
                // the action that was there becomes step 1 of the new macro
                Kind::Macro => Some(Action::Macro { steps: previous.filter(|p| !matches!(p, Action::Page { .. } | Action::Macro { .. })).into_iter().collect() }),
            };
        }
        ui.add_space(4.0);
        let pages = self.cfg.page_count();
        // Text fields leave room for the button beside them (text edits add their own margins).
        let room = |ui: &egui::Ui, extra: f32| (ui.available_width() - extra).max(60.0);
        match action {
            Some(Action::Run { command, args }) => {
                ui.horizontal(|ui| {
                    let w = room(ui, 110.0);
                    ui.add(egui::TextEdit::singleline(command).desired_width(w).hint_text("program, e.g. notepad.exe"));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Programs", &["exe", "bat", "cmd", "lnk"]).pick_file() {
                            *command = p.display().to_string();
                        }
                    }
                });
                let mut text = args.join(" ");
                let w = room(ui, 16.0);
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(w).hint_text("arguments (optional)")).changed() {
                    *args = text.split_whitespace().map(String::from).collect();
                }
            }
            Some(Action::Open { target }) => {
                ui.horizontal(|ui| {
                    let w = room(ui, 100.0);
                    ui.add(egui::TextEdit::singleline(target).desired_width(w).hint_text("https://… or a file / folder"));
                    if ui.button("File…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_file() {
                            *target = p.display().to_string();
                        }
                    }
                });
            }
            Some(Action::Hotkey { keys }) => {
                let recording = self.recording.as_deref() == Some(slot);
                if recording {
                    let ev = ui.ctx().input(|i| {
                        i.events.iter().find_map(|e| match e {
                            egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                            _ => None,
                        })
                    });
                    if let Some((key, m)) = ev {
                        if key == egui::Key::Escape && !m.any() {
                            self.recording = None;
                        } else {
                            let mut parts: Vec<String> = Vec::new();
                            if m.ctrl {
                                parts.push("ctrl".into());
                            }
                            if m.shift {
                                parts.push("shift".into());
                            }
                            if m.alt {
                                parts.push("alt".into());
                            }
                            parts.push(key_name(key));
                            *keys = parts.join("+");
                            self.recording = None;
                        }
                    }
                    ui.ctx().request_repaint();
                }
                ui.horizontal(|ui| {
                    let w = room(ui, 150.0);
                    ui.add_enabled(!recording, egui::TextEdit::singleline(keys).desired_width(w).hint_text("e.g. ctrl+shift+m"));
                    if recording {
                        if ui.button("Press keys… (Esc)").clicked() {
                            self.recording = None;
                        }
                    } else if ui.button("⏺ Record").clicked() {
                        self.recording = Some(slot.to_string());
                    }
                });
                if !step {
                    ui.weak("Record captures the combo you press. The Windows key can't be recorded; type win+… instead.");
                }
            }
            Some(Action::Media { key }) => {
                ui.horizontal_wrapped(|ui| {
                    for (k, name) in [
                        ("vol_down", "🔉 Vol −"),
                        ("vol_up", "🔊 Vol +"),
                        ("mute", "🔇 Mute"),
                        ("play_pause", "⏯ Play/Pause"),
                        ("prev", "⏮ Prev"),
                        ("next", "⏭ Next"),
                        ("stop", "⏹ Stop"),
                        ("mic_mute", "🎤 Mic mute"),
                    ] {
                        ui.selectable_value(key, k.to_string(), name);
                    }
                });
            }
            Some(Action::Page { to }) => {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(to, "next".to_string(), "Next page");
                    ui.selectable_value(to, "prev".to_string(), "Previous page");
                    ui.selectable_value(to, "home".to_string(), "Home");
                    for p in 1..=pages {
                        ui.selectable_value(to, p.to_string(), format!("Page {p}"));
                    }
                });
                if pages < 2 {
                    ui.weak("Add a second page with + above the deck first.");
                }
            }
            Some(Action::Print { text }) => {
                let w = room(ui, 16.0);
                ui.add(egui::TextEdit::singleline(text).desired_width(w).hint_text("message for the log"));
                ui.weak("Shows in the Log (bottom bar) when the gesture fires. Good for testing double tap / hold.");
            }
            Some(Action::Delay { ms }) => {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::DragValue::new(ms).range(10..=60000).suffix(" ms").speed(10));
                    for v in [50u32, 100, 250, 500, 1000, 2000] {
                        if ui.small_button(v.to_string()).clicked() {
                            *ms = v;
                        }
                    }
                });
            }
            Some(Action::Focus { window }) => {
                ui.horizontal(|ui| {
                    let w = room(ui, 150.0);
                    ui.add(egui::TextEdit::singleline(window).desired_width(w).hint_text("part of the title, or the program (discord)"));
                    ui.menu_button("Open windows ▾", |ui| {
                        ui.set_max_width(320.0);
                        let titles = actions::list_windows();
                        if titles.is_empty() {
                            ui.weak("No windows found");
                        }
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for (title, exe) in titles {
                                let shown: String = title.chars().take(48).collect();
                                if ui.button(format!("{shown}  ·  {exe}")).clicked() {
                                    // the program name is steadier than a title that changes (documents, tabs)
                                    *window = exe.trim_end_matches(".exe").to_string();
                                    ui.close();
                                }
                            }
                        });
                    });
                });
                ui.weak("Brings the first matching window to the front, as if you had clicked it (restores it if minimized). Put a Wait after it before typing.");
            }
            Some(Action::Text { text }) => {
                let w = room(ui, 16.0);
                ui.add(egui::TextEdit::multiline(text).desired_rows(3).desired_width(w).hint_text("text to type"));
                ui.weak(format!("{} characters, typed into whichever window has focus. A line break presses Enter.", text.chars().count()));
            }
            Some(Action::Macro { .. }) if !step && !self.in_macro_window && self.macro_pop.as_ref().is_some_and(|m| m.2 == slot && Some(m.1) == self.selected && m.0 == self.page) => {
                ui.add_space(4.0);
                ui.label("This macro is open in its own window.");
                ui.add_space(4.0);
                if ui.button("Bring it back here").clicked() {
                    self.macro_pop = None;
                }
            }
            Some(Action::Macro { steps }) => {
                let th = self.styles.theme.clone();
                let slot_s = slot.to_string();
                if !step && !self.in_macro_window {
                    if let Some(cell) = self.selected {
                        if ui.small_button("⧉ Pop out").on_hover_text("Open the steps in their own, bigger window").clicked() {
                            self.macro_pop = Some((self.page, cell, slot.to_string()));
                        }
                    }
                }
                crate::macro_ui::rail(ui, &th, slot, steps, &mut |ui, n, a| {
                    let mut act = Some(a.clone());
                    self.action_editor_ex(ui, &format!("{slot_s}.s{n}"), &mut act, true);
                    if let Some(x) = act {
                        *a = x;
                    }
                });
            }
            None => {
                ui.weak("Nothing happens.");
            }
        }
        if let Some(a) = action.clone().filter(|a| !step && !matches!(a, Action::Macro { .. })) {
            ui.add_space(4.0);
            match actions::validate(&a) {
                Err(e) => {
                    ui.colored_label(bad, e);
                }
                Ok(()) if !matches!(a, Action::Page { .. }) => {
                    if ui.button("▶ Test now").on_hover_text("Runs this action on your PC right now").clicked() {
                        let slow = matches!(a, Action::Macro { .. } | Action::Delay { .. } | Action::Text { .. });
                        self.test_msg = Some((slot.to_string(), if slow {
                            // typing / waiting must not freeze the window, and the text must land in the other window
                            let a2 = a.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_secs(3));
                                let _ = actions::execute(&a2);
                            });
                            (true, "Starts in 3 seconds: click into the window where it should happen.".to_string())
                        } else {
                            match (&a, actions::execute(&a)) {
                                (Action::Print { text }, _) => (true, format!("Would log: {text}")),
                                (_, Ok(())) => (true, "Ran.".to_string()),
                                (_, Err(e)) => (false, e),
                            }
                        }));
                    }
                }
                Ok(()) => {}
            }
            if let Some((s, (ok, m))) = &self.test_msg {
                if s == slot {
                    ui.colored_label(if *ok { Color32::from_rgb(0x3f, 0xb9, 0x50) } else { bad }, m);
                }
            }
        }
    }

    fn style_note(&self) -> Option<String> {
        self.styles.error.as_ref().map(|e| format!("Style problem (kept the previous look): {e}"))
    }

    /// (Re)apply the current style: colors, shapes, sizes, fonts.
    fn apply_style(&mut self, ctx: &egui::Context) {
        let th = self.styles.theme.clone();
        th.apply(ctx);
        let want = (self.cfg.glyph_font.clone(), th.font_path.clone());
        if want != self.fonts_for {
            install_fonts(ctx, &want.0, want.1.as_deref());
            self.fonts_for = want;
        }
        self.previews.clear();
        self.bg_tex = None;
        ctx.request_repaint();
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.styles.theme.clone();
        let bar = egui::Frame::new().fill(th.panel).inner_margin(egui::Margin::symmetric(16, 10));
        egui::Panel::top("top").frame(bar).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("deckd").strong().size(th.heading_size).color(th.accent));
                ui.add_space(14.0);
                let count = self.cfg.page_count();
                if self.page > count {
                    self.page = count;
                }
                for p in 1..=count {
                    let live = self.svc.status.lock().ok().is_some_and(|s| s.connected && s.page == p && count > 1);
                    let text = if live { format!("Page {p} ●") } else { format!("Page {p}") };
                    if ui.selectable_label(self.page == p, text).on_hover_text(if live { "currently shown on the deck" } else { "" }).clicked() {
                        self.page = p;
                    }
                }
                if count < crate::config::MAX_PAGES && ui.small_button("+").on_hover_text("Add a page").clicked() {
                    self.cfg.pages = count + 1;
                    self.page = count + 1;
                    self.changed();
                }
                let last_empty = !self.cfg.buttons.iter().any(|b| b.page == count);
                if count > 1 && last_empty && ui.small_button("−").on_hover_text("Remove the last page (it is empty)").clicked() {
                    self.cfg.pages = count - 1;
                    self.page = self.page.min(count - 1);
                    self.changed();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙").on_hover_text("Settings and window style").clicked() {
                        self.show_settings = true;
                    }
                    if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("↷")).on_hover_text("Redo (Ctrl+Y)").clicked() {
                        self.do_redo();
                    }
                    if ui.add_enabled(!self.undo.is_empty() || self.dirty_since.is_some(), egui::Button::new("↶")).on_hover_text("Undo (Ctrl+Z)").clicked() {
                        self.do_undo();
                    }
                    ui.add_space(8.0);
                    let mut br = self.cfg.brightness as f32;
                    let r = ui.add(egui::Slider::new(&mut br, 0.0..=100.0).integer().show_value(false).trailing_fill(true));
                    ui.label("☀").on_hover_text("Deck brightness");
                    self.cfg.brightness = br as u8;
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        self.changed();
                    }
                });
            });
        });
    }

    /// The steps of one macro in their own OS window, as big as you like (just the step list, nothing else).
    fn macro_window(&mut self, ctx: &egui::Context) {
        let Some((page, cell, slot)) = self.macro_pop.clone() else { return };
        let th = self.styles.theme.clone();
        let mut open = true;
        let builder = egui::ViewportBuilder::default().with_title("deckd macro").with_inner_size([620.0, 860.0]).with_min_inner_size([420.0, 300.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("deckd_macro"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                open = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(th.window).inner_margin(egui::Margin::same(16))).show(ui, |ui| {
                let Some(mut b) = self.cfg.button_at(page, cell.0, cell.1).cloned() else {
                    open = false;
                    return;
                };
                let before = serde_json::to_string(&b).unwrap_or_default();
                ui.label(egui::RichText::new(format!("Macro on button {},{} (page {page})", cell.0, cell.1)).strong().size(th.heading_size));
                ui.add_space(6.0);
                let slot_s = slot.clone();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match slot_macro(&mut b, &slot) {
                    Some(steps) => {
                        self.in_macro_window = true;
                        crate::macro_ui::rail(ui, &th, &slot, steps, &mut |ui, n, a| {
                            let mut act = Some(a.clone());
                            self.action_editor_ex(ui, &format!("{slot_s}.s{n}"), &mut act, true);
                            if let Some(x) = act {
                                *a = x;
                            }
                        });
                        self.in_macro_window = false;
                    }
                    None => {
                        open = false; // that action is no longer a macro
                    }
                });
                if before != serde_json::to_string(&b).unwrap_or_default() {
                    *self.cfg.button_mut(page, cell.0, cell.1) = b;
                    self.changed();
                }
            });
        });
        if !open {
            self.macro_pop = None;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut pick: Option<String> = None;
        let mut create = false;
        // its own OS window, so it can be moved aside and never covers the deck
        let builder = egui::ViewportBuilder::default().with_title("deckd settings").with_inner_size([520.0, 720.0]).with_min_inner_size([440.0, 420.0]);
        let bg = self.styles.theme.window;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("deckd_settings"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                open = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(bg).inner_margin(egui::Margin::same(18))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading("Window style");
            ui.weak("A style recolors the window and can rearrange it. Drop a style file into the styles folder and pick it here.");
            let selected = self.styles.list.iter().find(|e| e.id == self.styles.selected).map(|e| e.name.clone()).unwrap_or_else(|| self.styles.selected.clone());
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("style_pick").selected_text(selected).width(220.0).show_ui(ui, |ui| {
                    for e in self.styles.list.clone() {
                        if ui.selectable_label(e.id == self.styles.selected, &e.name).clicked() {
                            pick = Some(e.id.clone());
                        }
                    }
                });
                if ui.button("Rescan").on_hover_text("Look for new style files").clicked() {
                    self.styles.refresh_list();
                }
            });
            let th = &self.styles.theme;
            if !th.description.is_empty() || !th.author.is_empty() {
                ui.weak(format!("{}{}", th.description, if th.author.is_empty() { String::new() } else { format!(" · by {}", th.author) }));
            }
            if let Some(e) = &self.styles.error {
                ui.colored_label(th.error, e);
            }
            ui.horizontal(|ui| {
                if ui.button("Open styles folder").clicked() {
                    let _ = std::process::Command::new("explorer").arg(&self.styles.dir).spawn();
                }
                if ui.button("Create a style to edit").on_hover_text("Writes a commented template and selects it; save the file to see changes live").clicked() {
                    create = true;
                }
            });
            if let Some(m) = &self.style_msg {
                ui.weak(m);
            }
            ui.add_space(8.0);
            ui.separator();
            ui.heading("Startup");
            let mut auto = self.autostart;
            if ui.checkbox(&mut auto, "Start deckd when I sign in to Windows").on_hover_text("Starts minimized, with the deck working right away. Only for your Windows account.").changed() {
                if set_autostart(auto) {
                    self.autostart = auto;
                } else {
                    self.toast("Could not change the startup entry");
                }
            }
            ui.add_space(8.0);
            ui.separator();
            ui.heading("Weather");
            ui.weak("For the wide key's Weather and Clock + weather looks. Only the city name is sent, to Open-Meteo (free, no account).");
            ui.horizontal(|ui| {
                ui.label("City");
                if ui.add(egui::TextEdit::singleline(&mut self.cfg.weather_city).desired_width(200.0).hint_text("e.g. London")).changed() {
                    self.changed();
                }
                if ui.checkbox(&mut self.cfg.weather_fahrenheit, "°F").changed() {
                    self.changed();
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.heading("Animation");
            if ui.checkbox(&mut self.cfg.animate_gifs, "Play GIFs in this window").on_hover_text("Off shows the first frame only. The deck itself always plays them.").changed() {
                self.changed();
            }
            if ui.checkbox(&mut self.cfg.round_icons, "Round the corners of button pictures in this window").on_hover_text("Only changes how the window draws them; the deck's own screen stays as it is.").changed() {
                self.changed();
            }
            ui.add_space(8.0);
            ui.separator();
            ui.heading("Button feedback");
            ui.weak("A colored rim flashed on the deck button when a gesture fires.");
            let before = self.cfg.feedback.clone();
            let fb = &mut self.cfg.feedback;
            ui.checkbox(&mut fb.enabled, "Flash a rim on the button when a gesture fires");
            ui.add_enabled_ui(fb.enabled, |ui| {
                ui.add(egui::Slider::new(&mut fb.width, 2..=40).suffix(" px").text("rim width"));
                ui.add(egui::Slider::new(&mut fb.radius, 0..=60).suffix(" px").text("corner rounding"));
                ui.add(egui::Slider::new(&mut fb.flash_ms, 100..=3000).logarithmic(true).suffix(" ms").text("shown for"));
                for (label, field) in [("Tap", &mut fb.tap), ("Double tap", &mut fb.double), ("Hold (on release)", &mut fb.hold), ("While held (repeat)", &mut fb.repeat)] {
                    ui.horizontal(|ui| {
                        let mut rgb = parse_rgb(field).unwrap_or([255, 255, 255]);
                        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                            *field = hex(rgb);
                        }
                        ui.label(label);
                    });
                }
                ui.weak("While holding, the rim appears at each hold step and shifts color per step. A repeat keeps its rim while held. Each button can override the tap and double-tap colors in its Actions tab.");
            });
            if before != self.cfg.feedback {
                self.changed();
            }
            });
            });
        });
        if let Some(id) = pick {
            self.styles.select(&id);
        }
        if create {
            self.style_msg = Some(match self.styles.create_template() {
                Ok(p) => format!("Created {}. Edit it and save; the window follows.", p.display()),
                Err(e) => format!("Could not create the template: {e}"),
            });
        }
        self.show_settings = open;
    }

    /// Keyboard shortcuts (not while typing in a field).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let (undo, redo, copy, paste, dup, del, dx, dy) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                (cmd && i.key_pressed(egui::Key::Y)) || (cmd && i.modifiers.shift && i.key_pressed(egui::Key::Z)),
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
                cmd && i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::ArrowRight) as i32 - i.key_pressed(egui::Key::ArrowLeft) as i32,
                i.key_pressed(egui::Key::ArrowDown) as i32 - i.key_pressed(egui::Key::ArrowUp) as i32,
            )
        });
        if undo {
            self.do_undo();
        }
        if redo {
            self.do_redo();
        }
        if crate::macro_ui::rail_active(ctx) {
            return; // the macro step list owns arrows, Delete and Ctrl+D while it has focus
        }
        if let Some(cell) = self.selected {
            if copy {
                self.run_cmd(ctx, TileCmd::Copy(cell));
            }
            if paste {
                self.run_cmd(ctx, TileCmd::Paste(cell));
            }
            if dup {
                self.run_cmd(ctx, TileCmd::Duplicate(cell));
            }
            if del {
                self.run_cmd(ctx, TileCmd::Clear(cell));
            }
            if dx != 0 || dy != 0 {
                if let Some(n) = norm_cell(cell.0 as i32 + dx, cell.1 as i32 + dy) {
                    self.selected = Some(n);
                }
            }
        }
    }

    /// Rim color override row: a swatch showing the effective color, "reset" when overridden.
    fn rim_row(&self, ui: &mut egui::Ui, label: &str, field: &mut Option<String>, global: String, fallback: [u8; 3]) {
        ui.horizontal(|ui| {
            let mut c = field.as_deref().and_then(parse_rgb).or_else(|| parse_rgb(&global)).unwrap_or(fallback);
            if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
                *field = Some(hex(c));
            }
            ui.label(label);
            if field.is_some() {
                if ui.small_button("reset").on_hover_text("use the default from Settings").clicked() {
                    *field = None;
                }
            } else {
                ui.weak("(default)");
            }
        });
    }

    fn run_cmd(&mut self, ctx: &egui::Context, cmd: TileCmd) {
        match cmd {
            TileCmd::Copy(c) => self.copy_button(ctx, c),
            TileCmd::Paste(c) => self.paste_button(c),
            TileCmd::Duplicate(c) => {
                if let (Some(b), Some(to)) = (self.cfg.button_at(self.page, c.0, c.1).cloned(), self.first_empty_cell()) {
                    self.clipboard = Some(b);
                    self.paste_button(to);
                    self.toast("Duplicated");
                } else {
                    self.toast("No free key on this page");
                }
            }
            TileCmd::Clear(c) => {
                self.cfg.clear_button(self.page, c.0, c.1);
                self.changed();
            }
        }
    }

    /// The picture of a tile as the deck will show it (cached; live faces and GIFs get a new one whenever they change).
    fn preview_texture(&mut self, ctx: &egui::Context, b: &Button, wide: bool, alt: bool) -> Option<egui::TextureHandle> {
        let mut live = layout::Live::snapshot();
        let gif = b.icon.as_deref().filter(|p| layout::is_gif(p) && !b.face.is_live());
        let mut stamp = if b.face.is_live() { live.now.timestamp() } else { 0 };
        if let (Some(p), true) = (gif, self.cfg.animate_gifs) {
            live.tick_ms = Self::anim_ms(ctx);
            if let Some((idx, left)) = layout::gif_index_at(&self.cfg.base_dir.join(p), live.tick_ms) {
                stamp = idx as i64 + 1;
                ctx.request_repaint_after(Duration::from_millis(left.clamp(16, 1000)));
            }
        }
        let key = format!("{}|{wide}|{alt}|{stamp}", serde_json::to_string(b).unwrap_or_default());
        if !self.previews.contains_key(&key) {
            if self.previews.len() > 48 {
                self.previews.clear();
            }
            if let Ok(png) = layout::preview_png(&self.cfg, b, wide, alt, &live) {
                if let Ok(img) = image::load_from_memory(&png) {
                    let rgba = img.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let ci = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    let tex = ctx.load_texture(format!("preview{}", self.previews.len()), ci, egui::TextureOptions::LINEAR);
                    self.previews.insert(key.clone(), tex);
                }
            }
        }
        self.previews.get(&key).cloned()
    }

    /// The tile as the deck will show it, rendered by the same code that builds the deck's pictures.
    fn preview_tile(&mut self, ui: &mut egui::Ui, b: &Button, wide: bool, alt: bool, width: f32) {
        let height = if wide { width * 196.0 / 458.0 } else { width };
        let tex = self.preview_texture(ui.ctx(), b, wide, alt);
        let th = &self.styles.theme;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        match tex {
            Some(t) => {
                rounded_image(ui.painter(), t.id(), rect, if self.cfg.round_icons { th.tile_rounding } else { 0.0 }, Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(rect, th.tile_rounding, th.tile_empty);
                let msg = if b.face.is_firmware() { "drawn by\nthe deck" } else { "no preview" };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(11.0), th.text_dim);
            }
        }
        ui.painter().rect_stroke(rect, th.tile_rounding, Stroke::new(1.0, th.tile_border), StrokeKind::Inside);
    }

    /// A soft surface for a group of controls.
    fn card(&self) -> egui::Frame {
        let th = &self.styles.theme;
        egui::Frame::new()
            .fill(th.card.gamma_multiply(0.45))
            .stroke(Stroke::new(th.border_width, th.border))
            .corner_radius(egui::CornerRadius::same(th.card_rounding))
            .inner_margin(egui::Margin::same(12))
    }

    /// One strip of sections: Look, one per gesture, State. A dot marks the ones that are set up.
    fn tab_strip(&mut self, ui: &mut egui::Ui, on: [bool; 7]) {
        let th = self.styles.theme.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, (t, name)) in [(Tab::Look, "Look"), (Tab::Tap, "Tap"), (Tab::Double, "Double tap"), (Tab::Hold, "Hold"), (Tab::Repeat, "While held"), (Tab::TapHold, "Tap, then hold"), (Tab::State, "State")].into_iter().enumerate() {
                let sel = self.tab == t;
                let label = if on[i] { format!("{name}  •") } else { name.to_string() };
                let text = egui::RichText::new(label).strong().color(if sel { th.accent_text } else if on[i] { th.text } else { th.text_dim });
                let btn = egui::Button::new(text).fill(if sel { th.accent } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).corner_radius(egui::CornerRadius::same(255)).min_size(Vec2::new(0.0, 28.0));
                if ui.add(btn).clicked() {
                    self.tab = t;
                }
            }
        });
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let Some((col, row)) = self.selected else {
            ui.label("Click a button on the deck to edit it.");
            return;
        };
        let page = self.page;
        let mut b: Button = self.cfg.button_at(page, col, row).cloned().unwrap_or(Button { page, pos: format!("{col},{row}"), ..Button::default() });
        // compare the whole button as JSON so new fields are never forgotten here
        let snapshot = |b: &Button| serde_json::to_string(b).unwrap_or_default();
        let before = snapshot(&b);
        let tab = self.tab;
        let wide_slot = (col, row) == (3, 2);

        // header: the real tile (rendered by the same code that feeds the deck) and what to do with it
        let mut header_cmd: Option<TileCmd> = None;
        let mut detach = false;
        let on = [false, b.action.is_some(), b.double.is_some(), !b.holds.is_empty(), b.repeat.is_some(), b.tap_hold.is_some(), b.alt.is_some()];
        let th = self.styles.theme.clone();
        ui.vertical(|ui| {
            // the button itself and what to do with it
            ui.horizontal(|ui| {
                let w = if wide_slot { 118.0 } else { 84.0 };
                self.preview_tile(ui, &b, wide_slot, false, w);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(if wide_slot { "Wide button".to_string() } else { format!("Button {col},{row}") }).strong().size(th.heading_size));
                    ui.label(egui::RichText::new(format!("page {page} · #{}", row * COLS + col)).size(th.small_size).color(th.text_dim));
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button("Copy").on_hover_text("Ctrl+C").clicked() {
                            header_cmd = Some(TileCmd::Copy((col, row)));
                        }
                        if ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste").small()).on_hover_text("Ctrl+V").clicked() {
                            header_cmd = Some(TileCmd::Paste((col, row)));
                        }
                        if ui.small_button("Clear").on_hover_text("Delete").clicked() {
                            header_cmd = Some(TileCmd::Clear((col, row)));
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut b.pinned, "📌 Pin to all pages").on_hover_text("Shows this key at the same spot on every page. A page's own key there still wins.");
                        if b.pinned && b.page != page {
                            ui.label(egui::RichText::new(format!("(from page {})", b.page)).size(th.small_size).color(th.text_dim));
                            if ui.small_button("Different here").on_hover_text("Give this page its own key instead of the pinned one").clicked() {
                                detach = true;
                            }
                        }
                    });
                });
            });
            ui.add_space(8.0);
            // one section at a time
            ui.vertical(|ui| {
                self.tab_strip(ui, on);
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(820.0));

        // what the button shows: its picture, a live clock / stats drawn by deckd, or (wide slot only) the firmware's own display
        if tab == Tab::Look {
            let wide = wide_slot;
            let name = |f: Face| match f {
                Face::Image => "Picture / symbol / GIF",
                Face::LiveClock => "Live clock (drawn by deckd)",
                Face::LiveStats => "Live CPU / RAM stats (drawn by deckd)",
                Face::LiveWeather => "Weather (drawn by deckd)",
                Face::ClockWeather => "Clock + weather, alternating",
                Face::Everything => "Time + date + weather, all in one",
                Face::Clock => "Firmware clock",
                Face::Stats => "Firmware stats",
            };
            ui.horizontal(|ui| {
                ui.label("Shows:");
                egui::ComboBox::from_id_salt("face").selected_text(name(b.face)).width(240.0).show_ui(ui, |ui| {
                    for f in [Face::Image, Face::LiveClock, Face::Everything, Face::ClockWeather, Face::LiveWeather, Face::LiveStats, Face::Clock, Face::Stats] {
                        if f.is_firmware() && !wide {
                            continue;
                        }
                        ui.selectable_value(&mut b.face, f, name(f));
                    }
                });
            });
            if b.face.is_firmware() {
                ui.weak("The deck draws this itself: no picture, no rim. The look settings below are ignored.");
            }
            if b.face.is_live() {
                ui.horizontal(|ui| {
                    ui.label("Redraw every");
                    let mut ms = b.refresh_ms as f64;
                    if ui.add(egui::Slider::new(&mut ms, 200.0..=60000.0).logarithmic(true).suffix(" ms")).changed() {
                        b.refresh_ms = ms as u16;
                    }
                });
            }
            if matches!(b.face, Face::LiveWeather | Face::ClockWeather | Face::Everything) {
                if self.cfg.weather_city.trim().is_empty() {
                    ui.colored_label(th.warn, "Set your city in Settings (the gear, top right) so the weather knows where to look.");
                } else {
                    ui.weak(format!("Weather for {} (change it in Settings).", self.cfg.weather_city));
                }
            }
            if matches!(b.face, Face::LiveClock | Face::ClockWeather | Face::Everything) {
                ui.horizontal_wrapped(|ui| {
                    let mut f = b.clock_format.clone().unwrap_or_default();
                    ui.add(egui::TextEdit::singleline(&mut f).desired_width(120.0).hint_text("%H:%M"));
                    b.clock_format = (!f.is_empty()).then_some(f);
                    for (label, fmt) in [("24h", "%H:%M"), ("24h + s", "%H:%M:%S"), ("12h", "%I:%M %p"), ("time + date", "%H:%M\n%a %d %b")] {
                        if ui.small_button(label).clicked() {
                            b.clock_format = Some(fmt.to_string());
                        }
                    }
                });
                ui.weak("Format codes: %H hour, %M minute, %S second, %a weekday, %d day, %b month. Colors: background / symbol below.");
            }
            if b.face == Face::LiveStats {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Style");
                    let mut s = b.stats_style.clone().unwrap_or_else(|| "bars".into());
                    for st in ["bars", "line", "text"] {
                        ui.selectable_value(&mut s, st.to_string(), st);
                    }
                    b.stats_style = (s != "bars").then_some(s);
                });
            }
            ui.add_space(4.0);
        }

        // ---- look ----
        if tab == Tab::Look {
        self.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut label = b.label.clone().unwrap_or_default();
            let w = (ui.available_width() - 16.0).max(60.0);
            ui.add(egui::TextEdit::singleline(&mut label).desired_width(w).hint_text("Label under the icon"));
            b.label = (!label.is_empty()).then_some(label);
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let mut rgb = b.color.as_deref().and_then(parse_rgb).unwrap_or([0x20, 0x20, 0x20]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                    b.color = Some(hex(rgb));
                }
                ui.label("background");
                if b.color.is_some() && ui.small_button("reset").clicked() {
                    b.color = None;
                }
                ui.add_space(12.0);
                let mut gc = b.glyph_color.as_deref().and_then(parse_rgb).unwrap_or([255, 255, 255]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut gc).changed() {
                    b.glyph_color = Some(hex(gc));
                }
                ui.label("symbol");
            });
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let mut glyph = b.glyph.clone().unwrap_or_default();
                ui.add(egui::TextEdit::singleline(&mut glyph).desired_width(70.0).hint_text("symbol"));
                b.glyph = (!glyph.is_empty()).then_some(glyph);
                ui.menu_button("Pick symbol ▾", |ui| {
                    ui.set_max_width(250.0);
                    ui.horizontal_wrapped(|ui| {
                        for g in QUICK_GLYPHS {
                            if ui.add(egui::Button::new(egui::RichText::new(*g).size(18.0)).min_size(Vec2::new(32.0, 30.0))).clicked() {
                                b.glyph = Some((*g).to_string());
                                ui.close();
                            }
                        }
                    });
                });
                if b.glyph.is_some() && ui.small_button("✖").on_hover_text("remove symbol").clicked() {
                    b.glyph = None;
                }
            });
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let mut icon = b.icon.clone().unwrap_or_default();
                let w = (ui.available_width() - 110.0).max(60.0);
                ui.add(egui::TextEdit::singleline(&mut icon).desired_width(w).hint_text("or an image file"));
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "ico"]).pick_file() {
                        icon = p.display().to_string();
                    }
                }
                b.icon = (!icon.is_empty()).then_some(icon);
            });
        });
        }

        // ---- alternate look (state) ----
        if tab == Tab::State {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.strong("Alternate look");
            ui.weak("For toggles like mute: while the state is on, the button shows this look instead of the normal one.");
            if b.alt.is_some() {
                ui.horizontal(|ui| {
                    ui.weak("Normal");
                    self.preview_tile(ui, &b, wide_slot, false, 64.0);
                    ui.weak("  When on");
                    self.preview_tile(ui, &b, wide_slot, true, 64.0);
                });
            }
            ui.horizontal(|ui| {
                ui.label("State from:");
                let name = |s: Option<StateSource>| match s {
                    None | Some(StateSource::Toggle) => "Tap toggles it",
                    Some(StateSource::OutputMuted) => "Speaker is muted (real state)",
                    Some(StateSource::MicMuted) => "Microphone is muted (real state)",
                };
                egui::ComboBox::from_id_salt("state").selected_text(name(b.state)).width(230.0).show_ui(ui, |ui| {
                    for s in [StateSource::Toggle, StateSource::OutputMuted, StateSource::MicMuted] {
                        ui.selectable_value(&mut b.state, Some(s), name(Some(s)));
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Presets:");
                if ui.small_button("Mic mute").on_hover_text("tap flips the microphone mute; shows green when open, red when muted").clicked() {
                    b.action = Some(Action::Media { key: "mic_mute".into() });
                    b.state = Some(StateSource::MicMuted);
                    b.label = Some("Mic ON".into());
                    b.color = Some("#1f6f3a".into());
                    b.alt = Some(Alt { label: Some("Mic MUTED".into()), color: Some("#b62324".into()), ..Alt::default() });
                }
                if ui.small_button("Speaker mute").on_hover_text("tap flips the speaker mute; shows green when sound is on, red when muted").clicked() {
                    b.action = Some(Action::Media { key: "mute".into() });
                    b.state = Some(StateSource::OutputMuted);
                    b.label = Some("Sound ON".into());
                    b.color = Some("#1f6f3a".into());
                    b.alt = Some(Alt { label: Some("Sound MUTED".into()), color: Some("#b62324".into()), ..Alt::default() });
                }
            });
            ui.add_space(4.0);
            let mut a = b.alt.clone().unwrap_or_default();
            let w = (ui.available_width() - 16.0).max(60.0);
            let mut label = a.label.clone().unwrap_or_default();
            ui.add(egui::TextEdit::singleline(&mut label).desired_width(w).hint_text("Label while on (empty = keep)"));
            a.label = (!label.is_empty()).then_some(label);
            ui.horizontal(|ui| {
                let mut rgb = a.color.as_deref().and_then(parse_rgb).or_else(|| b.color.as_deref().and_then(parse_rgb)).unwrap_or([0x20, 0x20, 0x20]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                    a.color = Some(hex(rgb));
                }
                ui.label("background");
                if a.color.is_some() && ui.small_button("reset").clicked() {
                    a.color = None;
                }
                ui.add_space(12.0);
                let mut gc = a.glyph_color.as_deref().and_then(parse_rgb).or_else(|| b.glyph_color.as_deref().and_then(parse_rgb)).unwrap_or([255, 255, 255]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut gc).changed() {
                    a.glyph_color = Some(hex(gc));
                }
                ui.label("symbol");
            });
            ui.horizontal(|ui| {
                let mut glyph = a.glyph.clone().unwrap_or_default();
                ui.add(egui::TextEdit::singleline(&mut glyph).desired_width(70.0).hint_text("symbol"));
                a.glyph = (!glyph.is_empty()).then_some(glyph);
                let mut icon = a.icon.clone().unwrap_or_default();
                let w = (ui.available_width() - 110.0).max(60.0);
                ui.add(egui::TextEdit::singleline(&mut icon).desired_width(w).hint_text("or an image file"));
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "ico"]).pick_file() {
                        icon = p.display().to_string();
                    }
                }
                a.icon = (!icon.is_empty()).then_some(icon);
            });
            if b.alt.is_some() && ui.small_button("Remove the alternate look").clicked() {
                a = Alt::default();
                b.state = None;
            }
            b.alt = (!a.is_empty()).then_some(a);
        });
        }

        // ---- actions ----
        {
        if tab == Tab::Tap {
        self.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.action_editor(ui, "press", &mut b.action);
            ui.add_space(4.0);
            self.rim_row(ui, "Tap rim", &mut b.rim_tap, self.cfg.feedback.tap.clone(), [0x2f, 0x81, 0xf7]);
        });
        }

        if tab == Tab::Double { ui.vertical(|ui| {
            ui.weak("Runs instead of the normal action when you tap twice quickly. The normal action then waits ~0.35 s.");
            self.action_editor(ui, "double", &mut b.double);
            ui.add_space(4.0);
            self.rim_row(ui, "Double-tap rim", &mut b.rim_double, self.cfg.feedback.double.clone(), [0x3f, 0xb9, 0x50]);
        }); }
        if tab == Tab::Hold { ui.vertical(|ui| {
            ui.weak("Fires when you let go. Add as many steps as you like, each with its own duration; the steps with the longest duration you reached run. Steps sharing a duration are chained into one macro (in order).");
            let counts: Vec<usize> = b.holds.iter().map(|h| b.holds.iter().filter(|o| o.ms == h.ms).count()).collect();
            let (mut remove, mut chain) = (None, None);
            for (n, h) in b.holds.iter_mut().enumerate() {
                self.card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut h.ms, 100..=10000).logarithmic(true).suffix(" ms").text("held at least"));
                        if ui.small_button("🗑").on_hover_text("Remove this step").clicked() {
                            remove = Some(n);
                        }
                    });
                    if counts[n] > 1 {
                        ui.weak(format!("⛓ chained with {} other step(s) at {} ms, in list order", counts[n] - 1, h.ms));
                        ui.checkbox(&mut h.split, "Hard split: run separately (in parallel), not chained after the step above");
                    }
                    ui.horizontal(|ui| {
                        let mut own = h.color.is_some();
                        if ui.checkbox(&mut own, "own rim color while holding").on_hover_text("Otherwise the color shifts automatically for each hold step").changed() {
                            h.color = own.then(|| "#f0883e".to_string());
                        }
                        if let Some(c) = &mut h.color {
                            let mut rgb = parse_rgb(c).unwrap_or([255, 255, 255]);
                            if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                                *c = hex(rgb);
                            }
                        }
                    });
                    let mut act = Some(h.action.clone());
                    self.action_editor(ui, &format!("hold{n}"), &mut act);
                    match act {
                        Some(a) => h.action = a,
                        None => remove = Some(n), // "Nothing" = no step
                    }
                    if ui.small_button("＋ chain another step at this duration").clicked() {
                        chain = Some((n, h.ms));
                    }
                });
            }
            if let Some(n) = remove {
                b.holds.remove(n);
            } else if let Some((n, ms)) = chain {
                b.holds.insert(n + 1, crate::config::HoldStep { ms, action: Action::Print { text: "hold step".into() }, color: None, split: false });
            }
            if ui.button("＋ Add hold step").clicked() {
                let ms = b.holds.iter().map(|h| h.ms).max().map_or(500, |m| (m + 500).min(10000));
                b.holds.push(crate::config::HoldStep { ms, action: Action::Print { text: "hold step".into() }, color: None, split: false });
            }
            ui.weak("Drag a slider, or click its number and type an exact value.");
        }); }
        if tab == Tab::Repeat { ui.vertical(|ui| {
            ui.weak("Keeps running while you hold the button, until you let go. Great for volume up/down. A quick tap still runs the normal action once.");
            self.action_editor(ui, "repeat", &mut b.repeat);
            if b.repeat.is_some() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut b.repeat_ms, 30..=1000).suffix(" ms").text("every"));
                });
            }
            if b.repeat.is_none() && b.action.as_ref().is_some_and(|a| !matches!(a, Action::Page { .. })) && ui.button("Repeat the normal action while held").clicked() {
                b.repeat = b.action.clone();
            }
        }); }
        if tab == Tab::TapHold { ui.vertical(|ui| {
            ui.weak("Tap, then press again and keep holding: this repeats (e.g. volume down while Hold = volume up). The first tap does not fire in that case. Uses the same 'every' speed as 'While held'.");
            self.action_editor(ui, "tap_hold", &mut b.tap_hold);
            if b.tap_hold.is_some() || b.double.is_some() {
                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut b.window_ms, 150..=1000).suffix(" ms").text("second-press window"));
                ui.weak("A tap waits this long for a second press before it fires. Shorter = snappier taps, but harder double taps.");
            }
        }); }
        }
                });
            });
        });
        if let Some(c) = header_cmd {
            self.run_cmd(ui.ctx(), c);
            return;
        }
        if detach {
            // this page gets its own copy (not pinned); the pinned key stays on the other pages
            let mut own = b.clone();
            own.page = page;
            own.pinned = false;
            self.cfg.buttons.push(own);
            self.changed();
            return;
        }

        if before != snapshot(&b) {
            let empty = b.label.is_none() && b.color.is_none() && b.glyph.is_none() && b.icon.is_none() && b.face.is_image() && b.alt.is_none() && b.action.is_none() && b.double.is_none() && b.holds.is_empty() && b.repeat.is_none() && b.tap_hold.is_none();
            if empty {
                self.cfg.clear_button(page, col, row);
            } else {
                *self.cfg.button_mut(page, col, row) = b;
            }
            self.changed();
        }
    }
}

/// egui key -> the name our hotkey parser understands.
fn key_name(k: egui::Key) -> String {
    let n = k.name();
    match n {
        "Escape" => "esc".into(),
        "ArrowLeft" => "left".into(),
        "ArrowRight" => "right".into(),
        "ArrowUp" => "up".into(),
        "ArrowDown" => "down".into(),
        _ => match n.strip_prefix("Num") {
            Some(d) if d.len() == 1 => d.to_string(),
            _ => n.to_ascii_lowercase(),
        },
    }
}

/// Draw a texture clipped to a rounded rectangle (a triangle fan with the texture mapped over it).
fn rounded_image(p: &egui::Painter, tex: egui::TextureId, rect: Rect, radius: f32, tint: Color32) {
    if radius <= 0.5 {
        p.image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
        return;
    }
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let mut pts: Vec<Pos2> = Vec::new();
    let corners = [
        (Pos2::new(rect.max.x - r, rect.min.y + r), -std::f32::consts::FRAC_PI_2),
        (Pos2::new(rect.max.x - r, rect.max.y - r), 0.0),
        (Pos2::new(rect.min.x + r, rect.max.y - r), std::f32::consts::FRAC_PI_2),
        (Pos2::new(rect.min.x + r, rect.min.y + r), std::f32::consts::PI),
    ];
    for (c, start) in corners {
        for i in 0..=8 {
            let a = start + std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
            pts.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
    let uv = |q: Pos2| Pos2::new((q.x - rect.min.x) / rect.width(), (q.y - rect.min.y) / rect.height());
    let mut mesh = egui::Mesh::with_texture(tex);
    mesh.vertices.push(egui::epaint::Vertex { pos: rect.center(), uv: uv(rect.center()), color: tint });
    for q in &pts {
        mesh.vertices.push(egui::epaint::Vertex { pos: *q, uv: uv(*q), color: tint });
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.indices.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % n]);
    }
    p.add(egui::Shape::mesh(mesh));
}

fn paint_tile(p: &egui::Painter, rect: Rect, b: Option<&Button>, tex: Option<&egui::TextureHandle>, app: Option<&egui::TextureHandle>, a: f32, rounding: f32, empty: Color32, round_img: bool) {
    let plate = app.zip(b).filter(|(_, b)| b.color.is_none()).and_then(|(_, b)| crate::appicon::image_for(b)).and_then(|i| crate::appicon::plate_color(&i)).map(c32);
    let bg = b.and_then(|b| b.color.as_deref()).and_then(parse_rgb).map(c32).or(plate).unwrap_or(empty);
    p.rect_filled(rect, rounding, bg.gamma_multiply(a));
    if let Some(t) = tex {
        rounded_image(p, t.id(), rect, if round_img { rounding } else { 0.0 }, Color32::WHITE.gamma_multiply(a));
    }
    let label = b.and_then(|b| b.label.as_deref()).filter(|l| !l.trim().is_empty());
    let gc = b.and_then(|b| b.glyph_color.as_deref()).and_then(parse_rgb).unwrap_or([255, 255, 255]);
    let gcol = Color32::from_rgb(gc[0], gc[1], gc[2]).gamma_multiply(a);
    if let Some(g) = b.and_then(|b| b.glyph.as_deref()) {
        let up = if label.is_some() { 8.0 } else { 0.0 };
        p.text(rect.center() - Vec2::new(0.0, up), Align2::CENTER_CENTER, g, FontId::proportional(44.0), gcol);
    } else if let (Some(app), None) = (app, tex) {
        let side = rect.height() * if label.is_some() { 0.6 } else { 0.76 };
        let cy = if label.is_some() { rect.center().y - rect.height() * 0.04 } else { rect.center().y };
        let r = Rect::from_center_size(Pos2::new(rect.center().x, cy), Vec2::splat(side));
        p.image(app.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(a));
    } else if let (Some(l), None) = (label, tex) {
        if b.is_some_and(|b| b.face.is_image()) {
            // text-only button: the label is centered in the tile (deckd draws it into the picture)
            p.text(rect.center(), Align2::CENTER_CENTER, l, FontId::proportional(rect.height() * 0.24), gcol);
            return;
        }
    }
    if let Some(l) = label {
        // black outline so white text stays readable on light tile colors
        let at = rect.center_bottom() - Vec2::new(0.0, 6.0);
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            p.text(at + Vec2::new(dx, dy), Align2::CENTER_BOTTOM, l, FontId::proportional(12.0), Color32::BLACK.gamma_multiply(a));
        }
        p.text(at, Align2::CENTER_BOTTOM, l, FontId::proportional(12.0), Color32::WHITE.gamma_multiply(a));
    }
}

impl eframe::App for App {
    /// Closing the window must not cut a transfer to the deck in half (that wedges it until the cable is replugged).
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.svc.shutdown(std::time::Duration::from_secs(4));
        crate::tray::remove();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        use std::sync::atomic::Ordering::Relaxed;
        let tray_ok = crate::tray::AVAILABLE.load(Relaxed);
        if START_MINIMIZED.swap(false, Relaxed) {
            // started by "start with Windows": go straight to the tray (or the taskbar if there is no tray icon)
            ctx.send_viewport_cmd(if tray_ok { egui::ViewportCommand::Visible(false) } else { egui::ViewportCommand::Minimized(true) });
        }
        if crate::tray::SHOW_REQUEST.swap(false, Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if tray_ok && !crate::tray::QUITTING.load(Relaxed) && ctx.input(|i| i.viewport().close_requested()) {
            // the X button hides to the tray; Quit in the tray menu really exits
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.try_save();
        }
        self.styles.poll();
        if std::mem::take(&mut self.styles.changed) {
            self.apply_style(&ctx);
        }
        self.studio_top(ui);
        let mut cmds: Vec<TileCmd> = Vec::new();
        self.studio_body(ui, &mut cmds);
        for c in cmds {
            self.run_cmd(&ctx, c);
        }

        self.settings_window(&ctx);
        self.macro_window(&ctx);
        self.shortcuts(&ctx);

        if let Some((msg, t)) = self.toast.clone() {
            if t.elapsed() < Duration::from_millis(1500) {
                egui::Area::new(egui::Id::new("toast")).anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -48.0)).order(egui::Order::Tooltip).show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(msg);
                    });
                });
                ctx.request_repaint_after(Duration::from_millis(200));
            } else {
                self.toast = None;
            }
        }

        if let Some(t) = self.dirty_since {
            if t.elapsed() >= SAVE_DELAY {
                self.try_save();
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(500)); // keeps the status bar and press flash fresh
    }
}

/// Window fonts: the style's own font first (if any), the symbol font as fallback for glyphs.
fn install_fonts(ctx: &egui::Context, symbols: &str, style_font: Option<&std::path::Path>) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(bytes) = style_font.and_then(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("style_font".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().insert(0, "style_font".to_owned());
        }
    }
    if let Ok(bytes) = std::fs::read(symbols) {
        fonts.font_data.insert("symbols".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("symbols".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

/// Set by main when started with `--minimized`.
pub static START_MINIMIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn reg(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg.exe").args(args).creation_flags(0x0800_0000).output().is_ok_and(|o| o.status.success())
}

/// The command the sign-in entry must hold: this exe, started hidden.
fn autostart_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" gui --minimized", exe.display()))
}

/// Is deckd registered to start when you sign in to Windows?
fn autostart_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", "deckd"])
}

/// Rewrite a sign-in entry that exists but does not match this exe (an older build wrote a `conhost.exe --headless ...` wrapper,
/// and the exe may have moved), so "Launch on startup" keeps working without toggling it off and on.
fn heal_autostart() {
    use std::os::windows::process::CommandExt;
    let Some(want) = autostart_command() else { return };
    let Ok(out) = std::process::Command::new("reg.exe").args(["query", RUN_KEY, "/v", "deckd"]).creation_flags(0x0800_0000).output() else { return };
    if !out.status.success() {
        return; // not enabled: leave it off
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let current = text.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()));
    if current.as_deref() != Some(want.as_str()) {
        set_autostart(true);
    }
}

/// Register / remove the sign-in entry (current user only, no admin needed). It starts the window minimized. The release
/// build is a GUI program, so no console window appears.
fn set_autostart(on: bool) -> bool {
    if !on {
        return reg(&["delete", RUN_KEY, "/v", "deckd", "/f"]);
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let cmd = format!("\"{}\" gui --minimized", exe.display());
    reg(&["add", RUN_KEY, "/v", "deckd", "/t", "REG_SZ", "/d", &cmd, "/f"])
}

pub fn run(path: PathBuf) -> Result<(), String> {
    if !path.exists() {
        std::fs::write(&path, SAMPLE).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    }
    if !crate::tray::single_instance() {
        return Ok(()); // another deckd window is running; it was asked to come forward
    }
    heal_autostart();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("deckd")
            .with_inner_size([1360.0, 840.0])
            .with_min_inner_size([1100.0, 640.0])
            .with_icon(egui::IconData { rgba: crate::tray::icon_rgba(64), width: 64, height: 64 })
            .with_visible(!START_MINIMIZED.load(std::sync::atomic::Ordering::Relaxed)),
        ..Default::default()
    };
    eframe::run_native(
        "deckd",
        options,
        Box::new(move |cc| {
            let (cfg, load_error) = match Config::load(&path) {
                Ok(c) => (c, None),
                Err(e) => (Config::default(), Some(e)),
            };
            let styles = Styles::new(&path);
            let committed = cfg.clone();
            let fonts_for = (String::new(), None); // forces the first apply_style to install fonts
            crate::tray::start(cc.egui_ctx.clone());
            let repaint = cc.egui_ctx.clone();
            let svc = service::Ctx::new(false, Arc::new(move || repaint.request_repaint()));
            let (svc2, path2) = (svc.clone(), path.clone());
            std::thread::spawn(move || service::run(path2, svc2));
            Ok(Box::new(App {
                path,
                cfg,
                selected: Some((0, 0)),
                svc,
                dirty_since: None,
                save_error: None,
                load_error,
                textures: HashMap::new(),
                show_log: false,
                saved_at: None,
                test_msg: None,
                drag: None,
                recording: None,
                drag_hover: None,
                drag_page: 1,
                tab_rects: Vec::new(),
                tab_hover: None,
                page: 1,
                styles,
                tab: Tab::Look,
                undo: Vec::new(),
                redo: Vec::new(),
                committed,
                restoring: false,
                clipboard: None,
                show_settings: false,
                fit: 0.0,
                lib_query: String::new(),
                lib_pin: false,
                autostart: autostart_enabled(),
                macro_pop: None,
                in_macro_window: false,                lib_cat: "All".to_string(),
                previews: HashMap::new(),
                bg_tex: None,
                toast: None,
                style_msg: None,
                fonts_for,
            }))
        }),
    )
    .map_err(|e| e.to_string())
}

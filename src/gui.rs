//! The deckd window: a clickable copy of the deck, an editor for the selected button, and live status.
//! Edits are saved to deck.toml (debounced); the service thread notices the file change and re-uploads.

use crate::config::{Action, Alt, Button, Config, Face, StateSource, COLS, ROWS, SAMPLE};
use crate::theme::{Styles, TabStyle};
use crate::{actions, layout, service};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    Actions,
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
        (t.tile_size, t.tile_gap)
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

    fn texture(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
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
        }
        let pointer = ui.ctx().pointer_interact_pos();
        if resp.drag_stopped() {
            self.drag = None;
            // drop onto another tile = swap the two buttons
            if let Some(p) = pointer {
                let origin = rect.min - Vec2::new(col as f32 * (tile + gap), row as f32 * (tile + gap));
                let (tc, tr) = (((p.x - origin.x) / (tile + gap)).floor() as i32, ((p.y - origin.y) / (tile + gap)).floor() as i32);
                if let Some(t) = norm_cell(tc, tr) {
                    self.swap_cells((col, row), t);
                }
            }
        }

        // While dragging over another tile, preview the swap: the two tiles show each other's button.
        let page = self.page;
        let swapping = match (self.drag, self.drag_hover) {
            (Some(s), Some(h)) if s != h => Some((s, h)),
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
        let is_source = self.drag == Some((col, row));
        let alpha = if swapping.is_some() && shown != (col, row) {
            0.75 // phased preview of where the other button would land
        } else if is_source {
            0.3
        } else {
            1.0
        };
        paint_tile(ui.painter(), rect, b.as_ref(), tex.as_ref(), alpha, th.tile_rounding, th.tile_empty);
        let caption = match b.as_ref().map(|b| b.face) {
            None if (col, row) == (3, 2) => Some("wide slot: firmware clock\n(click to set up a button)"),
            Some(Face::Clock) => Some("firmware clock"),
            Some(Face::Stats) => Some("firmware stats"),
            Some(Face::LiveClock) => Some("live clock"),
            Some(Face::LiveStats) => Some("live stats"),
            _ => None,
        };
        if let Some(c) = caption {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, c, FontId::proportional(12.0), th.text_dim);
        }

        let is_target = swapping.is_some_and(|(_, h)| h == (col, row) || (is_source && shown != (col, row)));
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

        // translucent ghost that follows the pointer while dragging
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some(p) = pointer {
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
                let painter = ui.ctx().layer_painter(layer);
                let r = Rect::from_center_size(p, Vec2::new(tile_width(col, row, tile, gap), tile));
                paint_tile(&painter, r, own.as_ref(), own_tex.as_ref(), 0.6, th.tile_rounding, th.tile_empty);
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
        ui.horizontal_wrapped(|ui| {
            let kinds: &[(Kind, &str)] = if step {
                &[(Kind::Hotkey, "Hotkey"), (Kind::Text, "Type text"), (Kind::Delay, "Wait"), (Kind::Run, "Program"), (Kind::Open, "Open"), (Kind::Media, "Media"), (Kind::Print, "Print")]
            } else {
                &[(Kind::None, "Nothing"), (Kind::Run, "Program"), (Kind::Open, "Open"), (Kind::Hotkey, "Hotkey"), (Kind::Media, "Media"), (Kind::Text, "Type text"), (Kind::Macro, "Macro"), (Kind::Page, "Page"), (Kind::Print, "Print")]
            };
            for (k, name) in kinds {
                ui.selectable_value(&mut kind, *k, *name);
            }
        });
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
            Some(Action::Text { text }) => {
                let w = room(ui, 16.0);
                ui.add(egui::TextEdit::multiline(text).desired_rows(3).desired_width(w).hint_text("text to type"));
                ui.weak(format!("{} characters, typed into whichever window has focus. A line break presses Enter.", text.chars().count()));
            }
            Some(Action::Macro { steps }) => {
                ui.weak("Runs these steps one after the other. Add a Wait step to pause between two of them.");
                let (mut remove, mut swap) = (None, None);
                let count = steps.len();
                for (n, s) in steps.iter_mut().enumerate() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.strong(format!("Step {}", n + 1));
                            if ui.add_enabled(n > 0, egui::Button::new("↑").small()).on_hover_text("Move up").clicked() {
                                swap = Some((n, n - 1));
                            }
                            if ui.add_enabled(n + 1 < count, egui::Button::new("↓").small()).on_hover_text("Move down").clicked() {
                                swap = Some((n, n + 1));
                            }
                            if ui.small_button("🗑").on_hover_text("Remove this step").clicked() {
                                remove = Some(n);
                            }
                        });
                        let mut act = Some(s.clone());
                        self.action_editor_ex(ui, &format!("{slot}.s{n}"), &mut act, true);
                        if let Some(a) = act {
                            *s = a;
                        }
                    });
                }
                if let Some(n) = remove {
                    steps.remove(n);
                } else if let Some((a, b)) = swap {
                    steps.swap(a, b);
                }
                ui.menu_button("+ Add step", |ui| {
                    for (label, a) in [
                        ("Hotkey", Action::Hotkey { keys: String::new() }),
                        ("Type text", Action::Text { text: String::new() }),
                        ("Wait", Action::Delay { ms: 250 }),
                        ("Program", Action::Run { command: String::new(), args: vec![] }),
                        ("Open", Action::Open { target: String::new() }),
                        ("Media", Action::Media { key: "play_pause".into() }),
                        ("Print", Action::Print { text: "step".into() }),
                    ] {
                        if ui.button(label).clicked() && steps.len() < crate::config::MACRO_MAX_STEPS {
                            steps.push(a);
                            ui.close();
                        }
                    }
                });
                let waits: u32 = steps.iter().map(|s| if let Action::Delay { ms } = s { *ms } else { 0 }).sum();
                ui.weak(format!("{} step(s), {} ms of waiting plus a short gap between steps", steps.len(), waits));
            }
            None => {
                ui.weak("Nothing happens.");
            }
        }
        if let Some(a) = action.clone().filter(|_| !step) {
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
        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("deckd").strong().size(th.heading_size).color(th.accent));
                ui.add_space(6.0);
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

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut pick: Option<String> = None;
        let mut create = false;
        egui::Window::new("Settings").open(&mut open).collapsible(false).default_width(440.0).show(ctx, |ui| {
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

    /// The tile as the deck will show it, rendered by the same code that builds the deck's pictures.
    fn preview_tile(&mut self, ui: &mut egui::Ui, b: &Button, wide: bool, alt: bool, width: f32) {
        let height = if wide { width * 196.0 / 458.0 } else { width };
        let live = layout::Live::snapshot();
        let animated = b.face.is_live() || b.icon.as_deref().is_some_and(layout::is_gif);
        let key = format!("{}|{wide}|{alt}|{}", serde_json::to_string(b).unwrap_or_default(), if animated { live.now.timestamp() } else { 0 });
        if !self.previews.contains_key(&key) {
            if self.previews.len() > 32 {
                self.previews.clear();
            }
            if let Ok(png) = layout::preview_png(&self.cfg, b, wide, alt, &live) {
                if let Ok(img) = image::load_from_memory(&png) {
                    let rgba = img.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let ci = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    let tex = ui.ctx().load_texture(format!("preview{}", self.previews.len()), ci, egui::TextureOptions::LINEAR);
                    self.previews.insert(key.clone(), tex);
                }
            }
        }
        let th = &self.styles.theme;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        match self.previews.get(&key) {
            Some(t) => {
                ui.painter().image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(rect, th.tile_rounding, th.tile_empty);
                let msg = if b.face.is_firmware() { "drawn by\nthe deck" } else { "no preview" };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(11.0), th.text_dim);
            }
        }
        ui.painter().rect_stroke(rect, th.tile_rounding, Stroke::new(1.0, th.tile_border), StrokeKind::Inside);
    }

    /// Look / Actions / State, drawn in the style's tab look.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.styles.theme.clone();
        ui.horizontal(|ui| {
            for (t, name) in [(Tab::Look, "Look"), (Tab::Actions, "Actions"), (Tab::State, "State")] {
                let sel = self.tab == t;
                let clicked = match th.tabs {
                    TabStyle::Pills => {
                        let text = egui::RichText::new(name).color(if sel { th.accent_text } else { th.text });
                        let btn = egui::Button::new(text).fill(if sel { th.accent } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).corner_radius(egui::CornerRadius::same(255));
                        ui.add(btn).clicked()
                    }
                    TabStyle::Underline => {
                        let text = egui::RichText::new(name).color(if sel { th.text } else { th.text_dim }).strong();
                        let r = ui.add(egui::Label::new(text).sense(Sense::click()));
                        if sel {
                            let y = r.rect.bottom() + 2.0;
                            ui.painter().line_segment([Pos2::new(r.rect.left(), y), Pos2::new(r.rect.right(), y)], Stroke::new(2.0, th.accent));
                        }
                        ui.add_space(8.0);
                        r.clicked()
                    }
                    TabStyle::Boxed => ui.selectable_label(sel, name).clicked(),
                };
                if clicked {
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
        ui.horizontal(|ui| {
            let w = if wide_slot { 190.0 } else { 96.0 };
            self.preview_tile(ui, &b, wide_slot, false, w);
            ui.vertical(|ui| {
                ui.heading(if wide_slot { "Wide button".to_string() } else { format!("Button {col},{row}") });
                ui.weak(format!("page {page} · #{}", row * COLS + col));
                ui.horizontal(|ui| {
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
            });
        });
        if let Some(c) = header_cmd {
            self.run_cmd(ui.ctx(), c);
            return;
        }
        ui.add_space(6.0);
        self.tab_bar(ui);
        ui.add_space(4.0);

        // what the button shows: its picture, a live clock / stats drawn by deckd, or (wide slot only) the firmware's own display
        if tab == Tab::Look {
            let wide = wide_slot;
            let name = |f: Face| match f {
                Face::Image => "Picture / symbol / GIF",
                Face::LiveClock => "Live clock (drawn by deckd)",
                Face::LiveStats => "Live CPU / RAM stats (drawn by deckd)",
                Face::Clock => "Firmware clock",
                Face::Stats => "Firmware stats",
            };
            ui.horizontal(|ui| {
                ui.label("Shows:");
                egui::ComboBox::from_id_salt("face").selected_text(name(b.face)).width(240.0).show_ui(ui, |ui| {
                    for f in [Face::Image, Face::LiveClock, Face::LiveStats, Face::Clock, Face::Stats] {
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
            if b.face == Face::LiveClock {
                ui.horizontal(|ui| {
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
                ui.horizontal(|ui| {
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
        egui::Frame::group(ui.style()).show(ui, |ui| {
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
        if tab == Tab::Actions {
        ui.label(egui::RichText::new("WHEN PRESSED").small().weak());
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.action_editor(ui, "press", &mut b.action);
            ui.add_space(4.0);
            self.rim_row(ui, "Tap rim", &mut b.rim_tap, self.cfg.feedback.tap.clone(), [0x2f, 0x81, 0xf7]);
        });

        ui.add_space(6.0);
        egui::CollapsingHeader::new("On double tap").default_open(b.double.is_some()).show(ui, |ui| {
            ui.weak("Runs instead of the normal action when you tap twice quickly. The normal action then waits ~0.35 s.");
            self.action_editor(ui, "double", &mut b.double);
            ui.add_space(4.0);
            self.rim_row(ui, "Double-tap rim", &mut b.rim_double, self.cfg.feedback.double.clone(), [0x3f, 0xb9, 0x50]);
        });
        let title = if b.holds.is_empty() { "On hold (fires on release)".to_string() } else { format!("On hold ({} step{})", b.holds.len(), if b.holds.len() == 1 { "" } else { "s" }) };
        egui::CollapsingHeader::new(title).default_open(!b.holds.is_empty()).show(ui, |ui| {
            ui.weak("Fires when you let go. Add as many steps as you like, each with its own duration; the steps with the longest duration you reached run. Steps sharing a duration are chained into one macro (in order).");
            let counts: Vec<usize> = b.holds.iter().map(|h| b.holds.iter().filter(|o| o.ms == h.ms).count()).collect();
            let (mut remove, mut chain) = (None, None);
            for (n, h) in b.holds.iter_mut().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
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
        });
        egui::CollapsingHeader::new("While held (repeat)").default_open(b.repeat.is_some()).show(ui, |ui| {
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
        });
        egui::CollapsingHeader::new("Tap, then hold (repeat)").default_open(b.tap_hold.is_some()).show(ui, |ui| {
            ui.weak("Tap, then press again and keep holding: this repeats (e.g. volume down while Hold = volume up). The first tap does not fire in that case. Uses the same 'every' speed as 'While held'.");
            self.action_editor(ui, "tap_hold", &mut b.tap_hold);
            if b.tap_hold.is_some() || b.double.is_some() {
                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut b.window_ms, 150..=1000).suffix(" ms").text("second-press window"));
                ui.weak("A tap waits this long for a second press before it fires. Shorter = snappier taps, but harder double taps.");
            }
        });
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

fn paint_tile(p: &egui::Painter, rect: Rect, b: Option<&Button>, tex: Option<&egui::TextureHandle>, a: f32, rounding: f32, empty: Color32) {
    let bg = b.and_then(|b| b.color.as_deref()).and_then(parse_rgb).map(c32).unwrap_or(empty);
    p.rect_filled(rect, rounding, bg.gamma_multiply(a));
    if let Some(t) = tex {
        p.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(a));
    }
    if let Some(g) = b.and_then(|b| b.glyph.as_deref()) {
        let gc = b.and_then(|b| b.glyph_color.as_deref()).and_then(parse_rgb).unwrap_or([255, 255, 255]);
        p.text(rect.center() - Vec2::new(0.0, 8.0), Align2::CENTER_CENTER, g, FontId::proportional(44.0), Color32::from_rgb(gc[0], gc[1], gc[2]).gamma_multiply(a));
    }
    if let Some(l) = b.and_then(|b| b.label.as_deref()).filter(|l| !l.is_empty()) {
        p.text(rect.center_bottom() - Vec2::new(0.0, 6.0), Align2::CENTER_BOTTOM, l, FontId::proportional(12.0), Color32::WHITE.gamma_multiply(a));
    }
}

impl eframe::App for App {
    /// Closing the window must not cut a transfer to the deck in half (that wedges it until the cable is replugged).
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.svc.shutdown(std::time::Duration::from_secs(4));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.styles.poll();
        if std::mem::take(&mut self.styles.changed) {
            self.apply_style(&ctx);
        }
        let th = self.styles.theme.clone();

        self.top_bar(ui);

        egui::Panel::bottom("status").show(ui, |ui| {
            let (connected, info, config_error, last) = {
                let s = self.svc.status.lock().unwrap_or_else(|e| e.into_inner());
                (s.connected, s.info.clone(), s.config_error.clone(), s.log.back().cloned())
            };
            ui.horizontal(|ui| {
                if connected {
                    ui.colored_label(th.ok, "● Deck connected");
                    ui.weak(info.replace('{', "").replace('}', "").replace('"', ""));
                } else {
                    ui.colored_label(th.warn, "○ Waiting for the deck (is Ulanzi Studio closed?)");
                }
            });
            if let Some(e) = self.save_error.as_ref().or(config_error.as_ref()) {
                ui.colored_label(th.error, format!("Not saved / not applied: {e}"));
            } else if self.dirty_since.is_some() {
                ui.weak("Saving…");
            } else if self.saved_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4)) {
                ui.colored_label(th.ok, "✔ Saved, sending to the deck");
            } else if let Some(l) = last {
                ui.weak(l);
            }
            ui.toggle_value(&mut self.show_log, "Log");
            if self.show_log {
                egui::ScrollArea::vertical().max_height(120.0).stick_to_bottom(true).show(ui, |ui| {
                    for l in self.svc.status.lock().unwrap_or_else(|e| e.into_inner()).log.iter() {
                        ui.monospace(l);
                    }
                });
            }
        });

        // Fixed width: a text field wider than the panel used to make the panel grow every frame.
        let inspector = if th.inspector_left { egui::Panel::left("editor") } else { egui::Panel::right("editor") };
        inspector.resizable(false).exact_size(th.inspector_width).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.editor(ui));
        });

        let mut cmds: Vec<TileCmd> = Vec::new();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(th.window).inner_margin(egui::Margin::same(12))).show(ui, |ui| {
            // optional background picture, behind everything in this panel
            if let Some(path) = th.bg_image.clone() {
                if self.bg_tex.as_ref().map(|(p, _)| p != &path).unwrap_or(true) {
                    self.bg_tex = image::open(&path).ok().map(|img| {
                        let img = img.thumbnail(1600, 1600).to_rgba8();
                        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                        (path.clone(), ui.ctx().load_texture("style_bg", ci, egui::TextureOptions::LINEAR))
                    });
                }
                if let Some((_, t)) = &self.bg_tex {
                    let r = ui.max_rect();
                    // "cover": scale to fill, crop the overflow
                    let (iw, ih) = (t.size()[0] as f32, t.size()[1] as f32);
                    let s = (r.width() / iw).max(r.height() / ih);
                    let (vw, vh) = (r.width() / (iw * s), r.height() / (ih * s));
                    let uv = Rect::from_min_max(Pos2::new((1.0 - vw) / 2.0, (1.0 - vh) / 2.0), Pos2::new((1.0 + vw) / 2.0, (1.0 + vh) / 2.0));
                    ui.painter().image(t.id(), r, uv, Color32::WHITE.gamma_multiply(th.bg_opacity));
                }
            }
            if let Some(e) = self.load_error.clone() {
                ui.colored_label(th.error, format!("deck.toml could not be loaded, so saving is turned off: {e}"));
                ui.horizontal(|ui| {
                    if ui.button("Reload file").clicked() {
                        match Config::load(&self.path) {
                            Ok(c) => {
                                self.cfg = c;
                                self.load_error = None;
                            }
                            Err(e) => self.load_error = Some(e),
                        }
                    }
                    if ui.button("Start fresh (overwrite with the sample)").clicked() {
                        if std::fs::write(&self.path, SAMPLE).is_ok() {
                            if let Ok(c) = Config::load(&self.path) {
                                self.cfg = c;
                                self.load_error = None;
                            }
                        }
                    }
                });
                ui.separator();
            }
            if let Some(m) = self.style_note() {
                ui.colored_label(th.warn, m);
            }
            self.deck_panel(ui, &mut cmds);
            if th.show_hints {
                ui.add_space(10.0);
                ui.weak("Click a key to edit it · drag one onto another to swap · right-click for copy, paste, duplicate · Ctrl+Z undoes");
                ui.weak("The bottom-right wide key is the deck's clock slot: it can also be a normal button.");
            }
        });
        for c in cmds {
            self.run_cmd(&ctx, c);
        }

        self.settings_window(&ctx);
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

pub fn run(path: PathBuf) -> Result<(), String> {
    if !path.exists() {
        std::fs::write(&path, SAMPLE).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("deckd").with_inner_size([960.0, 600.0]).with_min_inner_size([820.0, 500.0]),
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
                page: 1,
                styles,
                tab: Tab::Look,
                undo: Vec::new(),
                redo: Vec::new(),
                committed,
                restoring: false,
                clipboard: None,
                show_settings: false,
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

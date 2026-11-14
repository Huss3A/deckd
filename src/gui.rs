//! The deckd window: a clickable copy of the deck, an editor for the selected button, and live status.
//! Edits are saved to deck.toml (debounced); the service thread notices the file change and re-uploads.

use crate::config::{Action, Button, Config, COLS, ROWS, SAMPLE};
use crate::{actions, layout, service};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TILE: f32 = 92.0;
const GAP: f32 = 8.0;
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
}

fn kind_of(a: &Option<Action>) -> Kind {
    match a {
        None => Kind::None,
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
}

fn valid_cell(c: i32, r: i32) -> bool {
    (0..COLS as i32).contains(&c) && (0..ROWS as i32).contains(&r) && !(r == 2 && c >= 3)
}

impl App {
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
                [&b.action, &b.double, &b.repeat]
                    .into_iter()
                    .flatten()
                    .chain(b.holds.iter().map(|h| &h.action))
                    .try_for_each(|a| actions::validate(a).map_err(|e| format!("button {:?} (page {}): {e}", b.pos, b.page)))
            })
            .and_then(|_| (1..=self.cfg.page_count()).try_for_each(|p| layout::build_zip(&self.cfg, p).map(|_| ())));
        self.save_error = match check.and_then(|_| self.cfg.save(&self.path)) {
            Ok(()) => {
                self.saved_at = Some(Instant::now());
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

    fn draw_tile(&mut self, ui: &mut egui::Ui, col: u8, row: u8, pressed: bool) {
        let rect = Rect::from_min_size(
            ui.min_rect().min + Vec2::new(col as f32 * (TILE + GAP), row as f32 * (TILE + GAP)),
            Vec2::splat(TILE),
        );
        let resp = ui.interact(rect, ui.scope_id().with(("tile", col, row)), Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() {
            self.selected = Some((col, row));
        }
        if resp.drag_started() {
            self.drag = Some((col, row));
        }
        let pointer = ui.ctx().pointer_interact_pos();
        if resp.drag_stopped() {
            self.drag = None;
            // drop onto another tile = swap the two buttons
            if let Some(p) = pointer {
                let origin = rect.min - Vec2::new(col as f32 * (TILE + GAP), row as f32 * (TILE + GAP));
                let (tc, tr) = (((p.x - origin.x) / (TILE + GAP)).floor() as i32, ((p.y - origin.y) / (TILE + GAP)).floor() as i32);
                if valid_cell(tc, tr) {
                    self.swap_cells((col, row), (tc as u8, tr as u8));
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
        paint_tile(ui.painter(), rect, b.as_ref(), tex.as_ref(), alpha);

        let is_target = swapping.is_some_and(|(_, h)| h == (col, row) || (is_source && shown != (col, row)));
        let selected = self.selected == Some((col, row));
        let stroke = if pressed {
            Stroke::new(3.0, Color32::YELLOW)
        } else if is_target {
            Stroke::new(3.0, Color32::from_rgb(0x3f, 0xb9, 0x50))
        } else if selected {
            Stroke::new(3.0, Color32::from_rgb(0x58, 0xa6, 0xff))
        } else if resp.hovered() {
            Stroke::new(2.0, Color32::GRAY)
        } else {
            Stroke::new(1.0, Color32::from_gray(70))
        };
        ui.painter().rect_stroke(rect, 8.0, stroke, StrokeKind::Inside);

        // translucent ghost that follows the pointer while dragging
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some(p) = pointer {
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
                let painter = ui.ctx().layer_painter(layer);
                let r = Rect::from_center_size(p, Vec2::splat(TILE));
                paint_tile(&painter, r, own.as_ref(), own_tex.as_ref(), 0.6);
                painter.rect_stroke(r, 8.0, Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 255, 255, 150)), StrokeKind::Inside);
            }
        }
    }

    fn deck_panel(&mut self, ui: &mut egui::Ui) {
        // flash only when the deck is showing the page we are looking at
        let on_page = self.svc.status.lock().ok().is_some_and(|s| s.page == self.page || s.page == 0);
        let pressed = self.svc.status.lock().ok().and_then(|s| s.pressed).filter(|(_, t)| on_page && t.elapsed() < Duration::from_millis(400));
        let size = Vec2::new(COLS as f32 * (TILE + GAP) - GAP, ROWS as f32 * (TILE + GAP) - GAP);
        let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(outer));
        self.drag_hover = match (self.drag, ui.ctx().pointer_interact_pos()) {
            (Some(_), Some(p)) => {
                let (c, r) = (((p.x - outer.min.x) / (TILE + GAP)).floor() as i32, ((p.y - outer.min.y) / (TILE + GAP)).floor() as i32);
                valid_cell(c, r).then_some((c as u8, r as u8))
            }
            _ => None,
        };
        for row in 0..ROWS {
            for col in 0..COLS {
                if row == 2 && col >= 3 {
                    continue;
                }
                let flashing = pressed.map(|(i, _)| i == row * COLS + col).unwrap_or(false);
                self.draw_tile(&mut inner, col, row, flashing);
            }
        }
        // the wide clock slot (also button 13 on the deck)
        let clock = Rect::from_min_size(
            outer.min + Vec2::new(3.0 * (TILE + GAP), 2.0 * (TILE + GAP)),
            Vec2::new(2.0 * TILE + GAP, TILE),
        );
        let flash = pressed.map(|(i, _)| i == 13).unwrap_or(false);
        ui.painter().rect_filled(clock, 8.0, Color32::from_gray(16));
        ui.painter().rect_stroke(clock, 8.0, Stroke::new(if flash { 3.0 } else { 1.0 }, if flash { Color32::YELLOW } else { Color32::from_gray(70) }), StrokeKind::Inside);
        ui.painter().text(clock.center(), Align2::CENTER_CENTER, "clock", FontId::proportional(14.0), Color32::GRAY);
    }

    /// Editor for one action slot (press / double tap / hold). `slot` identifies it for hotkey recording.
    fn action_editor(&mut self, ui: &mut egui::Ui, slot: &str, action: &mut Option<Action>) {
        let bad = Color32::from_rgb(0xff, 0x7b, 0x72);
        let mut kind = kind_of(action);
        let old_kind = kind;
        ui.horizontal_wrapped(|ui| {
            for (k, name) in [(Kind::None, "Nothing"), (Kind::Run, "Program"), (Kind::Open, "Open"), (Kind::Hotkey, "Hotkey"), (Kind::Media, "Media"), (Kind::Page, "Page"), (Kind::Print, "Print")] {
                ui.selectable_value(&mut kind, k, name);
            }
        });
        if kind != old_kind {
            self.recording = None;
            self.test_msg = None;
            *action = match kind {
                Kind::None => None,
                Kind::Run => Some(Action::Run { command: String::new(), args: vec![] }),
                Kind::Open => Some(Action::Open { target: String::new() }),
                Kind::Hotkey => Some(Action::Hotkey { keys: String::new() }),
                Kind::Media => Some(Action::Media { key: "play_pause".into() }),
                Kind::Page => Some(Action::Page { to: "next".into() }),
                Kind::Print => Some(Action::Print { text: "hello".into() }),
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
                ui.weak("Record captures the combo you press. The Windows key can't be recorded; type win+… instead.");
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
            None => {
                ui.weak("Nothing happens.");
            }
        }
        if let Some(a) = action.clone() {
            ui.add_space(4.0);
            match actions::validate(&a) {
                Err(e) => {
                    ui.colored_label(bad, e);
                }
                Ok(()) if !matches!(a, Action::Page { .. }) => {
                    if ui.button("▶ Test now").on_hover_text("Runs this action on your PC right now").clicked() {
                        self.test_msg = Some((slot.to_string(), match (&a, actions::execute(&a)) {
                            (Action::Print { text }, _) => (true, format!("Would log: {text}")),
                            (_, Ok(())) => (true, "Ran.".to_string()),
                            (_, Err(e)) => (false, e),
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

    fn editor(&mut self, ui: &mut egui::Ui) {
        let Some((col, row)) = self.selected else {
            ui.label("Click a button on the deck to edit it.");
            return;
        };
        let page = self.page;
        ui.horizontal(|ui| {
            ui.heading(format!("Button {},{}", col, row));
            ui.weak(format!("page {page} · #{}", row * COLS + col));
        });
        ui.add_space(4.0);
        let mut b: Button = self.cfg.button_at(page, col, row).cloned().unwrap_or(Button { page, pos: format!("{col},{row}"), ..Button::default() });
        // compare the whole button as JSON so new fields are never forgotten here
        let snapshot = |b: &Button| serde_json::to_string(b).unwrap_or_default();
        let before = snapshot(&b);

        // ---- look ----
        ui.label(egui::RichText::new("LOOK").small().weak());
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

            // per-button rim colors (default: the global ones under "Button feedback")
            let (def_tap, def_double) = (self.cfg.feedback.tap.clone(), self.cfg.feedback.double.clone());
            ui.horizontal(|ui| {
                ui.label("Rim:");
                let mut c = b.rim_tap.as_deref().and_then(parse_rgb).or_else(|| parse_rgb(&def_tap)).unwrap_or([0x2f, 0x81, 0xf7]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
                    b.rim_tap = Some(hex(c));
                }
                ui.label("tap");
                if b.rim_tap.is_some() && ui.small_button("reset").clicked() {
                    b.rim_tap = None;
                }
                ui.add_space(12.0);
                let mut c = b.rim_double.as_deref().and_then(parse_rgb).or_else(|| parse_rgb(&def_double)).unwrap_or([0x3f, 0xb9, 0x50]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
                    b.rim_double = Some(hex(c));
                }
                ui.label("double tap");
                if b.rim_double.is_some() && ui.small_button("reset").clicked() {
                    b.rim_double = None;
                }
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

        // ---- actions ----
        ui.add_space(10.0);
        ui.label(egui::RichText::new("WoEN PRESSED").small().weak());
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.action_editor(ui, "press", &mut b.action);
        });

        ui.add_space(6.0);
        egui::CollapsingHeader::new("On double tap").default_open(b.double.is_some()).show(ui, |ui| {
            ui.weak("Runs instead of the normal action when you tap twice quickly. The normal action then waits ~0.35 s.");
            self.action_editor(ui, "double", &mut b.double);
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

        ui.add_space(10.0);
        if ui.button("Clear this button").clicked() {
            self.cfg.clear_button(page, col, row);
            self.changed();
            return;
        }

        if before != snapshot(&b) {
            let empty = b.label.is_none() && b.color.is_none() && b.glyph.is_none() && b.icon.is_none() && b.action.is_none() && b.double.is_none() && b.holds.is_empty() && b.repeat.is_none();
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

fn paint_tile(p: &egui::Painter, rect: Rect, b: Option<&Button>, tex: Option<&egui::TextureHandle>, a: f32) {
    let bg = b.and_then(|b| b.color.as_deref()).and_then(parse_rgb).unwrap_or([0x20, 0x20, 0x20]);
    p.rect_filled(rect, 8.0, Color32::from_rgb(bg[0], bg[1], bg[2]).gamma_multiply(a));
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::Panel::bottom("status").show(ui, |ui| {
            let (connected, info, config_error, last) = {
                let s = self.svc.status.lock().unwrap_or_else(|e| e.into_inner());
                (s.connected, s.info.clone(), s.config_error.clone(), s.log.back().cloned())
            };
            ui.horizontal(|ui| {
                if connected {
                    ui.colored_label(Color32::from_rgb(0x3f, 0xb9, 0x50), "● Deck connected");
                    ui.weak(info.replace('{', "").replace('}', "").replace('"', ""));
                } else {
                    ui.colored_label(Color32::from_rgb(0xd2, 0x99, 0x22), "○ Waiting for the deck (is Ulanzi Studio closed?)");
                }
            });
            if let Some(e) = self.save_error.as_ref().or(config_error.as_ref()) {
                ui.colored_label(Color32::from_rgb(0xff, 0x7b, 0x72), format!("Not saved / not applied: {e}"));
            } else if self.dirty_since.is_some() {
                ui.weak("Saving…");
            } else if self.saved_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4)) {
                ui.colored_label(Color32::from_rgb(0x3f, 0xb9, 0x50), "✔ Saved, sending to the deck");
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
        egui::Panel::right("editor").resizable(false).exact_size(400.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.editor(ui));
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = self.load_error.clone() {
                ui.colored_label(Color32::from_rgb(0xff, 0x7b, 0x72), format!("deck.toml could not be loaded, so saving is turned off: {e}"));
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
            ui.horizontal(|ui| {
                ui.label("Brightness");
                let mut br = self.cfg.brightness as f32;
                let r = ui.add(egui::Slider::new(&mut br, 0.0..=100.0).integer());
                self.cfg.brightness = br as u8;
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    self.changed();
                }
            });
            ui.add_space(8.0);
            // page tabs
            ui.horizontal(|ui| {
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
                if count < crate::config::MAX_PAGES && ui.button("+").on_hover_text("Add a page").clicked() {
                    self.cfg.pages = count + 1;
                    self.page = count + 1;
                    self.changed();
                }
                let last_empty = !self.cfg.buttons.iter().any(|b| b.page == count);
                if count > 1 && last_empty && ui.button("−").on_hover_text("Remove the last page (it is empty)").clicked() {
                    self.cfg.pages = count - 1;
                    self.page = self.page.min(count - 1);
                    self.changed();
                }
            });
            ui.add_space(4.0);
            self.deck_panel(ui);
            ui.add_space(8.0);
            ui.weak("Click to edit, drag one button onto another to swap them, Delete clears. Bottom-right is the deck's clock.");
            ui.add_space(6.0);
            let before = self.cfg.feedback.clone();
            egui::CollapsingHeader::new("Button feedback (colored rim on the deck)").show(ui, |ui| {
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
                    ui.weak("While holding, the rim appears at each hold step and shifts color per step (set a step's own color in its hold section). A repeat keeps its rim while held. A dim rim after the first tap of a double-tap button means \"waiting for a second tap\". Needs the deck connected.");
                });
            });
            if before != self.cfg.feedback {
                self.changed();
            }
        });

        // Delete clears the selected button (only when no text field has focus)
        if let Some((c, r)) = self.selected {
            if ctx.memory(|m| m.focused().is_none()) && ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.cfg.clear_button(self.page, c, r);
                self.changed();
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

fn install_symbol_font(ctx: &egui::Context, path: &str) {
    if let Ok(bytes) = std::fs::read(path) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("symbols".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("symbols".to_owned());
        }
        ctx.set_fonts(fonts);
    }
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
            install_symbol_font(&cc.egui_ctx, &cfg.glyph_font);
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
            }))
        }),
    )
    .map_err(|e| e.to_string())
}

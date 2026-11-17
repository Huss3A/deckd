//! The window shell: top bar, action library (left), deck canvas (middle), inspector (right). All floating cards on a dark
//! canvas. Colors and sizes come from the window style.

use super::*;

pub(super) struct Preset {
    name: &'static str,
    hint: &'static str,
    cat: &'static str,
    glyph: &'static str,
    label: &'static str,
    color: &'static str,
    make: fn() -> Action,
    /// Extra setup (state-aware looks).
    extra: Option<fn(&mut Button)>,
}

fn media(k: &str) -> Action {
    Action::Media { key: k.into() }
}

fn mic_state(b: &mut Button) {
    b.state = Some(StateSource::MicMuted);
    b.label = Some("Mic ON".into());
    b.color = Some("#1f6f3a".into());
    b.alt = Some(Alt { label: Some("Mic MUTED".into()), color: Some("#b62324".into()), ..Alt::default() });
}

fn speaker_state(b: &mut Button) {
    b.state = Some(StateSource::OutputMuted);
    b.label = Some("Sound ON".into());
    b.color = Some("#1f6f3a".into());
    b.alt = Some(Alt { label: Some("Sound MUTED".into()), color: Some("#b62324".into()), ..Alt::default() });
}

pub(super) fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Mic mute", hint: "Toggle the microphone, shows its state", cat: "Audio", glyph: "🎤", label: "Mic", color: "#1f6f3a", make: || media("mic_mute"), extra: Some(mic_state) },
        Preset { name: "Speaker mute", hint: "Toggle sound, shows its state", cat: "Audio", glyph: "🔇", label: "Mute", color: "#1f6f3a", make: || media("mute"), extra: Some(speaker_state) },
        Preset { name: "Volume up", hint: "Hold to keep raising", cat: "Audio", glyph: "🔊", label: "Vol +", color: "#c9962a", make: || media("vol_up"), extra: None },
        Preset { name: "Volume down", hint: "Hold to keep lowering", cat: "Audio", glyph: "🔉", label: "Vol -", color: "#c9962a", make: || media("vol_down"), extra: None },
        Preset { name: "Play / Pause", hint: "Media key", cat: "Media", glyph: "⏯", label: "Play", color: "#2563d6", make: || media("play_pause"), extra: None },
        Preset { name: "Next track", hint: "Media key", cat: "Media", glyph: "⏭", label: "Next", color: "#2563d6", make: || media("next"), extra: None },
        Preset { name: "Previous track", hint: "Media key", cat: "Media", glyph: "⏮", label: "Prev", color: "#2563d6", make: || media("prev"), extra: None },
        Preset { name: "Open website", hint: "A link in your browser", cat: "Apps", glyph: "🌐", label: "Web", color: "#8d4bd6", make: || Action::Open { target: "https://".into() }, extra: None },
        Preset { name: "Launch program", hint: "Start an app", cat: "Apps", glyph: "▶", label: "App", color: "#2f9d4a", make: || Action::Run { command: String::new(), args: vec![] }, extra: None },
        Preset { name: "Open folder or file", hint: "Whatever Windows opens for it", cat: "Apps", glyph: "📂", label: "Open", color: "#8d4bd6", make: || Action::Open { target: String::new() }, extra: None },
        Preset { name: "Hotkey", hint: "Press a key combination", cat: "Keys", glyph: "⌨", label: "Keys", color: "#3b4a63", make: || Action::Hotkey { keys: String::new() }, extra: None },
        Preset { name: "Type text", hint: "Types a phrase for you", cat: "Keys", glyph: "✎", label: "Text", color: "#3b4a63", make: || Action::Text { text: String::new() }, extra: None },
        Preset { name: "Macro", hint: "Several steps in a row", cat: "Macros", glyph: "★", label: "Macro", color: "#8d4bd6", make: || Action::Macro { steps: vec![Action::Hotkey { keys: String::new() }] }, extra: None },
        Preset { name: "Copy, then paste", hint: "Ctrl+C, wait, Ctrl+V", cat: "Macros", glyph: "⎘", label: "Copy>Paste", color: "#8d4bd6", make: || Action::Macro { steps: vec![Action::Hotkey { keys: "ctrl+c".into() }, Action::Delay { ms: 150 }, Action::Hotkey { keys: "ctrl+v".into() }] }, extra: None },
        Preset { name: "Next page", hint: "Switch the deck page", cat: "Deck", glyph: "→", label: "Next", color: "#3b4a63", make: || Action::Page { to: "next".into() }, extra: None },
        Preset { name: "Previous page", hint: "Switch the deck page", cat: "Deck", glyph: "←", label: "Back", color: "#3b4a63", make: || Action::Page { to: "prev".into() }, extra: None },
        // ---- more media / audio ----
        pr("Stop", "Stop playback", "Media", "⏹", "Stop", "#2563d6", || media("stop")),
        // ---- Windows ----
        pr("Show desktop", "Win+D", "Windows", "🖥", "Desktop", "#2b6cb0", || hk("win+d")),
        pr("Lock PC", "Win+L", "Windows", "🔒", "Lock", "#2b6cb0", || hk("win+l")),
        pr("Task Manager", "Ctrl+Shift+Esc", "Windows", "📊", "Tasks", "#2b6cb0", || hk("ctrl+shift+esc")),
        pr("Screenshot", "Snipping tool (Win+Shift+S)", "Windows", "✂", "Snip", "#2b6cb0", || hk("win+shift+s")),
        pr("Switch window", "Alt+Tab", "Windows", "⇄", "Alt+Tab", "#2b6cb0", || hk("alt+tab")),
        pr("Task view", "All windows and desktops (Win+Tab)", "Windows", "▦", "Views", "#2b6cb0", || hk("win+tab")),
        pr("Close window", "Alt+F4", "Windows", "✖", "Close", "#b62324", || hk("alt+f4")),
        pr("Minimize all", "Win+M", "Windows", "▁", "Min all", "#2b6cb0", || hk("win+m")),
        pr("Desktop to the left", "Virtual desktop", "Windows", "◀", "Desk <", "#2b6cb0", || hk("ctrl+win+left")),
        pr("Desktop to the right", "Virtual desktop", "Windows", "▶", "Desk >", "#2b6cb0", || hk("ctrl+win+right")),
        pr("Clipboard history", "Win+V", "Windows", "📋", "Clips", "#2b6cb0", || hk("win+v")),
        pr("File Explorer", "Win+E", "Windows", "📁", "Files", "#c9962a", || hk("win+e")),
        pr("Run dialog", "Win+R", "Windows", "⌘", "Run", "#2b6cb0", || hk("win+r")),
        pr("Windows Settings", "Opens Settings", "Windows", "⚙", "Settings", "#3b4a63", || Action::Open { target: "ms-settings:".into() }),
        pr("Downloads folder", "Opens it in Explorer", "Windows", "⬇", "Downloads", "#c9962a", || Action::Open { target: "shell:Downloads".into() }),
        // ---- apps ----
        pr("Calculator", "Starts calc", "Apps", "🧮", "Calc", "#2f9d4a", || run("calc.exe")),
        pr("Notepad", "Starts Notepad", "Apps", "📝", "Notepad", "#2f9d4a", || run("notepad.exe")),
        pr("Terminal", "Windows Terminal", "Apps", "▮", "Terminal", "#2f9d4a", || run("wt.exe")),
        pr("YouTube", "Opens youtube.com", "Apps", "▶", "YouTube", "#b62324", || Action::Open { target: "https://www.youtube.com".into() }),
        pr("Gmail", "Opens your inbox", "Apps", "✉", "Mail", "#b62324", || Action::Open { target: "https://mail.google.com".into() }),
        pr("Google search", "Opens google.com", "Apps", "🔍", "Search", "#2563d6", || Action::Open { target: "https://www.google.com".into() }),
        // ---- editing keys ----
        pr("Copy", "Ctrl+C", "Keys", "⎘", "Copy", "#3b4a63", || hk("ctrl+c")),
        pr("Paste", "Ctrl+V", "Keys", "📋", "Paste", "#3b4a63", || hk("ctrl+v")),
        pr("Paste plain text", "Ctrl+Shift+V", "Keys", "📋", "Plain", "#3b4a63", || hk("ctrl+shift+v")),
        pr("Cut", "Ctrl+X", "Keys", "✂", "Cut", "#3b4a63", || hk("ctrl+x")),
        pr("Undo", "Ctrl+Z", "Keys", "↶", "Undo", "#3b4a63", || hk("ctrl+z")),
        pr("Redo", "Ctrl+Y", "Keys", "↷", "Redo", "#3b4a63", || hk("ctrl+y")),
        pr("Select all", "Ctrl+A", "Keys", "▣", "All", "#3b4a63", || hk("ctrl+a")),
        pr("Save", "Ctrl+S", "Keys", "💾", "Save", "#3b4a63", || hk("ctrl+s")),
        pr("Find", "Ctrl+F", "Keys", "🔎", "Find", "#3b4a63", || hk("ctrl+f")),
        pr("Fullscreen", "F11", "Keys", "⛶", "Full", "#3b4a63", || hk("f11")),
        pr("Enter", "The Enter key", "Keys", "⏎", "Enter", "#3b4a63", || hk("enter")),
        pr("Escape", "The Esc key", "Keys", "⎋", "Esc", "#3b4a63", || hk("esc")),
        // ---- browser ----
        pr("New tab", "Ctrl+T", "Browser", "➕", "New tab", "#d9822b", || hk("ctrl+t")),
        pr("Close tab", "Ctrl+W", "Browser", "✖", "Close tab", "#d9822b", || hk("ctrl+w")),
        pr("Reopen closed tab", "Ctrl+Shift+T", "Browser", "↺", "Reopen", "#d9822b", || hk("ctrl+shift+t")),
        pr("Next tab", "Ctrl+Tab", "Browser", "▷", "Tab >", "#d9822b", || hk("ctrl+tab")),
        pr("Previous tab", "Ctrl+Shift+Tab", "Browser", "◁", "Tab <", "#d9822b", || hk("ctrl+shift+tab")),
        pr("Refresh", "F5", "Browser", "⟳", "Reload", "#d9822b", || hk("f5")),
        pr("Back", "Alt+Left", "Browser", "←", "Back", "#d9822b", || hk("alt+left")),
        pr("Forward", "Alt+Right", "Browser", "→", "Forward", "#d9822b", || hk("alt+right")),
        pr("Private window", "Ctrl+Shift+N", "Browser", "🕶", "Private", "#d9822b", || hk("ctrl+shift+n")),
        // ---- macros ----
        pr("Mute mic and speakers", "Both toggles in one press", "Macros", "🔇", "Mute all", "#8d4bd6", || mac(vec![media("mic_mute"), media("mute")])),
        pr("Save and close", "Ctrl+S, wait, Ctrl+W", "Macros", "💾", "Save+Close", "#8d4bd6", || mac(vec![hk("ctrl+s"), Action::Delay { ms: 250 }, hk("ctrl+w")])),
        pr("Select all and copy", "Ctrl+A, Ctrl+C", "Macros", "▣", "All+Copy", "#8d4bd6", || mac(vec![hk("ctrl+a"), Action::Delay { ms: 100 }, hk("ctrl+c")])),
        pr("Cut line", "Select the line and cut it", "Macros", "✂", "Cut line", "#8d4bd6", || mac(vec![hk("home"), hk("shift+end"), hk("ctrl+x")])),
        pr("Delete line", "Select the line and delete it", "Macros", "⌫", "Del line", "#8d4bd6", || mac(vec![hk("home"), hk("shift+end"), hk("delete")])),
        pr("Paste and Enter", "Ctrl+V then Enter", "Macros", "⏎", "Paste+Go", "#8d4bd6", || mac(vec![hk("ctrl+v"), Action::Delay { ms: 150 }, hk("enter")])),
        pr("Screenshot, then paste", "Snip, wait for it, paste", "Macros", "✂", "Snip>Paste", "#8d4bd6", || mac(vec![hk("win+shift+s"), Action::Delay { ms: 4000 }, hk("ctrl+v")])),
        pr("Type and send", "Types a message, presses Enter", "Macros", "💬", "Say it", "#8d4bd6", || mac(vec![Action::Text { text: "On my way!".into() }, hk("enter")])),
        pr("Show desktop and lock", "Win+D, then Win+L", "Macros", "🔒", "Away", "#8d4bd6", || mac(vec![hk("win+d"), Action::Delay { ms: 300 }, hk("win+l")])),
        pr("Open app and type", "Launch, wait, type a line", "Macros", "▶", "Launch", "#8d4bd6", || mac(vec![run("notepad.exe"), Action::Delay { ms: 1200 }, Action::Text { text: "Hello!".into() }])),
        // ---- deck pages ----
        pr("Home page", "Back to page 1", "Deck", "⌂", "Home", "#3b4a63", || Action::Page { to: "home".into() }),
        pr("Go to page 2", "Jump to page 2", "Deck", "②", "Page 2", "#3b4a63", || Action::Page { to: "2".into() }),
        pr("Go to page 3", "Jump to page 3", "Deck", "③", "Page 3", "#3b4a63", || Action::Page { to: "3".into() }),
    ]
}

/// `s` cut with an ellipsis so it fits `max_w` pixels.
fn fit_text(p: &egui::Painter, s: &str, font: &FontId, max_w: f32) -> String {
    let w = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x;
    if w(s) <= max_w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let t = chars.iter().collect::<String>() + "…";
        if w(&t) <= max_w {
            return t;
        }
    }
    "…".to_string()
}

fn hk(keys: &str) -> Action {
    Action::Hotkey { keys: keys.into() }
}

fn run(cmd: &str) -> Action {
    Action::Run { command: cmd.into(), args: vec![] }
}

fn mac(steps: Vec<Action>) -> Action {
    Action::Macro { steps }
}

/// A plain preset (no state look).
fn pr(name: &'static str, hint: &'static str, cat: &'static str, glyph: &'static str, label: &'static str, color: &'static str, make: fn() -> Action) -> Preset {
    Preset { name, hint, cat, glyph, label, color, make, extra: None }
}

const CATS: [&str; 9] = ["All", "Audio", "Media", "Windows", "Apps", "Keys", "Browser", "Macros", "Deck"];

impl App {
    fn panel_card(th: &Theme) -> egui::Frame {
        egui::Frame::new()
            .fill(th.panel)
            .stroke(Stroke::new(th.border_width, th.border))
            .corner_radius(egui::CornerRadius::same(th.card_rounding))
            .inner_margin(egui::Margin::same(16))
    }

    fn apply_preset(&mut self, cell: (u8, u8), p: &Preset) {
        let page = self.page;
        let mut b = self.cfg.button_at(page, cell.0, cell.1).cloned().unwrap_or(Button { page, pos: format!("{},{}", cell.0, cell.1), ..Button::default() });
        b.action = Some((p.make)());
        if b.label.is_none() {
            b.label = Some(p.label.into());
        }
        if b.glyph.is_none() && b.icon.is_none() {
            b.glyph = Some(p.glyph.into());
        }
        if b.color.is_none() {
            b.color = Some(p.color.into());
        }
        if let Some(f) = p.extra {
            f(&mut b);
        }
        if self.lib_pin {
            b.pinned = true;
        }
        *self.cfg.button_mut(page, cell.0, cell.1) = b;
        self.tab = Tab::Tap;
        self.changed();
    }

    /// Left card: searchable list of ready-made actions. Click one to put it on the selected key.
    fn library(&mut self, ui: &mut egui::Ui) {
        let th = self.styles.theme.clone();
        ui.label(RichText::new("Actions").strong().size(th.heading_size));
        ui.label(RichText::new(if self.selected.is_some() { "Click one to put it on the selected key." } else { "Select a key on the deck first." }).size(th.small_size).color(th.text_dim));
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(&mut self.lib_query).hint_text("Search actions…").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 7)));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for c in CATS {
                let sel = self.lib_cat == c;
                let t = RichText::new(c).size(th.small_size + 0.5).color(if sel { th.accent_text } else { th.text_dim });
                if ui.add(egui::Button::new(t).fill(if sel { th.accent } else { th.card }).stroke(Stroke::NONE).corner_radius(egui::CornerRadius::same(255))).clicked() {
                    self.lib_cat = c.to_string();
                }
            }
        });
        ui.add_space(4.0);
        ui.checkbox(&mut self.lib_pin, "📌 Pin the next one to all pages").on_hover_text("The action you click shows on every page at that key's position");
        ui.add_space(4.0);
        let q = self.lib_query.to_lowercase();
        let all = presets();
        let mut apply: Option<usize> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            for (i, p) in all.iter().enumerate() {
                if (self.lib_cat != "All" && p.cat != self.lib_cat) || (!q.is_empty() && !p.name.to_lowercase().contains(&q) && !p.hint.to_lowercase().contains(&q)) {
                    continue;
                }
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::click());
                let paint = ui.painter();
                let cr = egui::CornerRadius::same(th.rounding + 2);
                paint.rect_filled(rect, cr, if resp.hovered() { th.hover } else { th.card.gamma_multiply(0.6) });
                if resp.hovered() {
                    paint.rect_stroke(rect, cr, Stroke::new(1.0, th.select.gamma_multiply(0.5)), StrokeKind::Inside);
                }
                let tile = Rect::from_center_size(Pos2::new(rect.min.x + 26.0, rect.center().y), Vec2::splat(32.0));
                let col = super::parse_rgb(p.color).map(super::c32).unwrap_or(th.accent);
                paint.rect_filled(tile, egui::CornerRadius::same(th.rounding), col);
                paint.text(tile.center(), Align2::CENTER_CENTER, p.glyph, FontId::proportional(17.0), Color32::WHITE);
                // text never runs past the row: cut with an ellipsis (hover shows the whole hint)
                let room = rect.max.x - tile.max.x - 24.0;
                let (nf, hf) = (FontId::proportional(th.body_size), FontId::proportional(th.small_size));
                let (name, hint) = (fit_text(paint, p.name, &nf, room), fit_text(paint, p.hint, &hf, room));
                paint.text(Pos2::new(tile.max.x + 12.0, rect.center().y - 8.0), Align2::LEFT_CENTER, name, nf, th.text);
                paint.text(Pos2::new(tile.max.x + 12.0, rect.center().y + 9.0), Align2::LEFT_CENTER, hint, hf, th.text_dim);
                let resp = resp.on_hover_text(format!("{} - {}", p.name, p.hint));
                if resp.clicked() {
                    apply = Some(i);
                }
            }
        });
        if let Some(i) = apply {
            match self.selected {
                Some(cell) => self.apply_preset(cell, &all[i]),
                None => self.toast("Select a key on the deck first"),
            }
        }
    }

    /// Page tabs under the deck. Dragging a key over a tab for a moment switches to that page; dropping it on a tab moves it there.
    fn page_tabs(&mut self, ui: &mut egui::Ui) {
        let th = self.styles.theme.clone();
        let count = self.cfg.page_count();
        let max = crate::config::MAX_PAGES;
        let last_empty = !self.cfg.buttons.iter().any(|b| b.page == count);
        let pointer = ui.ctx().pointer_interact_pos();
        let dragging = self.drag.is_some();
        // (kind, label): kind 0 = a page, 1 = add, 2 = remove
        let mut items: Vec<(u8, u8, String)> = Vec::new();
        for p in 1..=count {
            let live = self.svc.status.lock().ok().is_some_and(|s| s.connected && s.page == p && count > 1);
            items.push((0, p, if live { format!("Page {p} ●") } else { format!("Page {p}") }));
        }
        if count < max {
            items.push((1, 0, "+".into()));
        }
        if count > 1 && last_empty {
            items.push((2, 0, "−".into()));
        }
        let font = FontId::proportional(th.body_size);
        let widths: Vec<f32> = items
            .iter()
            .map(|(k, _, t)| if *k == 0 { ui.painter().layout_no_wrap(t.clone(), font.clone(), Color32::WHITE).size().x + 30.0 } else { 28.0 })
            .collect();
        let total: f32 = widths.iter().sum::<f32>() + 8.0 * (items.len().saturating_sub(1)) as f32;
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
        let mut x = row.center().x - total / 2.0;
        let mut rects = Vec::new();
        let mut hovered_page = None;
        for (i, (kind, p, text)) in items.iter().enumerate() {
            let rect = Rect::from_min_size(Pos2::new(x, row.min.y + 1.0), Vec2::new(widths[i], 28.0));
            x += widths[i] + 8.0;
            let resp = ui.interact(rect, ui.id().with(("ptab", i)), Sense::click());
            let over = pointer.is_some_and(|q| rect.contains(q));
            let sel = *kind == 0 && self.page == *p;
            let drop_here = dragging && over && *kind == 0 && self.page != *p;
            let fill = if sel { th.accent } else if drop_here || (resp.hovered() && !dragging) { th.hover } else { th.card };
            let color = if sel { th.accent_text } else if *kind == 0 { th.text_dim } else { th.text };
            ui.painter().rect_filled(rect, egui::CornerRadius::same(255), fill);
            if drop_here {
                ui.painter().rect_stroke(rect, egui::CornerRadius::same(255), Stroke::new(2.0, th.ok), egui::StrokeKind::Inside);
            }
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, if *kind == 0 { font.clone() } else { FontId::proportional(th.body_size + 3.0) }, color);
            match kind {
                0 => {
                    rects.push((*p, rect));
                    if dragging && over {
                        hovered_page = Some(*p);
                    }
                    if resp.clicked() {
                        self.page = *p;
                    }
                }
                1 if resp.clone().on_hover_text("Add a page").clicked() => {
                    self.cfg.pages = count + 1;
                    self.page = count + 1;
                    self.changed();
                }
                2 if resp.on_hover_text("Remove the last page (it is empty)").clicked() => {
                    self.cfg.pages = count - 1;
                    self.page = self.page.min(count - 1);
                    self.changed();
                }
                _ => {}
            }
        }
        self.tab_rects = rects;
        // hovering a dragged key over another page's tab for a moment switches to it
        match hovered_page {
            Some(p) if p != self.page => {
                let since = match self.tab_hover {
                    Some((q, t)) if q == p => t,
                    _ => Instant::now(),
                };
                self.tab_hover = Some((p, since));
                if since.elapsed() >= Duration::from_millis(450) {
                    self.page = p;
                    self.tab_hover = None;
                }
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
            _ => self.tab_hover = None,
        }
    }

    /// Top bar: brand, pages, connection, undo/redo, brightness, settings.
    pub(super) fn studio_top(&mut self, ui: &mut egui::Ui) {
        let th = self.styles.theme.clone();
        let bar = egui::Frame::new().fill(th.window).inner_margin(egui::Margin { left: 18, right: 18, top: 12, bottom: 6 });
        egui::Panel::top("top").frame(bar).show(ui, |ui| {
            ui.horizontal(|ui| {
                // brand
                let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                ui.painter().rect_filled(r, egui::CornerRadius::same(8), th.accent);
                ui.painter().circle_stroke(r.center(), 6.0, Stroke::new(2.0, th.accent_text));
                ui.label(RichText::new("deckd").strong().size(th.heading_size + 2.0));
                ui.add_space(14.0);
                let count = self.cfg.page_count();
                if self.page > count {
                    self.page = count;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("⚙").min_size(Vec2::new(32.0, 30.0))).on_hover_text("Settings and window style").clicked() {
                        self.show_settings = true;
                    }
                    if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("↷").min_size(Vec2::new(32.0, 30.0))).on_hover_text("Redo (Ctrl+Y)").clicked() {
                        self.do_redo();
                    }
                    if ui.add_enabled(!self.undo.is_empty() || self.dirty_since.is_some(), egui::Button::new("↶").min_size(Vec2::new(32.0, 30.0))).on_hover_text("Undo (Ctrl+Z)").clicked() {
                        self.do_undo();
                    }
                    ui.add_space(6.0);
                    let mut br = self.cfg.brightness as f32;
                    let r = ui.add(egui::Slider::new(&mut br, 0.0..=100.0).integer().show_value(false).trailing_fill(true));
                    ui.label(RichText::new("☀").color(th.text_dim)).on_hover_text("Deck brightness");
                    self.cfg.brightness = br as u8;
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        self.changed();
                    }
                    ui.add_space(10.0);
                    // status: connection, then the latest save / error message
                    let (connected, config_error, last) = {
                        let s = self.svc.status.lock().unwrap_or_else(|e| e.into_inner());
                        (s.connected, s.config_error.clone(), s.log.back().cloned())
                    };
                    if ui.add(egui::Button::new(RichText::new("Log").size(th.small_size)).small()).on_hover_text("Show the log").clicked() {
                        self.show_log = !self.show_log;
                    }
                    let msg: Option<(String, Color32)> = if let Some(e) = self.save_error.as_ref().or(config_error.as_ref()) {
                        Some((format!("Not saved: {e}"), th.error))
                    } else if self.dirty_since.is_some() {
                        Some(("Saving…".into(), th.text_dim))
                    } else if self.saved_at.is_some_and(|t| t.elapsed() < Duration::from_secs(4)) {
                        Some(("Saved".into(), th.ok))
                    } else {
                        last.map(|l| (l, th.text_dim))
                    };
                    if let Some((m, c)) = msg {
                        let m: String = m.chars().take(60).collect();
                        ui.label(RichText::new(m).size(th.small_size).color(c));
                    }
                    let (dot, txt) = if connected { (th.ok, "Deck connected") } else { (th.warn, "Waiting for the deck") };
                    ui.label(RichText::new(txt).size(th.small_size + 0.5).color(th.text));
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, dot);
                });
            });
        });
    }

    /// Everything below the top bar.
    pub(super) fn studio_body(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<TileCmd>) {
        let ctx = ui.ctx().clone();
        let th = self.styles.theme.clone();

        if self.show_log {
            egui::Panel::bottom("log").frame(egui::Frame::new().fill(th.window).inner_margin(egui::Margin { left: 18, right: 18, top: 4, bottom: 10 })).show(ui, |ui| {
                Self::panel_card(&th).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Log").strong());
                        if ui.small_button("Copy log").clicked() {
                            let text = self.svc.status.lock().unwrap_or_else(|e| e.into_inner()).log.iter().cloned().collect::<Vec<_>>().join("\n");
                            ui.ctx().copy_text(text);
                            self.toast("Log copied");
                        }
                    });
                    egui::ScrollArea::vertical().max_height(130.0).stick_to_bottom(true).show(ui, |ui| {
                        for l in self.svc.status.lock().unwrap_or_else(|e| e.into_inner()).log.iter() {
                            ui.monospace(l);
                        }
                    });
                });
            });
        }

        let win = ctx.content_rect().width();
        // the side columns grow with the window (the deck in the middle only takes what is left)
        let lib_w = (win * 0.25).clamp(300.0, 420.0);
        let side_w = (win * 0.31).clamp(400.0, 580.0);

        egui::Panel::left("library").frame(egui::Frame::new().fill(th.window).inner_margin(egui::Margin { left: 18, right: 7, top: 6, bottom: 16 })).resizable(false).show_separator_line(false).exact_size(lib_w).show(ui, |ui| {
            Self::panel_card(&th).show(ui, |ui| {
                ui.set_min_size(Vec2::new(ui.available_width(), ui.available_height()));
                self.library(ui);
            });
        });
        egui::Panel::right("inspector").frame(egui::Frame::new().fill(th.window).inner_margin(egui::Margin { left: 7, right: 18, top: 6, bottom: 16 })).resizable(false).show_separator_line(false).exact_size(side_w).show(ui, |ui| {
            // laid out in a fixed rect: whatever is too wide inside is clipped and can never push the panel around
            let rect = ui.max_rect();
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            inner.set_clip_rect(rect.intersect(ui.clip_rect()));
            Self::panel_card(&th).show(&mut inner, |ui| {
                ui.set_min_size(Vec2::new(ui.available_width(), ui.available_height()));
                self.editor(ui);
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(th.window).inner_margin(egui::Margin { left: 7, right: 7, top: 6, bottom: 16 })).show(ui, |ui| {
            Self::panel_card(&th).show(ui, |ui| {
                ui.set_min_size(Vec2::new(ui.available_width(), ui.available_height()));
                self.canvas(ui, cmds);
            });
        });
    }

    /// Middle card: the deck, as large as fits.
    fn canvas(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<TileCmd>) {
        let th = self.styles.theme.clone();
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
        }
        if let Some(m) = self.style_note() {
            ui.colored_label(th.warn, m);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Page {}", self.page)).strong().size(th.heading_size));
            ui.label(RichText::new("Click a key to edit it · drag to swap · right-click for more").size(th.small_size).color(th.text_dim));
        });
        let hints_h = 40.0;
        let ratio = th.tile_gap / th.tile_size;
        let (aw, ah) = (ui.available_width() - 8.0, ui.available_height() - hints_h);
        let t = ((aw / (5.0 + 4.0 * ratio)).min(ah / (3.0 + 2.0 * ratio))).clamp(56.0, 240.0).floor();
        self.fit = t;
        let deck_h = 3.0 * t + 2.0 * t * ratio;
        ui.add_space(((ah - deck_h) * 0.3).max(8.0));
        self.deck_panel(ui, cmds);
        ui.add_space(14.0);
        self.page_tabs(ui);
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("The wide bottom-right key is the deck's clock slot; it can also be a normal button.").size(th.small_size).color(th.text_dim));
        });
    }
}

#[cfg(test)]
mod tests {
    /// Every ready-made action must pass the same validation as a saved config (empty "fill me in" ones excepted).
    #[test]
    fn presets_are_valid() {
        for p in super::presets() {
            let a = (p.make)();
            let blank = matches!(&a, crate::config::Action::Run { command, .. } if command.is_empty())
                || matches!(&a, crate::config::Action::Open { target } if target.is_empty() || target == "https://")
                || matches!(&a, crate::config::Action::Hotkey { keys } if keys.is_empty())
                || matches!(&a, crate::config::Action::Text { text } if text.is_empty())
                || p.name == "Macro";
            if !blank {
                crate::actions::validate(&a).unwrap_or_else(|e| panic!("preset {:?}: {e}", p.name));
            }
        }
    }
}

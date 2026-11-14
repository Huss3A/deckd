//! The "Step Rail": the macro step list. Slim numbered rows between a start and an end cap, a one-bar timeline,
//! an insert "+" between rows, a searchable picker (Ctrl+K), inline step editing, validation badges, drag reorder
//! and an estimate-driven test-run playhead. See MACRO-DESIGN.md. All colors come from the window style (`Theme`).

use crate::actions;
use crate::config::{self, Action, MACRO_MAX_STEPS};
use crate::theme::Theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

/// How long after "Test run" before the steps start (time to click into the target window).
const TEST_DELAY_S: f64 = 3.0;

#[derive(Clone, Default)]
struct RailState {
    selected: Option<usize>,
    /// Insert index while the picker is open.
    picker: Option<usize>,
    query: String,
    fresh: bool,
    drag: Option<usize>,
    flash: Option<(usize, f64)>,
    run_start: Option<f64>,
    last_rect: Option<Rect>,
    focus: bool,
    jump: Option<usize>,
}

struct Meta {
    glyph: &'static str,
    accent: Color32,
    title: &'static str,
    summary: String,
}

fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

fn hue(th: &Theme, deg: f32) -> Color32 {
    let [r, g, b] = config::rotate_hue(rgb(th.accent), deg);
    Color32::from_rgb(r, g, b)
}

fn media_name(k: &str) -> (&'static str, &'static str) {
    match k {
        "vol_down" => ("🔉", "Volume down"),
        "vol_up" => ("🔊", "Volume up"),
        "mute" => ("🔇", "Mute"),
        "play_pause" => ("⏯", "Play / Pause"),
        "prev" => ("⏮", "Previous"),
        "next" => ("⏭", "Next"),
        "stop" => ("⏹", "Stop"),
        "mic_mute" => ("🎤", "Mic mute"),
        _ => ("♪", "Media"),
    }
}

fn fmt_ms(ms: f32) -> String {
    if ms < 1000.0 {
        format!("{} ms", ms.round() as i64)
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

fn meta(a: &Action, th: &Theme) -> Meta {
    match a {
        Action::Hotkey { keys } => Meta {
            glyph: "⌨",
            accent: th.accent,
            title: "Hotkey",
            summary: if keys.trim().is_empty() { "No keys set".into() } else { keys.clone() },
        },
        Action::Text { text } => {
            let first: String = text.chars().take(40).map(|c| if c == '\n' { '↵' } else { c }).collect();
            let n = text.chars().count();
            Meta {
                glyph: "✎",
                accent: hue(th, 60.0),
                title: "Type text",
                summary: if n == 0 { "Nothing to type".into() } else { format!("\"{first}{}\"  ({n} chars)", if n > 40 { "…" } else { "" }) },
            }
        }
        Action::Run { command, args } => {
            let name = std::path::Path::new(command).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            Meta {
                glyph: "▶",
                accent: th.ok,
                title: "Program",
                summary: if command.trim().is_empty() { "No program set".into() } else { format!("{name} {}", args.join(" ")).trim().to_string() },
            }
        }
        Action::Open { target } => Meta {
            glyph: if target.contains("://") { "🌐" } else { "📂" },
            accent: hue(th, 110.0),
            title: "Open",
            summary: if target.trim().is_empty() {
                "Nothing to open".into()
            } else {
                target.trim_start_matches("https://").trim_start_matches("http://").to_string()
            },
        },
        Action::Media { key } => {
            let (g, n) = media_name(key);
            Meta { glyph: g, accent: hue(th, -45.0), title: "Media", summary: n.to_string() }
        }
        Action::Delay { ms } => Meta { glyph: "⏱", accent: th.warn, title: "Wait", summary: fmt_ms(*ms as f32) },
        Action::Print { text } => Meta { glyph: "💬", accent: th.text_dim, title: "Print", summary: format!("\"{text}\"") },
        Action::Focus { window } => Meta {
            glyph: "◰",
            accent: hue(th, 160.0),
            title: "Focus window",
            summary: if window.trim().is_empty() { "No window chosen".into() } else { window.clone() },
        },
        Action::Page { .. } => Meta { glyph: "▤", accent: th.text_dim, title: "Page", summary: String::new() },
        Action::Macro { .. } => Meta { glyph: "☰", accent: th.text_dim, title: "Macro", summary: String::new() },
    }
}

fn is_delay(a: &Action) -> bool {
    matches!(a, Action::Delay { .. })
}

/// (start ms, length ms) per step plus the total, using the engine's rules: 40 ms between steps unless a Wait is
/// next to the gap, ~4 ms per typed character. Waits are exact, the rest are estimates.
fn schedule(steps: &[Action]) -> (Vec<(f32, f32)>, f32) {
    let (mut t, mut out) = (0.0f32, Vec::new());
    for (i, s) in steps.iter().enumerate() {
        if i > 0 && !is_delay(s) && !is_delay(&steps[i - 1]) {
            t += 40.0;
        }
        let len = match s {
            Action::Delay { ms } => *ms as f32,
            Action::Text { text } => text.chars().count() as f32 * 4.0,
            _ => 1.0,
        };
        out.push((t, len));
        t += len;
    }
    (out, t)
}

/// Soft advice that never blocks anything.
fn lint(steps: &[Action], i: usize) -> Option<String> {
    match (&steps[i], steps.get(i + 1)) {
        (Action::Run { .. } | Action::Open { .. }, Some(Action::Hotkey { .. } | Action::Text { .. })) => {
            Some("The app may not be ready yet: add a Wait after this step.".into())
        }
        (Action::Delay { .. }, Some(Action::Delay { .. })) => Some("Two waits in a row: merge them.".into()),
        (Action::Delay { .. }, None) => Some("Nothing runs after this wait.".into()),
        (Action::Text { text }, _) if text.chars().count() > 900 => Some("Long text types slowly (about 4 ms per character).".into()),
        _ => None,
    }
}

fn fade(c: Color32, k: f32) -> Color32 {
    c.gamma_multiply(k)
}

/// A new step of the given kind (index into `PICK`).
const PICK: [(&str, &str, &str); 8] = [
    ("Hotkey", "Press a key combination", "keys shortcut ctrl key combo"),
    ("Type text", "Type a phrase into the focused window", "text write type paste"),
    ("Media", "Volume, mute, play / pause, mic", "volume mute mic music audio sound"),
    ("Program", "Start a program", "run exe launch app program"),
    ("Open", "Open a web page, file or folder", "url web site folder file link open"),
    ("Wait", "Pause before the next step", "delay sleep pause wait time"),
    ("Print", "Write a message to the log", "log debug message print test"),
    ("Focus window", "Bring a window to the front, as if you clicked it", "window focus select switch activate click front app"),
];

fn new_step(kind: usize) -> Action {
    match kind {
        0 => Action::Hotkey { keys: String::new() },
        1 => Action::Text { text: String::new() },
        2 => Action::Media { key: "play_pause".into() },
        3 => Action::Run { command: String::new(), args: vec![] },
        4 => Action::Open { target: String::new() },
        5 => Action::Delay { ms: 250 },
        7 => Action::Focus { window: String::new() },
        _ => Action::Print { text: "step".into() },
    }
}

fn starters() -> Vec<(&'static str, Vec<Action>)> {
    let hk = |k: &str| Action::Hotkey { keys: k.into() };
    vec![
        ("Copy selection", vec![hk("ctrl+c"), Action::Delay { ms: 100 }]),
        ("Paste and submit", vec![hk("ctrl+v"), Action::Delay { ms: 120 }, hk("enter")]),
        (
            "Open app and type",
            vec![Action::Run { command: "notepad.exe".into(), args: vec![] }, Action::Delay { ms: 1500 }, Action::Text { text: "Hello".into() }],
        ),
    ]
}

/// Insert steps at `at` (clamped), respecting the step limit. Returns the index of the last inserted step.
fn insert(steps: &mut Vec<Action>, st: &mut RailState, now: f64, at: usize, new: Vec<Action>) -> Option<usize> {
    if new.is_empty() || steps.len() + new.len() > MACRO_MAX_STEPS {
        return None;
    }
    let at = at.min(steps.len());
    let n = new.len();
    for (k, a) in new.into_iter().enumerate() {
        steps.insert(at + k, a);
    }
    let last = at + n - 1;
    st.selected = Some(last);
    st.flash = Some((last, now));
    st.jump = Some(last);
    st.focus = true;
    Some(last)
}

fn paint_keycaps(p: &egui::Painter, th: &Theme, keys: &str, mut x: f32, y: f32, max_x: f32, k: f32) {
    let font = FontId::monospace(th.small_size);
    for part in keys.split('+').map(str::trim).filter(|s| !s.is_empty()) {
        let label = if part.chars().count() == 1 { part.to_uppercase() } else { part.to_string() };
        let g = p.layout_no_wrap(label, font.clone(), fade(th.text, k));
        let r = Rect::from_min_size(Pos2::new(x, y - g.size().y / 2.0 - 2.0), Vec2::new(g.size().x + 10.0, g.size().y + 4.0));
        if r.max.x > max_x {
            break;
        }
        p.rect_filled(r, CornerRadius::same(3), fade(th.panel, k));
        p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, fade(th.border, k)), StrokeKind::Inside);
        p.line_segment([r.left_bottom() + Vec2::new(2.0, 0.0), r.right_bottom() + Vec2::new(-2.0, 0.0)], Stroke::new(2.0, fade(th.border, k)));
        p.galley(r.min + Vec2::new(5.0, 2.0), g, fade(th.text, k));
        x = r.max.x + 4.0;
    }
}

fn truncate_to(p: &egui::Painter, s: &str, font: &FontId, max_w: f32) -> String {
    let w = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x;
    if w(s) <= max_w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if w(&t) <= max_w {
            return t;
        }
    }
    "…".into()
}

fn tile_btn(ui: &egui::Ui, rect: Rect, id: Id, th: &Theme, kind: u8) -> bool {
    let r = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    let hot = r.hovered();
    if hot {
        p.rect_filled(rect, CornerRadius::same(th.rounding), th.hover);
    }
    let c = if hot && kind == 0 { th.error } else if hot { th.text } else { th.text_dim };
    let m = rect.center();
    if kind == 0 {
        let d = 4.5;
        p.line_segment([m + Vec2::new(-d, -d), m + Vec2::new(d, d)], Stroke::new(1.6, c));
        p.line_segment([m + Vec2::new(-d, d), m + Vec2::new(d, -d)], Stroke::new(1.6, c));
    } else {
        for off in [Vec2::new(-2.0, -2.0), Vec2::new(2.0, 2.0)] {
            p.rect_stroke(Rect::from_center_size(m + off, Vec2::splat(7.0)), CornerRadius::same(1), Stroke::new(1.3, c), StrokeKind::Inside);
        }
    }
    r.on_hover_text(if kind == 0 { "Delete step (Del)" } else { "Duplicate step (Ctrl+D)" }).clicked()
}

/// Draw the whole rail for one macro. `fields` draws the inline editor of the selected step.
pub fn rail(ui: &mut egui::Ui, th: &Theme, slot: &str, steps: &mut Vec<Action>, fields: &mut dyn FnMut(&mut egui::Ui, usize, &mut Action)) {
    let ctx = ui.ctx().clone();
    let id = Id::new(("rail", slot));
    let mut st: RailState = ctx.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let now = ctx.input(|i| i.time);
    let width = ui.available_width();
    let row_h = if th.compact { 30.0 } else { 40.0 };
    let tile = if th.compact { 22.0 } else { 26.0 };

    // clicking outside the rail drops keyboard focus from it
    if let Some(r) = st.last_rect {
        if ctx.input(|i| i.pointer.any_pressed()) && !ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| r.contains(p))) {
            st.focus = false;
        }
    }
    let errors: Vec<Option<String>> = steps.iter().map(|s| actions::validate(s).err()).collect();
    let n_err = errors.iter().filter(|e| e.is_some()).count();
    let n_tip = (0..steps.len()).filter(|&i| lint(steps, i).is_some()).count() + usize::from(steps.len() == 1);
    let (sched, total) = schedule(steps);
    let mut deferred_del: Option<usize> = None;
    let mut deferred_dup: Option<usize> = None;
    let mut deferred_swap: Option<(usize, usize)> = None;
    let mut open_picker: Option<usize> = None;
    let mut test = false;
    let top = ui.cursor().min;

    // keyboard (only while the rail has focus and no text field is being edited)
    if st.focus && !ctx.egui_wants_keyboard_input() {
        let n = steps.len();
        let (alt_up, alt_dn, up, dn, del, dup, k, esc, run) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::ALT, Key::ArrowUp),
                i.consume_key(Modifiers::ALT, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::CTRL, Key::D),
                i.consume_key(Modifiers::CTRL, Key::K),
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::CTRL, Key::Enter),
            )
        });
        if let Some(s) = st.selected.filter(|&s| s < n) {
            if alt_up && s > 0 {
                deferred_swap = Some((s, s - 1));
            }
            if alt_dn && s + 1 < n {
                deferred_swap = Some((s, s + 1));
            }
            if del {
                deferred_del = Some(s);
            }
            if dup {
                deferred_dup = Some(s);
            }
            if up && s > 0 {
                st.selected = Some(s - 1);
                st.jump = Some(s - 1);
            }
            if dn && s + 1 < n {
                st.selected = Some(s + 1);
                st.jump = Some(s + 1);
            }
            if esc {
                st.selected = None;
            }
        } else if (dn || up) && n > 0 {
            st.selected = Some(if dn { 0 } else { n - 1 });
        }
        if k {
            open_picker = Some(st.selected.map_or(n, |s| s + 1));
        }
        if run {
            test = true;
        }
    }

    // header: counts, estimate, problems pill, test run
    ui.horizontal(|ui| {
        ui.strong(format!("{} step{}", steps.len(), if steps.len() == 1 { "" } else { "s" }));
        if !steps.is_empty() {
            ui.label(RichText::new(format!("· about {}", fmt_ms(total))).size(th.small_size).color(th.text_dim));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let running = st.run_start.is_some();
            if ui.add_enabled(!steps.is_empty() && n_err == 0 && !running, egui::Button::new("▶ Test run")).on_hover_text("Runs the steps for real after 3 seconds (Ctrl+Enter)").clicked() {
                test = true;
            }
            let (txt, col) = if n_err > 0 {
                (format!("{n_err} problem{}", if n_err == 1 { "" } else { "s" }), th.error)
            } else if n_tip > 0 {
                (format!("{n_tip} tip{}", if n_tip == 1 { "" } else { "s" }), th.warn)
            } else {
                ("No problems".to_string(), th.ok)
            };
            let b = ui.add(egui::Button::new(RichText::new(txt).size(th.small_size).color(col)).small());
            if b.clicked() {
                st.selected = errors.iter().position(|e| e.is_some()).or_else(|| (0..steps.len()).find(|&i| lint(steps, i).is_some()));
                st.jump = st.selected;
            }
        });
    });
    if test && st.run_start.is_none() && !steps.is_empty() {
        let a = Action::Macro { steps: steps.clone() };
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(TEST_DELAY_S));
            let _ = actions::execute(&a);
        });
        st.run_start = Some(now);
    }
    // test-run progress (estimate driven)
    let mut run_ms: Option<f32> = None;
    if let Some(t0) = st.run_start {
        let e = ((now - t0 - TEST_DELAY_S) * 1000.0) as f32;
        if e > total + 400.0 {
            st.run_start = None;
        } else {
            run_ms = Some(e);
            ctx.request_repaint();
        }
    }
    let running_idx = run_ms.filter(|&e| e >= 0.0).and_then(|e| sched.iter().rposition(|(s, _)| *s <= e));

    // one-bar timeline
    let bar_h = if th.compact { 10.0 } else { 14.0 };
    let (bar, bar_resp) = ui.allocate_exact_size(Vec2::new(width, bar_h), Sense::click());
    ui.painter().rect_filled(bar, CornerRadius::same(3), th.input);
    let mut seg_x: Vec<(f32, f32)> = Vec::new();
    if !steps.is_empty() {
        let raw: Vec<f32> = sched.iter().map(|(_, l)| (l / total.max(1.0)) * bar.width()).collect();
        let mut w: Vec<f32> = raw.iter().map(|r| r.max(6.0)).collect();
        let sum: f32 = w.iter().sum::<f32>() + (w.len().saturating_sub(1)) as f32;
        if sum > bar.width() {
            let k = (bar.width() - w.len() as f32) / w.iter().sum::<f32>();
            for x in &mut w {
                *x *= k.max(0.05);
            }
        }
        let mut x = bar.min.x;
        for (i, wi) in w.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, bar.min.y), Vec2::new(*wi, bar_h));
            let m = meta(&steps[i], th);
            let col = if errors[i].is_some() { th.error } else { m.accent };
            ui.painter().rect_filled(r.shrink2(Vec2::new(0.0, 1.0)), CornerRadius::same(2), fade(col, if is_delay(&steps[i]) { 0.7 } else { 0.55 }));
            if st.selected == Some(i) {
                ui.painter().rect_stroke(r, CornerRadius::same(2), Stroke::new(1.5, th.select), StrokeKind::Inside);
            }
            seg_x.push((r.min.x, r.max.x));
            x += wi + 1.0;
        }
        if let Some(e) = run_ms.filter(|&e| e >= 0.0) {
            if let Some(i) = running_idx {
                let (s, l) = sched[i];
                let f = ((e - s) / l.max(1.0)).clamp(0.0, 1.0);
                let px = seg_x[i].0 + (seg_x[i].1 - seg_x[i].0) * f;
                ui.painter().line_segment([Pos2::new(px, bar.min.y - 2.0), Pos2::new(px, bar.max.y + 2.0)], Stroke::new(2.0, th.press));
            }
        }
        if let Some(p) = bar_resp.hover_pos() {
            if let Some(i) = seg_x.iter().position(|(a, b)| p.x >= *a - 0.5 && p.x <= *b + 0.5) {
                let m = meta(&steps[i], th);
                let tip = format!("Step {} · {} · starts at +{} · lasts {}", i + 1, m.title, fmt_ms(sched[i].0), fmt_ms(sched[i].1));
                bar_resp.clone().on_hover_text_at_pointer(tip);
                if bar_resp.clicked() {
                    st.selected = Some(i);
                    st.jump = Some(i);
                    st.focus = true;
                }
            }
        }
    }
    if let Some(e) = run_ms.filter(|&e| e < 0.0) {
        ui.label(RichText::new(format!("Starting in {:.0} s: click into the window where it should happen.", (-e / 1000.0).ceil())).size(th.small_size).color(th.warn));
    }
    ui.add_space(4.0);

    // start cap
    ui.label(RichText::new("▶  When pressed").size(th.small_size).color(th.text_dim));
    let line_idx = ui.painter().add(egui::Shape::Noop);
    let line_x = ui.cursor().min.x + 8.0 + 22.0 + 24.0 + tile / 2.0 - 12.0;
    let line_top = ui.cursor().min.y;

    // rows
    let mut row_rects: Vec<Rect> = Vec::new();
    let mut gap_ys: Vec<f32> = Vec::new();
    if steps.is_empty() {
        egui::Frame::new().stroke(Stroke::new(th.border_width.max(1.0), th.border)).corner_radius(CornerRadius::same(th.card_rounding)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("No steps yet. A macro runs its steps one after another.").color(th.text_dim));
            ui.horizontal(|ui| {
                if ui.button("Add first step").clicked() {
                    open_picker = Some(0);
                }
                ui.menu_button("Start from an example", |ui| {
                    for (name, list) in starters() {
                        if ui.button(name).clicked() {
                            insert(steps, &mut st, now, 0, list);
                            ui.close();
                        }
                    }
                });
            });
        });
    }
    for i in 0..=steps.len() {
        // gap: hover to reveal "+", click to insert here
        let (g, gr) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::click());
        gap_ys.push(g.center().y);
        let hot = gr.hovered() && st.drag.is_none();
        if st.drag.is_some() {
            // handled after the loop
        } else if hot && steps.len() < MACRO_MAX_STEPS {
            let p = ui.painter();
            p.line_segment([Pos2::new(g.min.x + 40.0, g.center().y), Pos2::new(g.max.x - 8.0, g.center().y)], Stroke::new(1.0, fade(th.accent, 0.6)));
            p.circle_filled(Pos2::new(line_x + 12.0, g.center().y), 7.0, th.accent);
            p.text(Pos2::new(line_x + 12.0, g.center().y - 0.5), Align2::CENTER_CENTER, "+", FontId::proportional(12.0), th.accent_text);
        }
        if gr.on_hover_text("Insert a step here").clicked() {
            open_picker = Some(i);
        }
        if i == steps.len() {
            break;
        }

        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click_and_drag());
        row_rects.push(rect);
        let m = meta(&steps[i], th);
        let k = if st.drag == Some(i) { 0.35 } else { 1.0 };
        let sel = st.selected == Some(i);
        // pointer inside the row, even when the row's own delete / duplicate buttons are on top of it (resp.hovered() turns
        // false then, the tools would vanish a frame later and the click on the X would never land)
        let hover = st.drag.is_none() && ui.input(|i| i.pointer.hover_pos()).is_some_and(|pp| rect.contains(pp)) && ui.rect_contains_pointer(rect);
        let bad = errors[i].is_some();
        let tip = if bad { None } else { lint(steps, i) };
        let running = running_idx == Some(i);
        let p = ui.painter();
        let cr = CornerRadius::same(th.card_rounding);
        let mut fill = if hover || sel { th.hover } else { th.card };
        if running {
            fill = th.card.lerp_to_gamma(th.press, 0.25);
        }
        p.rect_filled(rect, cr, fade(fill, k));
        if let Some((fi, t0)) = st.flash {
            if fi == i {
                let a = (1.0 - (now - t0) / 0.5).clamp(0.0, 1.0) as f32;
                if a > 0.0 {
                    p.rect_filled(rect, cr, fade(th.accent, 0.35 * a));
                    ctx.request_repaint();
                }
            }
        }
        let (bc, bw) = if bad {
            (th.error, th.border_width.max(1.0))
        } else if running {
            (th.press, 1.5)
        } else if sel {
            (th.select, 1.5)
        } else {
            (th.border, th.border_width)
        };
        if bw > 0.0 {
            p.rect_stroke(rect, cr, Stroke::new(bw, fade(bc, k)), StrokeKind::Inside);
        }
        let cy = rect.center().y;
        // grip (on hover), number
        if hover || sel {
            for dx in [0.0, 4.0] {
                for dy in [-5.0, 0.0, 5.0] {
                    p.circle_filled(Pos2::new(rect.min.x + 9.0 + dx, cy + dy), 1.1, th.text_dim);
                }
            }
        }
        let numx = rect.min.x + 22.0;
        p.text(Pos2::new(numx, cy), Align2::LEFT_CENTER, format!("{:02}", i + 1), FontId::monospace(th.small_size), fade(th.text_dim, k));
        // icon tile
        let ac = if bad { th.error } else { m.accent };
        let tr = Rect::from_center_size(Pos2::new(rect.min.x + 22.0 + 24.0 + tile / 2.0, cy), Vec2::splat(tile));
        p.rect_filled(tr, CornerRadius::same(th.rounding), fade(ac, 0.18 * k));
        p.rect_stroke(tr, CornerRadius::same(th.rounding), Stroke::new(1.0, fade(ac, 0.4 * k)), StrokeKind::Inside);
        p.text(tr.center(), Align2::CENTER_CENTER, m.glyph, FontId::proportional(tile * 0.55), fade(ac, k));
        // right side: tools, then timing chip
        let tools_on = hover || sel;
        let mut right = rect.max.x - 6.0;
        let (mut do_del, mut do_dup) = (false, false);
        if tools_on {
            let d = Rect::from_center_size(Pos2::new(right - 11.0, cy), Vec2::splat(22.0));
            do_del = tile_btn(ui, d, id.with(("del", i)), th, 0);
            let u = Rect::from_center_size(Pos2::new(right - 11.0 - 24.0, cy), Vec2::splat(22.0));
            do_dup = tile_btn(ui, u, id.with(("dup", i)), th, 1);
            right -= 52.0;
        }
        let p = ui.painter();
        if bad || tip.is_some() {
            let (g, c) = if bad { ("⚠", th.error) } else { ("ⓘ", th.warn) };
            p.text(Pos2::new(right - 8.0, cy), Align2::CENTER_CENTER, g, FontId::proportional(13.0), fade(c, k));
            let br = Rect::from_center_size(Pos2::new(right - 8.0, cy), Vec2::splat(16.0));
            ui.interact(br, id.with(("badge", i)), Sense::hover()).on_hover_text(errors[i].clone().or(tip.clone()).unwrap_or_default());
            right -= 20.0;
        }
        let p = ui.painter();
        let chip = match &steps[i] {
            Action::Delay { ms } => Some((fmt_ms(*ms as f32), th.warn)),
            Action::Text { text } if !text.is_empty() => Some((format!("~{}", fmt_ms(text.chars().count() as f32 * 4.0)), th.text_dim)),
            _ => None,
        };
        if let Some((txt, c)) = chip {
            let g = p.layout_no_wrap(txt, FontId::monospace(th.small_size), fade(c, k));
            let cr2 = Rect::from_min_size(Pos2::new(right - g.size().x - 12.0, cy - g.size().y / 2.0 - 2.0), Vec2::new(g.size().x + 10.0, g.size().y + 4.0));
            p.rect_stroke(cr2, CornerRadius::same(th.rounding), Stroke::new(1.0, fade(c, 0.5 * k)), StrokeKind::Inside);
            p.galley(cr2.min + Vec2::new(5.0, 2.0), g, fade(c, k));
            right = cr2.min.x - 6.0;
        }
        // title + summary
        let tx = tr.max.x + 8.0;
        let font_t = FontId::proportional(th.body_size);
        let font_s = FontId::proportional(th.small_size);
        let sum_col = if bad { th.error } else { th.text_dim };
        let shown = errors[i].clone().unwrap_or_else(|| m.summary.clone());
        let clip = ui.painter().with_clip_rect(Rect::from_min_max(rect.min, Pos2::new(right, rect.max.y)));
        if th.compact {
            let tw = clip.text(Pos2::new(tx, cy), Align2::LEFT_CENTER, m.title, font_t, fade(th.text, k)).width();
            let sx = tx + tw + 8.0;
            if matches!(steps[i], Action::Hotkey { .. }) && !bad && !m.summary.starts_with("No keys") {
                if let Action::Hotkey { keys } = &steps[i] {
                    paint_keycaps(&clip, th, keys, sx, cy, right, k);
                }
            } else {
                let s = truncate_to(&clip, &shown, &font_s, (right - sx).max(10.0));
                clip.text(Pos2::new(sx, cy), Align2::LEFT_CENTER, s, font_s, fade(sum_col, k));
            }
        } else {
            clip.text(Pos2::new(tx, cy - 8.0), Align2::LEFT_CENTER, m.title, font_t, fade(th.text, k));
            if matches!(steps[i], Action::Hotkey { .. }) && !bad && !m.summary.starts_with("No keys") {
                if let Action::Hotkey { keys } = &steps[i] {
                    paint_keycaps(&clip, th, keys, tx, cy + 9.0, right, k);
                }
            } else {
                let s = truncate_to(&clip, &shown, &font_s, (right - tx).max(10.0));
                clip.text(Pos2::new(tx, cy + 8.0), Align2::LEFT_CENTER, s, font_s, fade(sum_col, k));
            }
        }
        // interactions
        if do_del {
            deferred_del = Some(i);
        } else if do_dup {
            deferred_dup = Some(i);
        } else if resp.clicked() {
            st.selected = if sel { None } else { Some(i) };
            st.focus = true;
        }
        if resp.drag_started() {
            st.drag = Some(i);
            st.selected = Some(i);
            st.focus = true;
        }
        if st.jump == Some(i) {
            ui.scroll_to_rect(rect, None);
            st.jump = None;
        }
        // inline editor
        if st.selected == Some(i) && st.drag.is_none() {
            egui::Frame::new()
                .fill(th.input)
                .stroke(Stroke::new(th.border_width, th.border))
                .corner_radius(CornerRadius::same(th.card_rounding))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    fields(ui, i, &mut steps[i]);
                    if let Some(e) = &errors[i] {
                        ui.colored_label(th.error, e);
                    } else if let Some(t) = &tip {
                        ui.colored_label(th.warn, t);
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(i > 0, egui::Button::new("Move up").small()).clicked() {
                            deferred_swap = Some((i, i - 1));
                        }
                        if ui.add_enabled(i + 1 < steps.len(), egui::Button::new("Move down").small()).clicked() {
                            deferred_swap = Some((i, i + 1));
                        }
                        if ui.add_enabled(steps.len() < MACRO_MAX_STEPS, egui::Button::new("Duplicate").small()).clicked() {
                            deferred_dup = Some(i);
                        }
                        if ui.add(egui::Button::new(RichText::new("Delete").color(th.error)).small()).clicked() {
                            deferred_del = Some(i);
                        }
                    });
                });
        }
    }

    // drag reorder: insertion line at the nearest gap, drop on release
    if let Some(from) = st.drag {
        let py = ctx.input(|i| i.pointer.interact_pos()).map(|p| p.y);
        if let Some(py) = py {
            let mut to = 0usize;
            for (gi, gy) in gap_ys.iter().enumerate() {
                if (py - gy).abs() < (py - gap_ys[to]).abs() {
                    to = gi;
                }
            }
            let gy = gap_ys[to];
            let x0 = top.x;
            ui.painter().line_segment([Pos2::new(x0 + 30.0, gy), Pos2::new(x0 + width - 6.0, gy)], Stroke::new(2.0, th.accent));
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            if ctx.input(|i| i.pointer.any_released()) {
                let dest = if to > from { to - 1 } else { to };
                if dest != from && from < steps.len() {
                    let s = steps.remove(from);
                    steps.insert(dest, s);
                    st.selected = Some(dest);
                    st.flash = Some((dest, now));
                }
                st.drag = None;
            }
        } else {
            st.drag = None;
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            st.drag = None;
        }
    }

    // end cap + add
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("✓  Done").size(th.small_size).color(th.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if steps.len() >= MACRO_MAX_STEPS {
                ui.add_enabled(false, egui::Button::new(format!("{}/{} steps", steps.len(), MACRO_MAX_STEPS)));
            } else if ui.button("+ Add step").on_hover_text("Search for a step (Ctrl+K)").clicked() {
                open_picker = Some(steps.len());
            }
        });
    });
    let bottom = ui.cursor().min.y;
    ui.painter().set(line_idx, egui::Shape::line_segment([Pos2::new(line_x, line_top), Pos2::new(line_x, bottom - 24.0)], Stroke::new(2.0, fade(th.border, 0.8))));
    st.last_rect = Some(Rect::from_min_max(top, Pos2::new(top.x + width, bottom)));

    // deferred edits
    if let Some(n) = deferred_del.filter(|&n| n < steps.len()) {
        steps.remove(n);
        st.selected = if steps.is_empty() { None } else { Some(n.min(steps.len() - 1)) };
    } else if let Some(n) = deferred_dup.filter(|&n| n < steps.len()) {
        if steps.len() < MACRO_MAX_STEPS {
            let c = steps[n].clone();
            insert(steps, &mut st, now, n + 1, vec![c]);
        }
    } else if let Some((a, b)) = deferred_swap {
        steps.swap(a, b);
        st.selected = Some(b);
        st.jump = Some(b);
    }
    if let Some(at) = open_picker {
        st.picker = Some(at);
        st.query.clear();
        st.fresh = true;
    }
    picker(&ctx, th, id, &mut st, steps, now);
    if st.focus {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| d.insert_temp(Id::new("rail_active"), pass));
    }
    ctx.data_mut(|d| d.insert_temp(id, st));
}

/// True while a macro rail owns the keyboard (the deck-tile shortcuts then stay out of the way).
pub fn rail_active(ctx: &egui::Context) -> bool {
    let now = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(Id::new("rail_active"))).is_some_and(|n| n + 2 >= now)
}

fn picker(ctx: &egui::Context, th: &Theme, id: Id, st: &mut RailState, steps: &mut Vec<Action>, now: f64) {
    let Some(mut at) = st.picker else { return };
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    let q = st.query.to_lowercase();
    let hits: Vec<usize> = (0..PICK.len()).filter(|&k| q.is_empty() || PICK[k].0.to_lowercase().contains(&q) || PICK[k].2.contains(&q)).collect();
    let mut chosen: Option<Vec<Action>> = None;
    let mut keep_open = false;
    let full = steps.len() >= MACRO_MAX_STEPS;
    egui::Window::new("Add a step").id(id.with("picker")).anchor(Align2::CENTER_CENTER, Vec2::ZERO).collapsible(false).resizable(false).fixed_size(Vec2::new(360.0, 0.0)).show(ctx, |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("Search: wait, keys, open, volume…").desired_width(f32::INFINITY));
        if st.fresh {
            r.request_focus();
            st.fresh = false;
        }
        if full {
            ui.colored_label(th.error, format!("A macro can have at most {MACRO_MAX_STEPS} steps."));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Quick wait").size(th.small_size).color(th.text_dim));
            for (label, ms) in [("100", 100u32), ("250", 250), ("500", 500), ("1 s", 1000)] {
                if ui.add_enabled(!full, egui::Button::new(label).small()).clicked() {
                    chosen = Some(vec![Action::Delay { ms }]);
                    keep_open = true;
                }
            }
        });
        ui.separator();
        if hits.is_empty() {
            ui.label(RichText::new("No step called that. Try \"wait\", \"keys\" or \"open\".").color(th.text_dim));
        }
        for &k in &hits {
            let m = meta(&new_step(k), th);
            let resp = ui.add_enabled(!full, egui::Button::new(RichText::new(format!("{}  {}", m.glyph, PICK[k].0)).color(m.accent).strong()).min_size(Vec2::new(ui.available_width(), 30.0)));
            let resp = resp.on_hover_text(PICK[k].1);
            ui.label(RichText::new(PICK[k].1).size(th.small_size).color(th.text_dim));
            if resp.clicked() {
                chosen = Some(vec![new_step(k)]);
            }
        }
        if q.is_empty() {
            ui.separator();
            ui.label(RichText::new("Starters").size(th.small_size).color(th.text_dim));
            ui.horizontal_wrapped(|ui| {
                for (name, list) in starters() {
                    if ui.add_enabled(steps.len() + list.len() <= MACRO_MAX_STEPS, egui::Button::new(name).small()).clicked() {
                        chosen = Some(list);
                    }
                }
            });
        }
        if ui.input(|i| i.key_pressed(Key::Enter)) && chosen.is_none() && !full {
            if let Some(&k) = hits.first() {
                chosen = Some(vec![new_step(k)]);
            }
        }
    });
    if let Some(list) = chosen {
        let n = list.len();
        if insert(steps, st, now, at, list).is_some() {
            at += n;
        }
        if keep_open {
            st.picker = Some(at);
        } else {
            close = true;
        }
    }
    if close {
        st.picker = None;
    }
}

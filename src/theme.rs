//! Window styles ("skins"): a style is a small TOML file (optionally in a folder with a font and a background image) that
//! recolors the window, changes shapes, spacing and font sizes, and rearranges the layout (inspector side, density, tabs).
//! Drop one into `<config folder>\styles\` (as `name.toml` or `name\style.toml`) and pick it in Settings; edits to the file
//! are picked up live. Anything a style leaves out falls back to its `base` ("dark" or "light").

use eframe::egui::{self, Color32};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabStyle {
    Pills,
    Underline,
    Boxed,
}

/// Fully resolved style: every value is concrete.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub author: String,
    pub description: String,
    pub dark: bool,
    // colors
    pub window: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub input: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub border: Color32,
    pub hover: Color32,
    pub select: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub tile_empty: Color32,
    pub tile_border: Color32,
    pub press: Color32,
    // shape and size
    pub rounding: u8,
    pub tile_rounding: f32,
    pub card_rounding: u8,
    pub border_width: f32,
    pub tile_size: f32,
    pub tile_gap: f32,
    // text
    pub body_size: f32,
    pub heading_size: f32,
    pub small_size: f32,
    pub monospace: bool,
    pub font_path: Option<PathBuf>,
    // layout
    pub inspector_left: bool,
    pub inspector_width: f32,
    pub compact: bool,
    pub tabs: TabStyle,
    pub deck_left: bool,
    pub show_hints: bool,
    pub bg_image: Option<PathBuf>,
    pub bg_opacity: f32,
}

fn c(hex: &str) -> Color32 {
    parse_color(hex).unwrap_or(Color32::MAGENTA)
}

/// `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        6 => Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)),
        8 => Some(Color32::from_rgba_unmultiplied((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)),
        _ => None,
    }
}

impl Theme {
    pub fn base(dark: bool) -> Theme {
        if dark {
            Theme {
                name: "Dark".into(),
                author: String::new(),
                description: "The default dark style".into(),
                dark,
                window: c("#0b0e14"),
                panel: c("#111722"),
                card: c("#172131"),
                input: c("#0c121b"),
                text: c("#e8edf5"),
                text_dim: c("#8a98ad"),
                accent: c("#7c9cff"),
                accent_text: c("#08101f"),
                border: c("#263244"),
                hover: c("#202d44"),
                select: c("#9db4ff"),
                ok: c("#50d6bd"),
                warn: c("#ffb86b"),
                error: c("#ff7185"),
                tile_empty: c("#141b27"),
                tile_border: c("#2a3648"),
                press: c("#ffd479"),
                rounding: 9,
                tile_rounding: 13.0,
                card_rounding: 14,
                border_width: 1.0,
                tile_size: 92.0,
                tile_gap: 8.0,
                body_size: 14.0,
                heading_size: 18.0,
                small_size: 11.5,
                monospace: false,
                font_path: None,
                inspector_left: false,
                inspector_width: 400.0,
                compact: false,
                tabs: TabStyle::Pills,
                deck_left: false,
                show_hints: true,
                bg_image: None,
                bg_opacity: 0.25,
            }
        } else {
            Theme {
                name: "Light".into(),
                description: "The default light style".into(),
                dark,
                window: c("#f3f4f6"),
                panel: c("#ffffff"),
                card: c("#f0f2f5"),
                input: c("#ffffff"),
                text: c("#1f2328"),
                text_dim: c("#6b7280"),
                accent: c("#0969da"),
                accent_text: c("#ffffff"),
                border: c("#d0d7de"),
                hover: c("#e6eaf0"),
                select: c("#0969da"),
                ok: c("#1a7f37"),
                warn: c("#9a6700"),
                error: c("#cf222e"),
                tile_empty: c("#e4e7eb"),
                tile_border: c("#c4cad2"),
                press: c("#bf8700"),
                ..Theme::base(true)
            }
        }
    }
}

// ---- the style file ----

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StyleFile {
    name: Option<String>,
    author: Option<String>,
    description: Option<String>,
    base: Option<String>,
    colors: Colors,
    shape: Shape,
    text: Text,
    layout: Layout,
    background: Background,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Colors {
    window: Option<String>,
    panel: Option<String>,
    card: Option<String>,
    input: Option<String>,
    text: Option<String>,
    text_dim: Option<String>,
    accent: Option<String>,
    accent_text: Option<String>,
    border: Option<String>,
    hover: Option<String>,
    select: Option<String>,
    ok: Option<String>,
    warn: Option<String>,
    error: Option<String>,
    tile_empty: Option<String>,
    tile_border: Option<String>,
    press: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Shape {
    rounding: Option<f32>,
    tile_rounding: Option<f32>,
    card_rounding: Option<f32>,
    border_width: Option<f32>,
    tile_size: Option<f32>,
    tile_gap: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Text {
    font: Option<String>,
    family: Option<String>,
    body_size: Option<f32>,
    heading_size: Option<f32>,
    small_size: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Layout {
    inspector: Option<String>,
    inspector_width: Option<f32>,
    density: Option<String>,
    tabs: Option<String>,
    deck_align: Option<String>,
    hints: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Background {
    image: Option<String>,
    opacity: Option<f32>,
}

impl Theme {
    /// Parse a style file. `dir` is the folder that relative paths (font, background image) are resolved against.
    pub fn parse(text: &str, dir: Option<&Path>) -> Result<Theme, String> {
        let f: StyleFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let dark = match f.base.as_deref().unwrap_or("dark") {
            "dark" => true,
            "light" => false,
            other => return Err(format!("base must be \"dark\" or \"light\", not {other:?}")),
        };
        let mut t = Theme::base(dark);
        if let Some(n) = f.name {
            t.name = n;
        }
        t.author = f.author.unwrap_or_default();
        t.description = f.description.unwrap_or_default();
        macro_rules! color {
            ($($field:ident),*) => {$(
                if let Some(v) = &f.colors.$field {
                    t.$field = parse_color(v).ok_or_else(|| format!("colors.{}: {v:?} must be #rrggbb or #rrggbbaa", stringify!($field)))?;
                }
            )*};
        }
        color!(window, panel, card, input, text, text_dim, accent, accent_text, border, hover, select, ok, warn, error, tile_empty, tile_border, press);
        let num = |name: &str, v: Option<f32>, lo: f32, hi: f32, cur: f32| -> Result<f32, String> {
            match v {
                None => Ok(cur),
                Some(x) if (lo..=hi).contains(&x) => Ok(x),
                Some(x) => Err(format!("{name}: {x} is out of range ({lo} to {hi})")),
            }
        };
        t.rounding = num("shape.rounding", f.shape.rounding, 0.0, 24.0, t.rounding as f32)? as u8;
        t.tile_rounding = num("shape.tile_rounding", f.shape.tile_rounding, 0.0, 40.0, t.tile_rounding)?;
        t.card_rounding = num("shape.card_rounding", f.shape.card_rounding, 0.0, 24.0, t.card_rounding as f32)? as u8;
        t.border_width = num("shape.border_width", f.shape.border_width, 0.0, 4.0, t.border_width)?;
        t.tile_size = num("shape.tile_size", f.shape.tile_size, 56.0, 150.0, t.tile_size)?;
        t.tile_gap = num("shape.tile_gap", f.shape.tile_gap, 0.0, 30.0, t.tile_gap)?;
        t.body_size = num("text.body_size", f.text.body_size, 9.0, 24.0, t.body_size)?;
        t.heading_size = num("text.heading_size", f.text.heading_size, 10.0, 40.0, t.heading_size)?;
        t.small_size = num("text.small_size", f.text.small_size, 8.0, 20.0, t.small_size)?;
        t.inspector_width = num("layout.inspector_width", f.layout.inspector_width, 300.0, 640.0, t.inspector_width)?;
        t.bg_opacity = num("background.opacity", f.background.opacity, 0.0, 1.0, t.bg_opacity)?;
        t.monospace = match f.text.family.as_deref().unwrap_or("proportional") {
            "proportional" => false,
            "monospace" => true,
            other => return Err(format!("text.family must be \"proportional\" or \"monospace\", not {other:?}")),
        };
        t.inspector_left = match f.layout.inspector.as_deref().unwrap_or("right") {
            "right" => false,
            "left" => true,
            other => return Err(format!("layout.inspector must be \"left\" or \"right\", not {other:?}")),
        };
        t.compact = match f.layout.density.as_deref().unwrap_or("comfortable") {
            "comfortable" => false,
            "compact" => true,
            other => return Err(format!("layout.density must be \"comfortable\" or \"compact\", not {other:?}")),
        };
        t.tabs = match f.layout.tabs.as_deref().unwrap_or("pills") {
            "pills" => TabStyle::Pills,
            "underline" => TabStyle::Underline,
            "boxed" => TabStyle::Boxed,
            other => return Err(format!("layout.tabs must be \"pills\", \"underline\" or \"boxed\", not {other:?}")),
        };
        t.deck_left = match f.layout.deck_align.as_deref().unwrap_or("center") {
            "center" => false,
            "left" => true,
            other => return Err(format!("layout.deck_align must be \"center\" or \"left\", not {other:?}")),
        };
        t.show_hints = f.layout.hints.unwrap_or(true);
        let rel = |p: &str| -> PathBuf {
            let p = PathBuf::from(p);
            match dir {
                Some(d) if p.is_relative() => d.join(p),
                _ => p,
            }
        };
        if let Some(p) = f.text.font.as_deref() {
            let full = rel(p);
            if !full.exists() {
                return Err(format!("text.font: {} does not exist", full.display()));
            }
            t.font_path = Some(full);
        }
        if let Some(p) = f.background.image.as_deref() {
            let full = rel(p);
            if !full.exists() {
                return Err(format!("background.image: {} does not exist", full.display()));
            }
            t.bg_image = Some(full);
        }
        Ok(t)
    }

    /// Push the colors, shapes, spacing and font sizes into egui.
    pub fn apply(&self, ctx: &egui::Context) {
        use egui::{CornerRadius, FontFamily, FontId, Stroke, TextStyle};
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.extreme_bg_color = self.input;
        v.faint_bg_color = self.card;
        v.code_bg_color = self.input;
        v.override_text_color = Some(self.text);
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.warn;
        v.error_fg_color = self.error;
        v.selection.bg_fill = self.accent;
        v.selection.stroke = Stroke::new(1.0, self.accent_text);
        v.window_stroke = Stroke::new(self.border_width, self.border);
        v.window_corner_radius = CornerRadius::same(self.card_rounding);
        v.menu_corner_radius = CornerRadius::same(self.rounding);
        let r = CornerRadius::same(self.rounding);
        let line = Stroke::new(self.border_width, self.border);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = line;
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.bg_fill = self.card;
        w.inactive.weak_bg_fill = self.card;
        w.inactive.bg_stroke = if self.border_width > 0.0 { Stroke::new(self.border_width, self.border) } else { Stroke::NONE };
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.bg_fill = self.hover;
        w.hovered.weak_bg_fill = self.hover;
        w.hovered.bg_stroke = Stroke::new(self.border_width.max(1.0), self.select.gamma_multiply(0.55));
        w.hovered.fg_stroke = Stroke::new(1.5, self.text);
        w.active.bg_fill = self.accent;
        w.active.weak_bg_fill = self.accent;
        w.active.bg_stroke = Stroke::new(self.border_width.max(1.0), self.accent);
        w.active.fg_stroke = Stroke::new(1.5, self.text);
        w.open.bg_fill = self.hover;
        w.open.weak_bg_fill = self.hover;
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = r;
        }
        ctx.set_visuals(v);

        ctx.all_styles_mut(|s| {
            let (item, pad, indent) = if self.compact { (egui::vec2(6.0, 3.0), egui::vec2(6.0, 2.0), 14.0) } else { (egui::vec2(8.0, 8.0), egui::vec2(12.0, 5.0), 18.0) };
            s.spacing.item_spacing = item;
            s.spacing.button_padding = pad;
            s.spacing.indent = indent;
            s.spacing.interact_size.y = if self.compact { 20.0 } else { 24.0 };
            let fam = if self.monospace { FontFamily::Monospace } else { FontFamily::Proportional };
            s.text_styles.insert(TextStyle::Body, FontId::new(self.body_size, fam.clone()));
            s.text_styles.insert(TextStyle::Button, FontId::new(self.body_size, fam.clone()));
            s.text_styles.insert(TextStyle::Heading, FontId::new(self.heading_size, fam.clone()));
            s.text_styles.insert(TextStyle::Small, FontId::new(self.small_size, fam.clone()));
            s.text_styles.insert(TextStyle::Monospace, FontId::new(self.body_size, FontFamily::Monospace));
        });
    }
}

// ---- finding, loading and watching styles ----

#[derive(Debug, Clone)]
pub struct Entry {
    /// What is stored in ui.toml: "dark", "light", or a file / folder name.
    pub id: String,
    pub name: String,
    pub path: Option<PathBuf>,
}

pub struct Styles {
    pub dir: PathBuf,
    ui_path: PathBuf,
    pub selected: String,
    pub theme: Theme,
    pub error: Option<String>,
    pub list: Vec<Entry>,
    watch: Option<(PathBuf, SystemTime)>,
    last_poll: Instant,
    /// Set when the theme changed since the GUI last looked (it then re-applies fonts and visuals).
    pub changed: bool,
}

const BUNDLED: &[(&str, &str)] = &[
    ("midnight.toml", include_str!("../styles/midnight.toml")),
    ("paper.toml", include_str!("../styles/paper.toml")),
    ("terminal.toml", include_str!("../styles/terminal.toml")),
    ("README.txt", include_str!("../styles/README.txt")),
];

pub const TEMPLATE: &str = include_str!("../styles/template.toml");

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Styles {
    pub fn new(config_path: &Path) -> Styles {
        let base = config_path.parent().map(Path::to_path_buf).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("styles");
        if !dir.exists() && std::fs::create_dir_all(&dir).is_ok() {
            for (name, text) in BUNDLED {
                let _ = std::fs::write(dir.join(name), text);
            }
        }
        let ui_path = base.join("ui.toml");
        let selected = std::fs::read_to_string(&ui_path)
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
            .and_then(|t| t.get("style").and_then(|v| v.as_str().map(String::from)))
            .unwrap_or_else(|| "dark".into());
        let mut s = Styles { dir, ui_path, selected: selected.clone(), theme: Theme::base(true), error: None, list: Vec::new(), watch: None, last_poll: Instant::now(), changed: true };
        s.refresh_list();
        s.load(&selected);
        s
    }

    pub fn refresh_list(&mut self) {
        let mut list = vec![
            Entry { id: "dark".into(), name: "Dark (built in)".into(), path: None },
            Entry { id: "light".into(), name: "Light (built in)".into(), path: None },
        ];
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            let mut found: Vec<Entry> = rd
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let (id, file) = if p.is_dir() {
                        (e.file_name().to_string_lossy().to_string(), p.join("style.toml"))
                    } else if p.extension().is_some_and(|x| x == "toml") {
                        (p.file_stem()?.to_string_lossy().to_string(), p.clone())
                    } else {
                        return None;
                    };
                    file.exists().then(|| {
                        let name = std::fs::read_to_string(&file)
                            .ok()
                            .and_then(|t| t.parse::<toml::Table>().ok())
                            .and_then(|t| t.get("name").and_then(|v| v.as_str().map(String::from)))
                            .unwrap_or_else(|| id.clone());
                        Entry { id, name, path: Some(file) }
                    })
                })
                .collect();
            found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            list.extend(found);
        }
        self.list = list;
    }

    fn load(&mut self, id: &str) {
        let entry = self.list.iter().find(|e| e.id == id).cloned();
        let result = match &entry {
            None if id == "dark" || id == "light" => Ok(Theme::base(id == "dark")),
            None => Err(format!("style {id:?} was not found in {}", self.dir.display())),
            Some(e) => match &e.path {
                None => Ok(Theme::base(e.id == "dark")),
                Some(p) => std::fs::read_to_string(p).map_err(|er| er.to_string()).and_then(|t| Theme::parse(&t, p.parent())).map_err(|er| format!("{}: {er}", p.display())),
            },
        };
        self.watch = entry.and_then(|e| e.path).and_then(|p| mtime(&p).map(|m| (p, m)));
        match result {
            Ok(t) => {
                self.theme = t;
                self.error = None;
            }
            Err(e) => {
                // keep the previous look; tell the user why the new one did not apply
                self.error = Some(e);
                if self.theme.name.is_empty() {
                    self.theme = Theme::base(true);
                }
            }
        }
        self.changed = true;
    }

    pub fn select(&mut self, id: &str) {
        self.selected = id.to_string();
        let _ = std::fs::write(&self.ui_path, format!("# window style (see the styles folder)\nstyle = {id:?}\n"));
        self.load(id);
    }

    /// Call every frame: reloads the style when its file was edited.
    pub fn poll(&mut self) {
        if self.last_poll.elapsed() < Duration::from_millis(700) {
            return;
        }
        self.last_poll = Instant::now();
        if let Some((p, old)) = self.watch.clone() {
            if mtime(&p).is_some_and(|m| m != old) {
                let id = self.selected.clone();
                self.load(&id);
            }
        }
    }

    /// Write a fully commented template style into a new folder and select it. Returns its path.
    pub fn create_template(&mut self) -> Result<PathBuf, String> {
        let mut n = 1;
        let folder = loop {
            let name = if n == 1 { "my-style".to_string() } else { format!("my-style-{n}") };
            let p = self.dir.join(&name);
            if !p.exists() {
                break (name, p);
            }
            n += 1;
        };
        std::fs::create_dir_all(&folder.1).map_err(|e| e.to_string())?;
        let file = folder.1.join("style.toml");
        std::fs::write(&file, TEMPLATE).map_err(|e| e.to_string())?;
        self.refresh_list();
        self.select(&folder.0);
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_and_bundled_styles_parse() {
        let t = Theme::parse(TEMPLATE, None).expect("template");
        assert_eq!(t.tile_size, 92.0);
        for (name, text) in BUNDLED.iter().filter(|(n, _)| n.ends_with(".toml")) {
            Theme::parse(text, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn bad_values_are_reported() {
        assert!(Theme::parse("[colors]\naccent = \"red\"", None).unwrap_err().contains("colors.accent"));
        assert!(Theme::parse("[shape]\ntile_size = 5", None).unwrap_err().contains("tile_size"));
        assert!(Theme::parse("base = \"blue\"", None).is_err());
        assert!(Theme::parse("[layout]\ninspector = \"top\"", None).is_err());
    }

    #[test]
    fn a_three_line_style_works() {
        let t = Theme::parse("base = \"light\"\n[colors]\naccent = \"#ff0000\"", None).unwrap();
        assert!(!t.dark);
        assert_eq!(t.accent, Color32::from_rgb(255, 0, 0));
    }
}

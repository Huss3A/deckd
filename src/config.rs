//! deck.toml: what each button shows and does.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const COLS: u8 = 5;
pub const ROWS: u8 = 3;
pub const MAX_PAGES: u8 = 9;
/// Press-and-hold threshold and double-tap window used by the service.
pub const DEFAULT_HOLD_MS: u16 = 500;
pub const DEFAULT_LONG_MS: u16 = 1500;
fn default_hold_ms() -> u16 {
    DEFAULT_HOLD_MS
}
fn default_long_ms() -> u16 {
    DEFAULT_LONG_MS
}
fn is_false(v: &bool) -> bool {
    !*v
}

/// One step of a hold macro: runs on release if the button was held at least `ms`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct HoldStep {
    /// Hold duration in ms (100-10000).
    #[serde(default = "default_hold_ms")]
    pub ms: u16,
    pub action: Action,
    /// Optional rim color (#rrggbb) shown while holding once this duration is reached (default: automatic per tier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Hard split: don't chain this step after the previous step at the same duration; start a separate, parallel run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub split: bool,
}
pub const DOUBLE_MS: u64 = 350;
/// Holding longer than this starts the `repeat` action; it then repeats every `repeat_ms`.
pub const REPEAT_DELAY_MS: u64 = 400;
/// Safety stop: a repeat never runs longer than this even if the deck's release event is lost.
pub const REPEAT_MAX_SECS: u64 = 20;
pub const DEFAULT_REPEAT_MS: u16 = 150;

fn default_repeat_ms() -> u16 {
    DEFAULT_REPEAT_MS
}
fn is_default_repeat(v: &u16) -> bool {
    *v == DEFAULT_REPEAT_MS
}

fn one() -> u8 {
    1
}
fn is_one(v: &u8) -> bool {
    *v == 1
}

/// Colored rim flashed on a button (drawn into its picture, sent as a partial update) when a gesture fires.
/// Verified on the deck 2026-11-14: a 10 px rim is visible.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Feedback {
    pub enabled: bool,
    /// Rim width in pixels (2-40) on the 196 px tile.
    pub width: u8,
    /// Corner rounding of the rim in pixels (0 = square, up to 60).
    pub radius: u8,
    /// How long the rim stays, in ms (100-3000). A repeat keeps its rim for as long as you hold.
    pub flash_ms: u16,
    pub tap: String,
    pub double: String,
    pub hold: String,
    pub repeat: String,
}

impl Default for Feedback {
    fn default() -> Self {
        Feedback { enabled: true, width: 12, radius: 18, flash_ms: 350, tap: "#2f81f7".into(), double: "#3fb950".into(), hold: "#f0883e".into(), repeat: "#a371f7".into() }
    }
}

/// "#rrggbb" -> [r, g, b]
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten().map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Rotate an RGB color's hue by `degrees` (keeps saturation and brightness).
pub fn rotate_hue(rgb: [u8; 3], degrees: f32) -> [u8; 3] {
    let [r, g, b] = [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0];
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let d = mx - mn;
    let mut h = if d == 0.0 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    h = (h + degrees).rem_euclid(360.0);
    let (s, v) = (if mx == 0.0 { 0.0 } else { d / mx }, mx);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r1 + m) * 255.0).round() as u8, ((g1 + m) * 255.0).round() as u8, ((b1 + m) * 255.0).round() as u8]
}

impl Feedback {
    /// Rim color shown while holding, once the step group with duration `ms` is reached. A step can set its own `color`;
    /// otherwise the hold color is hue-shifted by 55 degrees per tier so each stage looks different.
    pub fn tier_color(&self, button: &Button, ms: u16) -> [u8; 3] {
        if let Some(c) = button.holds.iter().filter(|h| h.ms == ms).find_map(|h| h.color.as_deref().and_then(parse_hex)) {
            return c;
        }
        let mut tiers: Vec<u16> = button.holds.iter().map(|h| h.ms).collect();
        tiers.sort_unstable();
        tiers.dedup();
        let idx = tiers.iter().position(|t| *t == ms).unwrap_or(0);
        rotate_hue(parse_hex(&self.hold).unwrap_or([0xf0, 0x88, 0x3e]), 55.0 * idx as f32)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(2..=40).contains(&self.width) {
            return Err("feedback.width must be 2-40".into());
        }
        if self.radius > 60 {
            return Err("feedback.radius must be 0-60".into());
        }
        if !(100..=3000).contains(&self.flash_ms) {
            return Err("feedback.flash_ms must be 100-3000".into());
        }
        for (n, c) in [("tap", &self.tap), ("double", &self.double), ("hold", &self.hold), ("repeat", &self.repeat)] {
            parse_hex(c).ok_or_else(|| format!("feedback.{n}: bad color {c:?}, use #rrggbb"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    /// Number of pages (1-9). Pages with buttons beyond this are still counted.
    #[serde(default = "one")]
    pub pages: u8,
    /// Font file used for `glyph` (default: Segoe UI Symbol).
    #[serde(default = "default_font")]
    pub glyph_font: String,
    /// Glow feedback on the deck's buttons.
    #[serde(default)]
    pub feedback: Feedback,
    /// Text of the label under each icon: font size (deck default is 10).
    #[serde(default, rename = "button")]
    pub buttons: Vec<Button>,
    /// Directory of the config file; relative icon paths resolve against it.
    #[serde(skip)]
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Button {
    /// Page number, 1-based (omitted = page 1).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub page: u8,
    /// "col,row" with col 0-4, row 0-2 (top-left is "0,0"). Row 2 only has cols 0-2; the rest is the clock slot.
    pub pos: String,
    /// Extra action when the button is tapped twice quickly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double: Option<Action>,
    /// Hold steps, fired when the button is released. Each has its own duration; on release the group of steps with the
    /// longest duration that was reached runs. Steps sharing a duration run together as one macro, in list order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<HoldStep>,
    // Legacy single-slot hold fields: read from old config files and migrated into `holds` on load; never written back.
    #[serde(default, skip_serializing)]
    pub hold: Option<Action>,
    #[serde(default = "default_hold_ms", skip_serializing)]
    pub hold_ms: u16,
    #[serde(default, skip_serializing)]
    pub hold_long: Option<Action>,
    #[serde(default = "default_long_ms", skip_serializing)]
    pub hold_long_ms: u16,
    /// Repeated while the button stays down (e.g. volume up), starting after 0.4 s.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Action>,
    /// Milliseconds between repeats (30-2000).
    #[serde(default = "default_repeat_ms", skip_serializing_if = "is_default_repeat")]
    pub repeat_ms: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Background color like "#1f6feb" (used when there is no icon).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Path to an image; it is scaled to 196x196.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Symbol drawn large in the middle of the button, e.g. "▶" or "🔇". Needs a font that has the glyph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    /// Glyph color, "#rrggbb" (default white).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

/// What happens when the button is pressed. Nothing runs unless a button defines an action.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Start a program.
    Run { command: String, #[serde(default)] args: Vec<String> },
    /// Open a file, folder or URL with its default app.
    Open { target: String },
    /// Press a key combination, e.g. "ctrl+shift+m" or "win+shift+s".
    Hotkey { keys: String },
    /// Media key: vol_up, vol_down, mute, play_pause, next, prev, stop.
    Media { key: String },
    /// Switch the deck to another page: "next", "prev", "home" (page 1) or a number.
    Page { to: String },
    /// Write a message to the deckd log (handy for testing double tap / hold).
    Print { text: String },
}

fn default_brightness() -> u8 {
    86
}

fn default_font() -> String {
    r"C:\Windows\Fonts\seguisym.ttf".into()
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        cfg.base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        // migrate the old single hold / long hold fields into the `holds` list
        for b in &mut cfg.buttons {
            if let Some(a) = b.hold.take() {
                b.holds.push(HoldStep { ms: b.hold_ms, action: a, color: None, split: false });
            }
            if let Some(a) = b.hold_long.take() {
                b.holds.push(HoldStep { ms: b.hold_long_ms, action: a, color: None, split: false });
            }
        }
        if !(1..=MAX_PAGES).contains(&cfg.pages) {
            return Err(format!("pages must be 1-{MAX_PAGES}"));
        }
        cfg.feedback.validate()?;
        for b in &cfg.buttons {
            b.cell()?;
            if !(1..=MAX_PAGES).contains(&b.page) {
                return Err(format!("button {:?}: page must be 1-{MAX_PAGES}", b.pos));
            }
            for h in &b.holds {
                if !(100..=10000).contains(&h.ms) {
                    return Err(format!("button {:?} (page {}): hold step duration must be 100-10000 ms", b.pos, b.page));
                }
                crate::actions::validate(&h.action).map_err(|e| format!("button {:?} (page {}): {e}", b.pos, b.page))?;
                if let Some(c) = &h.color {
                    parse_hex(c).ok_or_else(|| format!("button {:?}: hold step color {c:?} must be #rrggbb", b.pos))?;
                }
            }
            for a in [&b.action, &b.double, &b.repeat].into_iter().flatten() {
                crate::actions::validate(a).map_err(|e| format!("button {:?} (page {}): {e}", b.pos, b.page))?;
            }
            if matches!(b.repeat, Some(Action::Page { .. })) {
                return Err(format!("button {:?} (page {}): a page switch can't be used as a repeat action", b.pos, b.page));
            }
            if !(30..=2000).contains(&b.repeat_ms) {
                return Err(format!("button {:?}: repeat_ms must be 30-2000", b.pos));
            }
        }
        Ok(cfg)
    }

    /// How many pages exist (the configured count, or more if buttons live on later pages).
    pub fn page_count(&self) -> u8 {
        self.buttons.iter().map(|b| b.page).max().unwrap_or(1).max(self.pages).clamp(1, MAX_PAGES)
    }

    /// The button for an index reported by the deck (index = row*5 + col) on a page.
    pub fn button_for_index(&self, page: u8, index: u8) -> Option<&Button> {
        self.buttons.iter().find(|b| b.page == page && b.cell().map(|(c, r)| r * COLS + c) == Ok(index))
    }

    pub fn button_at(&self, page: u8, col: u8, row: u8) -> Option<&Button> {
        self.buttons.iter().find(|b| b.page == page && b.cell() == Ok((col, row)))
    }

    /// The button at (col, row) on a page, created empty if it does not exist yet.
    pub fn button_mut(&mut self, page: u8, col: u8, row: u8) -> &mut Button {
        let i = match self.buttons.iter().position(|b| b.page == page && b.cell() == Ok((col, row))) {
            Some(i) => i,
            None => {
                self.buttons.push(Button { page, pos: format!("{col},{row}"), ..Button::default() });
                self.buttons.len() - 1
            }
        };
        &mut self.buttons[i]
    }

    pub fn clear_button(&mut self, page: u8, col: u8, row: u8) {
        self.buttons.retain(|b| !(b.page == page && b.cell() == Ok((col, row))));
    }

    /// Write to disk atomically (temp file + rename) so a running service never reads a half-written file.
    /// Note: comments in the file are not preserved.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let body = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let text = format!("# deckd config. Written by the deckd window; hand edits are fine too (comments are lost on the next save).\n{body}");
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
    }
}

impl Default for Config {
    fn default() -> Self {
        Config { brightness: default_brightness(), pages: 1, glyph_font: default_font(), feedback: Feedback::default(), buttons: Vec::new(), base_dir: PathBuf::new() }
    }
}

impl Default for Button {
    fn default() -> Self {
        Button { page: 1, pos: String::new(), double: None, holds: Vec::new(), hold: None, hold_ms: DEFAULT_HOLD_MS, hold_long: None, hold_long_ms: DEFAULT_LONG_MS, repeat: None, repeat_ms: DEFAULT_REPEAT_MS, label: None, color: None, icon: None, glyph: None, glyph_color: None, action: None }
    }
}

impl Button {
    /// The hold steps that fire for a release after `held_ms`: all steps sharing the longest duration reached, in list order.
    pub fn hold_group(&self, held_ms: u64) -> Vec<&HoldStep> {
        let top = self.holds.iter().filter(|h| h.ms as u64 <= held_ms).map(|h| h.ms).max();
        match top {
            Some(t) => self.holds.iter().filter(|h| h.ms == t).collect(),
            None => Vec::new(),
        }
    }

    /// (col, row), validated against what the hardware has.
    pub fn cell(&self) -> Result<(u8, u8), String> {
        let bad = || format!("bad pos {:?}: use \"col,row\" (col 0-4, row 0-2; row 2 only cols 0-2)", self.pos);
        let (c, r) = self.pos.split_once(',').ok_or_else(bad)?;
        let (c, r): (u8, u8) = (c.trim().parse().map_err(|_| bad())?, r.trim().parse().map_err(|_| bad())?);
        if c >= COLS || r >= ROWS || (r == 2 && c >= 3) {
            return Err(bad());
        }
        Ok((c, r))
    }
}

pub const SAMPLE: &str = include_str!("../deck.sample.toml");


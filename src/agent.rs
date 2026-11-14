//! Agent CLI: non-interactive commands with JSON output, stable exit codes and safe defaults.
//! Contract is documented in AGENTS.md. Everything here works without the deck (file based).
//!
//! Exit codes: 0 ok, 1 internal error, 2 usage error, 3 deck not connected, 4 config invalid.

use crate::config::{Action, Button, Config};
use crate::{actions, layout};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

pub const COMMANDS: &[&str] =
    &["status", "list", "get", "set", "clear", "set-brightness", "feedback", "validate", "preview", "press", "schema"];

pub struct CliError {
    code: &'static str,
    message: String,
    exit: i32,
}

fn usage(m: impl Into<String>) -> CliError {
    CliError { code: "usage", message: m.into(), exit: 2 }
}
fn invalid(m: impl Into<String>) -> CliError {
    CliError { code: "config_invalid", message: m.into(), exit: 4 }
}
fn internal(m: impl Into<String>) -> CliError {
    CliError { code: "internal", message: m.into(), exit: 1 }
}

/// Parsed arguments: positionals plus `--flag value` pairs (`--flag` alone is "true"; `--arg` may repeat).
struct Args {
    pos: Vec<String>,
    flags: Vec<(String, String)>,
}

const BOOL_FLAGS: &[&str] = &["json", "execute", "dry-run", "split", "clear-holds"];

fn parse_args(raw: &[String]) -> Result<Args, CliError> {
    let (mut pos, mut flags) = (Vec::new(), Vec::new());
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            if BOOL_FLAGS.contains(&name) {
                flags.push((name.to_string(), "true".into()));
            } else {
                let v = it.next().ok_or_else(|| usage(format!("--{name} needs a value")))?;
                flags.push((name.to_string(), v.clone()));
            }
        } else {
            pos.push(a.clone());
        }
    }
    Ok(Args { pos, flags })
}

impl Args {
    fn get(&self, name: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn all(&self, name: &str) -> Vec<String> {
        self.flags.iter().filter(|(k, _)| k == name).map(|(_, v)| v.clone()).collect()
    }
    fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

fn config_path(a: &Args) -> PathBuf {
    PathBuf::from(a.get("config").unwrap_or("deck.toml"))
}

fn load(a: &Args) -> Result<Config, CliError> {
    Config::load(&config_path(a)).map_err(invalid)
}

fn cell_arg(a: &Args) -> Result<(u8, u8), CliError> {
    let s = a.pos.first().ok_or_else(|| usage("missing position, e.g. \"0,1\""))?;
    Button { pos: s.clone(), ..Button::default() }.cell().map_err(usage)
}

/// 1-based page from `--page` (default 1).
fn page_arg(a: &Args) -> Result<u8, CliError> {
    match a.get("page") {
        None => Ok(1),
        Some(s) => s.parse().ok().filter(|p| (1..=crate::config::MAX_PAGES).contains(p)).ok_or_else(|| usage("--page must be 1-9")),
    }
}

/// Everything the GUI/service checks before accepting a config.
fn check(cfg: &Config) -> Result<usize, CliError> {
    cfg.feedback.validate().map_err(invalid)?;
    for b in &cfg.buttons {
        b.cell().map_err(invalid)?;
        if let Some(c) = b.holds.iter().filter_map(|h| h.color.as_ref()).find(|c| crate::config::parse_hex(c).is_none()) {
            return Err(invalid(format!("button {:?}: hold step color {c:?} must be #rrggbb", b.pos)));
        }
        if b.holds.iter().any(|h| !(100..=10000).contains(&h.ms)) {
            return Err(invalid(format!("button {:?}: hold step durations must be 100-10000 ms", b.pos)));
        }
        for act in [&b.action, &b.double, &b.repeat].into_iter().flatten().chain(b.holds.iter().map(|h| &h.action)) {
            actions::validate(act).map_err(|e| invalid(format!("button {:?} (page {}): {e}", b.pos, b.page)))?;
        }
    }
    for b in &cfg.buttons {
        if matches!(b.repeat, Some(Action::Page { .. })) {
            return Err(invalid(format!("button {:?} (page {}): a page switch can't be used as a repeat action", b.pos, b.page)));
        }
        if !(30..=2000).contains(&b.repeat_ms) {
            return Err(invalid(format!("button {:?}: repeat_ms must be 30-2000", b.pos)));
        }
    }
    let mut total = 0;
    for p in 1..=cfg.page_count() {
        total += layout::build_zip(cfg, p).map_err(invalid)?.len();
    }
    Ok(total)
}

fn describe(a: &Action) -> String {
    match a {
        Action::Page { to } => format!("switch page: {to}"),
        Action::Print { text } => format!("print {text:?}"),
        Action::Run { command, args } => format!("run {command} {}", args.join(" ")).trim_end().to_string(),
        Action::Open { target } => format!("open {target}"),
        Action::Hotkey { keys } => format!("hotkey {keys}"),
        Action::Media { key } => format!("media {key}"),
    }
}

/// Runs one agent command; prints the JSON result on stdout.
pub fn run(cmd: &str, raw: &[String]) -> Result<(), CliError> {
    let a = parse_args(raw)?;
    let out: Value = match cmd {
        "status" => {
            let path = config_path(&a);
            let loaded = Config::load(&path);
            // Live connection state needs IPC to the running service (not built yet), so it is reported as unknown.
            json!({
                "config": path.display().to_string(),
                "config_valid": loaded.is_ok(),
                "config_error": loaded.as_ref().err(),
                "buttons": loaded.as_ref().map(|c| c.buttons.len()).ok(),
                "brightness": loaded.as_ref().map(|c| c.brightness).ok(),
                "deck_connected": null,
                "note": "deck_connected is unknown until the service exposes IPC; check the deckd window or serve log"
            })
        }
        "list" => {
            let cfg = load(&a)?;
            json!({"brightness": cfg.brightness, "pages": cfg.page_count(), "buttons": cfg.buttons})
        }
        "get" => {
            let (c, r) = cell_arg(&a)?;
            let cfg = load(&a)?;
            match cfg.button_at(page_arg(&a)?, c, r) {
                Some(b) => json!({"button": b}),
                None => json!({"button": null, "pos": format!("{c},{r}")}),
            }
        }
        "set" => {
            let (c, r) = cell_arg(&a)?;
            let mut cfg = load(&a)?;
            let page = page_arg(&a)?;
            if page > cfg.page_count() {
                cfg.pages = page; // setting a button on a new page creates that page
            }
            let b = cfg.button_mut(page, c, r);
            // An empty string clears an optional field.
            let opt = |v: &str| if v.is_empty() { None } else { Some(v.to_string()) };
            if let Some(v) = a.get("label") { b.label = opt(v); }
            if let Some(v) = a.get("color") { b.color = opt(v); }
            if let Some(v) = a.get("icon") { b.icon = opt(v); }
            if let Some(v) = a.get("glyph") { b.glyph = opt(v); }
            if let Some(v) = a.get("glyph-color") { b.glyph_color = opt(v); }
            if let Some(ms) = a.get("repeat-ms") {
                b.repeat_ms = ms.parse().map_err(|_| usage("--repeat-ms must be a number 30-2000"))?;
            }
            if a.has("clear-holds") {
                b.holds.clear();
            }
            if let Some(t) = a.get("action-type") {
                let need = |n: &str| a.get(n).map(str::to_string).ok_or_else(|| usage(format!("--action-type {t} needs --{n}")));
                let new: Option<Action> = match t {
                    "none" => None,
                    "page" => Some(Action::Page { to: need("to")? }),
                    "print" => Some(Action::Print { text: need("text")? }),
                    "run" => Some(Action::Run { command: need("command")?, args: a.all("arg") }),
                    "open" => Some(Action::Open { target: need("target")? }),
                    "hotkey" => Some(Action::Hotkey { keys: need("keys")? }),
                    "media" => Some(Action::Media { key: need("key")? }),
                    other => return Err(usage(format!("unknown --action-type {other:?}; use run|open|hotkey|media|page|print|none"))),
                };
                // --on press (default) | double | repeat | hold chooses where the action goes.
                // `hold` ADDS a step (--hold-ms, default 500; steps with the same duration chain into one macro,
                // --split makes it a separate parallel macro); `--action-type none --on hold --hold-ms N` removes the steps at N.
                match a.get("on").unwrap_or("press") {
                    "press" => b.action = new,
                    "double" => b.double = new,
                    "repeat" => b.repeat = new,
                    "hold" => {
                        let ms: u16 = a.get("hold-ms").unwrap_or("500").parse().map_err(|_| usage("--hold-ms must be a number 100-10000"))?;
                        match new {
                            None => b.holds.retain(|h| h.ms != ms),
                            Some(action) => b.holds.push(crate::config::HoldStep { ms, action, color: a.get("rim").map(str::to_string), split: a.has("split") }),
                        }
                    }
                    other => return Err(usage(format!("unknown --on {other:?}; use press|double|hold|repeat"))),
                }
            }
            check(&cfg)?;
            if !a.has("dry-run") {
                cfg.save(&config_path(&a)).map_err(internal)?;
            }
            json!({"saved": !a.has("dry-run"), "button": cfg.button_at(page, c, r)})
        }
        "clear" => {
            let (c, r) = cell_arg(&a)?;
            let mut cfg = load(&a)?;
            let page = page_arg(&a)?;
            let existed = cfg.button_at(page, c, r).is_some();
            cfg.clear_button(page, c, r);
            check(&cfg)?;
            if !a.has("dry-run") {
                cfg.save(&config_path(&a)).map_err(internal)?;
            }
            json!({"saved": !a.has("dry-run"), "cleared": existed})
        }
        // feedback [--enabled true|false] [--width 2-40] [--ms 100-3000] [--tap #rrggbb] [--double ..] [--hold ..] [--repeat ..] [--dry-run]
        // with no flags it just prints the current settings
        "feedback" => {
            let mut cfg = load(&a)?;
            let fb = &mut cfg.feedback;
            if let Some(v) = a.get("enabled") {
                fb.enabled = match v {
                    "true" | "on" | "1" => true,
                    "false" | "off" | "0" => false,
                    _ => return Err(usage("--enabled must be true or false")),
                };
            }
            if let Some(v) = a.get("width") {
                fb.width = v.parse().map_err(|_| usage("--width must be a number 2-40"))?;
            }
            if let Some(v) = a.get("radius") {
                fb.radius = v.parse().map_err(|_| usage("--radius must be a number 0-60"))?;
            }
            if let Some(v) = a.get("ms") {
                fb.flash_ms = v.parse().map_err(|_| usage("--ms must be a number 100-3000"))?;
            }
            for (name, field) in [("tap", &mut fb.tap), ("double", &mut fb.double), ("hold", &mut fb.hold), ("repeat", &mut fb.repeat)] {
                if let Some(v) = a.get(name) {
                    *field = v.to_string();
                }
            }
            cfg.feedback.validate().map_err(invalid)?;
            let changed = ["enabled", "width", "radius", "ms", "tap", "double", "hold", "repeat"].iter().any(|k| a.has(k));
            if changed && !a.has("dry-run") {
                cfg.save(&config_path(&a)).map_err(internal)?;
            }
            json!({"saved": changed && !a.has("dry-run"), "feedback": cfg.feedback})
        }
        "set-brightness" => {
            let v: u8 = a.pos.first().and_then(|s| s.parse().ok()).filter(|v| *v <= 100).ok_or_else(|| usage("usage: set-brightness <0-100>"))?;
            let mut cfg = load(&a)?;
            cfg.brightness = v;
            if !a.has("dry-run") {
                cfg.save(&config_path(&a)).map_err(internal)?;
            }
            json!({"saved": !a.has("dry-run"), "brightness": v})
        }
        "validate" => {
            let cfg = load(&a)?;
            let bytes = check(&cfg)?;
            json!({"valid": true, "buttons": cfg.buttons.len(), "layout_zip_bytes": bytes})
        }
        "preview" => {
            let cfg = load(&a)?;
            let dir = PathBuf::from(a.get("out").ok_or_else(|| usage("usage: preview --out <dir>"))?);
            std::fs::create_dir_all(&dir).map_err(|e| internal(e.to_string()))?;
            let mut files = Vec::new();
            for page in 1..=cfg.page_count() {
                let pdir = dir.join(format!("page{page}"));
                std::fs::create_dir_all(&pdir).map_err(|e| internal(e.to_string()))?;
                let zip = layout::build_zip(&cfg, page).map_err(invalid)?;
                let mut arch = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| internal(e.to_string()))?;
                for i in 0..arch.len() {
                    let mut f = arch.by_index(i).map_err(|e| internal(e.to_string()))?;
                    let full = f.name().map_err(|e| internal(e.to_string()))?.to_string();
                    if !full.ends_with(".png") {
                        continue;
                    }
                    let name = full.rsplit('/').next().unwrap_or("x.png").to_string();
                    let mut buf = Vec::new();
                    f.read_to_end(&mut buf).map_err(|e| internal(e.to_string()))?;
                    let path = pdir.join(&name);
                    std::fs::write(&path, buf).map_err(|e| internal(e.to_string()))?;
                    files.push(path.display().to_string());
                }
            }
            json!({"out": dir.display().to_string(), "files": files, "note": "<out>/page<N>/<col>_<row>_<hash>.png, 196x196"})
        }
        "press" => {
            let s = a.pos.first().ok_or_else(|| usage("usage: press <col,row|index> [--execute]"))?;
            let cfg = load(&a)?;
            let (c, r) = match s.parse::<u8>() {
                Ok(i) if i <= 12 => (i % 5, i / 5),
                Ok(_) => return Err(usage("index must be 0-12 (13 is the wide slot, which has no action)")),
                Err(_) => cell_arg(&a)?,
            };
            let page = page_arg(&a)?;
            let btn = cfg.button_at(page, c, r);
            // --on hold [--hold-ms N]: show which hold steps a release after N ms (default 500) would run
            if a.get("on") == Some("hold") {
                let held: u64 = a.get("hold-ms").unwrap_or("500").parse().map_err(|_| usage("--hold-ms must be a number"))?;
                let steps: Vec<_> = btn.map(|b| b.hold_group(held)).unwrap_or_default();
                let exec = a.has("execute");
                if exec {
                    for s in &steps {
                        if !matches!(s.action, Action::Page { .. }) {
                            actions::execute(&s.action).map_err(internal)?;
                        }
                    }
                }
                let list: Vec<Value> = steps.iter().map(|s| json!({"ms": s.ms, "split": s.split, "action": s.action, "description": describe(&s.action)})).collect();
                let out = json!({"page": page, "pos": format!("{c},{r}"), "held_ms": held, "steps": list, "executed": exec});
                println!("{}", serde_json::to_string_pretty(&out).map_err(|e| internal(e.to_string()))?);
                return Ok(());
            }
            // --on press (default) | double | repeat
            let action = btn.and_then(|b| match a.get("on").unwrap_or("press") {
                "double" => b.double.clone(),
                "repeat" => b.repeat.clone(),
                _ => b.action.clone(),
            });
            match action {
                None => json!({"page": page, "pos": format!("{c},{r}"), "action": null, "executed": false}),
                Some(act) => {
                    let exec = a.has("execute");
                    if exec {
                        actions::execute(&act).map_err(internal)?;
                    }
                    json!({"page": page, "pos": format!("{c},{r}"), "action": act, "description": describe(&act), "executed": exec})
                }
            }
        }
        "schema" => schema(),
        _ => return Err(usage(format!("unknown command {cmd:?}; commands: {}", COMMANDS.join(", ")))),
    };
    println!("{}", serde_json::to_string_pretty(&out).map_err(|e| internal(e.to_string()))?);
    Ok(())
}

pub fn report(e: &CliError) -> ! {
    eprintln!("{}", json!({"error": {"code": e.code, "message": e.message}}));
    std::process::exit(e.exit)
}

fn schema() -> Value {
    let color = json!({"type": "string", "pattern": "^#[0-9a-fA-F]{6}$"});
    let action = json!({"oneOf": [
        {"type": "object", "required": ["type", "command"], "properties": {"type": {"const": "run"}, "command": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}}}},
        {"type": "object", "required": ["type", "target"], "properties": {"type": {"const": "open"}, "target": {"type": "string"}}},
        {"type": "object", "required": ["type", "keys"], "properties": {"type": {"const": "hotkey"}, "keys": {"type": "string", "examples": ["ctrl+shift+m", "win+shift+s"]}}},
        {"type": "object", "required": ["type", "key"], "properties": {"type": {"const": "media"}, "key": {"enum": ["vol_up", "vol_down", "mute", "play_pause", "next", "prev", "stop"]}}},
        {"type": "object", "required": ["type", "text"], "properties": {"type": {"const": "print"}, "text": {"type": "string", "description": "message written to the deckd log"}}},
        {"type": "object", "required": ["type", "to"], "properties": {"type": {"const": "page"}, "to": {"type": "string", "description": "next, prev, home or a page number 1-9"}}}
    ]});
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "deck.toml (as JSON)",
        "type": "object",
        "properties": {
            "brightness": {"type": "integer", "minimum": 0, "maximum": 100, "default": 86},
            "pages": {"type": "integer", "minimum": 1, "maximum": 9, "default": 1},
            "feedback": {"type": "object", "description": "colored rim flashed on a deck button when a gesture fires", "properties": {
                "enabled": {"type": "boolean", "default": true},
                "width": {"type": "integer", "minimum": 2, "maximum": 40, "default": 12},
                "radius": {"type": "integer", "minimum": 0, "maximum": 60, "default": 18, "description": "corner rounding of the rim"},
                "flash_ms": {"type": "integer", "minimum": 100, "maximum": 3000, "default": 350},
                "tap": color, "double": color, "hold": color, "repeat": color}},
            "glyph_font": {"type": "string", "description": "Font file for glyph symbols", "default": "C:\\Windows\\Fonts\\seguisym.ttf"},
            "button": {"type": "array", "items": {
                "type": "object",
                "required": ["pos"],
                "properties": {
                    "pos": {"type": "string", "pattern": "^[0-4],[0-2]$", "description": "col,row; row 2 only has cols 0-2 (the rest is the wide slot)"},
                    "label": {"type": "string"},
                    "color": color,
                    "icon": {"type": "string", "description": "image path, scaled to 196x196"},
                    "glyph": {"type": "string", "description": "symbol drawn large in the middle"},
                    "glyph_color": color,
                    "page": {"type": "integer", "minimum": 1, "maximum": 9, "default": 1},
                    "action": action,
                    "double": {"description": "extra action on a quick double tap", "allOf": [action]},
                    "holds": {"type": "array", "description": "hold steps, fired on release. The steps sharing the longest ms that was reached run together as a macro in list order; split=true starts a separate parallel macro.",
                        "items": {"type": "object", "required": ["action"], "properties": {
                            "ms": {"type": "integer", "minimum": 100, "maximum": 10000, "default": 500},
                            "action": action,
                            "color": {"type": "string", "pattern": "^#[0-9a-fA-F]{6}$", "description": "rim color shown while holding once ms is reached (default: automatic per tier)"},
                            "split": {"type": "boolean", "default": false}}}},
                    "repeat": {"description": "runs repeatedly while the button is held (after 0.4 s), e.g. volume; not a page action", "allOf": [action]},
                    "repeat_ms": {"type": "integer", "minimum": 30, "maximum": 2000, "default": 150}
                }
            }}
        }
    })
}

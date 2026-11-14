//! What a button press does. Everything here is triggered only by an `action` in deck.toml.

use crate::config::Action;
use std::process::{Command, Stdio};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP};

const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12; // alt
const VK_LWIN: u16 = 0x5B;

fn media_vk(name: &str) -> Option<u16> {
    Some(match name {
        "vol_down" => 0xAE,
        "vol_up" => 0xAF,
        "mute" => 0xAD,
        "next" => 0xB0,
        "prev" => 0xB1,
        "stop" => 0xB2,
        "play_pause" => 0xB3,
        _ => return None,
    })
}

fn key_vk(name: &str) -> Option<u16> {
    let n = name.to_ascii_lowercase();
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some(c.to_ascii_uppercase() as u16);
        }
        if c.is_ascii_digit() {
            return Some(c as u16);
        }
    }
    if let Some(f) = n.strip_prefix('f').and_then(|x| x.parse::<u16>().ok()) {
        if (1..=12).contains(&f) {
            return Some(0x6F + f);
        }
    }
    Some(match n.as_str() {
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "tab" => 0x09,
        "space" => 0x20,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        _ => return None,
    })
}

/// "ctrl+shift+m" -> (modifier keys, main key)
fn parse_hotkey(keys: &str) -> Result<(Vec<u16>, u16), String> {
    let mut mods = Vec::new();
    let mut main = None;
    for part in keys.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.push(VK_CONTROL),
            "shift" => mods.push(VK_SHIFT),
            "alt" => mods.push(VK_MENU),
            "win" | "super" | "meta" => mods.push(VK_LWIN),
            other => {
                if main.replace(key_vk(other).ok_or(format!("unknown key {other:?} in hotkey {keys:?}"))?).is_some() {
                    return Err(format!("hotkey {keys:?} has more than one non-modifier key"));
                }
            }
        }
    }
    Ok((mods, main.ok_or(format!("hotkey {keys:?} needs a main key"))?))
}

/// Check an action up front so config mistakes show at load time, not on the first press.
pub fn validate(action: &Action) -> Result<(), String> {
    match action {
        Action::Hotkey { keys } => parse_hotkey(keys).map(|_| ()),
        Action::Media { key } if key == "mic_mute" => Ok(()),
        Action::Media { key } => media_vk(key).map(|_| ()).ok_or(format!("unknown media key {key:?}")),
        Action::Run { command, .. } if command.trim().is_empty() => Err("run action has an empty command".into()),
        Action::Open { target } if target.trim().is_empty() => Err("open action has an empty target".into()),
        Action::Page { to } => match to.as_str() {
            "next" | "prev" | "home" => Ok(()),
            n if n.parse::<u8>().is_ok_and(|p| (1..=crate::config::MAX_PAGES).contains(&p)) => Ok(()),
            other => Err(format!("bad page target {other:?}: use next, prev, home or a page number 1-{}", crate::config::MAX_PAGES)),
        },
        Action::Focus { window } if window.trim().is_empty() => Err("focus action needs a window title or program name".into()),
        Action::Delay { ms } if !(10..=60_000).contains(ms) => Err(format!("delay {ms} ms must be 10-60000")),
        Action::Text { text } if text.is_empty() => Err("text step is empty".into()),
        Action::Text { text } if text.chars().count() > crate::config::TEXT_MAX_CHARS => {
            Err(format!("text is longer than {} characters", crate::config::TEXT_MAX_CHARS))
        }
        Action::Macro { steps } => {
            if steps.is_empty() {
                return Err("a macro needs at least one step".into());
            }
            if steps.len() > crate::config::MACRO_MAX_STEPS {
                return Err(format!("a macro can have at most {} steps", crate::config::MACRO_MAX_STEPS));
            }
            for (n, s) in steps.iter().enumerate() {
                if matches!(s, Action::Macro { .. } | Action::Page { .. }) {
                    return Err(format!("step {}: a macro can't contain a page switch or another macro", n + 1));
                }
                validate(s).map_err(|e| format!("step {}: {e}", n + 1))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Type text into the focused window: Unicode key events (any character), Enter for line breaks, Tab for tabs.
fn type_text(text: &str) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE;
    for ch in text.chars().filter(|&c| c != '\r') {
        let mut seq = Vec::new();
        match ch {
            '\n' => {
                seq.push(key_event(0x0D, false));
                seq.push(key_event(0x0D, true));
            }
            '\t' => {
                seq.push(key_event(0x09, false));
                seq.push(key_event(0x09, true));
            }
            c => {
                let mut units = [0u16; 2];
                for &u in c.encode_utf16(&mut units).iter() {
                    for up in [false, true] {
                        seq.push(INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: u, dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
                        });
                    }
                }
            }
        }
        press(&seq);
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
}

/// Visible top-level windows as (title, program file name), top of the stack first.
pub fn list_windows() -> Vec<(String, String)> {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
        unsafe {
            let out = &mut *(lparam as *mut Vec<(String, String)>);
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if n <= 0 {
                return 1;
            }
            let title = String::from_utf16_lossy(&buf[..n as usize]);
            if title == "Program Manager" {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut exe = String::new();
            let h = OpenProcess(0x1000, 0, pid);
            if !h.is_null() {
                let mut path = [0u16; 520];
                let mut len = path.len() as u32;
                if QueryFullProcessImageNameW(h, 0, path.as_mut_ptr(), &mut len) != 0 {
                    exe = String::from_utf16_lossy(&path[..len as usize]).rsplit('\\').next().unwrap_or("").to_string();
                }
                CloseHandle(h);
            }
            out.push((title, exe));
            1
        }
    }

    let mut out: Vec<(String, String)> = Vec::new();
    unsafe {
        EnumWindows(Some(visit), &mut out as *mut Vec<(String, String)> as LPARAM);
    }
    out
}

/// Bring the first visible window whose title or program name contains `query` to the front (restoring it if minimized).
/// Returns the title of the window that was focused.
fn focus_window(query: &str) -> Result<String, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    struct Search {
        query: String,
        found: Option<(HWND, String)>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
        unsafe {
            let s = &mut *(lparam as *mut Search);
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if n <= 0 {
                return 1;
            }
            let title = String::from_utf16_lossy(&buf[..n as usize]);
            if title == "Program Manager" {
                return 1;
            }
            let mut exe = String::new();
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let h = OpenProcess(0x1000, 0, pid); // PROCESS_QUERY_LIMITED_INFORMATION
            if !h.is_null() {
                let mut path = [0u16; 520];
                let mut len = path.len() as u32;
                if QueryFullProcessImageNameW(h, 0, path.as_mut_ptr(), &mut len) != 0 {
                    let full = String::from_utf16_lossy(&path[..len as usize]);
                    exe = full.rsplit('\\').next().unwrap_or("").to_lowercase();
                }
                CloseHandle(h);
            }
            if title.to_lowercase().contains(&s.query) || exe.contains(&s.query) {
                s.found = Some((hwnd, title));
                return 0; // stop: the first match from the top of the stack
            }
            1
        }
    }

    let mut search = Search { query: query.trim().to_lowercase(), found: None };
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
    }
    let (hwnd, title) = search.found.ok_or_else(|| format!("no open window matches {query:?}"))?;
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        // Windows only lets a program take the foreground right after it has seen keyboard input: a tap of Alt counts
        press(&[key_event(VK_MENU, false), key_event(VK_MENU, true)]);
        BringWindowToTop(hwnd);
        if SetForegroundWindow(hwnd) == 0 {
            return Err(format!("Windows would not let {title:?} take the focus"));
        }
    }
    Ok(title)
}

fn key_event(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn press(sequence: &[INPUT]) {
    unsafe {
        SendInput(sequence.len() as u32, sequence.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn execute(action: &Action) -> Result<(), String> {
    execute_logged(action, &|m| println!("{m}"))
}

/// Like `execute`, but messages (print steps) go to `log`. Delays and macros block this thread until they finish,
/// so callers run them off the main loop.
pub fn execute_logged(action: &Action, log: &dyn Fn(String)) -> Result<(), String> {
    match action {
        Action::Delay { ms } => std::thread::sleep(std::time::Duration::from_millis(*ms as u64)),
        Action::Text { text } => {
            type_text(text);
            log(format!("typed {} character(s)", text.chars().count()));
        }
        Action::Macro { steps } => {
            for (n, s) in steps.iter().enumerate() {
                if n > 0 && !matches!(s, Action::Delay { .. }) && !matches!(steps[n - 1], Action::Delay { .. }) {
                    std::thread::sleep(std::time::Duration::from_millis(40)); // let the app see each step
                }
                execute_logged(s, log).map_err(|e| format!("step {}: {e}", n + 1))?;
            }
        }
        _ => return execute_single(action, log),
    }
    Ok(())
}

fn execute_single(action: &Action, log: &dyn Fn(String)) -> Result<(), String> {
    match action {
        Action::Run { command, args } => {
            Command::new(command)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("cannot start {command}: {e}"))?;
        }
        Action::Open { target } => {
            Command::new("cmd")
                .args(["/C", "start", "", target])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("cannot open {target}: {e}"))?;
        }
        Action::Hotkey { keys } => {
            let (mods, main) = parse_hotkey(keys)?;
            let mut seq: Vec<INPUT> = mods.iter().map(|&m| key_event(m, false)).collect();
            seq.push(key_event(main, false));
            seq.push(key_event(main, true));
            seq.extend(mods.iter().rev().map(|&m| key_event(m, true)));
            press(&seq);
        }
        // not a real media key: flips the default microphone's mute switch directly (works in every app)
        Action::Media { key } if key == "mic_mute" => {
            crate::audio::toggle(crate::audio::Kind::Mic)?;
        }
        Action::Media { key } => {
            let vk = media_vk(key).ok_or(format!("unknown media key {key:?}"))?;
            press(&[key_event(vk, false), key_event(vk, true)]);
        }
        Action::Page { .. } => return Err("page switches only happen on the deck itself".into()),
        Action::Focus { window } => {
            let title = focus_window(window)?;
            log(format!("focused window {title:?}"));
        }
        Action::Print { text } => log(format!("PRINT: {text}")),
        // handled in execute_logged
        Action::Delay { .. } | Action::Text { .. } | Action::Macro { .. } => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hot(k: &str) -> Action {
        Action::Hotkey { keys: k.into() }
    }

    #[test]
    fn macro_validation() {
        assert!(validate(&Action::Macro { steps: vec![hot("ctrl+c"), Action::Delay { ms: 200 }, Action::Text { text: "hi".into() }] }).is_ok());
        assert!(validate(&Action::Macro { steps: vec![] }).is_err());
        assert!(validate(&Action::Macro { steps: vec![Action::Page { to: "next".into() }] }).is_err());
        assert!(validate(&Action::Macro { steps: vec![Action::Macro { steps: vec![hot("ctrl+c")] }] }).is_err());
        let e = validate(&Action::Macro { steps: vec![hot("ctrl+c"), hot("ctrl+nope")] }).unwrap_err();
        assert!(e.contains("step 2"), "{e}");
        assert!(validate(&Action::Delay { ms: 5 }).is_err());
        assert!(validate(&Action::Delay { ms: 60_001 }).is_err());
        assert!(validate(&Action::Text { text: String::new() }).is_err());
        assert!(validate(&Action::Macro { steps: vec![Action::Delay { ms: 10 }; 51] }).is_err());
    }

    #[test]
    fn macro_runs_in_order_with_waits() {
        use std::sync::Mutex;
        let log = Mutex::new(Vec::new());
        let m = Action::Macro { steps: vec![
            Action::Print { text: "a".into() },
            Action::Delay { ms: 120 },
            Action::Print { text: "b".into() },
        ] };
        let t = std::time::Instant::now();
        execute_logged(&m, &|s| log.lock().unwrap().push(s)).unwrap();
        assert!(t.elapsed().as_millis() >= 120);
        assert_eq!(*log.lock().unwrap(), vec!["PRINT: a".to_string(), "PRINT: b".to_string()]);
    }

    #[test]
    fn macro_toml_roundtrip() {
        let text = "type = \"macro\"\n[[steps]]\ntype = \"hotkey\"\nkeys = \"ctrl+c\"\n[[steps]]\ntype = \"delay\"\nms = 200\n[[steps]]\ntype = \"text\"\ntext = \"x\"\n";
        let a: Action = toml::from_str(text).unwrap();
        assert!(matches!(&a, Action::Macro { steps } if steps.len() == 3));
        let back: Action = toml::from_str(&toml::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, back);
    }
}

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
        Action::Media { key } => media_vk(key).map(|_| ()).ok_or(format!("unknown media key {key:?}")),
        Action::Run { command, .. } if command.trim().is_empty() => Err("run action has an empty command".into()),
        Action::Open { target } if target.trim().is_empty() => Err("open action has an empty target".into()),
        Action::Page { to } => match to.as_str() {
            "next" | "prev" | "home" => Ok(()),
            n if n.parse::<u8>().is_ok_and(|p| (1..=crate::config::MAX_PAGES).contains(&p)) => Ok(()),
            other => Err(format!("bad page target {other:?}: use next, prev, home or a page number 1-{}", crate::config::MAX_PAGES)),
        },
        _ => Ok(()),
    }
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
        Action::Media { key } => {
            let vk = media_vk(key).ok_or(format!("unknown media key {key:?}"))?;
            press(&[key_event(vk, false), key_event(vk, true)]);
        }
        Action::Page { .. } => return Err("page switches only happen on the deck itself".into()),
        Action::Print { text } => println!("{text}"), // the service logs it to the window instead
    }
    Ok(())
}

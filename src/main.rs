//! deckd: lightweight Ulanzi D200H client (replacement for Ulanzi Studio, personal use).
//!
//!   deckd [gui] [deck.toml]             open the window (also runs the deck service)
//!   deckd serve [deck.toml]             run the deck service in the console
//!   deckd init [deck.toml]              write a sample config
//!   deckd build [deck.toml] [out.zip]   render the layout ZIP without touching the deck
//!
//! Debugging:
//!   deckd listen|run [seconds]          print button events (run also sends the heartbeat)
//!   deckd brightness <0-100>
//!   deckd upload|patch <file.zip> [seconds]

#![cfg_attr(all(not(debug_assertions), feature = "windowed"), windows_subsystem = "windows")]

mod actions;
mod agent;
mod config;
mod gui;
mod layout;
mod protocol;
mod service;

use config::{Config, SAMPLE};
use hidapi::{HidApi, HidDevice};
use protocol::{cmd, Event, HEARTBEAT_EVERY, PACKET};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Debug helper: print events, optionally with the heartbeat.
fn monitor(dev: &HidDevice, secs: u64, keepalive: bool) {
    let start = Instant::now();
    let end = start + Duration::from_secs(secs);
    let mut last_beat = Instant::now() - HEARTBEAT_EVERY;
    let mut buf = [0u8; PACKET + 1];
    println!("running for {secs}s (heartbeat {})...", if keepalive { "on" } else { "off" });
    while Instant::now() < end {
        if keepalive && last_beat.elapsed() >= HEARTBEAT_EVERY {
            if protocol::heartbeat(dev).is_err() {
                break;
            }
            last_beat = Instant::now();
        }
        match dev.read(&mut buf) {
            Ok(0) => std::thread::sleep(Duration::from_millis(10)),
            Ok(n) => match protocol::parse(&buf[..n]) {
                Some(Event::Button { index, state, pressed }) => {
                    println!(
                        "{:7} ms  button {index:2} state={state:#04x} {}  raw={:02x?}",
                        start.elapsed().as_millis(),
                        if pressed { "DOWN" } else { "up" },
                        &buf[..16.min(n)]
                    )
                }
                Some(Event::Other { cmd, payload }) => println!("cmd {cmd:#06x} payload={:02x?}", &payload[..payload.len().min(16)]),
                None => {}
            },
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }
    }
}

fn with_deck<T>(f: impl FnOnce(&HidDevice) -> Result<T, String>) -> Result<T, String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    f(&protocol::open(&api)?)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let config_path = |i: usize| PathBuf::from(arg(i).unwrap_or("deck.toml"));
    let secs = |i: usize, default: u64| arg(i).and_then(|s| s.parse().ok()).unwrap_or(default);

    if let Some(c) = arg(0).filter(|c| agent::COMMANDS.contains(c)) {
        match agent::run(c, &args[1..]) {
            Ok(()) => return,
            Err(e) => agent::report(&e),
        }
    }

    let result: Result<(), String> = match arg(0) {
        None => gui::run(config_path(1)),
        Some("gui") => gui::run(config_path(1)),
        Some("init") => {
            let path = config_path(1);
            if path.exists() {
                Err(format!("{} already exists; not overwriting", path.display()))
            } else {
                std::fs::write(&path, SAMPLE).map_err(|e| e.to_string()).map(|_| println!("wrote {}", path.display()))
            }
        }
        Some("build") => Config::load(&config_path(1)).and_then(|cfg| {
            let zip = layout::build_zip(&cfg, 1)?;
            let out = arg(2).unwrap_or("layout.zip");
            std::fs::write(out, &zip).map_err(|e| e.to_string())?;
            println!("wrote {out} ({} bytes)", zip.len());
            Ok(())
        }),
        // Experiment (owner approved 2026-11-14): flash a colored rim on one button with partial updates (0x000D).
        // usage: deckd glowtest [col,row] [deck.toml]   (close the window / other deck users first)
        Some("glowtest") => Config::load(&config_path(2)).and_then(|cfg| {
            let (col, row): (u8, u8) = arg(1)
                .and_then(|s| s.split_once(','))
                .and_then(|(c, r)| Some((c.trim().parse().ok()?, r.trim().parse().ok()?)))
                .unwrap_or((0, 0));
            with_deck(|d| {
                println!("deck: {}", protocol::handshake(d, cfg.brightness)?);
                protocol::upload_zip(d, cmd::FULL_LAYOUT, &layout::build_zip(&cfg, 1)?, true)?;
                println!("full layout sent; flashing button {col},{row} in 2 s (watch it)...");
                std::thread::sleep(Duration::from_secs(2));
                // progressively thicker red rims; the last one is a fully solid tile (width 98 = the whole 196 px tile)
                let red = [0xf8, 0x31, 0x31];
                let colors = [("1) thin 10px red rim", red, 10u32), ("2) thick 30px red rim", red, 30), ("3) very thick 60px red rim", red, 60), ("4) SOLID red tile", red, 98)];
                for (name, rgb, width) in colors {
                    protocol::heartbeat(d)?;
                    println!("--- {name}");
                    let on = layout::cell_zip(&cfg, 1, col, row, Some((rgb, width, 18)))?;
                    let t = Instant::now();
                    protocol::upload_zip(d, cmd::PARTIAL_LAYOUT, &on, false)?;
                    println!("{name} rim sent: {} bytes in {} ms", on.len(), t.elapsed().as_millis());
                    std::thread::sleep(Duration::from_millis(2000));
                    let off = layout::cell_zip(&cfg, 1, col, row, None)?;
                    let t = Instant::now();
                    protocol::upload_zip(d, cmd::PARTIAL_LAYOUT, &off, false)?;
                    println!("restored: {} bytes in {} ms", off.len(), t.elapsed().as_millis());
                    std::thread::sleep(Duration::from_millis(1200));
                }
                println!("done. The button should look normal again.");
                Ok(())
            })
        }),
        // Offline: render a button with a rim to a PNG so the look can be checked without the deck.
        // usage: deckd rimpreview <out.png> [col,row] [deck.toml] [#rrggbb]
        Some("rimpreview") => Config::load(&config_path(3)).and_then(|cfg| {
            use std::io::Read;
            let out = arg(1).ok_or("usage: deckd rimpreview <out.png> [col,row] [deck.toml] [#rrggbb]")?;
            let (col, row): (u8, u8) = arg(2)
                .and_then(|s| s.split_once(','))
                .and_then(|(c, r)| Some((c.trim().parse().ok()?, r.trim().parse().ok()?)))
                .unwrap_or((0, 0));
            let rgb = config::parse_hex(arg(4).unwrap_or("#2f81f7")).ok_or("bad color")?;
            let fb = &cfg.feedback;
            let zip = layout::cell_zip(&cfg, 1, col, row, Some((rgb, fb.width as u32, fb.radius as u32)))?;
            let mut a = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
            for i in 0..a.len() {
                let mut f = a.by_index(i).map_err(|e| e.to_string())?;
                if f.name().map_err(|e| e.to_string())?.ends_with(".png") {
                    let mut buf = Vec::new();
                    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                    std::fs::write(out, buf).map_err(|e| e.to_string())?;
                    println!("wrote {out} (width {} px, radius {} px)", fb.width, fb.radius);
                    return Ok(());
                }
            }
            Err("no image in the update".into())
        }),
        Some("serve") => {
            let ctx = service::Ctx::new(true, Arc::new(|| {}));
            service::run(config_path(1), ctx);
            Ok(())
        }
        Some("listen") => with_deck(|d| Ok(monitor(d, secs(1, 30), false))),
        Some("run") => with_deck(|d| Ok(monitor(d, secs(1, 30), true))),
        Some("brightness") => with_deck(|d| {
            let v: u8 = arg(1).and_then(|s| s.parse().ok()).ok_or("usage: deckd brightness <0-100>")?;
            protocol::send(d, cmd::BRIGHTNESS, v.min(100).to_string().as_bytes())?;
            println!("brightness {v} sent");
            Ok(())
        }),
        Some(which @ ("upload" | "patch")) => with_deck(|d| {
            let path = arg(1).ok_or("usage: deckd upload|patch <file.zip> [seconds]")?;
            let zip = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            println!("deck: {}", protocol::handshake(d, 86)?);
            if which == "upload" {
                protocol::upload_zip(d, cmd::FULL_LAYOUT, &zip, true)?;
            } else {
                protocol::upload_zip(d, cmd::PARTIAL_LAYOUT, &zip, false)?;
            }
            println!("{which}: sent {} bytes", zip.len());
            monitor(d, secs(2, 20), true);
            Ok(())
        }),
        _ => Err("usage: deckd [gui] | serve | init | build [deck.toml] | listen|run [seconds] | brightness <0-100> | upload|patch <zip>".into()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

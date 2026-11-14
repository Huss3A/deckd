//! The deck service: connect, upload the layout, run button actions, keep the heartbeat, reload on config change.
//! Runs on its own thread; the GUI (or the console `serve` command) watches `Status`.

use crate::{
    actions,
    config::{Action, Config, StateSource, DOUBLE_MS, REPEAT_DELAY_MS, REPEAT_MAX_SECS},
    layout,
    protocol::{self, cmd, Event, HEARTBEAT_EVERY, PACKET},
};
use hidapi::{HidApi, HidDevice};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

const DEBOUNCE: Duration = Duration::from_millis(40); // a flaky cable once produced phantom presses; kept short so double taps work
const LOG_LINES: usize = 200;

#[derive(Default)]
pub struct Status {
    pub connected: bool,
    pub info: String,
    pub log: VecDeque<String>,
    /// Last button press seen (index, when) so the GUI can flash it.
    pub pressed: Option<(u8, Instant)>,
    /// Set when the config file on disk could not be loaded (the service keeps the old config).
    pub config_error: Option<String>,
    /// Page currently shown on the deck (1-based, 0 = not connected yet).
    pub page: u8,
}

#[derive(Clone)]
pub struct Ctx {
    pub status: Arc<Mutex<Status>>,
    /// Called after any status change (the GUI uses it to repaint).
    pub notify: Arc<dyn Fn() + Send + Sync>,
    /// Also print log lines to the console.
    pub echo: bool,
}

impl Ctx {
    pub fn new(echo: bool, notify: Arc<dyn Fn() + Send + Sync>) -> Ctx {
        Ctx { status: Arc::new(Mutex::new(Status::default())), notify, echo }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Status) -> R) -> R {
        let r = f(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
        (self.notify)();
        r
    }

    pub fn log(&self, msg: impl Into<String>) {
        let msg = msg.into();
        if self.echo {
            println!("{msg}");
        }
        let line = format!("{} {msg}", chrono::Local::now().format("%H:%M:%S"));
        self.with(|s| {
            s.log.push_back(line);
            while s.log.len() > LOG_LINES {
                s.log.pop_front();
            }
        });
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn upload_layout(dev: &HidDevice, cfg: &Config, page: u8, ctx: &Ctx, live: &layout::Live) -> Result<(), String> {
    let zip = layout::build_zip_live(cfg, page, live)?;
    protocol::upload_zip(dev, cmd::FULL_LAYOUT, &zip, true)?;
    ctx.log(format!("page {page}/{}: layout uploaded ({} bytes)", cfg.page_count(), zip.len()));
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Fire {
    Single,
    Double,
    Repeat,
}

/// Which gesture fired (picks the rim color).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Glow {
    Tap,
    Double,
    Hold,
    Repeat,
}

/// Flash requests collected while handling a press; the session loop sends them to the deck.
type GlowQueue = Vec<(u8, Glow)>;

fn send_cell(dev: &HidDevice, cfg: &Config, page: u8, idx: usize, rim: Option<([u8; 3], u32, u32)>, live: &layout::Live) -> Result<(), String> {
    // a wide slot whose display belongs to the firmware (clock / stats) has no picture to draw a rim on
    if idx == 13 && cfg.button_at(page, 3, 2).is_some_and(|b| b.face.is_firmware()) {
        return Ok(());
    }
    let zip = layout::cell_zip(cfg, page, (idx % 5) as u8, (idx / 5) as u8, rim, live)?;
    protocol::upload_zip(dev, cmd::PARTIAL_LAYOUT, &zip, false)
}

/// Per-button press tracking for double-tap and hold.
#[derive(Clone, Copy, Default)]
struct Key {
    down: Option<Instant>,
    /// A first tap is waiting to see whether a second one follows.
    tap_at: Option<Instant>,
    /// The repeat action is running; `last_repeat` is when it last fired.
    repeating: bool,
    last_repeat: Option<Instant>,
    /// While holding: the hold-step duration (ms) whose rim is currently shown on the deck.
    tier_shown: Option<u16>,
}

/// The wide slot (button 13) does not report like the normal buttons (decoded 2026-11-14, see NOTES.md): a short tap arrives as a
/// burst of quick down/up pairs with stepping state bytes (c8, c9, ca, ...), a long press as one down (c8) and one up with state 01.
/// The owner asked (2026-11-14) that nothing is dropped: every down/up from the wide slot is handled as a real press/release, whatever
/// its state byte, and the owner judges whether detection is right. (A burst filter existed before; see git history.)
#[derive(Default)]
struct WideFilter;

impl WideFilter {
    /// Normal buttons: only state 01 counts. The wide slot (index 13): every event counts.
    fn accept(&mut self, index: u8, state: u8, _pressed: bool) -> bool {
        index == 13 || state == 0x01
    }
}

fn make_live(sampler: &crate::sysstats::Sampler, anim_start: Instant, active: [bool; 14]) -> layout::Live {
    layout::Live { now: chrono::Local::now(), stats: sampler.stats(), tick_ms: anim_start.elapsed().as_millis() as u64, active }
}

/// Buttons (page, index) whose `toggle` state is currently "on".
static TOGGLES: Mutex<Vec<(u8, u8)>> = Mutex::new(Vec::new());

fn flip_toggle(page: u8, index: u8) {
    let mut t = TOGGLES.lock().unwrap_or_else(|e| e.into_inner());
    match t.iter().position(|&k| k == (page, index)) {
        Some(p) => {
            t.remove(p);
        }
        None => t.push((page, index)),
    }
}

/// Real system state polled for state-aware buttons (speaker / microphone mute).
struct SysState {
    out_muted: bool,
    mic_muted: bool,
    polled: Instant,
}

impl SysState {
    fn new() -> SysState {
        SysState { out_muted: false, mic_muted: false, polled: Instant::now() - Duration::from_secs(5) }
    }

    /// Poll the audio devices (about 3 times a second), but only when a button on this page follows them.
    fn poll(&mut self, cfg: &Config, page: u8) {
        if self.polled.elapsed() < Duration::from_millis(300) {
            return;
        }
        let uses = |s: StateSource| (0..14u8).any(|i| cfg.button_for_index(page, i).and_then(|b| b.state_source()) == Some(s));
        if uses(StateSource::OutputMuted) {
            self.out_muted = crate::audio::muted(crate::audio::Kind::Output).unwrap_or(self.out_muted);
        }
        if uses(StateSource::MicMuted) {
            self.mic_muted = crate::audio::muted(crate::audio::Kind::Mic).unwrap_or(self.mic_muted);
        }
        self.polled = Instant::now();
    }
}

/// Which buttons on `page` show their alternate look right now.
fn compute_active(cfg: &Config, page: u8, sys: &SysState) -> [bool; 14] {
    let toggles = TOGGLES.lock().unwrap_or_else(|e| e.into_inner());
    let mut a = [false; 14];
    for (i, slot) in a.iter_mut().enumerate() {
        *slot = match cfg.button_for_index(page, i as u8).and_then(|b| b.state_source()) {
            Some(StateSource::Toggle) => toggles.contains(&(page, i as u8)),
            Some(StateSource::OutputMuted) => sys.out_muted,
            Some(StateSource::MicMuted) => sys.mic_muted,
            None => false,
        };
    }
    a
}

/// Runs an action. Returns the new page when it was a page switch.
fn run_action(action: &Action, page: u8, cfg: &Config, ctx: &Ctx) -> Option<u8> {
    if let Action::Page { to } = action {
        let n = cfg.page_count();
        return Some(match to.as_str() {
            "next" => if page >= n { 1 } else { page + 1 },
            "prev" => if page <= 1 { n } else { page - 1 },
            "home" => 1,
            s => s.parse().unwrap_or(page).clamp(1, n),
        });
    }
    if let Action::Print { text } = action {
        ctx.log(format!("PRINT: {text}"));
        return None;
    }
    if let Err(e) = actions::execute(action) {
        ctx.log(format!("action failed: {e}"));
    }
    None
}

/// Runs a group of hold steps (all share one duration). Consecutive steps are chained into one macro that runs in order
/// on its own thread; a step marked `split` starts a separate macro that runs in parallel. Returns a page switch, if any.
fn run_hold_group(steps: Vec<crate::config::HoldStep>, index: u8, held_ms: u128, page: u8, cfg: &Config, ctx: &Ctx, glow: &mut GlowQueue) -> Option<u8> {
    glow.push((index, Glow::Hold));
    let mut jobs: Vec<Vec<Action>> = vec![Vec::new()];
    let mut switch = None;
    for s in steps.iter() {
        if s.split && !jobs.last().is_some_and(|j| j.is_empty()) {
            jobs.push(Vec::new());
        }
        if matches!(s.action, Action::Page { .. }) {
            switch = run_action(&s.action, page, cfg, ctx).or(switch);
        } else if let Some(j) = jobs.last_mut() {
            j.push(s.action.clone());
        }
    }
    jobs.retain(|j| !j.is_empty());
    ctx.log(format!(
        "button {index} hold detected (released after {held_ms} ms, tier {} ms): {} step(s), {} macro(s){}",
        steps.first().map_or(0, |s| s.ms),
        steps.len(),
        jobs.len(),
        if switch.is_some() { ", page switch" } else { "" }
    ));
    for job in jobs {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for (n, a) in job.iter().enumerate() {
                if n > 0 {
                    std::thread::sleep(Duration::from_millis(60)); // small gap so apps see each step
                }
                match a {
                    Action::Print { text } => ctx.log(format!("PRINT: {text}")),
                    other => {
                        if let Err(e) = actions::execute(other) {
                            ctx.log(format!("action failed: {e}"));
                        }
                    }
                }
            }
        });
    }
    switch
}

fn fire(which: Fire, index: u8, page: u8, cfg: &Config, ctx: &Ctx, glow: &mut GlowQueue) -> Option<u8> {
    let action = cfg.button_for_index(page, index).and_then(|b| match which {
        Fire::Single => b.action.as_ref(),
        Fire::Double => b.double.as_ref(),
        Fire::Repeat => b.repeat.as_ref(),
    });
    let gesture = match which {
        Fire::Single => "tap",
        Fire::Double => "double tap",
        Fire::Repeat => "repeat",
    };
    match action {
        Some(a) => {
            ctx.log(format!("button {index} {gesture} detected -> {a:?}"));
            if matches!(which, Fire::Single) && cfg.button_for_index(page, index).and_then(|b| b.state_source()) == Some(StateSource::Toggle) {
                flip_toggle(page, index); // the alternate look follows the tap
            }
            glow.push((index, match which {
                Fire::Single => Glow::Tap,
                Fire::Double => Glow::Double,
                Fire::Repeat => Glow::Repeat,
            }));
            run_action(a, page, cfg, ctx)
        }
        None => {
            ctx.log(format!("button {index} {gesture} detected, but nothing is assigned to it on page {page}"));
            None
        }
    }
}

/// One connected session. Returns an error when the deck goes away.
fn session(cfg: &mut Config, path: &Path, seen: &mut Option<SystemTime>, ctx: &Ctx) -> Result<(), String> {
    let api = HidApi::new().map_err(|e| e.to_string())?; // fresh enumeration on every (re)connect
    let dev = protocol::open(&api)?;
    let info = protocol::handshake(&dev, cfg.brightness)?;
    ctx.log("deck connected");
    ctx.with(|s| {
        s.connected = true;
        s.info = info;
    });
    let result = (|| {
        let mut page: u8 = 1;
        ctx.with(|s| s.page = page);
        let mut sampler = crate::sysstats::Sampler::new();
        std::thread::sleep(Duration::from_millis(50));
        sampler.sample();
        let anim_start = Instant::now();
        let mut last_sample = Instant::now();
        let mut sys = SysState::new();
        sys.poll(cfg, page);
        let mut live = make_live(&sampler, anim_start, compute_active(cfg, page, &sys));
        upload_layout(&dev, cfg, page, ctx, &live)?;
        // live faces / GIFs: when each cell's picture was last (re)sent; and which look (normal / alternate) it was drawn with
        let mut last_active = live.active;
        let mut last_draw = [Instant::now(); 14];
        let mut last_frame: [Option<usize>; 14] = [None; 14];
        let mut last_live = Instant::now();
        let mut last_beat = Instant::now();
        let mut last_check = Instant::now();
        let mut last_press = [Instant::now() - DEBOUNCE; 16];
        let mut keys = [Key::default(); 14];
        let mut wide = WideFilter::default();
        let mut switch: Option<u8> = None;
        let mut glow_q: GlowQueue = Vec::new();
        let mut glow_until: [Option<Instant>; 14] =[None; 14];
        let mut buf = [0u8; PACKET + 1];
        loop {
            if last_sample.elapsed() >= Duration::from_secs(1) {
                last_sample = Instant::now();
                sampler.sample();
            }
            sys.poll(cfg, page);
            live = make_live(&sampler, anim_start, compute_active(cfg, page, &sys));
            if let Some(p) = switch.take() {
                if p != page {
                    page = p;
                    keys = [Key::default(); 14];
                    glow_until = [None; 14]; // the full upload below redraws every tile
                    glow_q.clear();
                    ctx.with(|s| s.page = page);
                    sys.polled = Instant::now() - Duration::from_secs(5); // the new page may follow other devices: poll now
                    sys.poll(cfg, page);
                    live.active = compute_active(cfg, page, &sys);
                    upload_layout(&dev, cfg, page, ctx, &live)?;
                    last_draw = [Instant::now(); 14];
                    last_frame = [None; 14];
                    last_active = live.active;
                }
            }
            // the firmware's own stats readout needs fresh numbers every second; otherwise the 5 s heartbeat is enough
            let mode = cfg.wide_mode(page);
            let beat_every = if mode == 0 { Duration::from_secs(1) } else { HEARTBEAT_EVERY };
            if last_beat.elapsed() >= beat_every {
                protocol::heartbeat_mode(&dev, mode, live.stats.cpu.round() as u8, live.stats.mem.round() as u8)?;
                last_beat = Instant::now();
            }
            // live faces and GIF animations: re-send a cell's picture when its interval is up (not while a rim is shown or the
            // button is held down: an upload blocks the read loop and would delay the release event)
            if last_live.elapsed() >= Duration::from_millis(50) {
                last_live = Instant::now();
                for i in 0..14usize {
                    let Some(b) = cfg.button_for_index(page, i as u8) else { continue };
                    if b.face.is_firmware() || glow_until[i].is_some() || keys[i].down.is_some() {
                        continue;
                    }
                    // live faces redraw on their refresh interval; a GIF whenever its frame changes
                    let frame = layout::gif_frame_index(b, cfg, i == 13, live.tick_ms);
                    let due = live.active[i] != last_active[i]
                        || if b.face.is_live() {
                            last_draw[i].elapsed() >= Duration::from_millis(b.refresh_ms as u64)
                        } else {
                            frame.is_some() && frame != last_frame[i]
                        };
                    if due {
                        last_draw[i] = Instant::now();
                        last_frame[i] = frame;
                        last_active[i] = live.active[i];
                        if let Err(e) = send_cell(&dev, cfg, page, i, None, &live) {
                            ctx.log(format!("live redraw failed on button {i}: {e}"));
                        }
                    }
                }
            }
            if last_check.elapsed() >= Duration::from_secs(1) {
                last_check = Instant::now();
                let now = modified(path);
                if now != *seen {
                    *seen = now;
                    match Config::load(path) {
                        Ok(new_cfg) => {
                            *cfg = new_cfg;
                            if page > cfg.page_count() {
                                page = 1;
                                ctx.with(|s| s.page = page);
                            }
                            ctx.with(|s| s.config_error = None);
                            ctx.log("config reloaded");
                            glow_until = [None; 14];
                            glow_q.clear();
                            protocol::send(&dev, cmd::BRIGHTNESS, cfg.brightness.min(100).to_string().as_bytes())?;
                            sys.polled = Instant::now() - Duration::from_secs(5);
                            sys.poll(cfg, page);
                            live.active = compute_active(cfg, page, &sys);
                            upload_layout(&dev, cfg, page, ctx, &live)?;
                            last_draw = [Instant::now(); 14];
                            last_frame = [None; 14];
                            last_active = live.active;
                        }
                        Err(e) => {
                            ctx.log(format!("config not reloaded (keeping the old one): {e}"));
                            ctx.with(|s| s.config_error = Some(e));
                        }
                    }
                }
            }
            // timers: hold threshold reached, or the double-tap window ran out
            for i in 0..14usize {
                let b = cfg.button_for_index(page, i as u8);
                // repeat while held: starts after REPEAT_DELAY_MS, then every repeat_ms, until release (or the safety limit)
                if let (Some(d), Some(rep)) = (keys[i].down, b.and_then(|b| b.repeat.as_ref())) {
                    let every = Duration::from_millis(b.map_or(150, |b| b.repeat_ms) as u64);
                    if d.elapsed() >= Duration::from_secs(REPEAT_MAX_SECS) {
                        if keys[i].repeating {
                            ctx.log(format!("button {i}: repeat stopped (safety limit {REPEAT_MAX_SECS}s, release event never arrived)"));
                        }
                        keys[i].down = None;
                        keys[i].repeating = false;
                        keys[i].tier_shown = None;
                        glow_until[i] = Some(Instant::now()); // end the repeat's rim
                    } else if !keys[i].repeating && d.elapsed() >= Duration::from_millis(REPEAT_DELAY_MS) {
                        keys[i].repeating = true;
                        keys[i].tap_at = None;
                        keys[i].last_repeat = Some(Instant::now());
                        switch = fire(Fire::Repeat, i as u8, page, cfg, ctx, &mut glow_q).or(switch); // logs "repeat detected" once
                    } else if keys[i].repeating && keys[i].last_repeat.is_none_or(|t| t.elapsed() >= every) {
                        keys[i].last_repeat = Some(Instant::now());
                        let _ = run_action(rep, page, cfg, ctx); // quiet: no log line per repeat
                    }
                }
                // live hold feedback: each time a longer hold step is reached, the rim changes color (release to fire it)
                if cfg.feedback.enabled && !keys[i].repeating {
                    if let (Some(d), Some(b)) = (keys[i].down, b) {
                        let top = b.hold_group(d.elapsed().as_millis() as u64).first().map(|h| h.ms);
                        if let (Some(ms), true) = (top, top != keys[i].tier_shown) {
                            let fb = &cfg.feedback;
                            let rgb = fb.tier_color(b, ms);
                            match send_cell(&dev, cfg, page, i, Some((rgb, fb.width as u32, fb.radius as u32)), &live) {
                                Ok(()) => {
                                    keys[i].tier_shown = Some(ms);
                                    glow_until[i] = Some(Instant::now() + Duration::from_secs(REPEAT_MAX_SECS + 1)); // until release
                                    ctx.log(format!("button {i}: holding, reached the {ms} ms step (release to fire it)"));
                                }
                                Err(e) => ctx.log(format!("glow failed: {e}")),
                            }
                        }
                    }
                }
                if let Some(t) = keys[i].tap_at {
                    if t.elapsed() >= Duration::from_millis(DOUBLE_MS) {
                        keys[i].tap_at = None;
                        switch = fire(Fire::Single, i as u8, page, cfg, ctx, &mut glow_q).or(switch);
                    }
                }
            }
            // a tap may just have flipped a toggle: draw the rim on the new look, not the old one
            live.active = compute_active(cfg, page, &sys);
            // glow feedback: start requested rims, then restore tiles whose rim time is up
            let requests: Vec<(u8, Glow)> = glow_q.drain(..).collect();
            for (idx, g) in requests {
                let fb = &cfg.feedback;
                if !fb.enabled || idx as usize >= 14 {
                    continue;
                }
                let btn = cfg.button_for_index(page, idx);
                let hex = match g {
                    Glow::Tap => btn.and_then(|b| b.rim_tap.as_ref()).unwrap_or(&fb.tap),
                    Glow::Double => btn.and_then(|b| b.rim_double.as_ref()).unwrap_or(&fb.double),
                    Glow::Hold => &fb.hold,
                    Glow::Repeat => &fb.repeat,
                };
                let rgb = crate::config::parse_hex(hex).unwrap_or([255, 255, 255]);
                match send_cell(&dev, cfg, page, idx as usize, Some((rgb, fb.width as u32, fb.radius as u32)), &live) {
                    Ok(()) => {
                        ctx.log(format!("rim: {g:?} on button {idx} (#{:02x}{:02x}{:02x})", rgb[0], rgb[1], rgb[2]));
                        // a repeat keeps its rim until release (the release handler ends it)
                        let hold_for = match g {
                            Glow::Repeat => Duration::from_secs(REPEAT_MAX_SECS + 1),
                            _ => Duration::from_millis(fb.flash_ms as u64),
                        };
                        glow_until[idx as usize] = Some(Instant::now() + hold_for);
                    }
                    Err(e) => ctx.log(format!("glow failed: {e}")),
                }
            }
            for i in 0..14usize {
                if glow_until[i].is_some_and(|t| Instant::now() >= t) {
                    // same reason as above: never block the loop with a restore while the button is mid-press
                    // (a live hold/repeat rim is restored on release instead)
                    if keys[i].down.is_some() && !keys[i].repeating && keys[i].tier_shown.is_none() {
                        continue;
                    }
                    glow_until[i] = None;
                    last_draw[i] = Instant::now();
                    last_frame[i] = None;
                    last_active[i] = live.active[i];
                    if let Err(e) = send_cell(&dev, cfg, page, i, None, &live) {
                        ctx.log(format!("glow restore failed: {e}"));
                    }
                }
            }
            match dev.read(&mut buf).map_err(|e| format!("read error: {e}"))? {
                0 => std::thread::sleep(Duration::from_millis(10)),
                n => match protocol::parse(&buf[..n]) {
                    Some(Event::Button { index, state, pressed }) if (index as usize) < 14 && wide.accept(index, state, pressed) => {
                        let i = index as usize;
                        let b = cfg.button_for_index(page, index);
                        let (has_double, has_hold) = (b.is_some_and(|b| b.double.is_some()), b.is_some_and(|b| !b.holds.is_empty()));
                        if pressed {
                            if last_press[i].elapsed() < DEBOUNCE {
                                continue;
                            }
                            last_press[i] = Instant::now();
                            ctx.with(|s| s.pressed = Some((index, Instant::now())));
                            keys[i].down = Some(Instant::now());
                            if !has_double && !has_hold {
                                switch = fire(Fire::Single, index, page, cfg, ctx, &mut glow_q).or(switch);
                            }
                        } else {
                            let held = keys[i].down.take().map(|d| d.elapsed());
                            if keys[i].repeating {
                                // released after a repeat: that was a hold, not a tap
                                keys[i].repeating = false;
                                keys[i].tier_shown = None;
                                glow_until[i] = Some(Instant::now()); // end the repeat's rim
                                ctx.log(format!("button {i}: repeat stopped (released)"));
                                continue;
                            }
                            let Some(held) = held else { continue };
                            if !has_double && !has_hold {
                                continue;
                            }
                            // holds fire on release: the steps sharing the longest duration that was reached run as a macro
                            let group: Vec<crate::config::HoldStep> =
                                b.map(|b| b.hold_group(held.as_millis() as u64).into_iter().cloned().collect()).unwrap_or_default();
                            // the live hold rim doubles as the "fired" confirmation: keep it for flash_ms, then restore
                            let progress = keys[i].tier_shown.take().is_some();
                            if progress {
                                glow_until[i] = Some(Instant::now() + Duration::from_millis(cfg.feedback.flash_ms as u64));
                            }
                            if !group.is_empty() {
                                keys[i].tap_at = None;
                                switch = run_hold_group(group, index, held.as_millis(), page, cfg, ctx, &mut glow_q).or(switch);
                                if progress {
                                    glow_q.retain(|(ix, g)| !(*ix == index && *g == Glow::Hold));
                                }
                            } else if has_double {
                                if keys[i].tap_at.take().is_some() {
                                    switch = fire(Fire::Double, index, page, cfg, ctx, &mut glow_q).or(switch);
                                } else {
                                    keys[i].tap_at = Some(Instant::now());
                                }
                            } else {
                                switch = fire(Fire::Single, index, page, cfg, ctx, &mut glow_q).or(switch);
                            }
                        }
                    }
                    Some(Event::Other { cmd: cmd::CONFIG_REQUESTED, .. }) => {
                        ctx.log("deck asked for its layout again");
                        glow_until = [None; 14];
                        upload_layout(&dev, cfg, page, ctx, &live)?;
                        last_draw = [Instant::now(); 14];
                        last_frame = [None; 14];
                        last_active = live.active;
                    }
                    _ => {}
                },
            }
        }
    })();
    ctx.with(|s| s.connected = false);
    result
}

/// Never returns: keeps (re)connecting to the deck.
pub fn run(path: PathBuf, ctx: Ctx) {
    let mut seen = modified(&path);
    let mut cfg = match Config::load(&path) {
        Ok(c) => c,
        Err(e) => {
            ctx.log(format!("config problem: {e}"));
            ctx.with(|s| s.config_error = Some(e));
            Config::default()
        }
    };
    ctx.log(format!("service started ({})", path.display()));
    loop {
        if let Err(e) = session(&mut cfg, &path, &mut seen, &ctx) {
            ctx.log(e);
        }
        ctx.log("waiting for the deck (is Ulanzi Studio closed?)...");
        std::thread::sleep(Duration::from_secs(2));
    }
}

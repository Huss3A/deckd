# deckd

A lightweight Rust replacement for Ulanzi Studio, for personal use with my own **Ulanzi D200H** deck
(USB `2207:0019`). Single ~7 MB executable instead of a ~200 MB Qt/WebEngine app.

Protocol knowledge lives in [NOTES.md](NOTES.md). Instructions for AI agents working on or with this project: [AGENTS.md](AGENTS.md).

## Goals

1. **Replace Ulanzi Studio** with more granular control and a lighter footprint. *(core done, see Status)*
2. **A GUI** to design the deck: click a button, set label / color / symbol / image / action. *(done, needs refinement)*
3. **A CLI for agents.** AI agents should be able to inspect and change the deck, simulate presses, preview layouts and read
   events without a GUI, with machine-readable output and safe defaults. *(offline commands built: status, list, get, set, clear,
   set-brightness, validate, preview, press, schema; live `events`/connection status need IPC; see [AGENTS.md](AGENTS.md))*
4. Run at login, hidden, no console window. *(not built)*
5. Wide slot control: custom clock / system stats / image or GIF. *(researched, not built)*
6. Multi-page layouts *(built, untested on the deck)*, double-tap and hold actions *(built, untested on the deck)*;
   real program icons for `run` buttons *(not built)*.

## Status (2026-11-14)

Working and verified on the real deck:
- Full connect handshake and 5 s heartbeat (needed for the deck to report presses), brightness.
- Own layout rendering: colored buttons with symbol/label/image, built into the ZIP the deck expects, uploaded over HID.
- Button presses mapped to actions: `run`, `open`, `hotkey`, `media` (300 ms debounce).
- Config file `deck.toml`, hot reload, reconnect when the deck drops.
- GUI window (egui): clickable deck, button editor, brightness, live status/log. Edits save to `deck.toml`; the service re-uploads.

Not done: see Goals 3-6. Known: GUI needs UX refinement; saving rewrites `deck.toml` (comments are lost).

## Using it

```powershell
# cargo lives here on this machine
$env:Path += ";$env:USERPROFILE\.cargo\bin"
cargo build --release
.\target\release\deckd.exe            # opens the window (also runs the deck service)
```

**Close Ulanzi Studio first.** Only one program can hold the deck. The window, `serve`, and the debug commands all
compete for it: run only one at a time.

| Command | What it does |
|---|---|
| `deckd` / `deckd gui [deck.toml]` | Window + deck service |
| `deckd serve [deck.toml]` | Deck service in the console (no window) |
| `deckd init [deck.toml]` | Write the sample config |
| `deckd build [deck.toml] [out.zip]` | Render the layout ZIP without touching the deck (safe) |
| `deckd listen\|run [secs]` | Print button events (`run` also sends the heartbeat) |
| `deckd brightness <0-100>` | Set brightness |
| `deckd upload\|patch <zip> [secs]` | Send a raw layout ZIP (debugging) |
| `deckd status\|list\|get\|set\|clear\|set-brightness\|validate\|preview\|press\|schema` | Agent CLI: JSON output, file based, see AGENTS.md |

## Config (`deck.toml`)

Buttons are addressed `"col,row"` (cols 0-4, rows 0-2; row 2 has cols 0-2, and `"3,2"` is the wide slot, a 458x196 button; with
no button there it shows the firmware clock; `keep_clock = true` on that button keeps the clock and uses the slot for actions
only. Wide-slot support is built but only partly tried on the deck).
Button number reported by the deck = `row*5 + col`. See `deck.sample.toml` for every option.

```toml
[[button]]
pos = "0,0"
label = "Notepad"
color = "#1f6feb"
glyph = "✎"
action = { type = "run", command = "notepad.exe" }
```

Actions: `run {command,args}`, `open {target}`, `hotkey {keys}` (e.g. `win+shift+s`), `media {key}`
(`vol_up vol_down mute play_pause next prev stop`), `page {to}` (`next`, `prev`, `home` or a page number).
Nothing runs unless a button defines an action.

**Faces (what a button shows):** `face = "image"` (default: picture / symbol; an `icon` ending in `.gif` animates, using the GIF's own
frame timing, min 100 ms), `"live_clock"` (time drawn by deckd, `clock_format` strftime like `%H:%M` or `%H:%M\n%a %d %b`, colors =
`color` background / `glyph_color` text), `"live_stats"` (CPU and RAM, `stats_style` = `bars` | `line` | `text`; GPU is not sampled),
both redrawn every `refresh_ms` (200-60000, default 1000). Any button can use these. On the wide slot `"3,2"` only: `"clock"` keeps the
firmware clock and `"stats"` hands the slot to the firmware's CPU/RAM readout (heartbeat then carries the numbers every second); neither
draws a picture or a rim, so the button's actions still work under them. CLI: `deckd set 3,2 --face live_clock --clock-format %H:%M`.
(Live faces and GIFs are built but not yet tried on the deck; the firmware stats mode comes from community docs only.)

**State-aware buttons (icon flips, e.g. mute):** give a button an `[button.alt]` look (`label`, `color`, `glyph`, `glyph_color`, `icon`; whatever
is set replaces the normal look while the state is "on") and a `state`: `toggle` (default; flips on every tap of the normal action, kept
in memory only), `output_muted` (the default speaker is muted: follows the `mute` media key and anything else that mutes it) or
`mic_muted` (the default microphone is muted). The media key `mic_mute` flips the microphone mute directly, in every app, so a mic
button is `action = media mic_mute` + `state = "mic_muted"`. Window: "Alternate look" with one-click Mic mute / Speaker mute presets.
CLI: `deckd set 0,0 --action-type media --key mic_mute --state mic_muted --alt-label "Mic MUTED" --alt-color "#b62324"`,
`deckd preview --alt` renders the alternate looks, `deckd audio` prints the current mute states (read-only). Not tried on the deck yet.

**Pages:** top-level `pages = N` (1-9) and `page = 2` on a button (omitted = page 1). A `page` action on a button switches
the deck to another page (the service re-uploads that page's layout, ~1 s). Put a "back" button on each later page yourself.

**Extra gestures:** besides `action` (normal press) a button can have `double` (two quick taps, 0.35 s window), `holds`
(any number of hold steps, **fired on release**: `[[button.holds]] ms = 500` plus an `action`; `ms` is 100-10000. On release,
the steps that share the longest `ms` that was reached run; steps with the same `ms` are chained into one macro in list order,
and `split = true` on a step starts a separate macro that runs in parallel instead. Old `hold` / `hold_long` fields are
migrated automatically) and `repeat` (runs again and again while held, e.g. volume; starts after 0.4 s, every `repeat_ms`,
default 150, range 30-2000; stops on release, with a 20 s safety limit). With `double` set, the normal action waits for the
window to expire; with `hold` or `repeat`, a tap fires on release (so holding never also taps). `tap_hold` is a repeat for "tap, then press again
and keep holding" (the first tap then does not fire; a quick second tap is still a double tap); it shares `repeat_ms`. `window_ms`
(150-1000, default 350) is how long a tap waits for a second press. One-button audio: tap = `media mute` (with `state =
"output_muted"` for the flip icon), `repeat = media vol_up`, `tap_hold = media vol_down`. CLI: `set --on tap_hold --window-ms N`.
`print {text}` is an action that just writes to the log, for testing gestures.
**Feedback:** when a gesture fires, the button flashes a colored rim on the deck (`[feedback]`: `enabled`, `width` 2-40 px,
`radius` 0-60 px corner rounding, `flash_ms` 100-3000, colors `tap` / `double` / `hold` / `repeat`). A repeat keeps its rim while
you hold. While holding a button that has hold steps, the rim appears at each step you reach and changes color per step
(automatic hue shift, or a step's own `color`); releasing keeps the rim for `flash_ms` as the "fired" confirmation. After the
first tap of a button, nothing is shown while the double-tap window is open (an "armed" rim was tried and removed: it felt wrong).
Each button can override the tap and double-tap rim colors (`rim_tap`, `rim_double`, `#rrggbb`; window: "Rim:" row; CLI:
`set --rim-tap/--rim-double`, empty value resets). It works by re-sending
that one button's picture with a border, then the normal picture. Also in the window under "Button feedback", and `deckd feedback`.
Example volume-up button: `action = media vol_up` plus `repeat = media vol_up` (tap = one step, hold = continuous).

## Window styles (skins)

The window's look is separate from the deck config. A style is a TOML file: drop `name.toml` (or a folder `name\style.toml` that
can also hold a font and a background image) into `<config folder>\styles\` (default `%APPDATA%\deckd\styles`), then pick it in
the window: gear icon, Settings, Window style. Saving the file updates the open window live. A style can change colors, corner
rounding, deck key size and gap, font and sizes, and the layout: which side the editor is on, its width, compact or comfortable
spacing, tab look (pills / underline / boxed), deck alignment, hints on or off, and a background picture. Anything left out falls
back to its `base` (dark or light), so a style can be three lines. A broken style never breaks the window: the previous look stays
and Settings shows what is wrong. "Create a style to edit" writes a commented template. Bundled examples: Midnight, Paper, Terminal
(copied into the styles folder on first run; source in `styles/`). The choice is stored in `ui.toml` beside `deck.toml`.
Styles only change the window, never what the deck shows.

**Window shortcuts:** Ctrl+Z / Ctrl+Y undo and redo (one step per saved batch of edits), Ctrl+C / Ctrl+V copy and paste a button
(also as TOML text on the clipboard), Ctrl+D duplicates to the next free key, Del clears, arrow keys move the selection, right-click
a key for the same actions. The editor has Look / Actions / State tabs with a live preview of the tile rendered by the same code
that feeds the deck; per-button rim colors sit next to the gesture they belong to; global feedback settings are in Settings.

## Layout of the repo

| Path | Purpose |
|---|---|
| `src/protocol.rs` | HID framing, handshake, ZIP upload |
| `src/layout.rs` | Renders icons, builds the layout ZIP (see gotchas in NOTES.md) |
| `src/config.rs` | `deck.toml` types, load/save/validate |
| `src/actions.rs` | run / open / hotkey / media execution |
| `src/service.rs` | Background service: connect, upload, events, reload, reconnect |
| `src/gui.rs` | egui window |
| `src/agent.rs` | Agent CLI commands (JSON, exit codes) |
| `src/main.rs` | CLI entry |
| `tools/hook_hid.js` | Frida hook that logged Ulanzi Studio's HID traffic |
| `tools/*.py`, `tools/*.ps1` | Early test scripts |
| `captures/` | Captured traffic and real layout ZIPs from Ulanzi Studio |

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
no button there it shows the firmware clock. Wide-slot support is built but not yet tried on the deck).
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

**Pages:** top-level `pages = N` (1-9) and `page = 2` on a button (omitted = page 1). A `page` action on a button switches
the deck to another page (the service re-uploads that page's layout, ~1 s). Put a "back" button on each later page yourself.

**Extra gestures:** besides `action` (normal press) a button can have `double` (two quick taps, 0.35 s window), `holds`
(any number of hold steps, **fired on release**: `[[button.holds]] ms = 500` plus an `action`; `ms` is 100-10000. On release,
the steps that share the longest `ms` that was reached run; steps with the same `ms` are chained into one macro in list order,
and `split = true` on a step starts a separate macro that runs in parallel instead. Old `hold` / `hold_long` fields are
migrated automatically) and `repeat` (runs again and again while held, e.g. volume; starts after 0.4 s, every `repeat_ms`,
default 150, range 30-2000; stops on release, with a 20 s safety limit). With `double` set, the normal action waits for the
window to expire; with only `hold`, a tap fires on release; with only `repeat`, the normal action still fires instantly on
press. `print {text}` is an action that just writes to the log, for testing gestures.
**Feedback:** when a gesture fires, the button flashes a colored rim on the deck (`[feedback]`: `enabled`, `width` 2-40 px,
`radius` 0-60 px corner rounding, `flash_ms` 100-3000, colors `tap` / `double` / `hold` / `repeat`). A repeat keeps its rim while
you hold. While holding a button that has hold steps, the rim appears at each step you reach and changes color per step
(automatic hue shift, or a step's own `color`); releasing keeps the rim for `flash_ms` as the "fired" confirmation. After the
first tap of a button, nothing is shown while the double-tap window is open (an "armed" rim was tried and removed: it felt wrong).
Each button can override the tap and double-tap rim colors (`rim_tap`, `rim_double`, `#rrggbb`; window: "Rim:" row; CLI:
`set --rim-tap/--rim-double`, empty value resets). It works by re-sending
that one button's picture with a border, then the normal picture. Also in the window under "Button feedback", and `deckd feedback`.
Example volume-up button: `action = media vol_up` plus `repeat = media vol_up` (tap = one step, hold = continuous).

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

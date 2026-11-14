# Instructions for AI agents

Project: `deckd`, a Rust app that drives the owner's **Ulanzi D200H** deck (USB `2207:0019`). Read [README.md](README.md)
for goals/status and [NOTES.md](NOTES.md) for the wire protocol and the hard-won gotchas. The owner is on Windows 11,
PowerShell is the primary shell.

## Rules for working on this project

- **Only one process may hold the deck.** Ulanzi Studio (`UlanziDeck.exe`), `deckd gui`, `deckd serve`, and the debug
  commands (`listen`, `run`, `upload`, ...) all compete. Before using the deck, make sure none of the others is running.
  Stop things **narrowly by name** (`Stop-Process -Name deckd`). Never kill broad sets (all `python.exe` was denied once).
- **Ask the owner before sending anything new to the deck**: a command ID not listed as verified in NOTES.md, firmware-level
  anything, or anything that could change stored state. Known-safe and already used: brightness, heartbeat, GETBASE,
  label style, full/partial layout ZIPs (partial `0x000D` verified 2026-11-14 via the rim glow). Never flash firmware.
- Prefer verification that does not need the deck: `deckd build deck.toml out.zip` renders the full layout offline;
  extract it and look at the PNGs (contact-sheet trick: draw `Images/*.png` into a grid with System.Drawing).
- Do not run actions blindly: pressing a configured button really runs programs / sends keys. Physical-press testing needs
  the owner at the deck.
- The owner has to confirm what the deck screen shows; an agent cannot see it. Say what you expect, then ask.
- Installing software (Rust, USBPcap, Wireshark, Frida tooling) was approved case by case earlier. Ask again for anything new.
- **Edit source with the Edit tool, not scripted find/replace.** There is no git history here; a PowerShell `.Replace()` loop over a
  single-element array once iterated a string's characters and rewrote every capital `H` as `o` in two files (repaired by hand).
  If you must script a replace, test it on a copy first. Consider `git init` before large refactors.
- After changing code, rebuild: `$env:Path += ";$env:USERPROFILE\.cargo\bin"; cargo build --release` (in `D:\My Deck App`).
  Stop a running `deckd` first or the exe is locked.

## Gotchas that cost hours (details in NOTES.md)

1. The deck stays in "connected mode" (sends press events) only while the host sends the `0x0006` heartbeat (every ~5 s).
2. Writes need a leading `0x00` report-ID byte (1025-byte HID write); reads are 1024 bytes.
3. Layout ZIP entries must carry **current timestamps** (the `zip` crate's default 1980 date makes the deck ignore images).
4. Image **filenames must change whenever the picture changes** (the deck keeps the first image unpacked under a name).
   `layout.rs` names images `Images/<col>_<row>_<content-hash>.png`.
5. Button number = `row*5 + col`; wide clock slot (`3_2`) is button 13.
6. A bad USB cable once caused phantom volume keys, odd enumeration (Billboard device) and no events. Suspect cabling early.

## CLI for agents (offline part BUILT in `src/agent.rs`; live part not built)

The owner wants a CLI so agents can operate the deck without the GUI. **Built and tested (file based, no deck needed):**
`status list get set clear set-brightness validate preview press schema`. All print one JSON document on stdout (no `--json`
flag needed), errors go to stderr as `{"error":{"code","message"}}`, exit codes 0/1/2/3/4 as below. Global flag
`--config <path>` (default `deck.toml`). `set`/`clear`/`set-brightness` take `--dry-run`; `press` only runs with `--execute`.
Examples:
```
deckd set 4,0 --label Test --color "#aa3300" --glyph "★" --action-type hotkey --keys ctrl+alt+t
deckd set 0,1 --action-type run --command notepad.exe --arg file.txt     # --arg repeats; empty value ("--label ''") clears a field
deckd press 4,0            # shows what would happen; add --execute to really do it
deckd preview --out C:\tmp\prev ; deckd validate ; deckd schema
```
Pages and gestures: `--page N` (default 1) on `get set clear press`; `set --on press|double|hold|repeat` picks where the action goes. `--on hold` ADDS a hold step
(`--hold-ms` 100-10000, default 500; same duration = chained macro; `--split` = separate parallel macro; `--action-type none --on hold --hold-ms N`
removes the steps at N; `--clear-holds` removes all). `press <pos> --on hold --hold-ms N` previews which steps a release after N ms runs.
`feedback [--enabled true|false --width N --ms N --tap/--double/--hold/--repeat #rrggbb]` edits (or with no flags prints) the
rim-glow settings. Other: `--repeat-ms 30-2000`; `--action-type print --text ...` logs a message, useful to test gestures); `--action-type page --to next|prev|home|N`. Setting a button on a page beyond the current count creates that page.
`set` validates (positions, actions, renders the ZIP) before saving, then saves atomically; a running window/service
reloads within ~1 s. Still missing: `events` and live `status.deck_connected` (need IPC), `wide`. Contract:

**Principles**
- Non-interactive: never prompt, never open a window, never block on input.
- `--json` on every command: one JSON document on stdout; errors as `{"error": {"code": "...", "message": "..."}}` on stderr.
- Stable exit codes: `0` ok, `1` internal/other error, `2` usage error, `3` deck not connected, `4` config invalid.
- Safe defaults: anything that executes a button action or writes to the deck needs an explicit flag (`--execute`, `--apply`).
  Everything else is dry-run / read-only.
- Same validation as the GUI (`actions::validate`, `layout::build_zip`); invalid input is rejected before anything is saved.
- Edits go through `Config::save` (atomic temp file + rename), so a running `deckd gui`/`serve` reloads within ~1 s.

**Planned commands** (names are a proposal; keep them consistent once chosen)
| Command | Purpose |
|---|---|
| `deckd status --json` | Is the service running / deck connected, serial, firmware, config path |
| `deckd list --json` | All configured buttons |
| `deckd get <col,row> --json` | One button |
| `deckd set <col,row> [--label --color --glyph --glyph-color --icon] [--action-type run\|open\|hotkey\|media ...]` | Create/update a button |
| `deckd clear <col,row>` | Remove a button |
| `deckd brightness <0-100>` | Set brightness in config (and live if the service runs) |
| `deckd validate [deck.toml]` | Check the config without side effects |
| `deckd preview --out <dir>` | Render icons / contact sheet PNG without the deck |
| `deckd press <col,row\|index> [--execute]` | Show what a press would do; run it only with `--execute` |
| `deckd events --json` | Stream button events as NDJSON (needs IPC to the running service) |
| `deckd schema` | Print a JSON Schema of `deck.toml` so agents can generate valid configs |
| `deckd wide ...` | Wide slot (clock / stats / image) once that feature exists |

**Live queries** (`status`, `events`, applying changes instantly) need the CLI to talk to the running service.
Options to decide: a localhost-only socket with a token file, a named pipe, or a `status.json` that the service writes.
File-based edits to `deck.toml` already work without IPC.

**Until the CLI exists**, an agent can: edit `deck.toml` directly (see `deck.sample.toml`; run `deckd build` to validate and
render offline), and use `deckd serve`/the GUI to apply it. Do not hand-edit while the GUI has unsaved changes.

## Where things are

`src/protocol.rs` (wire), `layout.rs` (ZIP/icons), `config.rs` (deck.toml), `actions.rs` (what presses do),
`service.rs` (connect/upload/events/reload), `gui.rs` (egui), `main.rs` (commands). Real captured traffic and layouts from
Ulanzi Studio are in `captures/`; `tools/hook_hid.js` is the Frida hook used to capture them.

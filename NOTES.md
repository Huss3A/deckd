# Ulanzi D200H protocol notes

Goal: a small Rust replacement for Ulanzi Studio for personal use on my own D200H.
Status key: **[verified]** = tested on my unit (own client or captured from Ulanzi Studio with Frida), **[doc]** = from community projects only.

## Device
- USB `2207:0019`, manufacturer string "Zkswe". **[verified]**
- It reports itself as `DeviceType: "D200"`, `Dversion` 5.3.6, `HardwareVersion` SSD210V100 in the GETBASE reply. **[verified]**
- HID interface 0 (usage page 0x0C) is the data channel; interface 1 is a plain keyboard. **[verified]**
- Only one process should use the HID interface: quit Ulanzi Studio (`UlanziDeck.exe`) first. **[verified]**

## Framing **[verified]**
- Packet = 1024 bytes: `7C 7C` | command (u16, **big-endian**) | payload length (u32, **little-endian**) | payload, zero-padded.
- Writes need a leading `0x00` report-ID byte (1025-byte HID write). Reads come back as 1024 bytes without it.

## Connected mode needs a heartbeat **[verified]**
- The deck reports `0x0101` button events only while the host keeps talking to it. Without host traffic it falls back to
  standalone mode (buttons act on their own, e.g. launched Notepad/Calculator through its HID keyboard interface).
- Ulanzi Studio sends small-window data `0x0006` every ~5 s: `mode|cpu|mem|HH:MM:SS|gpu|12H|` (mode 1 = clock).
  A 1 s interval also worked in my tests. `deckd` uses 5 s.

## Connect sequence used by Ulanzi Studio (captured with Frida) **[verified]**
1. `0x0006` heartbeat
2. `0x0003` GETBASE -> deck replies `0x0303` JSON `{SerialNumber, Dversion, error:"0", DeviceType, HardwareVersion}`
3. `0x000B` label style JSON, then `0x0003` GETBASE again
4. `0x000A` brightness (ASCII "86"), `0x001A` payload `"0"` (**meaning unknown**, the app always sends it after brightness)
5. `0x0001` full layout ZIP

## Commands
| cmd | meaning | status |
|---|---|---|
| `0x0001` | full layout ZIP (see below). First packet: total length + first 1016 bytes; rest as raw 1024-byte chunks, zero-padded | **[verified]** with deckd |
| `0x000D` | partial update: same ZIP format containing only the changed cells + their images | captured, deckd `patch` untested |
| `0x0003` | GETBASE barrier, reply `0x0303` | **[verified]** |
| `0x0006` | small window / heartbeat | **[verified]** |
| `0x000A` | brightness, ASCII 0-100 (the app's slider sends every step, e.g. 87, 86, 85 ...) | **[verified]** |
| `0x000B` | label style JSON `{Align, Color(int), FontName, ShowTitle, Size, Weight}` | captured |
| `0x001A` | unknown, payload `"0"` | captured, unknown |

## Layout ZIP **[verified]**
- `manifest.json` + `Images/<uuid>.png|jpg`. Icons are 196x196 (PNG with alpha or JPG).
- Manifest keys are `"col_row"` (cols 0-4, rows 0-2). Each cell:
  `{"State":0,"ViewParam":[{"Font":{Align,Color,FontName,ShowTitle,Size,Weight},"Icon":"Images/<file>","Text":""}]}`
- Cell `3_2` is the wide clock slot: `{"SmallViewMode":1,"State":0,"ViewParam":[{"Font":{...},"Text":""}]}` with no icon.
- Real examples: `captures/x_zip_1_cmd0001/` (full) and `captures/x_zip_3_cmd000d/` (one-cell patch).

## Layout ZIP gotchas **[verified]**
- **Entry timestamps must be current.** The Rust `zip` crate's default 1980-01-01 timestamp makes the deck ignore the images
  (only the manifest `Text` labels show). Same content re-written by Python with the current time showed every icon.
  `layout.rs` sets `last_modified_time(now)`. (Bisect: `tools/make_bisect.py`, `captures/bisect*.zip`.)
- **Image filenames must change when the picture changes.** The deck keeps the first image it unpacked under a given name and
  ignores later ones with the same name (labels still update because the manifest is re-read). Symptom: edits to color/symbol/
  image never show, and a new button whose position name already held a default image stays blank. `layout.rs` now names
  images `Images/<col>_<row>_<fnv1a-64 of the PNG>.png`; the stock app uses a fresh uuid each time. **[verified]**
- The stock app also starts the ZIP with an explicit `Images/` directory entry; we keep it (not proven necessary).
- Our own PNGs (plain `Images/sigma.png` names or uuid names) display fine; only the container mattered.

## Faces: live clock / stats / GIF (built 2026-11-14, offline-verified only)
- `layout::Live{now, stats, tick_ms}` is what changes between redraws; `render_icon` draws live clock/stats and picks the GIF frame.
  The service re-sends a cell as a 0x000D partial update (no rim, button not held) when its `refresh_ms` is up or its GIF frame changes;
  rims restore the *current* frame. Stats come from `sysstats.rs` (GetSystemTimes + GlobalMemoryStatusEx, once a second, 60 samples).
- Firmware faces on the wide slot: `clock` = manifest `SmallViewMode` 1, `stats` = `SmallViewMode` 0 (no icon) and heartbeat mode 0 with
  `cpu`/`mem` percent in fields 2-3 (community docs, unverified on the deck), heartbeat every 1 s in stats mode.
- Wide slot own picture (mode 2) is verified working (owner, 2026-11-14). Added crates: `gif` (image feature), no others.

## Wide slot as a button (built 2026-11-14; picture mode and tap/hold verified by the owner)
- Config pos `"3,2"` is the wide slot (`"4,2"` is rejected). With no button there, the layout keeps the firmware clock (`SmallViewMode` 1,
  heartbeat mode 1). With a button, the cell is `{"SmallViewMode":2, ..., "Icon": <458x196 png>}` and the heartbeat's first field is 2.
  Both come from community docs only; if the deck shows nothing / a broken slot, that is the first suspect.
- **Double tap on the wide slot is impossible (measured 2026-11-14, 5 beep-cued taps):** single, double and triple taps all gave the
  same two pairs (`c8` then `c9`, once `c8` twice, ~0.2 s apart). The deck does not report extra taps; counting burst pairs as presses
  would turn every single tap into a double. Use tap and hold only. Other apps: racerxdl/ulanzi-d200-linux documents the slot as button
  14 (index 13) usable as a normal button alongside its stats/GIF display, but says nothing about the state bytes or gestures.
- Events: the burst filter was REMOVED at the owner's request (nothing is dropped; every wide-slot down/up is a real press/release; see git history for the filter). Expect one physical tap to look like 2-3 quick taps.
  wide slot may not work: the burst lasts ~0.9 s and the filter ignores presses within 0.7 s of the last event. Test and adjust.

## Wide slot (button 13, manifest cell `3_2`) **[doc, not yet tested on my deck]**
- From racerxdl/ulanzi-d200-linux: the wide slot is a **458 x 196 px** area. Modes: 1 = firmware clock (default),
  2 = background image (host sends a 458x196 PNG into the slot with a partial update `0x000D`, cell `3_2`, normal `Icon`),
  0 = firmware stats (strmdck). Linux renders its own stats graphs / GIF frames host-side and sends them as that image
  (GIF: frame names hashed, 0.1-4 s per frame, first frame sent after the full layout).
- The mode is set both in the manifest cell (`SmallViewMode`) and in the `0x0006` heartbeat (first field).
- Heartbeat carries real cpu/mem/gpu values (Ulanzi Studio sends them even in clock mode) and a `12H` token (maybe `24H` switches the clock format).
- Same rule applies as for buttons: changed images need new filenames.

## Incoming
- `0x0101` button event **[verified]**: payload = `[state, index, 0x01, pressed]` (bytes 8..11); `pressed` is 1 down / 0 up.
  - Buttons 0-12 send state `0x01`. Index 13 is the wide clock slot and it behaves differently. **Decoded 2026-11-14** (timestamped
    `deckd run` captures with beep-cued actions; owner confirmed it is ONE button, left/middle/right gave the same patterns):
    - It is silent when untouched (25 s idle: no events), so the stream is not caused by our heartbeat.
    - **Short tap = a burst** of quick DOWN/up pairs (~100 ms each, ~370 ms apart) whose state steps `c8, c9, ca` (longer bursts seen
      with the full cycle `c8 c9 ca cb 00 02 64 01`, likely the firmware's own display-mode values; mapping unproven).
    - **Long press (>~1 s) = one DOWN (`c8`) ... one up whose state is `01`**, duration matches the real press (hold 2 s -> 2.3 s).
    - A slide (touch left, drag right) gave one DOWN `c8` ... up `c8` ~1 s later.
    - Plan for using it as a button: treat a burst (gap < ~700 ms between events) as ONE tap, a DOWN with no up for > ~300 ms as a
      hold (release = the up). Not implemented. Open: why the burst, whether the display changes on tap (owner to say).
  - **Mapping verified** (numbered test layout, `tools/make_map_test.ps1`): index = row*5 + col, i.e. `col_row` key `c_r` -> index `r*5 + c`.
    Top row = 0-4, middle row = 5-9, bottom row = 10-12 (cols 0-2), wide slot `3_2` (+ hidden `4_2`) = 13.
- `0x0102` button event **[doc]**, `0x010B` firmware asks for the layout to be re-sent **[doc]**.

## Tools in this folder
- `src/` is the app (see README.md for the module map and the command table).
- `tools/hook_hid.js`: Frida hook for UlanziDeck.exe (logs WriteFile/ReadFile traffic, reassembles uploaded ZIPs into `captures/`).
  How it was used: launch Ulanzi Studio normally, attach with REA's `start_frida_session` (attach, by pid), `load_frida_script`, then
  replug the deck / change brightness / edit buttons in the app; read `captures/traffic.log`. Close the Frida session and Studio afterwards.
- `tools/make_map_test.ps1` (numbered cells, used to verify the button mapping), `tools/make_bisect.py` (mixes stock and own ZIP parts;
  how the timestamp problem was found), `tools/listen.py`, `tools/brightness.py` (early Python tests).
- USBPcap + Wireshark were installed but never used: the xHCI root hubs are AMD USB 3 and need USBPcap's `-I` registry step plus a reboot.

## Pages and gestures (deckd feature, not protocol)
- A page is just another full layout ZIP; a `page` action makes the service upload that page's ZIP (same mechanism as a config reload).
- Double tap / hold are timed host-side from the `0x0101` down/up events (debounce 40 ms, double window 350 ms, hold 500 ms).
  Needs reliable "up" events (seen working for buttons 0-12). Tap / double / hold **verified on the deck 2026-11-14** (via `print` actions).
- Holds are a list of steps (`holds`, each with `ms`); on release the group with the longest `ms` reached runs. Same-`ms` steps chain
  into one macro (sequential, 60 ms apart, own thread); `split` steps start parallel macros. Hard-split meaning was my interpretation
  (owner asked for "a hard split" without defining it). Not yet verified on the deck.
- **Rim glow [verified on the deck 2026-11-14]**: no LEDs; the rim is drawn into the button's picture and sent as a one-cell partial
  update (`0x000D`, same ZIP shape as the stock app's: `Images/` dir entry + one PNG + manifest with just that cell). The deck accepts
  it with no barrier packets, and a 10 px rim was visible (owner confirmed). Restoring = sending the normal tile again (same hashed
  filename as in the full layout). The icon also appeared to "shrink" during the first test (unexplained, maybe the deck's own
  reaction to a cell update). `deckd glowtest [col,row]` is the standalone experiment; `deckd rimpreview <out.png>` renders a rimmed
  tile offline (rim is a rounded-rect distance field: solid band, 6 px fade, black outside the rounded corner). Wired into the service as `[feedback]`
  in deck.toml (per-gesture colors, width, duration; a repeat keeps its rim until release).
- `hold` fires on **release** (down >= 500 ms), not at the 0.5 s mark (owner preference 2026-11-14; changed from fire-at-threshold).
- `repeat`: starts 400 ms after press, then every `repeat_ms` until the release event; stops after 20 s regardless (lost-release safety).
  Not yet verified on the deck.
- GUI gotcha: a text field plus a button wider than a resizable egui side panel made the panel grow every frame
  (clicking "Program" covered the screen). The editor panel is now fixed width and fields leave ~110 px for the button.

## Window / service facts
- The GUI saves `deck.toml` (debounced 0.5 s, atomic temp+rename); the service thread polls the file mtime every 1 s, re-uploads the full
  layout and re-sends brightness. A bad edit is rejected before saving (same validation as the service, incl. rendering the ZIP).
- The service also re-uploads when the deck sends `0x010B` **[doc, never seen on my unit]**.

## Sources
- redphx/strmdck (Python, MIT): https://github.com/redphx/strmdck
- racerxdl/ulanzi-d200-linux (Python, MIT): https://github.com/racerxdl/ulanzi-d200-linux
- Ulanzi plugin SDK (plugins inside Ulanzi Studio only): https://github.com/UlanziTechnology/UlanziDeckPlugin-SDK

## Open questions
- Meaning of `0x001A`; what the state byte on index 13 means (`0xc8`...).
- Does the deck keep its layout without a heartbeat, and does it send `0x010B` after a power cycle?
- USB-C Billboard device (057E:0000) seen earlier was the cable/port, not the deck protocol.

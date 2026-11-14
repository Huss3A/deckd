# deckd UI research and redesign proposal

Date: 2026-11-14. Read-only research; nothing else in the repo was touched.
Confidence tags: **[src]** = taken from a page I fetched (linked); **[mem]** = general knowledge of the product, not re-verified this session. Several vendor pages (Elgato help articles, Ulanzi manuals) were blocked or marketing-only, so the Stream Deck and Ulanzi Studio sections lean on [mem]. Check those before copying behaviour exactly.

## Current deckd, in one paragraph

Layout today (`src/gui.rs`): bottom status bar and collapsible log; fixed 400 px right panel holding one long scrolling button editor; central panel with brightness slider, page tabs, the 5x3 deck (click to select, drag to swap, Delete to clear), a hint line, and a collapsible feedback section. Config (`src/config.rs`): each `Button` has fixed gesture slots (`action`, `double`, `holds: Vec<HoldStep{ms, action, color, split}>`, `repeat`+`repeat_ms`), a look (`label/color/icon/glyph/glyph_color`), `face`, and a state-aware `alt` look driven by `state`. An `Action` is a single enum value (`Run/Open/Hotkey/Media/Page/Print`), so there is **no ordered multi-step macro with delays**. The only "macro" is hold steps sharing a duration. That is the biggest gap against every app below.

---

## (a) Findings per app

### Elgato Stream Deck (benchmark) [mem unless linked]
- **Layout:** three zones. Top bar: device and profile dropdowns. Centre: device preview with the key grid. Right: action library, grouped by category (System, Hotkey, Website, Open, Multi Action, Navigation...) with a search box. Under the grid: property inspector for the selected key (title, font, alignment, icon picker, per-action settings). Empty state is the grid itself: empty keys are drop targets.
- **Actions are dragged** from the library onto a key. Dragging key to key swaps; modifier-drag copies. Right-click on a key gives copy, paste, cut, delete.
- **Multi Action:** a container action. You drag actions into an ordered list inside it; items higher run first. A Delay is its own draggable step in ms, plus per-action timing (gap between virtual key press and release; pause before the next action) ([Elgato: multi action](https://www.elgato.com/eu/sk/explorer/products/stream-deck/what-is-a-multi-action-for-stream-deck/), [delay settings](https://help.elgato.com/hc/en-us/articles/7092527123725)). Nesting a Multi Action inside another is not allowed. **Multi Action Switch** is the same list but each press runs the next entry, so it cycles through states [mem].
- **States:** a key can have 2 states (toggle actions); the editor shows State 1 / State 2 tabs, each with its own title, icon, font. [mem]
- **Folders and profiles:** a folder is a key that opens a second layer with a back key; profiles are named sets, auto-switching by focused app is available ([Elgato overview](https://www.elgato.com/eu/hu/explorer/products/stream-deck/how-to-be-more-productive-with-stream-deck/)). Per-device profile import/export as a single file. [mem]
- **Gestures:** the stock app only fires on key down (some actions on key up). Long press/double press come from plugins; this is exactly the gap Companion and deckd fill.
- **Polish touches:** title preview updates live while typing, built-in icon packs plus an icon maker, drag an image file onto the icon, undo/redo, dark theme, tray icon and start at login. [mem]

### Bitfocus Companion (best gesture/state model)
- Per button: **Press actions**, **Release actions**, and optional **duration groups** for long press (add a group, set its ms, choose fire-on-release or fire-while-held). With a duration group, the release list becomes "short release" and only runs if released before the first duration ([actions docs](https://companion.free/user-guide/v5.0/config/buttons/creating/actions)).
- **Delay = a Wait action** in the list. Lists can contain **Action Groups** with mode Inherit / Concurrent / Sequential, so you pick parallel vs ordered per group ([same page](https://companion.free/user-guide/v5.0/config/buttons/creating/actions)).
- **Steps:** a button can have several steps; each press runs the next step's actions and cycles, with step progression switchable off so an expression/action sets the step ([steps docs](https://companion.free/user-guide/beta/config/buttons/creating/steps)).
- **Feedbacks:** boolean conditions that override the button's style (colour, text, icon) while true. In 5.0 buttons are layered graphic elements and feedbacks override element properties ([search summary of v5 docs](https://companion.free/user-guide/beta/config/buttons/creating/feedbacks)). This is the proper generalisation of deckd's `alt` look.
- **Triggers** run actions on events or schedules, separate from buttons. Variables like `$(internal:time_hms)` can go in labels. [mem]
- UX: grid + button editor with tabs (Style / Actions / Feedbacks), live preview, copy/paste/drag buttons, per-action "Learn" recorders for some modules. [mem]
- Open request showing demand for hold-aware visuals: [issue #2833](https://github-redirect.dependabot.com/bitfocus/companion/issues/2833).

### OpenDeck (Tauri/Rust + Svelte) [src: [README](https://github.com/nekename/OpenDeck)]
- Copies Stream Deck's model on purpose: device and profile dropdowns top-right; left-click an action opens its property inspector; right-click offers Edit (image/text per state) and Delete; click the image to pick a file, right-click to reset it.
- Multi Action and Toggle Action built in; profile folders via `Folder/Name` naming; profiles can bind to a focused app. Runs Elgato SDK plugins. Lesson: copying the Elgato mental model is accepted, and a small set of well-chosen controls is enough.

### StreamController (Python/GTK4) [src: [README](https://github.com/StreamController/StreamController)]
- Notable ideas: **sticky actions** that show on every page without copying; auto page switch by focused window; page backgrounds (image/video); screensaver and auto-lock; CLI/D-Bus API to switch pages, set states, emulate presses; pages editable offline. Plugin store inside the app. (README has no UI text; layout is from its screenshot only.)

### Ulanzi Studio and Linux clients for the D200
- **Ulanzi Studio** (marketing pages only, so [mem] for details): up to 10 folders per profile, profile/subpage/folder navigation, multi-action macros, hotkey actions, plugin store (OBS, Photoshop, weather, clock), wide window showing clock / CPU/RAM/GPU / custom background ([Ulanzi blog](https://www.ulanzi.com/blogs/news/stream-deck-d200), [D200H product page](https://www.ulanzi.com/products/d200h-deck-dock)). I could not confirm long-press or "hotkey switch" from public pages.
- **racerxdl/ulanzi-d200-linux** (Python, MIT) [src: [README](https://github.com/racerxdl/ulanzi-d200-linux)]: YAML `buttons:` list with `icon_spec` (generated solid/gradient/text icons), `label`, `action` (command/app/key/obs) + `params`; daemon that restores the config after reconnect; **local web UI** with drag-to-rearrange, icon upload, **desktop app picker** that imports installed apps' icons, **background editor that spreads one image across all keys**, named **layout presets as cards**, wide slot with CPU/RAM/GPU text/bars/line or a GIF.
- **redphx/strmdck** is the protocol reference several projects cite; **jcalado/companion-surface-d200** exposes the D200 as a Companion surface (so Companion's whole editor can drive it) ([ecosyste.ms listing](https://awesome.ecosyste.ms/projects/github.com%2Fjcalado%2Fcompanion-surface-d200)). Ideas: layout presets, app-icon import, whole-deck background tiling.

### Others (brief)
- **Touch Portal** [src: [button edit window](https://www.touch-portal.com/blog/post/tutorials/button-edit-window-explanation.php)]: four areas: live Button Preview (with "snapshot" for sharing), Button Settings, an Actions list grouped by category where **clicking an action appends it** to the list, and three event lists: On Press, On Event, On Hold (hold added later, limited actions). Logic (If/Else) is itself an action wrapped around others ([Logic reference](https://www.touch-portal.com/docs/logic/)). Global Events live apart from buttons.
- **streamdeck-ui (Crosley and forks)** [src: [repo](https://github.com/timothycrosley/streamdeck-ui)]: minimal: per-button text, icon, command, key press, page switch, per-button choice of icon+text / icon only / text only; abandoned original, forks point people to StreamController. Lesson: a thin editor is why people leave; no macros, no states.
- **Loupedeck, Deckboard, Mirabox/VSD:** not researched in depth this pass (no sources fetched). Loupedeck's profile-per-app and dial layers are mostly irrelevant for a 13-key deck.

---

## (b) Gesture and macro model comparison

| App | Gestures | Multi-step | Delay | Parallel/serial | Cycling states | Conditional | Look follows state |
|---|---|---|---|---|---|---|---|
| Stream Deck | key down (plugins add long/double) | Multi Action list | Delay step + per-action gaps | serial | Multi Action Switch, 2-state keys | no (plugins) | State 1/2 titles+icons |
| Companion | press, release, short release, duration groups (on release or while held) | action list, groups | Wait action | per-group: concurrent/sequential | Steps | feedbacks, expressions | feedbacks restyle button |
| Touch Portal | On Press, On Hold (limited), On Event | action list | wait action | serial | toggle buttons | If/Else actions | per-state visuals |
| OpenDeck / StreamController | as Stream Deck | Multi Action / actions | step delay | serial | toggle / states | no | per-state image+text |
| Ulanzi Studio | tap (hold unconfirmed) | multi-action | yes [mem] | serial | hotkey switch [mem] | no | limited |
| racerxdl D200 | tap only | no | no | n/a | no | no | no |
| **deckd today** | tap, double, N hold tiers (fire on release), repeat | **no** (hold steps chain only) | **no** | chain vs `split` | `toggle`/mute state with `alt` look | no | `alt` look (one) |

deckd is already ahead of the Linux clients and level with Companion on gestures. It lacks: a step list per gesture, delays, typed text, and a general "look when X".

---

## (c) Redesign proposal for deckd (egui, realistic)

### Data model changes (do first, everything else rests on it)

1. **`Action` becomes a `Vec<Step>` per gesture** ("macro"). Keep the current `Action` enum as one step kind and add:
   - `Delay { ms }`
   - `Text { text, per_char_ms? }` (type a string)
   - `KeyDown/KeyUp { key }` optional later, for "hold Shift while ..." macros
   - `Group { mode: Sequential|Parallel, steps }` only if needed (Companion's Concurrent/Sequential; skip in v1, `split` already covers parallel for holds)
   - later: `If { cond, then, else }` where cond uses the same state sources as `alt` (mic_muted, output_muted, toggle, window focused), and `Toggle/SetState`.
2. **Gesture table instead of fixed fields.** Replace `action`, `double`, `holds`, `repeat` with `gestures: Vec<Gesture>` where `Gesture { kind: Tap | Double | Hold{ms} | TapHold{ms} | Repeat{every_ms}, steps: Vec<Step>, rim_color: Option }`. Same-`ms` hold chaining falls out naturally (one gesture, ordered steps); `split` disappears or becomes "separate gesture with same ms". Keep a loader that migrates the old fields (the code already does this for `hold/hold_long`), so existing `deck.toml` and the CLI keep working. Treat CLI compatibility as part of the work (`set --on press|double|hold|repeat` maps onto kinds).
3. **Tap-then-hold** (tap, release, press again and keep holding): add as `TapHold{ms}`. Detection reuses the 350 ms double-tap window: second press that is still down after `ms` fires. Needs deck testing; flag as unverified until the owner tries it.
4. **Looks as a list of conditional overrides** (Companion-feedback style, but tiny): `looks: Vec<LookRule{ when: StateSource|Pressed|Hold-armed, overrides: Alt }>`, first match wins, base look underneath. `alt` + `state` becomes the first rule, so mute buttons keep working. This also gives you "page-indicator highlight", "stats threshold turns red" later.

### Screen layout

```
+-----------------------------------------------------------------------------------+
| deckd   [Profile/Layout v]  Page 1 2 3 [+]          Brightness ---o--   * Connected |  top bar
+----------------+-------------------------------------------+----------------------+
| LIBRARY        |                                           |  INSPECTOR           |
| [search....]   |        [ 5x3 deck + wide slot ]           |  Preview (big tile)  |
| > Keys/Hotkey  |        selection, drag target             |  Look | Actions | Rim|
| > Text         |                                           |  ...                 |
| > Run / Open   |   page tabs under deck, status line       |                      |
| > Media        |                                           |                      |
| > Pages        |   Undo/Redo, Copy/Paste                   |                      |
| > Timing/Delay |                                           |                      |
| > Live faces   |                                           |                      |
+----------------+-------------------------------------------+----------------------+
| status: saved - deck connected - fw ...                       [Log v]             |
+-----------------------------------------------------------------------------------+
```

- Left `SidePanel` (collapsible, ~220 px): **library** of step types and ready-made presets (Mic mute toggle, Volume up with repeat, Screenshot hotkey, Open URL, Page next/back, Clock face, Stats face). Search box on top. Drag an entry onto a key (creates it) or onto the macro list (appends a step). Click appends to the selected gesture, Touch Portal style, which is the keyboard/touch-friendly fallback to drag.
- Centre: the deck (keep as is: it works) plus a **drag source/target** for library items and files (dropped image file sets the icon; dropped .exe/.lnk makes a Run button with its icon).
- Right inspector (still ~400 px, but with **tabs** instead of one long scroll): `Look` (label, colour, glyph, icon, font size, face), `Actions` (gesture list), `State` (look rules), `Rim` (per-button feedback overrides). Global feedback settings move to a Settings window, they are not per-button.
- Top bar replaces the in-panel brightness slider; keeps connection status visible without the bottom bar. Bottom bar only for transient "Saved / error" text and the log.

### Macro step editor (Actions tab)

Gesture list as collapsible rows, each row a header + step list:

```
[Tap]            [+ step v]                       (x)
   1 [Hotkey ] ctrl+shift+m         [rec]  ^ v x
   2 [Delay  ] 200 ms                      ^ v x
   3 [Text   ] "hello"                     ^ v x
[Hold 500 ms ]   color [o]          [+ step v]    (x)
   1 [Media  ] vol_up                      ^ v x
[+ Add gesture  v Double | Hold | Tap-hold | Repeat ]
[Test run]  (dry run to the log, never presses the real thing without a confirm)
```

- Each step row: drag handle (`egui::dnd` via `egui_dnd` crate or hand-rolled reorder with up/down buttons first), kind combo, inline fields, enable checkbox (mute a step without deleting), duplicate/delete. Use `ui.push_id` per step to avoid ID clashes.
- Hotkey field: **click-to-record** (capture the next key combo via `ctx.input`, you already have `key_name`). Show modifiers as chips. This removes the biggest typing error source.
- Delay steps rendered as a thin row with a drag-value (ms) and quick chips 50/100/250/500/1000.
- A **timeline summary** under the gesture ("0 ms Hotkey - +200 ms Text ...") helps people see total length; cheap to draw with `painter`.
- Gesture rows show a small icon + the rim colour swatch so what you see on the deck matches the editor.
- Validation inline (red outline + message) using `actions::validate`, same rules as the CLI.

### State-aware alternate looks

- Inspector `State` tab: a **When** dropdown (Mic muted / Speaker muted / Toggle / later: process running, window focused) and a **two-tile preview** "Normal | When true" side by side; clicking a tile edits that look in the Look tab (header toggles "Editing: Normal / Alternate", exactly like Stream Deck's State 1/State 2 tabs).
- Presets button: "Mic mute", "Speaker mute", "Toggle". Already exist; move into the library.
- Rules list (look rules, section above) can be hidden behind "Advanced" until needed.

### Pages, folders, profiles

- Pages stay. Add **folders** as sugar: a "Folder" button = create a new page + auto "Back" button on it (the same thing StreamController/Stream Deck do) and a breadcrumb in the top bar. Cheap, because folders are pages with a generated back button.
- **Sticky buttons** (StreamController idea): a flag "show on all pages" for back/home/mic mute. High value for 13 keys, small schema cost.
- **Layouts/profiles** as named files in a `layouts/` directory (racerxdl "preset cards"), switched manually; per-app auto-switch later.

### Import / export / sharing

- Export button or whole page as one `.deckd.zip` (toml fragment + icons) so a button can be shared and re-imported with drag-and-drop. Copy/paste of buttons as TOML text on the clipboard (works between two deckd windows and in chat).
- "Export snapshot PNG" of the deck preview, like Touch Portal's snapshot, for docs.

### Build order

**Phase 1 (biggest value, mostly model work):** step lists + Delay + Text steps, gesture table with migration, tabbed inspector, hotkey recorder, undo/redo, copy/paste/duplicate, right-click menu on keys, dropped image file sets icon.
**Phase 2:** library sidebar with search and presets, drag from library, test-run/dry-run button, step reorder via drag, Settings window, tray + start-at-login, light/dark follow system.
**Phase 3:** look rules and conditions (`If`), sticky buttons, folders and breadcrumbs, layout presets and import/export, app picker with icon extraction for Run buttons, wide-slot designer.
**Later / maybe:** per-app auto-profile, key down/up steps, variables in labels, plugin API (skip; the CLI already serves agents).

### egui feasibility notes

- Everything above is doable with stock egui 0.37: `SidePanel`/`TopBottomPanel`, `CollapsingHeader`, `ComboBox`, `DragValue`, `dnd_drag_source`/`dnd_drop_zone` (egui has built-in DnD payload helpers; use them for key-to-key and library-to-key), context menus via `response.context_menu`, `Frame` for cards.
- Weak spots: no native file-picker (use `rfd` crate), no tray (use `tray-icon`), no real list reorder widget (`egui_dnd` crate or up/down buttons), IME/accessibility are rough. Keep state in a single `Vec<Step>` and rebuild the preview from the config; the layout renderer already exists, so a **real render of the 196 px tile** in the inspector (texture from `layout.rs`) beats re-drawing it with `Painter`, and guarantees "what you see is what the deck gets".
- Undo/redo: snapshot the `Config` clone after each committed change (it is small), cap at ~100; store `Vec<Config>` plus an index. Group changes by `response.drag_stopped()`/`lost_focus()` so typing does not create one entry per character.

---

## (d) Ten highest-value polish items

1. **Ordered step list with Delay and Type-text steps** per gesture (the one functional gap versus every competitor).
2. **Click-to-record hotkey field** with modifier chips; no hand-typed `ctrl+shift+m`.
3. **Undo/redo (Ctrl+Z / Ctrl+Y)** plus copy/paste/duplicate of buttons (Ctrl+C/V/D) and a right-click menu on keys (Copy, Paste, Duplicate, Clear, Move to page).
4. **Tabbed inspector (Look / Actions / State / Rim)** with a large live tile preview rendered by the real `layout.rs` renderer, instead of one long scroll.
5. **Action/preset library sidebar with search** and drag-onto-key (or click-to-add); includes "Mic mute", "Volume with repeat", "Open URL", "Screenshot" presets.
6. **Drop an image, .exe or .lnk onto a key** to set its icon/command (and pull the program icon, which the README lists as not built); the racerxdl app picker shows users love this.
7. **Two-state editor ("Normal | Alternate")** shown side by side with a plain-language condition dropdown, replacing the "Alternate look" section.
8. **Gesture editor that reads like a story**: rows for Tap / Double / Hold 500 ms / Repeat with rim colour swatch, plus a "Test" button that dry-runs to the log without pressing real keys.
9. **Tray icon, start at login, run hidden, light/dark follow system** (Goal 4 in the README, and the thing that makes it feel like an app instead of a tool).
10. **Empty states, tooltips and a status top bar**: empty key shows "Drop an action here / click +"; hover tooltips with the shortcut; connection state, page, and "Saved" always visible in the same spot; keyboard navigation of the grid with arrow keys.

Runners-up: sticky buttons, folders with auto Back key, layout presets and `.deckd.zip` export, whole-deck background tiling, per-button font size.

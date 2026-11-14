# deckd macro overhaul: design review and design language

Date: 2026-11-14. Inputs: three HTML mockups plus our real constraints (`config.rs`, `actions.rs`, `gui.rs::action_editor_ex`, `theme.rs`, `UI-RESEARCH.md`).
Nothing in the source was touched. All three files were readable.

Attribution (from voice and construction, not certain):
- `deckforge_studio_pc_macro_stream_deck_layout.html` = most likely **Gemini** (Tailwind CDN, emoji everywhere, "photorealistic chassis", marketing-speak, fake OBS/Git integrations).
- `orbit_macro_designer.html` = most likely **GPT** (small vanilla page, generic catalogue, uses `prompt()` for "Add step", JSON export gimmick).
- `Stepwise-Macro-Builder.html` = most likely AI-made (design tokens, light/dark, inline SVG icon set, lint/checks, keyboard model, honest "estimates" wording).

Hard facts used throughout: a macro is an ordered `Vec<Action>`; step kinds are run, open, hotkey, media (incl. `mic_mute`), text, delay, print; no nesting, no page switch, max 50 steps, delay 10-60000 ms, text max 1000 chars, steps run sequentially with a 40 ms gap (skipped next to a Delay), a failing step aborts the rest (`step N: ...`). No branching, loops, variables, mouse, window control, notifications.

---

## 1. Verdicts

**DeckForge Studio (Gemini).** Best at the *whole-app* feel: a categorised, searchable action library where items can be dragged onto a key or clicked to assign; a click-to-record hotkey field with a pulsing "listening" state; right-click copy/paste/clear on keys; a toggle key whose two states can be previewed; per-step highlight while a sequence plays; an execution log. Weak or fake: the multi-action "sequencer" is the thinnest part (a `<select>` plus a text input per row, up/down arrows, three add buttons, no timing view, no validation); most of the library (OBS, soundboard, git, window snap) does not exist in any engine we have; the skeuomorphic hardware chassis, curved-glass keycaps and OSD toast are decoration (we already render the real 196 px tile); the delay is silently clamped to 2 s in preview, which is a lie. Steal: library behaviour, recorder UX, run-highlight idea. Ignore: chrome.

**Orbit (GPT).** Best at *legibility of order*: numbered cards joined by connector lines, a clear "End of macro" cap, an overview of counts, and a per-step proportional timeline tab. The category chips and search in the library are tidy. Weak or fake: each card is ~90 px tall so 50 steps is a scroll marathon; "Add step" uses a browser `prompt()`; the inspector renders every field as a plain text box (mouse position, condition, window operation are free text); "delay before this action" duplicates the Delay step and muddles the timing model; zoom, JSON export, "Save" are theatre; Condition and Loop are catalogue entries with no behaviour. Steal: the end cap, the number-then-icon-then-title header order, the duration bar per step. Ignore: node-graph pretence.

**Stepwise Macro Builder.** Clearly the strongest, and the only one that treats the step list as a real editing surface: gutter with step number, hover-only tools, disabled steps shown dashed and struck through, a "+" that appears in the gap between any two rows to insert at that spot, drag with a visible insertion line, Ctrl+K command palette, snippets, inline lint badges plus a header "2 errors / 1 warning" pill, keycap-style hotkey chips, quick-value chips (100/250/500 ms), a lane timeline with a playhead and estimated total, per-step notes, undo with typing coalescing. Weak for us: it is built on an engine we do not have (variables, `{{templating}}`, If/Else, While, Repeat, Run macro, mouse, clipboard, notification, window waits), four panes plus modal recorder plus palette is too much UI for a 13-key deck and a 7-step-type engine, and the lane timeline has little to show when there is effectively one lane. Steal the interaction model almost wholesale; cut the content.

---

## 2. Ranked steal list

Legend: **BUILD NOW** = works on today's engine and egui. **BUILD LATER** = needs engine work. **SKIP** = not worth it.

| # | Idea | From | Why it is good | egui mapping | Engine reality | Verdict |
|---|---|---|---|---|---|---|
| 1 | **Compact numbered row**: gutter number, icon tile, title + one-line summary, right-side timing chip, hover-only tools | Stepwise (`.blk`), Orbit header order | Scannable at 50 steps, ~40 px per row instead of a 140 px framed group | One custom-painted row via `allocate_exact_size` + `Painter`; tools shown when `response.hovered()` using `ctx.animate_bool` | none | BUILD NOW |
| 2 | **Insert-at-position "+" in the gap between rows** | Stepwise (`.gap-add`) | Removes "append then drag it up"; the single biggest ergonomic win over our current end-only menu | 6 px gap rect; on hover grow to 18 px and paint a circled "+" that opens the picker with an insert index | none (just `steps.insert(i, ..)`) | BUILD NOW |
| 3 | **Searchable action picker (popup), Ctrl+K**, plus quick-wait chips | Stepwise palette, DeckForge/Orbit library search | Faster than a 7-item menu once habit forms; keyboard-only flow; but a permanent sidebar is wasted on 7 types, so use a popup | `egui::Area`/`popup_below_widget` with auto-focused `TextEdit`, 2-column grid of tiles, Enter inserts first match | none | BUILD NOW |
| 4 | **Inline expand-in-place step editor** (select a row, its fields open under it) | Stepwise inspector, Companion; but inline instead of a separate pane | Same widget works in the 400 px inspector and in a wide workbench; no fourth pane | `CollapsingState`-style open/close with `animate_bool`; reuse existing per-kind field code from `action_editor_ex` | none | BUILD NOW |
| 5 | **Inline validation**: error/warn badge on the row, red border, error text replaces the summary, header pill "2 problems" | Stepwise (`runChecks`, `lintMap`, `.pill`) | Mistakes visible before saving; we already own `actions::validate` and its `step N:` messages | per-row `actions::validate(step)`; extra soft lints (below); pill is a clickable label that selects the first bad row | validate already exists | BUILD NOW |
| 6 | **Keycap chips for hotkeys** (`Ctrl` `Shift` `M`) and a pulsing Record state | Stepwise (`.keys kbd`), DeckForge (`rec-pulse`) | Hotkeys become glanceable; recorder state is obvious | small rounded rects with `border` stroke, bottom edge 2 px thicker; recording pulse = `animate_value` on `error` alpha | recorder exists (`self.recording`) | BUILD NOW |
| 7 | **Drag to reorder with insertion line, dimmed source** | Stepwise (`.blk.drag`, `.gap.over`) | Direct manipulation; keeps up/down as keyboard fallback | `dnd_drag_source` / hand-rolled with `ui.input(pointer)`; paint 2 px `accent` line at nearest gap; dragged row painted at 35% alpha in place, ghost under cursor | none | BUILD NOW |
| 8 | **One-bar timeline with estimated total** | Orbit (timeline tab), Stepwise (ruler, total) | Shows where the time goes; our only "timing" is waits + 40 ms gaps + typing, so one bar is enough, not lanes | `Painter` rects with widths proportional to estimated ms, min width 6 px, colored by step accent; click selects the step; tooltip with start offset | estimates computed in UI from known rules (40 ms gap rule, ~4 ms per typed char) | BUILD NOW |
| 9 | **Start cap and End cap** ("When pressed" / "Done") | Stepwise (`.edge`), Orbit (`.end-node`) | Frames the sequence, gives the empty state a visual anchor, tells the user nothing happens after | two pill shapes + 2 px connector line painted with `border` color | none | BUILD NOW |
| 10 | **Keyboard model**: Up/Down select, Alt+Up/Down move, Del, Ctrl+D, Enter to edit, Esc to deselect | Stepwise | Makes the list feel like an editor, not a form | `ui.input_mut(|i| i.consume_key(..))` on a focus-owning list response | none | BUILD NOW |
| 11 | **Duplicate step** and copy/paste step | Stepwise (`dup`), DeckForge (copy key) | Macros are repetitive (Ctrl+V, Wait, Enter); duplicate beats re-picking | button + Ctrl+D; paste can come later | none | BUILD NOW |
| 12 | **Undo/redo with typing coalescing** | Stepwise (`commit(key)`) | Safe to experiment; UI-RESEARCH already specifies snapshot undo | snapshot `Config` clone, coalesce on `drag_stopped()`/`lost_focus()` | none | BUILD NOW (shared with the global undo item) |
| 13 | **Test-run playhead** (row highlight in `press` color, sweeping marker on the timeline) | DeckForge (`step-running`), Stepwise (playhead) | Turns "Test now" into feedback you can watch | UI-side clock from `Instant` against the estimated schedule drives highlight; accurate enough, no engine change needed for v1 | v1 estimate-driven; exact version needs a progress hook | BUILD NOW (estimate), BUILD LATER (exact) |
| 14 | **Per-step enable/disable (dashed + strike-through)** | Stepwise (`enabled`) | Lets users mute one step while debugging without deleting it | `RichText::strikethrough()`, 50% alpha, dashed border via `Shape::dashed_line` | no field today | BUILD LATER (small engine impact, section 5) |
| 15 | **Hotkey / quick-value chips** (50 100 250 500 1000 2000 for waits) | Stepwise (`.chips`) | Already in our editor; keep, make them chips | `small_button` styled as chip | none | BUILD NOW (already partly there) |
| 16 | **Snippets** ("Copy selection", "Paste and submit", "Open app, wait, type") | Stepwise (`SNIPS`) | Teaches idioms; zero engine cost because they are just step lists; picker section "Starters" | picker entries that insert 2-4 steps | all expressible today | BUILD NOW (after picker) |
| 17 | **Per-step note / custom name** | Stepwise (`note`, `label`) | Long macros need annotation | would need a field on the step | needs a wrapper struct around `Action` | SKIP for now (do with item 14 if the step wrapper ever lands) |
| 18 | **Right-click key menu: copy / paste / clear** | DeckForge | Fast deck editing | `response.context_menu` | none | BUILD NOW but it is a deck-grid item, tracked in UI-RESEARCH not here |
| 19 | **Categorised library sidebar, drag onto a key** | DeckForge, Orbit | Right for a catalogue of dozens | `SidePanel` | we have 7 types | SKIP (popup picker instead; revisit if step types exceed ~15) |
| 20 | **Condition / Loop / Run macro / Variables / templating** | Orbit, Stepwise | The power-user dream | n/a | none of it exists; nesting is explicitly forbidden | SKIP |
| 21 | **Mouse click/move/scroll, window control, clipboard, notification, sound** | Orbit, Stepwise | Real automation breadth | needs per-type field UIs | none exist; see section 5 for the cheap ones | BUILD LATER (individually) |
| 22 | **Record keystrokes with timing into steps** | Stepwise (modal recorder) | Fast authoring | needs a global keyboard hook, not just window events | needs `SetWindowsHookEx` thread and a privacy story | SKIP for now; our per-hotkey Record already covers 90% |
| 23 | **JSON export, node-graph zoom, "overview" stats tab, hardware chassis skin, OSD toast** | Orbit, DeckForge | Looks impressive | n/a | deck.toml and CLI already export | SKIP |
| 24 | **Breakpoints / pause points in test runs** | Stepwise (`.bp`) | Debugging long macros | gutter dot | needs a controllable runner | SKIP (revisit only after a progress hook exists) |

---

## 3. The design language

Name: **Step Rail**. A macro is a vertical rail of slim rows between a start cap and an end cap, with a one-bar timeline above it. It is one reusable widget used in two places: the Actions tab of the right inspector (compact), and a wide "macro workbench" that takes over the central panel on demand.

All visuals below are expressed in `Theme` field names from `theme.rs`. Where a derived color is required it is computed from existing tokens at draw time (alpha via `gamma_multiply`, hue via `config::rotate_hue`), so every built-in and user style keeps working with no change.

### 3.1 Layout and where it lives

Existing structure stays: right inspector (`th.inspector_width`, default 400, can be left) with tabs Look / Actions / State; central panel with the deck.

1. **Inside the Actions tab (compact mode).** When a gesture's action is a Macro, the old nested framed "Step n" groups are replaced by the Step Rail at the inspector's width: header strip, one-bar timeline (10 px tall in compact), rows, end cap, "+ Add step". Row height follows `th.compact`: 30 px compact, 40 px comfortable. At 400 px the summary goes on the same line as the title and truncates with an ellipsis.
2. **Macro workbench (wide mode).** The Rail header has an "Open full editor" button (also: Enter on the gesture header, double-click the step list). It replaces the central deck area with: a top strip (back chevron "Deck", a 56 px rendered tile of the button via the existing `layout.rs` renderer, breadcrumb "Page 1 · 2,1 · Tap > Macro", total time, problem pill, Test run button), the 28 px timeline bar, then the Rail centred at max 720 px width with the selected step's fields expanded inline. The right inspector stays visible and unchanged (Look / Actions / State) so the user can still tweak rim colors or the label. Esc or the back chevron returns to the deck. No fourth pane, no modal.
3. **Hold macros.** Each hold tier (the `holds: Vec<HoldStep>` groups sharing a duration) uses the same Rail for its chained steps. First slice only restyles the press/double macro; hold groups adopt the Rail in a later slice because they have their own duration chrome.
4. **Coexistence with the deck grid.** Selecting another tile while the workbench is open is disabled (deck is hidden); closing the workbench restores the same selection and scroll position. The compact Rail in the inspector remains the always-available path.

Sizes: Rail column min 340 px, max 720 px; picker popup 360 x up to 420 px; timeline 10 px (compact Rail) or 28 px (workbench); gap between rows 6 px (18 px while hovered or while dragging); start/end caps 26 px tall.

### 3.2 Step-row anatomy

Left to right, all painted into one allocated rect so hit-testing is cheap:

| Part | Spec | Tokens |
|---|---|---|
| Drag grip | 12 px, `⠿` shown on row hover only; whole row also draggable by the grip | `text_dim` |
| Gutter | step number, 2 digits, `small_size`, monospace; under it the start offset ("+1.2 s", `small_size`, dimmer) once the macro has any wait | `text_dim` |
| Icon tile | 26 px square (22 compact), rounding `th.rounding`, fill = step accent at 18% alpha, 1 px border = step accent at 40% alpha, glyph in the accent | step accent (3.7), `th.border_width` |
| Title | step kind label ("Hotkey", "Type text", "Wait", "Program", "Open", "Media", "Print"), `body_size`, strong | `text` |
| Summary | one line, `small_size`, truncating; hotkeys as keycap chips; file/URL values in monospace; see table 4 | `text_dim`, chip uses `card` fill and `border` stroke |
| Timing chip | right aligned, rounded pill, the *step's own* duration: Wait "250 ms" / "1.5 s" in `warn`; Text "~40 ms" typing estimate in `text_dim`; others "instant" omitted (no chip) | `warn` / `text_dim` |
| Hover tools | duplicate `⧉`, delete `🗑` (and later enable `⏻`), 22 px ghost buttons, fade in with `animate_bool` (0.1 s) | `text_dim` -> `text`; delete hover -> `error` |

Row surfaces: resting fill `card`; hovered `hover`; selected `hover` plus a 1.5 px border in `select` (like our deck selection) and the row expanded; running (test run) fill `press` at 22% alpha and border `press`; invalid border `error` with a 12 px `⚠` badge before the tools (badge tooltip repeats the message); disabled: dashed border, 50% alpha, title struck through. Corner radius `th.card_rounding`; border width `th.border_width` (so a style with border_width 0 gets borderless rows automatically).

**Wait rows are different.** A Wait renders as a slim connector row (24 px comfortable, 20 compact) on the rail: an amber bar whose length is a log-scaled hint of the duration, the duration as an inline `DragValue` plus quick chips when selected. Wait is the only step type users edit constantly, so it must be editable without expanding anything. Everything else expands to show fields.

**Rail geometry.** A 2 px vertical line in `border` runs through the icon tile centres, behind rows; the start cap ("▶ When pressed", pill, `card` fill, `border` stroke) tops it; the end cap ("✓ Done", dashed `border`, `text_dim`) closes it. Gaps are the hit areas for insert (3.3).

### 3.3 Action picker and adding

- Trigger points: the end cap's "+ Add step" button (appends), the "+" that appears in any gap on hover (inserts there), Ctrl+K (inserts after the selected row, or appends if none), and the empty-state buttons. All open the same popup, with an `insert_at: usize`.
- Popup: auto-focused search box; below it a 2-column grid of 7 tiles (icon tile, name, one-line description), grouped under tiny headings Input (Hotkey, Type text, Media), Launch (Program, Open), Timing (Wait), Debug (Print). Search matches name and keywords ("delay", "sleep" -> Wait; "url", "web", "folder" -> Open; "keys", "shortcut" -> Hotkey; "volume", "mute", "mic" -> Media; "exe" -> Program).
- A second row of quick-insert chips: "Wait 100", "250", "500", "1 s". Clicking inserts a Wait with that value immediately and keeps the popup open (so you can alternate hotkey and wait). This is the one thing no mockup does and our engine most needs, since the 40 ms gap is rarely enough.
- A third section "Starters" (snippets): "Copy selection" (Ctrl+C, Wait 100), "Paste and submit" (Ctrl+V, Wait 120, Enter), "Open app and type" (Program, Wait 1500, Type text). Inserted as plain steps at the position.
- Keyboard: type to filter, Up/Down to move, Enter to insert and select the new step with its first field focused, Esc to cancel. Arrow keys also move between tiles.
- Behaviour: inserting selects the new row and expands it; the row flashes (background `accent` alpha 0.35 -> 0 over 0.5 s via `animate_value_with_time`) so the eye finds it. At 50 steps the "+" affordances are replaced by a disabled "50/50 steps" label and the picker refuses with a tooltip.

### 3.4 Step inspector (inline)

Selecting a row expands a panel directly under it (rounded to match, `input` fill for field wells, `border` stroke, 8 px padding, 0.12 s open animation). It contains only that step type's fields (table 4), a compact footer ("Move up", "Move down", "Duplicate", "Delete" as small buttons, mostly redundant with hotkeys but discoverable), and the validation message in `error` if any. Only one row is expanded at a time; Esc collapses it. The same fields appear whether the Rail is in the inspector or the workbench; only the available width changes.

### 3.5 Timeline and duration

One bar, not lanes. Above the rows (inspector: 10 px, workbench: 28 px with a ruler row of tick labels).
- Segments are laid left to right in step order with width proportional to estimated duration, minimum 6 px so tiny steps remain visible, total capped by horizontal compression (never scrolls horizontally in the Rail; the workbench may zoom later).
- Fill = step accent at 55% alpha; Wait segments use `warn` at 70% with a diagonal hatch (painter line segments clipped to the rect). Gaps (40 ms) are drawn as 1 px ticks in `border`.
- Hover: tooltip "Step 4 · Wait · starts at +1.34 s · lasts 500 ms". Click selects the row and scrolls to it. The selected step's segment gets a 1.5 px `select` outline.
- Label to the right of the header: "≈ 2.4 s". The word "about" is honest: waits are exact, typing and process spawns are estimates. Compute: sum of Waits + 40 ms between steps when neither neighbour is a Wait (mirrors `execute_logged`) + `chars * 4 ms` per Text step + ~1 ms per other step. Show "+ typing" when a Text step is present.
- Test run: a 2 px `press` colored playhead moves along the bar and the active row takes the "running" surface, driven by an `Instant` since the click and the estimated schedule. Stop button replaces Test run while running. When the engine later gets a progress hook (section 5) the playhead is exact.

### 3.6 Empty states

- Empty macro (no steps): the Rail shows the start cap, a dashed rounded `border` box (`text_dim` text) "No steps yet. A macro runs its steps one after another." with two buttons: "Add first step" (opens picker) and "Start from an example" (small menu of the Starters). Then the end cap.
- Empty timeline: the bar shows a 1 px `border` line and "0 ms".
- Empty gesture (no action at all) is unchanged: the kind picker row ("Nothing / Program / ...", "Macro" among them). Switching to Macro keeps the previous action as step 1 (existing behaviour, keep it).
- Search with no match in the picker: "No step called that. Try 'wait', 'keys' or 'open'."

### 3.7 Errors and validation

Source of truth is `actions::validate(step)` for hard errors (prefix stripped, because the row already says its number) and a few soft lints computed in the UI.
- Hard (blocks save in CLI and config load; in the GUI it blocks nothing but is loud): empty hotkey / unknown key, empty program, empty open target, empty or >1000-char text, delay out of 10-60000, macro with 0 or >50 steps. Presentation: row border `error`, `⚠` badge, summary line replaced by the message in `error`, timeline segment outlined in `error`, header pill "2 problems" (click jumps to the first), and the existing red message under the Rail. Step 51 cannot be added at all.
- Soft (warn, never block): a Program/Open step followed immediately by Hotkey/Text with no Wait ("The app may not be ready; add a Wait."), two Waits in a row ("Merge these waits"), a trailing Wait ("Nothing runs after this wait"), a macro with one step ("A single action does not need a macro"), Text over 900 chars. Presentation: `warn` badge, same tooltip rule. Pill shows "1 tip" in `warn` when there are no errors.
- Tooltips use `on_hover_text`. The header pill says "No problems" in `ok` when clean.

### 3.8 Selection and keyboard

One selected row at a time (`selected: Option<usize>` in the Rail state, per gesture). Click selects and expands; clicking the selected row's title again collapses. The Rail owns focus when clicked; keys work only while it has focus so text fields keep normal behaviour.

| Key | Action |
|---|---|
| Up / Down | previous / next row |
| Alt+Up / Alt+Down | move step up / down |
| Enter | focus the first field of the expanded step |
| Esc | collapse; second Esc clears selection and returns focus to the list |
| Delete | delete step (no confirm; undo covers it) |
| Ctrl+D | duplicate below |
| Ctrl+K | open the picker at the selection |
| Ctrl+Z / Ctrl+Y | undo / redo (global, shared with the rest of the editor) |
| Ctrl+Enter | Test run |
| Ctrl+/ | enable/disable step (once the engine supports it) |

### 3.9 Density

Follows `th.compact` only (no new token): row 30 vs 40 px, icon tile 22 vs 26, `item_spacing` already differs; summary hides its second line in compact; timeline 10 vs 28 px; chip padding 2/6 vs 3/8. `th.show_hints` hides the helper sentences (e.g. "Runs these steps one after the other") exactly as it does elsewhere.

### 3.10 Motion

Small and subtle, all through `Context::animate_bool_with_time` / `animate_value_with_time`, so no state machines:
- row tools fade in 0.1 s; inline editor opens 0.12 s; new-row flash 0.5 s; drag source dims instantly to 35% alpha; insertion line snaps (no easing); playhead is linear.
- A single `style.animation_time = 0` switch (a "Reduce motion" toggle in Settings, default off) removes all of it.
- No blur, glow, gradients or shadows. Premium here means alignment, restraint and instant response, not effects.

### 3.11 Token map (what we use, what we might add)

| Role | Token |
|---|---|
| Rail background | `panel` |
| Row fill / chip fill | `card` |
| Row hover | `hover` |
| Field wells in inline editor | `input` |
| Row border, rail line, ticks | `border` (width `border_width`) |
| Selected border, selected segment outline, focus | `select` |
| Primary buttons, insertion line | `accent` / `accent_text` |
| Wait color, warnings | `warn` |
| Run step, "No problems", success | `ok` |
| Errors | `error` |
| Running highlight, playhead | `press` |
| Secondary text, gutter, summaries | `text_dim` |
| Rounding | `rounding` (chips, tools), `card_rounding` (rows, popup) |
| Sizes | `body_size`, `small_size`, `heading_size` (workbench title), `monospace` for values |

**New tokens: none required.** Step accents are derived (3.7). An optional override group is proposed for style authors, to be added last: `[colors] step_hotkey, step_text, step_run, step_open, step_media, step_wait, step_print` (all `#rrggbb[aa]`, optional, each defaulting to the derived value in table 4). Add them to `Colors`, the `color!` macro list, the `Theme` struct and `styles/template.toml` in one change.

---

## 4. Step-type visual identity

Glyphs are chosen from characters already used successfully by the current editor (⏺ ▶ ⏯ 🔊 🔇 🗑 ↑ ↓ ＋) or from the same Unicode blocks that egui's bundled Noto Emoji / emoji-icon-font cover. Do one 10-minute check: render all primary and fallback glyphs in a throwaway strip in the app before committing; if one shows a tofu box, use the fallback.

Accent derivation (all from the active theme, no hard-coded colors): `accent` for Hotkey, `rotate_hue(accent, +60)` for Text, `ok` for Program, `rotate_hue(accent, +110)` for Open, `rotate_hue(accent, -45)` for Media, `warn` for Wait, `text_dim` for Print. In the built-in dark style that reads as blue, violet, green, magenta, teal, amber, grey; in light and the bundled styles the relationships hold. `config::rotate_hue` takes an `[u8; 3]` and already exists.

| Step | Glyph (fallback) | Accent | Title | Summary format | Inspector fields |
|---|---|---|---|---|---|
| Hotkey | ⌨ (`Ctrl`) | `accent` | Hotkey | keycap chips: `Ctrl` `Shift` `M`; empty -> "No keys set" in `error` | key text (monospace) + Record (pulses while listening, Esc cancels); chips preview below; note "Windows key can't be recorded" |
| Text | ✎ (`T`) | `accent` +60 deg | Type text | `"first 40 chars..."` with line breaks shown as `↵`, then dim `(n chars)`; over 900 chars turns `warn` | multiline box (3-6 rows), live `n / 1000` counter, hint "typed into whichever window has focus; line break presses Enter" |
| Program | ▶ (`>`) | `ok` | Program | file name only in monospace (full path in tooltip), then dim arguments | command + Browse...; arguments (space separated) |
| Open | 🌐 for URLs / 📂 for paths (`o`) | `accent` +110 deg | Open | host + path for URLs ("github.com/..."), file or folder name for paths | target + File...; the glyph switches automatically on `://` |
| Media | 🔊 per key (`m`) | `accent` -45 deg | Media | the chosen key with its own glyph: "🔉 Volume down", "🎤 Mic mute", "⏯ Play/Pause" | segmented buttons for vol_down, vol_up, mute, play_pause, prev, next, stop, plus **mic_mute** (the Rail picker must list it; the current step-media row omits it) |
| Wait | ⏱ (`~`) | `warn` | Wait | "250 ms" or "1.5 s" (exact, bold); idle rows also show "+gap 40 ms" dim only in the tooltip | `DragValue` 10-60000 with suffix ms, chips 50 100 250 500 1000 2000, live seconds readout |
| Print | 💬 (`#`) | `text_dim` | Print | the message in quotes; tooltip "Shows in the Log when it runs" | single-line message; hint "for testing gestures" |

Badge conventions: error `⚠` in `error`; tip `ⓘ` in `warn`; both 12 px, before the hover tools.

---

## 5. Engine features the mockups imply (value / cost, short and honest)

Ordered best first. Impact notes are rough and refer to `config.rs::Action`, `actions.rs::execute_logged`, and the GUI.

1. **Per-step enable flag.** Value: high for debugging long macros, lets the dashed/strike row appear. Cost: low. Lowest-risk shape is `Macro { steps, #[serde(default, skip_serializing_if = "Vec::is_empty")] off: Vec<u16> }` (indices of disabled steps, GUI keeps it in sync on reorder/delete) so existing `deck.toml` still loads; skip those indices in `execute_logged` and in `validate`. Clean alternative is a `Step { enabled, action }` wrapper but that changes the TOML shape and the CLI.
2. **Progress hook for test runs.** Value: medium (exact playhead, jump-to-failing-step). Cost: low. Give `execute_logged` an extra `&dyn Fn(usize, StepEvent)` (start/finish), or pass step starts through the existing log callback with a reserved prefix. The GUI already runs slow tests on a thread.
3. **Text step paste mode and speed.** Value: high for long text (1000 chars at ~4 ms is 4 s and breaks in some apps). Cost: low-medium: set clipboard (Win32 `SetClipboardData` or the `arboard` crate) then synthesise Ctrl+V; optional `mode = "type" | "paste"`, optional `cps`. Field is `#[serde(default)]` so old files are unaffected.
4. **Repeat N on the macro.** Value: medium (spam, retries). Cost: low: `Macro { steps, repeat: u8 }` default 1, clamp 1-99, loop in `execute_logged`; one DragValue in the Rail header. Still no nesting, so the existing "no nested macro" rule stands.
5. **Wait for window (title contains, timeout, continue or stop).** Value: highest for "open app then type/hotkey" macros, which is the dominant real-world macro. Cost: medium: `EnumWindows` + `GetWindowTextW` poll every 100 ms up to a timeout; new `Action::WaitWindow { title, timeout_ms, on_timeout }`; extend `validate`, the picker (new tile in Timing), and macro-only enforcement like Delay.
6. **Focus window.** Value: high with item 5. Cost: medium: `SetForegroundWindow` is restricted by Windows focus-stealing rules; needs the `AttachThreadInput` workaround or `AllowSetForegroundWindow`; must be tested on the owner's machine.
7. **Continue on error per macro or per step.** Value: medium (today one bad program path aborts the rest). Cost: low: a `stop_on_error: bool` on the macro; in `execute_logged` log and continue instead of `?`.
8. **Hotkey modes: hold N ms, press-only, release-only.** Value: medium (games, drag-modifiers). Cost: low-medium: extra optional fields on `Hotkey`, key-down/up sequences already exist in `execute_single`.
9. **Play sound / beep.** Value: low-medium (audible confirmation when the macro ends). Cost: low: `PlaySound`/`MessageBeep`, new step kind or an "on finish" option.
10. **Mouse click / move / scroll.** Value: medium. Cost: medium, and the inspector needs a position picker; screen coordinates break across DPI and monitor changes. Do after the cheap ones; the position UI is the real work.
11. **Clipboard set/paste, desktop notification.** Value: low-medium. Cost: low-medium. Print already covers "tell me it ran" inside the app.
12. **Variables, {{templating}}, If/Else, While, Run macro, Group.** Value: high in theory, cost: high (parser, scoping, safety limits, nested validation, CLI schema). Do not promise. Decide only after items 1-7 have shipped and the owner reports real macros that need it.

---

## 6. Proposed implementation order (small shippable slices)

Each slice leaves the app working and deck.toml unchanged unless stated. All new code in one new module, `src/macro_ui.rs`, so `gui.rs` only calls into it; keep `action_editor_ex` for the non-macro single-action path. Rebuild per `AGENTS.md` (stop `deckd` first).

1. **Step identity and row restyle.** Add `step_meta(&Action, &Theme) -> {glyph, accent, title, summary}` and `draw_step_row(...)`; replace the framed "Step n / arrows / trash" groups in the `Action::Macro` arm with rows (number, tile, title, summary, delete/duplicate). Keep up/down buttons for now. Add the start/end caps. Still append-only via the existing "+ Add step" menu.
2. **Inline expand editor and selection.** Move the per-kind field code out of `action_editor_ex` into `step_fields(ui, action, ...)` (reused by both single actions and steps); selecting a row expands it. Add Up/Down/Enter/Esc/Delete/Ctrl+D/Alt+Up/Alt+Down.
3. **Picker with insert-at-position.** Popup with search, grid, quick-wait chips, Starters; gap "+" on hover; Ctrl+K; empty state. Remove the old menu. Include **mic_mute** in Media.
4. **Validation display.** Per-row error border and badge using `actions::validate`, soft lints, header pill, jump-to-first. Cap handling at 50.
5. **Drag reorder.** Grip, dimmed source, ghost, insertion line, drop at nearest gap; undo grouping on drop. Up/down keys stay.
6. **Timeline and estimate.** Header "step count, about X s", one-bar timeline (compact 10 px), tooltips, click-to-select. Wait rows become the slim connector style here (needs the timeline's duration function anyway).
7. **Test-run feedback.** Estimate-driven row highlight and playhead; Stop button; move the existing "Test now" delay-3 s behaviour into this flow without changing what actually runs.
8. **Macro workbench.** "Open full editor" button, central-panel takeover with top strip (tile preview from `layout.rs`, breadcrumb, total, problem pill), wide timeline (28 px with ruler), back chevron.
9. **Hold tiers adopt the Rail.** Replace the chained-steps editing inside "On hold" with the same widget.
10. **Optional `step_*` style tokens** in `theme.rs`, `styles/template.toml`, `styles/README.txt`, plus a "Reduce motion" setting.
11. **Engine, one at a time, each as its own change:** per-step enable (1), progress hook (2), paste mode (3), repeat N (4), wait for window (5), focus window (6), stop-on-error (7). Each adds a field or step kind with `#[serde(default)]` so existing configs and the agent CLI schema keep loading; update `deckd schema`, `actions::validate`, and the picker together.

Verification stance for slices 1-8: all visual, no deck needed. Check with the window itself, `deckd build deck.toml out.zip` for config round-trips, and the owner's eyes on both built-in styles (dark, light) plus one bundled style (e.g. midnight) before calling a slice done. Nothing in these slices sends anything new to the deck.

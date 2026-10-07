# Design: the overlay and guidance layer

Status: design, not built. Covers `caretline-overlay` (hints, arrows, callouts, spotlights) and
`caretline-tour` (walkthroughs), the few generic hooks the engine needs for them, and the MVP.

## 0. In one page

| Question | Answer |
|---|---|
| What is it? | A one-way, view-only layer drawn over whatever a caretline host shows: callouts, arrows, spotlights, rings, key badges, step dots, and walkthroughs made of them. It never edits text and never takes focus by itself |
| Where is it drawn? | Into caretline's frame pipeline: a **frame pass** at the end of `render` rewrites cells and roles. Every consumer of a frame sees it: the terminal, `--snapshot`, the protocol's `render` and `subscribe`, and an agent's `read {render: true}` |
| What medium? | **Text cells first.** It must work in WezTerm, which can't layer kitty graphics above text ([wezterm#7222](https://github.com/wezterm/wezterm/issues/7222)), and in herdr, tmux and SSH. So: truecolor roles, box drawing with rounded corners, sub-cell glyphs, dimming by restyling. Pixels are a later renderer behind the same layout |
| Where does it live? | Two crates: **`caretline-overlay`** (layers, anchors, layout, the cell renderer, protocol ops) and **`caretline-tour`** (walkthroughs as data). The engine gets five generic hooks and nothing that says "overlay" |
| What is a layer? | Serializable data in the view: `{id, owner, z, anchor, items, ttl}`. Changed only by messages, so traces record it and replay draws it |
| Anchors | Stable keys resolved to cells every frame: a text range, a block (mark), the caret, a screen rect, or a kind the host registers. They survive scroll, resize and edits because no cell position is stored |
| Agents | `hint.show` over the protocol and a `show_hint` MCP tool: capped, expiring, never modal, never dimming unless the person allows it |
| MVP | Engine hooks, a callout with arrow and spotlight in cells, and `caretline demo guide` (the existing tour as data): about 2.5 engineer-weeks |

## 1. Scope and where it lives

### 1.1 The crates

| Crate | Contains | Depends on |
|---|---|---|
| `caretline` (engine) | Five generic hooks (1.2). Nothing named overlay, tour, callout or spotlight | Unchanged dependencies |
| `caretline-overlay` | `Layers` state and `LayerOp`; `Anchor` and its resolvers; `layout` (placement, collision, routing) into a `Scene`; the cell renderer (`compose`) with glyph sets and the capability ladder; a default `Theme`; the `layer.*` and `hint.*` protocol ops; `install(host)` | `caretline`, `serde`. Optional feature `ratatui` (a `CellGrid` for `ratatui::Buffer`). No terminal, no async |
| `caretline-tour` | `Tour` (TOML or JSON), steps, predicates, branching, the tour reducer, seen-state as data, `tour.*` ops, a `LayerSource` that turns the current step into layers | `caretline`, `caretline-overlay`, `serde`, `toml` |
| `caretline-cli` | Registers both; the `caretline demo guide` walkthrough; theme colours for the new roles; seen-state file; F-key bindings | |
| `caretline-mcp` | `show_hint`, `hide_hint`, `start_guide`, `guide_step`, `list_hints` tools | |

Hosts with plenty of hints and no walkthroughs use `caretline-overlay` alone. Both crates are pure,
so they compile to `wasm32` and the site's playground can run guides too.

### 1.2 What the engine must expose, and no more

| # | Hook | Why | Size |
|---|---|---|---|
| E1 | **View payloads.** `View.ext: BTreeMap<String, serde_json::Value>`, serialized, left out when empty, never read by the engine. The view-level twin of mark payloads | Layers and walkthrough progress are per view (a hint is on the person's screen, not the agent's), must be in the state for traces and replay, and must not be a typed engine concept | ~40 LOC |
| E2 | **`Msg::Ext { key, op }` and ext reducers.** `Host::ext(key, ExtFns { apply, observe })`. `apply(ctx, current, op) -> ExtOut` runs for `Msg::Ext`. `observe(ctx, current, &Observed { msg, effects, changes }) -> Option<ExtOut>` runs after every message for each view whose `ext` holds that key, with the message's composed `ChangeSet` (any view's edit, undo, `external`). `ExtOut { value, effects, status, frame_clock }`. `Msg::Ext` is passive (doesn't end a typing run or clear the status) and is accepted on read-only views | Applying layer and walkthrough ops; mapping text anchors through edits; "advance when" over messages; expiry on `tick`; pulses on the frame clock | ~180 LOC |
| E3 | **Frame passes.** `Host::frame_pass(name, f: Fn(&Ctx, &mut Frame))`, run in registration order at the end of `render`. `view::render_plain` skips them. `Frame` gains public, grapheme-safe writers: `set(x, y, grapheme, role) -> u16` (overwriting either half of a wide grapheme blanks the other half; a wide grapheme that doesn't fit is drawn as a space), `restyle(x, y, role)`, `role(name) -> Role` (today's private `named`). `Frame.regions: Vec<Region { x, y, w, h, id }>` and `Frame::region_at(x, y)` | The compositing hook and the hit map for what an overlay draws. With no pass registered or nothing to draw, the cost is one map lookup | ~150 LOC |
| E4 | **`view::locate(doc, view, pos) -> Locate`.** The inverse of `hit`: `At { x, y }`, `Above`, `Below`, `Left`, `Right` (no-wrap scroll) or `Folded { block }` | Anchor resolution for text positions, marks and blocks, including which way an off-screen anchor lies. It is hit-testing, squarely in scope | ~80 LOC |
| E5 | **Host catalog entries and ops.** `Host::catalog(Vec<HostCommandInfo { id, name, description, category: String, keys: Vec<String>, msg: Msg }>)`; `Host::op(name, OpFns { to_msgs, reply })`. `hello`, `commands.list` and `keymap.get` list them with `source: "host"` | Overlay commands appear in help (F1) and in agents' `commands`; `layer.push` and friends are routed without the protocol knowing them | ~150 LOC |

That's about 600 LOC in the engine, all generic. Text, marks, selections, undo and `view::hit`
are untouched.

### 1.3 Staying out of core scope

- `tests/scope.rs` gains `overlay`, `tour`, `tours`, `callout`, `spotlight` and `walkthrough` in
  `WORDS`, so the engine can't grow these concepts. The hooks are named `ext`, `frame_pass`,
  `locate`, `catalog` and `op`.
- `caretline-overlay` and `caretline-tour` get their own scope test with the host-word list:
  they name no host concept either.
- CI's `cargo tree` check extends to both crates: no terminal crate (except behind `ratatui`) and
  no async runtime.
- The engine never reads `View.ext` and never reads overlay roles. Colours stay with the host,
  as for decorations.

## 2. The model

### 2.1 Layers

A layer is pure data in `view.ext["overlay"]`:

```json
{"layers": [
  {"id": "guide:2", "owner": "guide", "z": 10, "since_ms": 1791367201000, "ttl_ms": null,
   "anchor": [{"text": {"from": 412, "to": 421}}, {"block": 7}],
   "dim": true, "capture": false,
   "items": [
     {"spotlight": {"holes": ["anchor", "callout"]}},
     {"callout": {"title": "Jump by word", "body": "{{key:move.word_left}} and {{key:move.word_right}} move one word.",
                  "place": ["below", "above", "right", "left"], "max_width": 52}},
     {"arrow": {"from": "callout", "to": "anchor"}},
     {"ring": {"around": "anchor", "pulse": {"period_ms": 800, "cycles": 2}}},
     {"steps": {"of": 11, "at": 2}}]}],
 "next": 4}
```

- **The stack:** draw order is `z`, then insertion. Owners have z bands: `host` 0–9, `guide` 10–19,
  `agent` 20–29. An agent can't draw over a walkthrough's callout.
- **Owner:** `person` (a hint the person pinned), `host`, `guide` (a walkthrough) or
  `agent:<name>`. The owner decides styling (`overlay.agent.*` roles), limits (5.4) and who may
  remove it.
- **Lifetime:** `ttl_ms` from `since_ms`, measured against `doc.now_ms`. The `observe` reducer drops
  expired layers on `tick` and `frame`. `null` means it lasts until popped.
- **Anchors** are an ordered list of fallbacks (2.3).
- **Text:** `{{key:<command id>}}` renders the keys currently bound to a catalog command, so remaps
  show the right key; `{{code:…}}` renders in the code role. No other markup.

### 2.2 Primitives

| Primitive | Cells (what the MVP draws) |
|---|---|
| **Callout / tooltip** | A rounded box `╭─╮│╰─╯` in `overlay.callout.border`, its inside cleared in `overlay.callout`, the title in `overlay.callout.title`, wrapped body text, and optional chips (`[F2 next]`) that are click regions |
| **Arrow / leader** | A box-drawing route (`─│╭╮╰╯`, with `┬┴├┤` where it leaves the callout) ending in `▲▼◀▶` next to the anchor (3.4) |
| **Spotlight / dim mask** | Every cell outside the holes is restyled `dim.<role>`. The symbols stay the same (3.3) |
| **Ring / highlight** | The anchor's cells are restyled `ring.<role>` (a tinted background). With `frame: true` and room, corner ticks `┌ ┐ └ ┘` go in blank cells around a block |
| **Key badge** | ` ⌃O ` in `overlay.key`, padded by one space, with no brackets in truecolor. `[Ctrl-O]` in ASCII |
| **Step dots** | `●●○○ 2 of 4` in `overlay.dots` and `overlay.dots.on`. `**--` in ASCII |
| **Pulse** | A ring whose tint steps through 3 levels over `period_ms`, driven by the frame clock (`ExtOut.frame_clock = 15` while a pulse runs, `0` after). Never SGR blink. Off under reduced motion |
| **Strip** (narrow mode) | One row: dots, a shortened text and the next key, in `overlay.strip` (2.5) |
| **Edge chip** | `↓ 2/11 here` at the edge an off-screen anchor lies beyond, a click region that reveals it |

Text is always text: callout words are ordinary cells, so they show in snapshots, can be copied,
and work on every rung.

### 2.3 Anchors

| Anchor | JSON | Resolved by |
|---|---|---|
| Text range | `{"text": {"from": 412, "to": 421}}` (chars) | Visible rows: `Frame.rows` (`RowInfo::Text.chars`, `x`) to find the rows, then `cells[].char_idx` inside them. Several rects when it wraps. Off-screen or folded: `view::locate` |
| Text in a block | `{"text": {"block": 7, "from": 4, "to": 9}}` (offsets within the block) | The block's position plus the offsets: moves with the block through cut, paste and undo |
| Block | `{"block": 7}` (mark id) | The block's visible rows from `Frame.rows` (`block`, `first`, `last`); `locate` when it isn't visible. `Marks::pos` is a linear scan, so visible rows are checked first |
| Caret | `{"caret": true}` | `Frame.cursor`, else `locate(caret)` |
| Screen rect | `{"cells": {"x": 10, "y": 5, "w": 4, "h": 1}}` | Itself. Not stable: for agents that only have a rendered frame |
| Screen | `{"screen": "center"}` | Placement only (a welcome or a closing step) |
| Find (authoring only) | `{"find": "## 2 · Move"}` | Resolved **once**, when a step starts, to a block or text-in-block anchor, by the reducer (a recorded, deterministic message). Never searched per frame |
| Host kind | `{"host": {"kind": "row", "key": "a1b2"}}` | A resolver the host registers (5.1) |

**Surviving change:**
- **Edits:** `observe` maps every char-range anchor through the message's `ChangeSet`: the start
  with `Assoc::After`, the end with `Assoc::Before`. A range that collapses to nothing falls to the
  next fallback. Block anchors need nothing: marks already follow their blocks.
- **Scroll and resize:** rects are recomputed every frame, and nothing positional is stored but
  chars and mark ids.
- **Folds:** a folded block's contents resolve to the fold's first row with `Locate::Folded`.

### 2.4 Placement and collision

1. **The area** is the text rows. The status row is left alone except by the strip, and only when
   the host turned its status bar off.
2. **Candidates:** for each side in `place` (default below, above, right, left), size the box
   (body wrapped to at most `max_width` and at most two-thirds of the width), then **shift** along
   the side to stay inside the area.
3. **Score:** text cells covered (1 each), cells that are already dimmed (0.3), the anchor or a
   hole (not allowed), the caret (not allowed for agent layers, 50 otherwise), and the distance to
   the anchor (0.5 per cell). The lowest score wins, and ties go to the first side listed (**flip**
   comes from this ordering).
4. **The sliver rule:** if a box would leave fewer than 8 covered text cells between it and the
   area's edge, it extends or shifts to that edge, so no ragged ends of words show.
5. **Clamp:** a box never leaves the area. If no candidate fits, the layer drops to strip mode.
6. **Off-screen:** an anchor above or below the view gets an edge chip on the first or last text
   row near its column. The callout docks beside the chip, or stays hidden with `hide_off_screen`.
   Clicking the chip, or `guide.reveal`, scrolls the view (an ordinary `scroll` message). Agent
   layers never scroll the person's view.
7. **Narrow widths:** below 48 text columns or 12 text rows, a layer draws as a **strip** (one row
   at the top, or the bottom if the anchor is on the top row) plus the ring. Below 24 columns the
   dots are dropped. The full text is one key away (`guide.more` shows the callout full width on
   rows the anchor doesn't use).

### 2.5 Input routing

| Mode | What happens |
|---|---|
| **Pass-through** (default, and always for agents) | Every key and message reaches the editor. A click inside a callout's rect is swallowed (via `Frame.regions`), so the caret never lands in text the person can't see. A click on a chip runs its command. A click anywhere else passes through |
| **Capture** (a modal step, `capture: true`, walkthroughs only) | The runtime asks `caretline_overlay::keymap(&View, &Key) -> Option<Msg>` before its own keymap. Enter means next, Shift-Tab means back, Esc stops, and every other key is swallowed except quit. Protocol clients sending `msgs` aren't captured: they aren't keys |
| **Advance when** | A walkthrough step's predicate is checked in the tour's `observe` against each message, its effects and its changes (4.2) |

Overlay commands are catalog entries (5.2), so they work from keys, chips and the protocol alike.

## 3. Rendering in cells

### 3.1 Compositing order

`render` draws the host frame (text, selection, hang, gutter, status). Then the frame passes run
in registration order. The overlay's single pass:

1. Resolves anchors (2.3) and lays out every layer into a `Scene` (rects, routes, text runs, and
   each item's role), lowest z first.
2. **Dims:** for each layer with a spotlight, restyles cells outside its holes `dim.<role>`. Dims
   don't compound: a cell is dimmed once.
3. **Rings:** restyles anchor cells `ring.<role>`.
4. **Arrows,** then **callouts, strips and chips,** in z order: a later layer's box covers an
   earlier one's arrow.
5. Pushes `Frame.regions` for callouts and chips.
6. Leaves `Frame.cursor` alone unless a callout covers it, in which case it's hidden.

A host that composes its whole screen (5.1, mode B) calls the same `compose(&Scene, &mut impl
CellGrid)` after drawing.

### 3.2 Roles and themes

Roles stay names; colours stay with the host, as for decorations. The overlay names:

| Role | Default dark / light (truecolor) | 16-colour and NO_COLOR |
|---|---|---|
| `overlay.callout` | bg #1f2430 / #f4f1ea | default bg |
| `overlay.callout.border` | fg #8aa4ff / #3b5bdb | bold |
| `overlay.callout.title` | bold, fg as border | bold |
| `overlay.arrow` | fg as border | bold |
| `overlay.key` | bg #33405c / #dde4ff | reverse |
| `overlay.dots`, `overlay.dots.on` | fg muted / fg as border | faint / bold |
| `overlay.agent.*` | the same set in the agent hue (#5fb3ff / #1c6fd1) | as above, plus a `◆ name` title |
| `overlay.strip`, `overlay.chip` | as callout | reverse |
| `dim.<role>` | the role's fg blended 60% toward the bg, its bg unchanged | SGR 2 (faint) |
| `ring.<role>` | the role's colours on bg blended 20% toward the border | underline |

`caretline_overlay::Theme::{dark, light}` gives these as RGB for hosts that want them;
`Theme::resolve(name, base_style)` handles the `dim.` and `ring.` prefixes for any base role,
including a host's decoration roles. The glyph set is chosen separately: `Glyphs::Rounded`
(default), `Glyphs::Square`, or `Glyphs::Ascii` (`+-|`, `^v<>`, `*-`, `[x]`).

### 3.3 Dim and spotlight without alpha

Cells have no alpha, so the overlay restyles them:
- The symbol and `char_idx` stay the same, so copying text and accessibility don't change.
- The role becomes `dim.<base>`. The host's theme blends in truecolor (it owns every cell's
  colours), uses the nearest colour in 256 colours, and SGR 2 in 16 colours.
- A spotlight is never the only signal: the callout and ring still say what matters, because
  NO_COLOR and plain-text snapshots can't show dimming.
- Holes are rects of whole cells: the anchor's rects plus a margin of one cell (zero vertically),
  and the callout. On a wrapped range the hole is each row's rect.

### 3.4 Arrow routing

- **Search:** A* on the cell grid inside a corridor (the bounding box of the callout edge and the
  anchor, grown by 4 cells, clipped to the area). Typically under 500 nodes.
- **Cost:**
  - a blank cell costs 1, a dimmed cell 2, and a text cell 6;
  - a hole, another callout, or the second half of a wide grapheme can't be crossed;
  - a bend costs 3, and a route with more than 2 bends costs 20.
- **Ends:** the route starts on the callout edge facing the anchor (with a `┬┴├┤` junction) and
  ends one cell outside the anchor's nearest edge, with a head pointing in.
- **Glyphs** come from each cell's neighbours on the route: `─│` straight, `╭╮╰╯` at bends,
  `▲▼◀▶` for the head. In ASCII: `-|+` and `^v<>`.
- **Diagonals:** box drawing by default. `leader = "braille"` (the sub-cell rung) draws a straight
  Braille dot line instead.
- **Memo:** the route is memoised by (callout rect, anchor rect, the corridor's cell kinds). It's
  derived data, like the wrap cache, so scrolling under a still hint re-routes only when the
  corridor changes.

### 3.5 Wide characters and grapheme safety

- Everything writes through `Frame::set`, which never splits a wide grapheme: overwriting either
  half blanks the other half (a space, with the old role) before writing.
- Callout text is laid out with `view::display_width` and the engine's `printable` rule (control
  and zero-width characters show as U+FFFD). A wide grapheme that would cross the box's inner edge
  wraps to the next line.
- The router and the box edges treat a wide pair as one obstacle; an arrow or border never lands
  on a second half.
- **Ambiguous-width glyphs** (`●○◆▲─│╭`) are one cell in WezTerm's default config (and tmux and
  herdr follow the terminal's wcwidth). Hosts in CJK locales, or with
  `treat_east_asian_ambiguous_width_as_wide`, should select `Glyphs::Ascii`. The CLI picks it when
  `CARETLINE_GLYPHS=ascii` is set.

### 3.6 Performance budget

| Situation | Budget | How |
|---|---|---|
| No layers (almost always) | +0 measurable | The pass does one `ext.get("overlay")`. `observe` runs only for views whose `ext` has the key |
| A hint (callout, arrow, ring) | ≤ 30 µs per frame at 100×40 | Anchor resolution touches only visible rows; layout and routing are proportional to the callout's and the corridor's area |
| A spotlight | ≤ 60 µs per frame at 100×40 | Restyling is one pass over the screen's cells (proportional to the screen, never the document) |
| `observe` per message | ≤ 2 µs | Mapping a few ranges through a ChangeSet; predicates are field matches |
| Typing in a 5,000-line page in a host (≈1.7 ms per key today) | ≤ +3% with a hint showing, +0% without | Nothing scans the document. Mark anchors check visible rows before `Marks::pos` |

`render` at 100×40 costs 127 µs today. The overlay must stay within these numbers, which a bench
guards (7.3).

### 3.7 The capability ladder

Every rung consumes the same `Scene`; only the last step differs.

| Rung | When | Draws |
|---|---|---|
| **A · ASCII** | `Glyphs::Ascii`, dumb terminals | `+-|^v<>*`, `[key]`, dimming only with colour |
| **B · cells** (MVP) | Any terminal | Rounded box drawing, `▲▼◀▶●○`, roles for dim and ring. Works in herdr, tmux and SSH: they're plain cells |
| **C · sub-cell** (MVP, opt-in per item) | Truecolor (`COLORTERM=truecolor`; tmux needs `Tc` or `RGB`) | Half blocks `▀▄` for a soft drop shadow and rounded panel edges, eighth blocks `▁▔` for a thin ring, Braille leaders, quadrant and sextant arrowheads (WezTerm draws block and legacy-computing glyphs itself). Octants are opt-in: font support is still thin |
| **D · pixels in cleared cells** (later) | kitty graphics answer, no z support (WezTerm with `enable_kitty_graphics`) | Pixel arrows and rings only through cells the overlay cleared; callout and spotlight from rung C |
| **E · pixels over text** (later) | kitty or Ghostty, probed | A translucent veil, feathered holes, anti-aliased curves. Words stay text |

Rungs D and E are a runtime stage after the cell frame (IO), not a frame pass. They never change
the layout, so goldens hold.

## 4. Walkthroughs (`caretline-tour`)

### 4.1 Authoring format

```toml
id = "caretline.guide"
version = 1
title = "caretline in eleven steps"

[[step]]
id = "type"
anchor = { find = "## 1 · Type" }
title = "Type"
body = "Just start typing, and keep going past the edge: lines wrap at words."
advance = { msg = "insert_text", count = 5 }
items = ["ring", "callout", "arrow"]

[[step]]
id = "move"
anchor = { find = "⌥← and ⌥→" }
title = "Jump by word"
body = "{{key:move.word_left}} and {{key:move.word_right}} move one word. ↑ and ↓ keep the column."
advance = { any = [{ command = "move.word_right" }, { command = "move.word_left" }] }
spotlight = true

[[step]]
id = "fold"
anchor = { find = "Put the caret on this line" }
body = "Press {{key:view.fold_toggle}} on the lit line: the lines under it fold away."
advance = { command = "view.fold_toggle" }
skip_if = { folded = "anchor" }

[[step]]
id = "done"
anchor = { screen = "center" }
body = "That's the guide. {{key:guide.restart}} plays it again."
capture = true
next = [{ if = { ext = { key = "agent", present = true } }, goto = "agent" }, { goto = "end" }]
```

**Fields of a step:**
- `id`, `anchor` (one or a list), `title`, `body`;
- `items` (default: ring, callout and arrow);
- `spotlight`, `capture`, `place`;
- `advance`, `skip_if`, `nudge` (`{ after_ms, body }` to replace the text when the person
  hesitates);
- `next` (branching: the first matching `if` wins; `goto` names a step id or `end`).

**Predicates** (pure; they see the message, its effects and changes, and the view):

| Predicate | True when |
|---|---|
| `msg = "insert_text"` (+ `count`) | That message kind was applied (`count` times since the step began) |
| `command = "view.fold_toggle"` | The message equals that catalog command's message, host catalog included, so remapped keys count |
| `effect = "clipboard_set"` / `host_effect = "name"` | That effect came out |
| `caret_in = "anchor"` | The primary caret is inside the anchor |
| `selection = "nonempty"` | Any range is non-empty |
| `changed = "anchor"` | The message's changes touched the anchor |
| `folded = "anchor"` | The anchor's block is folded |
| `ext = { key, present / match }` | Another ext value is present or matches a subset |
| `after_ms = N` | `now_ms` minus the step's start is at least N (ticks keep replay exact) |
| `any`, `all`, `not` | Composition |

### 4.2 State and replay

- **State:** walkthrough progress lives in `view.ext["tour"]`: `{tour: {…the whole tour…}, step:
  "move", since_ms, counts, seen}`. `tour.start` puts the whole tour in the message, so a trace is
  self-contained and replays with no tour file.
- **Recording:** each step change is derived inside `observe` from the message that satisfied the
  predicate, so it adds no trace lines. Starts, stops and manual next and back are `Msg::Ext` lines.
- **Drawing:** the tour registers a `LayerSource` with the overlay. The current step becomes layers
  at draw time; they aren't copied into `ext["overlay"]`, so there's no second copy to keep in step.
- **Replay** of a trace with a guide reproduces the same frames, guide included, with the same
  crates registered (as for host commands: `trace::replay_trace_with(input, &host)`).

### 4.3 Seen-state is the host's

- **Loading:** the engine and the crates never touch storage. A host loads seen-state into the
  initial state (`ext["tour"].seen = {"caretline.guide": {"version": 1, "done": true}}`), so the
  trace's first `state` line carries it.
- **Saving:** changes leave as `Effect::Host { name: "tour.seen", data }`, and the host persists
  them. The standalone editor keeps `guides.json` in its state directory (`$XDG_STATE_HOME/caretline`,
  or `~/Library/Application Support/caretline`).
- **Policy (enforced by the reducer):**
  - Stopping is final for that version; a later version re-offers the guide only if `reoffer = true`.
  - A walkthrough never starts by itself unless the host passes `autostart` (the standalone editor
    doesn't).

## 5. Pluggability

### 5.1 How a host plugs in

| Integration | When | What the host does |
|---|---|---|
| **A · In-frame** | The screen is a caretline frame (the standalone editor, a panel) | `let host = caretline_overlay::install(host, OverlayConfig::default())`, plus `caretline_tour::install(host, tours)`. Everything else is automatic: the frame pass, the reducers, the ops, the catalog |
| **B · Screen-level** | The host draws more than caretline (lists, tabs, panes around editors) | The host keeps `Layers` (in a view's `ext` or its own model), records an `AnchorMap` while drawing (`anchors.put(AnchorKey::host("row", key), rect)`), and adds each embedded editor's frame and offset. Then it calls `layout` and `compose` on its grid (`impl CellGrid for ratatui::Buffer` behind the `ratatui` feature) |

**Extending:**
- **Anchor kinds:** `OverlayConfig::anchor_kind("row", fn(&Ctx, &Frame, &Value) -> Resolved)` for
  mode A, or the `AnchorMap` for mode B.
- **Primitives:** `OverlayConfig::primitive("badge.count", impl Primitive)`, where `Primitive` has
  `measure`, `place` and `draw(&mut dyn CellGrid)`.
- **Renderers:** `impl Rung` for a later pixel stage.
- **Layer sources:** `OverlayConfig::source(name, fn(&Ctx) -> Vec<Layer>)`, used by
  `caretline-tour` and, later, by a lint-style plugin that rings text.

### 5.2 The command catalog

`install` adds `HostCommandInfo` entries (E5), so they appear in F1 help, `caretline keys`,
`commands.list` and the MCP `commands` tool, tagged with their source:

| Id | Name | Default keys | Message |
|---|---|---|---|
| `guide.next` | Guide: next / skip | `F2` | `ext tour next` |
| `guide.back` | Guide: back | `⇧F2` | `ext tour back` |
| `guide.stop` | Guide: stop | `F3` | `ext tour stop` |
| `guide.more` | Guide: full text | `⇧F3` | `ext tour more` |
| `guide.restart` | Guide: start again | (none) | `ext tour restart` |
| `guide.reveal` | Show what the hint points at | (none) | `ext overlay reveal` |
| `hint.dismiss` | Dismiss the newest hint | `F4` | `ext overlay pop_newest` |
| `overlay.toggle` | Hide or show all hints | `⇧F4` | `ext overlay toggle` |

Defaults bind only if free; the host's table and the person's remaps win. F-keys pass through
herdr, tmux and SSH where `Alt` chords often don't.

### 5.3 Protocol ops and MCP tools

All ops take an optional `view`, defaulting to the person's view (0): showing a hint on the
person's screen is the point. Each becomes `Msg::Ext` lines in the trace, with the client as
`actor`.

```json
{"id":1,"op":"layer.push","actor":"claude","layer":{"anchor":{"block":7},"items":[{"callout":{"body":"…"}}],"ttl_ms":8000}}
{"id":1,"result":{"rev":42,"layer":"L-4","resolved":[{"rect":{"x":0,"y":19,"w":15,"h":1}}]}}

{"id":2,"op":"layer.pop","layer":"L-4"}            // or {"owner":"agent:claude"} or {"all":true}
{"id":2,"result":{"rev":43,"popped":["L-4"]}}

{"id":3,"op":"layer.list"}
{"id":3,"result":{"rev":43,"layers":[…]}}

{"id":4,"op":"hint.show","actor":"claude","anchor":{"text":{"from":412,"to":421}},
 "title":"Jump by word","text":"⌥← and ⌥→ move one word.","ttl_ms":8000,"place":["below","above"]}
{"id":4,"result":{"rev":44,"layer":"h-5","resolved":{"rect":{"x":17,"y":13,"w":4,"h":1}}}}
// not visible: "resolved": {"off":"below"}; missing: "resolved": null, "reason": "not_found"

{"id":5,"op":"hint.hide","layer":"h-5"}            // or {"all":true}: this actor's hints only

{"id":6,"op":"tour.start","tour":"caretline.guide"}  // or "tour": {…inline TOML-shaped JSON…}, optional "at": "move"
{"id":6,"result":{"rev":45,"step":"type","of":11}}

{"id":7,"op":"tour.step","to":"next"}             // "back" | "stop" | {"id":"fold"}
{"id":7,"result":{"rev":46,"step":"move","of":11}}
```

The `caretline-mcp` tools take MCP's 1-based `{line, col}` positions and convert them to chars:

| Tool | Arguments | Op |
|---|---|---|
| `show_hint` | `at: {line, col, to_line?, to_col?}` or `search: "text"` (must be unique, as `edit` requires), `text`, `title?`, `ttl_ms?` | `hint.show` |
| `hide_hint` | `id?` or `all` | `hint.hide` |
| `start_guide` | `guide` (an id) or `steps` (inline) | `tour.start` |
| `guide_step` | `to` | `tour.step` |
| `list_hints` | | `layer.list` |

The tools are annotated non-destructive. `resolved` tells the agent whether the hint landed, and
`read {render: true}` shows it what the person sees.

### 5.4 Safety limits for agent hints

These are enforced in the overlay reducer, so they hold for every client:

- **Cap:** at most 3 agent layers per view (the host may lower it, never raise it past 8). A
  fourth replaces that actor's oldest one.
- **Expiry:** `ttl_ms` defaults to 8 s and is clamped to 1–60 s. No permanent agent layers.
- **Rate:** at most 2 pushes per second per actor, measured by `now_ms`, so it stays deterministic.
  Excess is refused with `rate_limited`.
- **No focus steal:**
  - agents can't use `capture`;
  - agent layers never move the caret or scroll (an off-screen anchor gets a chip; the person
    reveals it);
  - a callout never covers the caret or the person's selection.
- **No dimming without consent:** agents can't use `spotlight` or `dim` unless the person allowed
  it (host config `agent_dim = "never" | "always"`, default `never`), or the person started the
  walkthrough the agent is stepping.
- **Size:** body at most 280 characters and 6 lines; title at most 40.
- **Attribution:** every agent layer is titled `◆ <actor>` and styled `overlay.agent.*`; it can't
  pass as the host's.
- **Dismissal:** the person always wins: `hint.dismiss` and `overlay.toggle` act on agent layers
  regardless of owner.

### 5.5 Plugin tiers

| Tier | Now or later | Overlay fit |
|---|---|---|
| 1 · Rust crates on `Host` | Now | `caretline-overlay` and `caretline-tour` are the first real tier-1 plugins and the reason for E1–E5 |
| 2 · Out of process (protocol) | Phase 2 | Agents and tools push `layer.*`, `hint.*` and `tour.*` ops. Their outputs (messages) are recorded, so replay needs no plugin. They can't register frame passes or anchor kinds (that's code), but can use registered kinds and `cells` |
| 3 · WASM | Later | Both crates are pure and already build for `wasm32`. A sandboxed module could add a `Primitive` or an anchor kind with the same determinism as tier 1 |

## 6. API sketch

### 6.1 Engine hooks (`caretline`)

```rust
// state.rs
pub struct View { /* … */ #[serde(default, skip_serializing_if = "BTreeMap::is_empty")] pub ext: BTreeMap<String, Value>, /* … */ }

// msg.rs
pub enum Msg { /* … */ Ext { key: String, #[serde(default)] op: Value } }   // passive

// host.rs
pub struct ExtOut { pub value: Option<Value>, pub effects: Vec<(String, Value)>, pub status: Option<String>, pub frame_clock: Option<u16> }
pub struct Observed<'a> { pub msg: &'a Msg, pub effects: &'a [Effect], pub changes: Option<&'a ChangeSet> }
pub type ExtApplyFn   = dyn Fn(&Ctx, Option<&Value>, &Value) -> Result<ExtOut, String> + Send + Sync;
pub type ExtObserveFn = dyn Fn(&Ctx, &Value, &Observed) -> Option<ExtOut> + Send + Sync;
pub type FramePassFn  = dyn Fn(&Ctx, &mut Frame) + Send + Sync;
pub struct HostCommandInfo { pub id: String, pub name: String, pub description: String, pub category: String, pub keys: Vec<String>, pub msg: Msg }
pub struct OpFns { pub to_msgs: Arc<dyn Fn(&Ctx, &Value) -> Result<Vec<Msg>, String> + Send + Sync>,
                   pub reply: Option<Arc<dyn Fn(&Ctx, &Frame, &Value) -> Value + Send + Sync>> }
impl Host {
    pub fn ext(self, key: &str, apply: impl Fn(..) + 'static, observe: Option<impl Fn(..) + 'static>) -> Host;
    pub fn frame_pass(self, name: &str, f: impl Fn(&Ctx, &mut Frame) + Send + Sync + 'static) -> Host;
    pub fn catalog(self, entries: Vec<HostCommandInfo>) -> Host;
    pub fn op(self, name: &str, fns: OpFns) -> Host;
}

// view.rs
pub struct Region { pub x: u16, pub y: u16, pub w: u16, pub h: u16, pub id: String }
impl Frame {
    pub fn set(&mut self, x: u16, y: u16, grapheme: &str, role: Role) -> u16;   // width written; never splits a wide grapheme
    pub fn restyle(&mut self, x: u16, y: u16, role: Role);
    pub fn role(&mut self, name: &str) -> Role;
    pub fn region_at(&self, x: u16, y: u16) -> Option<&Region>;
}
pub struct Frame { /* … */ pub regions: Vec<Region> }
pub enum Locate { At { x: u16, y: u16 }, Above, Below, Left, Right, Folded { block: MarkId } }
pub fn locate(doc: &Document, view: &View, pos: usize) -> Locate;
pub fn render_plain(doc: &Document, view: &View) -> Frame;   // without frame passes
```

### 6.2 `caretline-overlay`

```rust
#[derive(Serialize, Deserialize, Clone, PartialEq)] #[serde(deny_unknown_fields)]
pub struct Layer { pub id: String, pub owner: Owner, pub z: i16, pub since_ms: u64, pub ttl_ms: Option<u64>,
                   pub anchor: Vec<Anchor>, pub items: Vec<Item>, pub dim: bool, pub capture: bool }
pub enum Owner { Person, Host, Guide, Agent(String) }
pub enum Anchor { Text { from: usize, to: usize }, BlockText { block: MarkId, from: usize, to: usize },
                  Block(MarkId), Caret, Cells(Rect), Screen(ScreenPos), Find(String), Host { kind: String, key: Value } }
pub enum Item { Callout(Callout), Arrow(Arrow), Spotlight(Spotlight), Ring(Ring), Keys(Vec<String>), Steps { of: u16, at: u16 }, Custom { kind: String, data: Value } }
pub struct Layers { pub layers: Vec<Layer>, pub next: u64, pub hidden: bool }
pub enum LayerOp { Push(Layer), Update(Layer), Pop(Selector), PopNewest, Toggle, Reveal, Clear }

pub fn apply(layers: &mut Layers, op: LayerOp, actor: Option<&str>, now_ms: u64, limits: &Limits) -> Result<(), Refusal>;
pub fn observe(layers: &mut Layers, changes: Option<&ChangeSet>, now_ms: u64) -> bool;   // map anchors, expire

pub struct Grid { pub area: Rect, pub caret: Option<(u16, u16)>, pub protect: Vec<Rect> }
pub struct Resolved { pub rects: Vec<Rect>, pub off: Option<Off> }
pub trait Resolve { fn resolve(&self, a: &Anchor) -> Option<Resolved>; }   // FrameResolver<'a> (mode A), AnchorMap (mode B)
pub fn layout(layers: &[Layer], anchors: &dyn Resolve, grid: &Grid, glyphs: Glyphs) -> Scene;   // pure
pub trait CellGrid { fn size(&self) -> (u16, u16); fn set(&mut self, x: u16, y: u16, g: &str, role: &str); fn restyle(&mut self, x: u16, y: u16, f: &dyn Fn(&str) -> String); fn text_at(&self, x: u16, y: u16) -> &str; }
pub fn compose(scene: &Scene, grid: &mut impl CellGrid);
pub fn keymap(view: &View, key: &Key) -> Option<Msg>;   // capture steps only
pub struct OverlayConfig { pub glyphs: Glyphs, pub limits: Limits, pub agent_dim: AgentDim, /* anchor kinds, primitives, sources */ }
pub fn install(host: Host, cfg: OverlayConfig) -> Host;   // ext "overlay", frame pass, ops, catalog
pub struct Theme; impl Theme { pub fn dark() -> Theme; pub fn light() -> Theme; pub fn resolve(&self, role: &str, base: Style) -> Style; }
```

### 6.3 `caretline-tour`

```rust
pub struct Tour { pub id: String, pub version: u32, pub title: String, pub steps: Vec<Step> }
pub struct Step { pub id: String, pub anchor: Vec<Anchor>, pub title: Option<String>, pub body: String, pub items: Vec<ItemKind>,
                  pub spotlight: bool, pub capture: bool, pub advance: Option<Pred>, pub skip_if: Option<Pred>,
                  pub nudge: Option<Nudge>, pub next: Vec<Branch> }
pub enum Pred { Msg { kind: String, count: u32 }, Command(String), Effect(String), HostEffect(String), CaretIn(AnchorRef),
                Selection, Changed(AnchorRef), Folded(AnchorRef), Ext { key: String, test: ExtTest }, AfterMs(u64),
                Any(Vec<Pred>), All(Vec<Pred>), Not(Box<Pred>) }
pub fn parse_toml(src: &str) -> Result<Tour, String>;
pub fn install(host: Host, built_in: Vec<Tour>) -> Host;   // ext "tour", layer source, tour.* ops, guide.* catalog
```

## 7. MVP and phases

### 7.1 Phases

| Phase | Scope | Size |
|---|---|---|
| **1 · Engine hooks and the guide** (MVP) | E1–E5 with tests and the scope words. `caretline-overlay`: layers, built-in anchors and `find`, placement, the router, compose on rungs A–C, the theme, strip and chip modes, region hits. `caretline-tour`: TOML, predicates, branching, the reducer, the layer source, seen-state effects. `caretline-cli`: `caretline demo guide` (the 11 `## N ·` sections of `tour.md` as `demo/guide.toml`), with `demo tour` kept as an alias and the hand-written `tour_hint` match deleted; roles in the renderer; F-keys; region clicks; `guides.json` | Engine ~600 LOC (3 days); overlay ~1,800 LOC (6 days); tour ~800 LOC (3 days); CLI and demo ~400 LOC (2 days); goldens and bench (2 days): **about 2.5 engineer-weeks** |
| **2 · Agents** | The ops in `hello`; `hint.*`, `layer.*` and `tour.*` routed through `Host::op`; agent limits and styling; the MCP tools; `docs/protocol.md`, `docs/mcp.md`; `caretline demo agent` shows a hint as it edits | ~700 LOC: **1 week** |
| **3 · Host integration** | Mode B: `AnchorMap`, `CellGrid` for `ratatui::Buffer`, host anchor kinds, `LayerSource`; an `examples/overlay_ratatui.rs`; an "Overlays and guides" section in `docs/embedding.md`; the site playground runs the guide | ~600 LOC and docs: **1 week** |
| 4 · Later | Pixel rungs D and E (probe, raster, kitty diffing); tier-2 manifests; WASM primitives | Separate design |

### 7.2 Tests

- **Golden frames:** each mockup below, as text and in the `cells` format (role spans show `dim.*`,
  `ring.*` and `overlay.*`), at 80×24, 120×32, 44×16 and 20×8, with `Glyphs::Rounded` and
  `Glyphs::Ascii`.
- **Replay:** a trace that starts the guide, does every step's action and finishes replays to
  identical frames, step by step. So does a trace with an agent `hint.show` in the middle of
  typing, with `--replay … --snapshot`.
- **Properties** (the fuzzer, random sizes 20×6 to 300×80, random edits and scrolls):
  - a callout never covers its anchor, a hole, the status row, or (for agents) the caret;
  - nothing splits a wide grapheme;
  - a mapped text anchor equals the anchor recomputed from scratch;
  - the layout is the same after a resize there and back.
- **Guide lint:** every `find` in built-in tours resolves in `tour.md`, and every `command`
  predicate names a catalog id.
- **Protocol:** limits are refused with reasons (`rate_limited`, `dim_not_allowed`,
  `capture_not_allowed`); `resolved` reports `null` with a reason or `off`; `if_rev` works.
- **Performance guard** (`caretline bench` and a CI threshold test):
  - `render` 100×40 with no layers within 2% of the baseline;
  - a hint ≤ +30 µs and a spotlight ≤ +60 µs;
  - typing in a 5,000-block outline with a hint showing ≤ +3% per key.

### 7.3 Risks

| Risk | Mitigation |
|---|---|
| `View.ext` becomes a junk drawer, a second state | One key per installed crate; a size cap (64 KB per key); strict schemas (`deny_unknown_fields`) inside the crates |
| Replay without the crates registered skips `Msg::Ext`, so states differ | The CLI and MCP always register them; `replay_trace_with`; see open decision 3 |
| Ambiguous-width glyphs render wide in some setups and break boxes | `Glyphs::Ascii` and `CARETLINE_GLYPHS`; golden in both glyph sets |
| Dimming is invisible without colour | The callout and ring always carry the meaning; spotlight is decoration |
| `observe` adds per-message cost | Runs only when the key is present; bench threshold |
| A callout over text hides what the person is reading | Placement scoring, the sliver rule, `overlay.toggle`, short default TTLs |
| F-key conflicts in some terminals or multiplexers | Bindings are data; remappable; chips and palette commands too |
| Screen readers read callout text mid-line | Strip mode via config; `overlay = off` |

## 8. Mockups (80×24, over real `caretline` frames)

Each is `caretline crates/caretline-cli/src/demo/tour.md --snapshot 80x24` (plain or `--outline`)
with the overlay composited. Roles are listed under each, since plain text can't show colour.

### 8.1 A callout with an arrow at a word

```text
# Welcome to caretline

A terminal editor where the whole editor is one value you can save, send and
replay. Work down this page with ↓: the status bar names the keys for the
section the caret is in.

## 1 · Type
              ╭────────────────────────────────────────────────────────────────╮
The caret is a│ Jump by word                                                   │
keep going pas│ ⌥← and ⌥→ move one word at a time. Hold ⇧ to select            │
              │ as you go.                                   F4 dismisses      │
## 2 · Move   ╰───┬────────────────────────────────────────────────────────────╯
                  ▼
⌥← and ⌥→ jump a word at a time. ↑ and ↓ move by the rows you see, not by lines,
 and keep their column: go down through this paragraph and watch the caret hold
its place across wrapped rows and short ones.
Short row.
And a long row again, so the column has somewhere to come back to as you keep
going down past the short one.

## 3 · Select

Hold ⇧ with any arrow to select, and ⇧⌥ to select by words. As in a macOS text
 tour.md                                                                    1:1
```

The anchor is "word" on row 13 (`ring.text`). Below was the first choice, but it would cover
text, so the callout flipped above. The arrow runs only through the blank row 12. The sliver rule
pushed the box to the right edge.

### 8.2 A spotlight with dimmed surroundings

```text
# Welcome to caretline

A terminal editor where the whole editor is one value you can save, send and
replay. Work down this╭──────────────────────────────────────────────────╮
section the caret is i│ Lines wrap at words                              │
                      │ ↑ and ↓ move by the rows you see and keep the    │
## 1 · Type           │ column. Try it in the lit paragraph.             │
                      ╰───────┬──────────────────────────────────────────╯
The caret is already at the en│ of this paragraph, so just start typing, and
keep going past the edge of th│ window.
                              ▼
## 2 · Move

⌥← and ⌥→ jump a word at a time. ↑ and ↓ move by the rows you see, not by lines,
 and keep their column: go down through this paragraph and watch the caret hold
its place across wrapped rows and short ones.
Short row.
And a long row again, so the column has somewhere to come back to as you keep
going down past the short one.

## 3 · Select

Hold ⇧ with any arrow to select, and ⇧⌥ to select by words. As in a macOS text
 tour.md                                                                    1:1
```

The role map is what the `cells` golden checks: `.` is `dim.*`, `#` is `overlay.callout*`, `>` is
`overlay.arrow`, a blank is lit (unchanged), and `s` is the status bar (never dimmed).

```text
................................................................................
................................................................................
................................................................................
......................####################################################......
......................####################################################......
......................####################################################......
......................####################################################......
......................####################################################......
..............................>.................................................
..............................>.................................................
..............................>.................................................








................................................................................
................................................................................
................................................................................
................................................................................
ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss
```

### 8.3 An agent hint

```text

- Pack the bag
- Passport
  - and the charger
- Book

## 7 · Folds

- Put the caret on this line and press ⌃O to fold away the lines under it
  - a passport
  - a charger
  - a good book

## 8 · Marks

Every block carries a mark: an id that follows it through edits, moves, undo,
and cut and paste. With the caret here, the status bar shows the block's mark.
Move the line below with ⌥↑, cut and paste it, undo: the number stays.
                    ╭──────────────────────────────────────────────────╮
- Hold on to me ◀───┤ ◆ claude · hint                                  │
                    │ This line keeps mark #31 through ⌥↑, cut and     │
## 9 · A second view│ paste, and undo. Watch the status bar.           │
                    ╰──────────────────────────────────────────────────╯
 tour.md                                                                   21:3
```

An agent hint anchored to a block (`{"block": 31}`), placed to the right. It uses
`overlay.agent.*` roles, has no spotlight (agents may not dim by default), expires after 8 s, and
`F4` dismisses it. The callout avoids the caret and never covers the status row.

### 8.4 A walkthrough step with dots

```text

- Pack the bag
- Passport
  - and the charger
- Book

## 7 · Folds

- Put the caret on this line and press ⌃O to fold away the lines under it
  - a passport
  - a charger
  - a good book
            ▲
## 8 ·╭─────┴──────────────────────────────────────────────────────────────────╮
      │ Folds                                         7 of 11  ●●●●●●●○○○○     │
Every │ Press ⌃O on the lit line: the lines under it fold away.                │
and cu│                                                                        │
Move t│ F2 skip · ⇧F2 back · F3 stop                                           │
      ╰────────────────────────────────────────────────────────────────────────╯
- Hold on to me

## 9 · A second view

 tour.md                                                                   21:3
```

This is a "do" step: it advances when `view.fold_toggle` runs, by any key bound to it. The
"Put the caret…" block and its children (rows 8–11) are lit, with `ring.*` on row 8; everything
else is `dim.*`. `⌃O` is a `{{key:view.fold_toggle}}` badge (`overlay.key`). The F-key chips are
click regions.

### 8.5 Narrow widths (44×16)

The anchor is off-screen below, so the layer draws as a strip plus an edge chip:

```text
 2/11 ⌥← ⌥→ by word · ↓ below · F2 ▸

A terminal editor where the whole editor is
one value you can save, send and replay.
Work down this page with ↓: the status bar
names the keys for the section the caret is
in.

## 1 · Type

The caret is already at the end of this
paragraph, so just start typing, and keep
going past the edge of the window.

## 2 · Move                    ↓ 2/11 here
 tour.md                                1:1
```

After scrolling, the anchor is visible: the strip stays (the area is under 48 columns), the dots
appear, and the anchor's cells get the ring:

```text
 2/11 ●●○○○○○○○○○ ⌥← ⌥→ by word · F2 ▸
one value you can save, send and replay.
Work down this page with ↓: the status bar
names the keys for the section the caret is
in.

## 1 · Type

The caret is already at the end of this
paragraph, so just start typing, and keep
going past the edge of the window.

## 2 · Move

⌥← and ⌥→ jump a word at a time. ↑ and ↓
 tour.md                                9:1
```

`⇧F3` (`guide.more`) shows the full callout on rows the anchor doesn't use.

## 9. Open decisions

1. **Generic `View.ext` or a typed field.** I recommend generic (E1): the engine stays free of
   overlay concepts, and any future per-view plugin state fits. The alternative is a typed
   `View.layers` in core, which is simpler but is overlay scope inside the engine.
2. **How dimming is represented.** I recommend the role prefix (`dim.<role>`, `ring.<role>`): no
   change to `Cell`, and it's visible in the `cells` wire format. The alternative is a `Cell` flag
   byte, which is cleaner for hosts but changes `Cell`'s size and the wire format.
3. **What traces record.**
   - Ops (`Msg::Ext { op }`, recommended): readable intent, but replay needs the crates registered,
     as host commands do.
   - The resulting value (`Msg::Ext { set }`): host-free replay, bigger traces.
   - Both: ops plus a value checkpoint per step.
4. **Walkthrough keys.** F2 next or skip, ⇧F2 back, F3 stop, F4 dismiss a hint (proposed:
   F-keys survive herdr, tmux and SSH), versus Alt chords, versus Esc and Enter only in capture
   steps.
5. **Should frame passes run in every `render`?** I recommend yes, so agents and snapshots see
   hints. The alternative is running them only in the interactive runtime.
6. **Agent hint policy.** The defaults: cap 3, 8 s TTL (60 s maximum), 2 pushes per second,
   `agent_dim = never`. Also: should hint tools work under `caretline-mcp --read-only`? I recommend
   yes, since hints are view-only, with a `--no-hints` flag.
7. **`caretline demo guide` versus `demo tour`.** Should `guide` replace `tour` (with `tour` kept
   as an alias), and should the existing status-bar hint stay as the narrow-mode strip?
8. **Seen-state and autostart.** Should the standalone editor ever start the guide on first run,
   or only on `caretline demo guide`? I recommend only on request.
9. **Sub-cell defaults.** Should half-block shadows and Braille leaders be on by default in
   truecolor terminals, or opt-in until goldens and the font matrix (WezTerm, herdr, tmux, SSH)
   are checked? Octants stay opt-in either way.
10. **Where text-range rects come from.** The overlay can work them out from `Frame` (proposed),
    or a `view::locate_range` could go in core next to `locate`.
11. **Crate count.** Keep `caretline-tour` separate (proposed), or fold it into `caretline-overlay`
    behind a feature.

## 10. As built: step 1a (`caretline-layers`)

The first step built the screen-level half (mode B, 5.1) as its own crate, and the owner
narrowed its scope. Where it differs from the sections above:

- **Name and scope.** `caretline-overlay` became **`caretline-layers`**: placement, tracking
  and lifecycle only. It draws nothing: no `Theme`, glyph sets, roles, `compose` or
  `CellGrid`. `plan(&layers, &anchors, &grid, &renderers) -> Plan` returns geometry (each
  box or strip, the edge chip, the arrow's route as cells with directions, ring cells,
  spotlight holes, click regions) and the host draws it. The engine is unchanged; E1–E5 are
  step 1b.
- **Content is opaque.** `Item` (2.1, 6.2) became layer fields: `content: {kind, data}`
  (measured by a `Renderer` the host registers per kind), `arrow: bool`, `ring`, `spotlight`.
  The conventional kind is `hint` (`{"title"?, "text"}`); `{{key:…}}` markup, key badges and
  step dots are a host's content now.
- **Dimming is a flag, not a role prefix** (open decision 2): a host asks `Plan::dimmed(x,
  y)` or reads `Plan.spots`.
- **No screen-cell anchors.** `{"cells": …}` (2.3) was dropped: anchors store chars, mark ids
  and host keys only.
- **Owners and bands.** `person` layers take z 30–39, above agents.
- **Routing costs.** A text cell costs 16 (not 6) and a blank cell between words 6, so an
  arrow goes round words when a blank way exists; placement adds 300 per word cell an arrow
  would cross. The arrow may leave the box anywhere within 10 cells of the anchor's middle.
  The third-bend penalty is added to the finished route, not searched.
- **Ops** (5.3) are protocol-neutral: `ops::parse` and `ops::reply` serve any host's JSON
  protocol; `layer.list` parses to a listing, not an op.
- **Performance** (3.6): at 100×40, placing a hint box costs about 10 µs; with its arrow
  about 90 µs, and a spotlight with an arrow about 150 µs (release build), over the 30 and
  60 µs budgets. The routes for the candidate boxes dominate; a cost grid built once per
  plan and reusing the chosen box's route are the next steps.

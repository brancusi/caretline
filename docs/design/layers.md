# Design: the layer system

Status: design; steps 1a and 1b, and `caretline-tour`, built (section 12). Covers `caretline-layers` (where things go over a caretline
screen, and for how long) and `caretline-tour` (walkthroughs), the few generic hooks the engine
needs for them, the kitty graphics plumbing for Ghostty, and the phases. How a layer looks is not
in any caretline crate: hosts draw.

## 0. In one page

| Question | Answer |
|---|---|
| What is it? | A one-way, view-only layer system over whatever a caretline host shows: hints, callouts, arrows, spotlights and walkthroughs made of them. It never edits text and never takes focus by itself |
| The split | **caretline-layers owns placement, tracking and lifecycle. Hosts own all styling and drawing.** Hosts differ in styles, imagery and content, so nothing that decides how a layer looks goes in a caretline crate |
| What is a layer? | Serializable data in the view: `{id, owner, z, anchor, place, content, ttl}`. `content` is opaque: `{kind, data}`. Changed only by messages, so traces record it and replay draws it |
| How is it drawn? | Every frame the crate resolves anchors, asks the host's renderer to `measure` the content, places the box, routes a connector and cuts spotlight holes. The host's renderer draws the result, in cells and/or pixels |
| Content kinds | Hosts register a renderer per kind and list the kinds in `hello`, like commands. One conventional kind, **`hint`** (`{title?, text}`), is rendered by every host in its own style, so agents can point at things anywhere |
| Anchors | Stable keys resolved every frame: a text range, text in a block, a block (mark), the caret, cells, the screen, or a kind the host registers. They survive scroll, resize and edits because no cell position is stored |
| Pixels | Target **Ghostty 1.3.1**. Behind the `kitty` feature the crate owns the plumbing: the host hands an RGBA image per layer part, the crate transmits, re-places, swaps and deletes it. No raster, theme or glyph in the crate |
| Purity | One state atom; everything serializable; time only from ticks; the cell pixel size enters as a message; renderers are pure; the kitty memory is a cache. Pixel goldens replay across machines (4) |
| Agents | `hint.show` over the protocol and a `show_hint` MCP tool: capped, expiring, never modal, never dimming unless the person allows it. Agents get the host's cell composite; pixels are drawn only by the process that owns the terminal |
| Phase 1 | 1a the `caretline-layers` core (built); 1b the kitty plumbing and the CLI's `hint` renderer (built, 12.2); 1c the engine hooks (with `cell_px`), in-frame mode, `caretline-tour` and `caretline demo guide`. About 7 engineer-weeks (9.1) |

## 1. Scope and where it lives

### 1.1 The crates

| Crate | Contains | Depends on |
|---|---|---|
| `caretline` (engine) | Five generic hooks and one message change (1.3). Nothing named layer, tour, callout or spotlight, and no pixels | Unchanged dependencies |
| `caretline-layers` | The layer model (`Layer`, `Layers`, `LayerOp`, `apply`, `observe`); `Anchor`, its resolvers and `AnchorMap`; char anchors mapped through ChangeSets; off-screen state; placement (sides, flip, shift, clamp, collision, protected rects, strip fallback); connector route geometry; spotlight holes; regions and hit-testing; an opt-in agent policy a host may apply (none by default); the `layer.*` and `hint.*` protocol ops; the `Renderer` trait and registry; `install(host)` | `caretline`, `serde`. Feature `kitty`: the pixel plumbing (`miniz_oxide`, base64; no IO). No raster crate, no terminal, no async |
| `caretline-tour` | `Tour` (TOML or JSON), steps that name anchors, a content kind and data, predicates, branching, the tour reducer, seen-state as data, `tour.*` ops, a `LayerSource` that turns the current step into layers | `caretline`, `caretline-layers`, `serde`, `toml` |
| `caretline-cli` | A host. Its renderers for `hint` and for `cli.guide` (callout panel, curved arrow, ring, veil), in cells and in pixels (`tiny-skia` lives here); its theme and roles; the graphics probe and the write of the kitty bytes; `caretline demo guide`; seen-state file; F-key bindings | |
| `caretline-mcp` | `show_hint`, `hide_hint`, `start_guide`, `guide_step`, `list_hints` tools | |

Hosts with plenty of hints and no walkthroughs use `caretline-layers` alone. Both crates are pure,
so they compile to `wasm32` and the site's playground can run guides too, with its own renderers.

A shared renderer crate can come later, if a second host wants the CLI's look. It would sit beside
the hosts and never change the layer model.

### 1.2 The split

| caretline-layers decides | The host decides |
|---|---|
| Which layers exist, their order (`z`), owner and expiry | What each content kind looks like |
| Where each anchor is this frame, or that it is above, below or missing | Colours, roles, themes, glyph sets, fonts, images |
| The box: rect and side, given the size the host's `measure` returned | The box's size for given content (`measure`, pure) |
| The connector's route, as cell points | Whether it's a box-drawing line, a curve or nothing |
| Spotlight holes, as rects | Whether to dim, veil, tint or ignore them |
| Click regions for boxes and hit-testing | Sub-regions (chips) and what they run |
| Agent limits | How an agent layer is marked as an agent's |
| Kitty transport, ids, re-placing, swaps and deletes (`kitty`) | The pixels: one RGBA image per layer part |

### 1.3 What the engine must expose, and no more

| # | Hook | Why | Size |
|---|---|---|---|
| E1 | **View payloads.** `View.ext: BTreeMap<String, serde_json::Value>`, serialized, left out when empty, never read by the engine. The view-level twin of mark payloads | Layers and walkthrough progress are per view (a hint is on the person's screen, not the agent's), must be in the state for traces and replay, and must not be a typed engine concept | ~40 LOC |
| E2 | **`Msg::Ext { key, op }` and ext reducers.** `Host::ext(key, ExtFns { apply, observe })`. `apply(ctx, current, op) -> ExtOut` runs for `Msg::Ext`. `observe(ctx, current, &Observed { msg, effects, changes }) -> Option<ExtOut>` runs after every message for each view whose `ext` holds that key, with the message's composed `ChangeSet` (any view's edit, undo, `external`). `ExtOut { value, effects, status, frame_clock }`. `Msg::Ext` is passive (doesn't end a typing run or clear the status) and is accepted on read-only views | Applying layer and walkthrough ops; mapping text anchors through edits; "advance when" over messages; expiry on `tick`; pulses on the frame clock | ~180 LOC |
| E3 | **Frame passes and cell flags.** `Host::frame_pass(name, f: Fn(&Ctx, &mut Frame))`, run in registration order at the end of every `render`. `view::render_plain` skips them all; `view::render_skipping(doc, view, &[name])` skips named ones (the host is the document's). `Frame` gains public, grapheme-safe writers: `set(x, y, grapheme, role) -> u16` (overwriting either half of a wide grapheme blanks the other half; a wide grapheme that doesn't fit is drawn as a space), `restyle(x, y, role)`, `flag(x, y, CellFlags)`, `role(name) -> Role` (today's private `named`). `Cell` gains `flags: CellFlags` (one byte: `DIM`, `RING`). `Frame.regions: Vec<Region { x, y, w, h, id }>` and `Frame::region_at(x, y)` | The compositing hook for in-frame renderers, two generic marks a renderer may set, and the hit map. With no pass registered or nothing to draw, the cost is one map lookup | ~180 LOC |
| E4 | **`view::locate(doc, view, pos) -> Locate`.** The inverse of `hit`: `At { x, y }`, `Above`, `Below`, `Left`, `Right` (no-wrap scroll) or `Folded { block }` | Anchor resolution for text positions, marks and blocks, including which way an off-screen anchor lies. It is hit-testing, squarely in scope | ~80 LOC |
| E5 | **Host catalog entries and ops.** `Host::catalog(Vec<HostCommandInfo { id, name, description, category: String, keys: Vec<String>, msg: Msg }>)`; `Host::op(name, OpFns { to_msgs, reply })`. `hello`, `commands.list` and `keymap.get` list them with `source: "host"` | Layer commands appear in help (F1) and in agents' `commands`; `layer.push` and friends are routed without the protocol knowing them | ~150 LOC |
| E6 | **Cell pixel size as a message.** `Msg::Resize { width, height, cell_px: Option<CellPx> }` (`CellPx { w, h }`, device pixels; `#[serde(default)]`, left out when `None`). It is stored in `View.cell_px` (as built; the sketch had `Viewport`) and copied to `Frame.cell_px` | Pixel output becomes a pure function of the state, so pixel goldens replay across machines (4). Old traces replay unchanged | ~60 LOC, a golden and a CHANGELOG line (breaking for code that builds `Msg::Resize`) |

That's about 690 LOC in the engine, all generic. Text, marks, selections, undo and `view::hit`
are untouched.

### 1.4 Staying out of core scope

- `tests/scope.rs` gains `overlay`, `tour`, `tours`, `callout`, `spotlight` and `walkthrough` in
  `WORDS`, so the engine can't grow these concepts. The hooks are named `ext`, `frame_pass`,
  `locate`, `catalog` and `op`; the flags are named for what they ask of a renderer (`DIM`, `RING`).
- `caretline-layers` and `caretline-tour` get their own scope test with the host-word list: they
  name no host concept. `caretline-layers` also names no styling: no `theme`, `rgb`, `glyph`,
  `role` or raster crate.
- CI's `cargo tree` check extends to both crates: no terminal crate, no image crate and no async
  runtime. `kitty` adds only compression and base64.
- The engine never reads `View.ext`, roles or cell flags. Colours stay with the host, as for
  decorations.

## 2. The model

### 2.1 Layers

A layer is pure data in `view.ext["layers"]`:

```json
{"layers": [
  {"id": "guide:2", "owner": "guide", "z": 10, "since_ms": 1791367201000, "ttl_ms": null,
   "anchor": [{"text": {"from": 412, "to": 421}}, {"block": 7}],
   "place": {"sides": ["below", "above", "right", "left"], "max_w": 52,
             "connector": true, "spotlight": true},
   "capture": false,
   "content": {"kind": "cli.guide",
               "data": {"title": "Jump by word",
                        "text": "{{key:move.word_left}} and {{key:move.word_right}} move one word.",
                        "at": 2, "of": 11, "ring": true, "pulse_ms": 800}}}],
 "next": 4, "hidden": false}
```

- **The stack:** draw order is `z`, then insertion. Owners have z bands: `host` 0–9, `guide` 10–19,
  `agent` 20–29. An agent can't draw over a walkthrough's box. (Kitty z values are the
  renderer's, per part; 3.6.)
- **Owner:** `person` (a hint the person pinned), `host`, `guide` (a walkthrough) or
  `agent:<name>`. The owner decides the z band; a host's policy (7.5, none by default) may
  hold agents to limits by it. Renderers see it, and attribute an agent's layers their own way.
- **Lifetime:** `ttl_ms` from `since_ms`, measured against `doc.now_ms`. `observe` drops expired
  layers on `tick` and `frame`. `null` means it lasts until popped.
- **Anchors** are an ordered list of fallbacks (2.3).
- **Placement request** (`place`): the sides to try, a width cap, whether to route a connector,
  whether to cut spotlight holes, `hide_off_screen`, or a `screen` position (2.4).
- **Content** is opaque to the crate: a kind and its data. The crate checks only that it is
  well formed (a kind, and a `hint`'s shape); size caps are a host policy (7.5).

### 2.2 Content kinds

| Kind | Who renders it | Data |
|---|---|---|
| `hint` | **Every host**, in its own style. The one conventional kind | `{"title"?: string, "text": string}`. `text` may hold `{{key:<command id>}}` (the keys currently bound to a catalog command, so remaps show the right key) and `{{code:…}}`. No other markup |
| `<host>.<name>` | That host only, carried opaquely | Whatever the host's renderer reads. The CLI registers `cli.guide` (a walkthrough step: title, text, step dots, ring, pulse) |

- **Registration:** a host registers one renderer per kind (7.1). Placement needs its `measure`,
  so a layer of an unregistered kind is refused with `unknown_kind`.
- **Discovery:** `hello` lists the kinds as `layer_kinds: [{kind, agent}]`, like commands. `agent:
  true` means agents may push it; `hint` is always agent-allowed.
- **Old fields:** what was a typed item list (callout, arrow, ring, spotlight, steps) is now the
  placement request plus the kind's data. The crate knows "box", "connector" and "holes"; the
  host's renderer knows what they look like.

### 2.3 Anchors

| Anchor | JSON | Resolved by |
|---|---|---|
| Text range | `{"text": {"from": 412, "to": 421}}` (chars) | Visible rows: `Frame.rows` (`RowInfo::Text.chars`, `x`) to find the rows, then `cells[].char_idx` inside them. Several rects when it wraps. Off-screen or folded: `view::locate` |
| Text in a block | `{"text": {"block": 7, "from": 4, "to": 9}}` (offsets within the block) | The block's position plus the offsets: moves with the block through cut, paste and undo |
| Block | `{"block": 7}` (mark id) | The block's visible rows from `Frame.rows` (`block`, `first`, `last`); `locate` when it isn't visible. `Marks::pos` is a linear scan, so visible rows are checked first |
| Caret | `{"caret": true}` | `Frame.cursor`, else `locate(caret)` |
| Cells | `{"cells": {"x": 10, "y": 5, "w": 4, "h": 1}}` | Itself. Not stable: for agents that only have a rendered frame |
| Screen | `{"screen": "center"}` | Placement only (a welcome or a closing step) |
| Find (authoring only) | `{"find": "## 2 · Move"}` | Resolved **once**, when a step starts, to a block or text-in-block anchor, by the reducer (a recorded, deterministic message). Never searched per frame |
| Host kind | `{"host": {"kind": "row", "key": "a1b2"}}` | A resolver the host registers, or the host's `AnchorMap` (7.1) |
| In a view | `{"text": {"from": 4, "to": 9}, "in": "panel:2"}`: a text, block or caret anchor with `in` beside it | Only the view with that id (one `FrameResolver` per view of a document, each with its offset and clip). Unscoped anchors resolve in the focused view, else the first view that shows them, else which way they lie from the focused view. As built: 12 |

**Surviving change:**
- **Edits:** `observe` maps every char-range anchor through the message's `ChangeSet`: the start
  with `Assoc::After`, the end with `Assoc::Before`. A range that collapses to nothing falls to the
  next fallback. Block anchors need nothing: marks already follow their blocks.
- **Scroll and resize:** rects are recomputed every frame. Nothing positional is stored but chars
  and mark ids.
- **Folds:** a folded block's contents resolve to the fold's first row with `Locate::Folded`.

**Off-screen state.** Resolution gives rects, or `off`: `above`, `below` (`left`, `right` in a
no-wrap view) or `missing` (every fallback failed: a deleted mark, a collapsed range). Placement
passes it on (3.1), and `layer.push` replies with it.

### 2.4 Placement and collision

1. **The area** is the text rows. The status row is protected, except for a strip when the host
   turned its status bar off.
2. **Size:** the host's `measure(data, avail)` gives the box size for at most `max_w` and at most
   two-thirds of the width, once per candidate side with that side's room (as built: 12.3). It
   is pure: the same data and room give the same size.
3. **Candidates:** for each side in `sides` (default below, above, right, left), place the box
   beside the anchor, then **shift** along the side to stay inside the area.
4. **Score:** text cells covered (1 each), cells inside another layer's holes (0.3), the anchor,
   a hole or another layer's box (not allowed: **collision**), a protected rect (not allowed: the
   status row, and for agent layers the caret and the person's selection; the caret costs 50
   otherwise), and the distance to the anchor (0.5 per cell). The lowest score wins; ties go to
   the first side listed (**flip** comes from this ordering). Layers are placed in z order, so a
   later layer avoids an earlier one.
5. **The sliver rule:** if a box would leave fewer than 8 covered text cells between it and the
   area's edge, it extends or shifts to that edge, so no ragged ends of words show.
6. **Clamp:** a box never leaves the area. If no candidate fits, the layer falls back to a strip.
7. **Off-screen:** for an anchor above or below the view, the placement carries `off` and an
   `edge` cell on the first or last text row near the anchor's column. The box docks beside it,
   or is left out with `hide_off_screen`. Revealing it (`guide.reveal`, or a click on what the
   host drew there) scrolls the view with an ordinary `scroll` message. Agent layers never scroll
   the person's view.
8. **Strip fallback:** below 48 text columns or 12 text rows, or when no box fits, the layer gets
   one row (the top, or the bottom if the anchor is on the top row), measured with `Fit::Strip`.
   The anchor rects and holes are still given.

Placement is in cells. Pixels only change how the host draws the result.

### 2.5 Regions and input routing

| Mode | What happens |
|---|---|
| **Pass-through** (default, and always for agents) | Every key and message reaches the editor. A click inside a box is swallowed (the box is a region), so the caret never lands in text the person can't see. A click on a sub-region a renderer declared (a chip) runs its command. A click anywhere else passes through |
| **Capture** (`capture: true`, walkthroughs only) | The runtime asks `caretline_layers::keymap(&View, &Key) -> Option<Msg>` before its own keymap. Enter means next, Shift-Tab means back, Esc stops, and every other key is swallowed except quit. Protocol clients sending `msgs` aren't captured: they aren't keys |
| **Advance when** | A walkthrough step's predicate is checked in the tour's `observe` against each message, its effects and its changes (6.1) |

`hit(&placements, &regions, x, y) -> Option<Hit { layer, region }>` is the crate's. In-frame,
regions go into `Frame.regions`. Layer commands are catalog entries (7.3), so they work from keys,
chips and the protocol alike.

### 2.6 What the CLI's renderers draw (informative)

Not part of the model. This is how `caretline-cli` draws `hint` and `cli.guide` from a placement;
another host draws its own way.

| Primitive | Cells (fallback, goldens) | Pixels (Ghostty) |
|---|---|---|
| **Callout** (the box) | A rounded box `╭─╮│╰─╯`, its inside cleared, the title, wrapped text, and optional chips (`[F2 next]`) declared as regions | A panel image (rounded corners, soft shadow, rim) under the text; the box's cells hold only the words, with the default background |
| **Arrow** (the route) | Box drawing along the route's points (`─│╭╮╰╯`, `┬┴├┤` where it leaves the box), ending in `▲▼◀▶` | An anti-aliased curve through the route's points, over text |
| **Spotlight** (the holes) | The `dim` flag on every cell outside the holes | A translucent veil image with feathered holes, over text |
| **Ring** (the anchor rects) | The `ring` flag on the anchor's cells | An anti-aliased rounded outline around the anchor rects |
| **Key badge** | ` ⌃O ` padded by one space; `[Ctrl-O]` in ASCII | Text, as in cells |
| **Step dots** | `●●○○ 2 of 4`; `**--` in ASCII | Text, as in cells |
| **Pulse** | The ring's tint steps through 3 levels on the frame clock. Never SGR blink. Off under reduced motion | 6–8 pre-rendered ring variants swapped on the frame clock (3.6) |
| **Strip** | One row: dots, a shortened text and the next key | Text, as in cells |
| **Edge chip** (at `edge`) | `↓ 2/11 here`, a region that reveals the anchor | Text, as in cells |

Text is always text: the words are ordinary cells, so they show in snapshots, can be copied, and
work on every rung.

## 3. Geometry and the kitty plumbing

### 3.1 Every frame

The crate's work, after the host frame is drawn:

1. **Resolve** each layer's anchors (2.3): rects, or `off`.
2. **Measure:** call the kind's renderer, `measure(data, avail, fit)`.
3. **Place** in z order (2.4): rect and side, or a strip, or nothing.
4. **Route** a connector when asked (3.2).
5. **Cut holes** when asked (3.3).
6. **Regions:** one per box, plus the renderer's sub-regions.

The output is one `Placement` per layer (8.2): rect, side, anchor rects, off-screen state, edge
cell, route points and holes. It is derived, serializable (`render` with `format: "layers"`) and
never stored. The host's renderer draws from it.

In-frame (7.1, mode A) the crate's frame pass runs these steps and then calls each renderer's
`draw` into the `Frame`. A host that draws its own screen (mode B) calls `place` and draws itself.

### 3.2 Connector routes

- **Search:** A* on the cell grid inside a corridor (the bounding box of the box edge and the
  anchor, grown by 4 cells, clipped to the area). Typically under 500 nodes.
- **Cost:**
  - a blank cell costs 1, a cell inside another layer's holes 2, and a text cell 6;
  - a hole, another box, or the second half of a wide grapheme can't be crossed;
  - a bend costs 3, and a route with more than 2 bends costs 20.
- **Ends:** the route starts on the box edge facing the anchor and ends one cell outside the
  anchor's nearest edge.
- **Output:** `Route { points, from, head }`: the cell points (start, each bend, end), the box side
  it leaves from, and the direction it enters the anchor. No glyphs. A cell renderer picks
  `─│╭╮╰╯▲▼◀▶`; a pixel renderer draws a curve through the cell centres.
- **Memo:** the route is memoised by (box rect, anchor rect, the corridor's cell kinds). It's
  derived data, like the wrap cache, so scrolling under a still hint re-routes only when the
  corridor changes.

### 3.3 Spotlight holes

- Holes are rects of whole cells: the anchor's rects plus a margin of one cell (zero vertically),
  and the box. On a wrapped range the hole is each row's rect.
- The crate gives the holes; it doesn't dim. The renderer dims, veils, tints or ignores them.
- A spotlight is never the only signal: hosts should keep the meaning in the box and anchor,
  because NO_COLOR and plain-text snapshots can't show dimming.

### 3.4 Wide characters

- Box edges, routes and holes treat a wide pair as one obstacle: no rect edge or route point lands
  on a second half.
- In-frame renderers write through `Frame::set`, which never splits a wide grapheme.

### 3.5 Performance budget

| Situation | Budget | How |
|---|---|---|
| No layers (almost always) | +0 measurable | The pass does one `ext.get("layers")`. `observe` runs only for views whose `ext` has the key |
| A hint's geometry (resolve, place, route) | ≤ 20 µs per frame at 100×40, plus the renderer | Anchor resolution touches only visible rows; placement and routing are proportional to the box's and the corridor's area |
| Holes | ≤ 2 µs | A few rects |
| `observe` per message | ≤ 2 µs | Mapping a few ranges through a ChangeSet; predicates are field matches |
| Typing in a 5,000-line page in a host (≈1.7 ms per key today) | ≤ +3% with a hint showing, +0% without | Nothing scans the document. Mark anchors check visible rows before `Marks::pos` |

`render` at 100×40 costs 127 µs today. A bench guards the crate's numbers and the CLI's renderers
together (9.2).

As built (12): placement measures 6 µs for a box (four measures, one per side), 27 µs with
an arrow and 37 µs for a spotlight with an arrow. A host skips even that when nothing changed by keeping the last plan
with its inputs (12, "Unchanged inputs").

### 3.6 Pixel plumbing (feature `kitty`)

The target is Ghostty 1.3.1, the current stable release. Other terminals that answer kitty
graphics may work with `CARETLINE_LAYERS=pixels`, but only Ghostty is tested. Running inside
multiplexers is out of scope for pixels.

**Stages.**

| # | Stage | Where | IO |
|---|---|---|---|
| 1 | Placements in cells | `caretline_layers::place` | none |
| 2 | One RGBA image per layer part (panel, connector, ring, veil), each with a shape key | the host's renderer (the CLI's, with `tiny-skia`) | none |
| 3 | `KittyState::frame(&plan, &pictures, cell_px, files) -> Output`: transmit what the terminal lacks, re-place what moved, delete what's gone | `caretline-layers`, feature `kitty` | none (`t=t` files through the host's `TempFiles`) |
| 4 | Write the bytes after the text frame, inside the same DEC 2026 synchronized update | the runtime (`caretline-cli`, or a host's) | here only |

The engine never gets pixels. In pixel mode the renderers draw only their words into the frame
(`Surface::TextOnly`); every other consumer gets the cell composite (`Surface::Cells`). Once
in-frame mode lands (1c) the runtime renders with `view::render_skipping(…, &["layers"])`; in 1b
the CLI's demo draws over the composed frame itself.

**What the plumbing does.**
- **Ids from content:** an image id is a hash of the part's shape key (the renderer's inputs:
  kind, data, size in cells, `cell_px`, part name). A placement id is a hash of (layer id, part).
  Never counters, so the bytes sent are identical on replay. A hash collision with a different
  key in the cache is resolved by rehashing with a fixed salt.
- **Re-raster only on a new shape key:** `KittyState::holds(key, cell_px)` says whether the
  terminal already has a key's pixels; a `Picture` for one comes with `image: None`, and the host
  rasterises only the rest. A picture without pixels the terminal lacks is reported in
  `Output.missing` and not shown.
- **Re-place on scroll:** unchanged pixels at a new place are `a=p` with the same `i` and `p`. A
  part may carry a source crop (`x=`, `y=`, `w=`, `h=`), so a tall image scrolls by re-cropping.
- **Swaps:** a new image for the same part is placed before the old one is deleted, in one 2026
  update, so nothing flickers.
- **Deletes:** when a layer goes, its parts are deleted by id (`a=d,d=I,i=<id>`, freeing the data).
  On exit or `layers.toggle`, every id placed is deleted. Never `d=A`: the screen may hold other
  programs' images.
- **Transport:** `t=d` inline, zlib level 6 (`o=z`), in 4096-byte chunks (`m=1`), by default: it
  works over SSH. `t=t` (a file in `$TMPDIR` whose name contains `tty-graphics-protocol`) when the
  session is local and a probe confirmed it: about 15× fewer bytes on the terminal.
- **Cell size:** a `CellPx` value the host passes to every `frame` call, never read from the
  terminal by the crate. It is part of the image id, so a new cell size re-sends. Until E6 (1c)
  the CLI keeps it in its runtime, from the probe; then it comes from the frame (`cell_px`).

**The probe.** The crate builds the bytes; the runtime sends them in one write and reads the
answers through its own input parser:
1. `a=q` on a 1×1 image (`ESC _ G i=31,s=1,v=1,a=q,t=d,f=24;AAAA ESC \`);
2. XTVERSION (`CSI > q`);
3. `CSI 16 t` (cell size);
4. DA1 (`CSI c`) as the fence.

Pixels are on when the `a=q` answer is OK, XTVERSION names Ghostty (as built, kitty too), and the
cell size came back before the DA1 answer or a 200 ms timeout. Any missing answer means cells. Inside tmux the APC
never reaches the terminal, so the probe fails. `CARETLINE_LAYERS=auto|pixels|cells` overrides it.

**The runtime owns raw input parsing**, because terminal replies arrive mid-session, between keys.
The parser turns a reply into a message (the cell size into `Msg::Resize { cell_px }`, the
graphics answer into a `layers` caps op) and never changes state directly (4). As built (1b) the
crate's `probe::scan` recognises the replies, and the CLI's parser hands them to its event loop as
inputs that are never keys; a new cell size updates the runtime's copy until E6 makes it a message. It re-probes when
the `TIOCGWINSZ` pixel fields disagree with cells × cell size: changing the font size changes
pixels without always changing cells.

**What Ghostty 1.3.1 does.** Checked against the v1.3.1 source and its issue tracker:

| Fact | Detail | Where |
|---|---|---|
| Kitty graphics on by default | Only `image-storage-limit = 0` turns it off | `src/config/Config.zig` (`image-storage-limit`) |
| Transmission media | `t=d`, `t=f`, `t=t` and `t=s` are accepted. A `t=t` file must be in the temp dir and have `tty-graphics-protocol` in its name. On macOS use `$TMPDIR`, not `/tmp` | `src/terminal/kitty/graphics_image.zig`; ghostty issue 14567 |
| z ordering | As the spec: z < 0 under text, z < -1073741824 under non-default cell backgrounds, z ≥ 0 over text | `src/renderer/image.zig` (`bg_limit = minInt(i32) / 2`) |
| Moving a placement | `a=p` with the same `i` and `p` moves it without re-sending pixels | `src/terminal/kitty/graphics_exec.zig` |
| Placement keys | `X`/`Y` pixel offsets, `c`/`r` scaling, `C=1` (don't move the cursor) | `src/terminal/kitty/graphics_command.zig` |
| Deletes | Every `a=d` selector | `src/terminal/kitty/graphics_storage.zig` |
| Animation | **None in 1.3.1.** `a=f`, `a=a` and `a=c` answer "unimplemented". It has landed on main, unreleased | `src/terminal/kitty/graphics_exec.zig` |
| Unicode placeholders | Supported | `src/terminal/kitty/graphics_unicode.zig` |
| Query | `a=q` answers OK | `src/terminal/kitty/graphics_exec.zig` |
| XTVERSION | Answers `ghostty <version>` | `src/termio/stream_handler.zig` |
| DA1 | Answers `?62;22;52c` | `src/termio/stream_handler.zig` |
| Sizes | `CSI 16 t` gives the cell size in physical (device) pixels on Retina; `CSI 14 t` the window size; `TIOCGWINSZ` fills the pixel fields | `src/termio/stream_handler.zig`, `src/termio/Termio.zig`, `src/termio/Exec.zig` |
| Synchronized output | Mode 2026 | `src/terminal/modes.zig` |
| Storage | 320 MB of images per screen | `src/config/Config.zig` |

**Measured in the spike** (158×37 cells, 16×34 px cells, release build; visual check passed in
Ghostty 1.3.1):

| Situation | Cost |
|---|---|
| Full re-raster of callout, arrow, ring and veil | ≈ 0.8 ms; 22 KB with `t=d` zlib 6, 1.4 KB with `t=t` |
| A scroll step | 215 bytes: re-places only, the veil re-cropped with `y=` |
| A pulse frame | 87 bytes per swap, at 30 fps |
| Typing, caret moves, text repaints | 0 raster, 0 bytes |

**Recommendations from the spike**, now the defaults: one image per layer part, re-rasterised
only when its shape key changes; place-then-delete swaps in one 2026 update; `t=d` zlib 6 by
default and `t=t` when local and probed; re-probe the cell size when `TIOCGWINSZ` disagrees.
Renderer-side recommendations are in 5.4.

**Goldens.**
- Placements as JSON and the CLI's cell frames.
- PNG goldens of the CLI's raster at the `cell_px` recorded in the trace and a pinned `tiny-skia`
  version, byte-exact on one platform (Linux x86_64 in CI); other platforms compare within a
  tolerance.
- Kitty byte goldens for a sequence of frames (first place, scroll, change one part, pulse, remove),
  identical on every machine, because ids and `cell_px` come from the state. As built (1b):
  `tests/goldens/kitty.{first,scroll,change,remove}.txt` at a pinned cell size, and PNG goldens of
  the CLI's panel, arrow, ring and veil at 8×16 px cells; no pulse yet.

## 4. Purity and replay

The layer system follows the engine's rule: the state is one value, changed only by messages, and
everything on screen is a function of it.

| Rule | How |
|---|---|
| **One state atom** | In-frame, layers live in `View.ext["layers"]` inside `State`. A host that draws its own screen keeps `Layers` in its own single state and calls the crate's pure `apply` and `observe`, passing each message's ChangeSet. No second store |
| **Everything serializable** | `Layer`, `Layers`, anchors, placement requests and content are serde types with `deny_unknown_fields` |
| **Changed only by messages** | `LayerOp`s arrive as `Msg::Ext` (or the host's message); expiry and anchor mapping happen in `observe` |
| **Time only from ticks** | `since_ms`, expiry, rate limits and pulse phases use `now_ms` from `tick` and `frame` messages. Never the wall clock |
| **Anchors store chars and mark ids only** | No cell, row or pixel position is stored |
| **Derived every frame** | Resolution, placement, routes, holes and hit-testing. Memos (routes) are caches keyed by their inputs |
| **Pure renderers** | A host renderer's `measure` and `draw` depend only on their arguments. Placement depends on `measure`, so an impure one breaks replay |
| **Replay needs the same renderers** | As for host commands: `trace::replay_trace_with(input, &host)` with the same kinds registered. `hello` lists them, so a mismatch is visible |
| **Cell pixel size is a message** | `Msg::Resize { cell_px }` (E6; in a host, its equivalent). Pixel output is a pure function of the state, so pixel goldens replay across machines. Before E6, `cell_px` came from the terminal at draw time: that gap is closed in phase 1c |
| **The kitty memory is a renderer cache** | Images transmitted and ids placed are derived only from frames drawn, never fed back into state, and safe to drop: dropping it costs one full re-send. Ids come from content hashes, never counters, so the bytes sent are identical on replay |
| **Terminal replies are messages** | The runtime parses probe and size replies into messages. Nothing writes state from the input stream directly |

## 5. Host renderers (informative)

Everything here is the CLI's choice, as one host. Another host keeps the layer model and draws its
own way.

### 5.1 Roles and theme (the CLI)

Roles stay names; colours stay in the CLI's theme, as for decorations.

| Role or flag | Default dark / light (truecolor) | 16-colour and NO_COLOR |
|---|---|---|
| `layer.callout` | bg #1f2430 / #f4f1ea | default bg |
| `layer.callout.border` | fg #8aa4ff / #3b5bdb | bold |
| `layer.callout.title` | bold, fg as border | bold |
| `layer.arrow` | fg as border | bold |
| `layer.key` | bg #33405c / #dde4ff | reverse |
| `layer.dots`, `layer.dots.on` | fg muted / fg as border | faint / bold |
| `layer.agent.*` | the same set in the agent hue (#5fb3ff / #1c6fd1) | as above, plus a `◆ name` title |
| `layer.strip`, `layer.chip` | as callout | reverse |
| `dim` flag | the cell role's fg blended 60% toward the bg, its bg unchanged | SGR 2 (faint) |
| `ring` flag | the cell role's colours on bg blended 20% toward the border | underline |

The glyph set is chosen separately: rounded (default), square, or ASCII (`+-|`, `^v<>`, `*-`,
`[x]`). Ambiguous-width glyphs (`●○◆▲─│╭`) are one cell in Ghostty's default config; in CJK
locales, or terminals that treat them as wide, the CLI uses ASCII (`CARETLINE_GLYPHS=ascii`).

### 5.2 Dim and ring in cells

Cells have no alpha, so the CLI's cell renderer marks them with the engine's generic flags (E3):
- The symbol, role and `char_idx` stay the same, so copying text, accessibility and a host's
  decoration roles don't change.
- A cell outside the holes gains `dim`; an anchor cell gains `ring`. The theme blends in
  truecolor, uses the nearest colour in 256 colours, and SGR 2 in 16 colours.
- Flags don't compound: a cell is dimmed once.
- In pixel mode the veil replaces `dim`; the flags stay in the cell composite for agents and
  goldens, and the runtime draws flagged cells undimmed under the veil.

### 5.3 The capability ladder (the CLI)

| Rung | When | Draws |
|---|---|---|
| **P · pixels** (primary) | Ghostty 1.3.1 or later, probed; not inside a multiplexer | Panels at z=-1 under the words; arrows, rings and the veil at z=+1 over text. Words are text |
| **C · sub-cell** (opt-in) | Truecolor without pixels | Half blocks `▀▄` for a soft shadow and rounded panel edges, eighth blocks `▁▔` for a thin ring, Braille leaders, quadrant and sextant arrowheads. Octants stay opt-in: font support is still thin |
| **B · cells** (fallback, goldens) | Any terminal, tmux, SSH without graphics, snapshots, the protocol | Rounded box drawing, `▲▼◀▶●○`, the `dim` and `ring` flags |
| **A · ASCII** | ASCII glyphs, dumb terminals | `+-|^v<>*`, `[key]`, dimming only with colour |

Every rung draws from the same placements, so the geometry goldens hold for all of them. Built so far
(1b): P (Ghostty and kitty) and B; C and A are not.

### 5.4 Pixel notes (the CLI)

| Part | Kitty z | What the cells hold |
|---|---|---|
| Callout panel: rounded rect, soft shadow, a 1 px rim | -1: above cell backgrounds, below text | The box's cells cleared and written with its words, default background, so the panel shows through |
| Arrow (a curve through the route points), ring | +1: over text, anti-aliased | Unchanged text |
| Veil | +1: over the text area, translucent, feathered holes | Unchanged text, undimmed |
| Key badges, dots, strip, chips | none | Text, as in cells |

From the spike (3.6):
- **One image per part**, with a shape key; nothing re-rasterised while typing or scrolling.
- **The veil** at about 4×9 px per cell, scaled up with `c=` and `r=` (the feathering hides it),
  and rendered 3× the text height so a scroll is a re-crop with `y=`, not a re-raster.
- **Pulses** as 6–8 pre-rendered ring variants, swapped on the frame clock (at most 30 fps):
  Ghostty 1.3.1 has no kitty animation. Never blink. Off under reduced motion.
- On Retina, `CSI 16 t` reports device pixels, so images are twice the size in each direction.
  Panels are mostly flat and compress well.

## 6. Walkthroughs (`caretline-tour`)

### 6.1 Authoring format

Steps name anchors, a content kind and its data, a placement request, and predicates. The tour
crate knows nothing about how a step looks.

```toml
id = "caretline.guide"
version = 1
title = "caretline in eleven steps"
kind = "cli.guide"                      # the default content kind for every step

[[step]]
id = "type"
anchor = { find = "## 1 · Type" }
data = { title = "Type", text = "Just start typing, and keep going past the edge: lines wrap at words.", ring = true }
place = { connector = true }
advance = { msg = "insert_text", count = 5 }

[[step]]
id = "move"
anchor = { find = "⌥← and ⌥→" }
data = { title = "Jump by word", text = "{{key:move.word_left}} and {{key:move.word_right}} move one word. ↑ and ↓ keep the column." }
place = { connector = true, spotlight = true }
advance = { any = [{ command = "move.word_right" }, { command = "move.word_left" }] }

[[step]]
id = "fold"
anchor = { find = "Put the caret on this line" }
data = { text = "Press {{key:view.fold_toggle}} on the lit line: the lines under it fold away." }
advance = { command = "view.fold_toggle" }
skip_if = { folded = "anchor" }

[[step]]
id = "done"
anchor = { screen = "center" }
kind = "hint"                           # any registered kind
data = { text = "That's the guide. {{key:guide.restart}} plays it again." }
capture = true
next = [{ if = { ext = { key = "agent", present = true } }, goto = "agent" }, { goto = "end" }]
```

**Fields of a step:**
- `id`;
- `layers`: the step's layers, `[{id?, anchor, kind?, data, place?, capture?}]`. Each is a
  layer the tour owns (owner `guide`); `anchor` is one or a list, `kind` defaults to the
  tour's, and `id` defaults to `<step id>/<index>` (`move/0`, `move/1`). The single-layer
  fields `anchor`, `kind`, `data`, `place` and `capture` on the step are shorthand for a
  one-element `layers`; a step with both `layers` and any of them is an error;
- `narration` (optional): `{title?, text}`, the step's own words. It is not a layer: no
  anchor, nothing placed, no box. The host shows it where it likes (a docked panel, a strip,
  read aloud, or nowhere). The tour carries it untouched and exposes it on the current step;
  it is distinct from each layer's `data`, which is what that layer's box shows;
- `host` (optional): an opaque JSON value the host applies when the step starts, such as a
  view-state patch (which panel is open, what is focused or scrolled). The tour never reads
  it; it holds only the host's own state, not layers or narration;
- `advance`, `skip_if`, `nudge` (`{ after_ms, data }` merged into each layer's `data` when
  the person hesitates);
- `next` (branching: the first matching `if` wins; `goto` names a step id or `end`).

The tour adds `at` and `of` to the data of each layer whose kind isn't `hint`, so a renderer
can draw step dots (a `hint`'s data stays `{title?, text}`).

So a host authors a walkthrough as **view state + layers + narration**: `host` sets the
scene, `layers` point at things in it, `narration` says what the step is about.
`caretline-tour` reads all three (12.4). A step with two layers, one in each view of a document
(view-scoped anchors, 2.3):

```toml
[[step]]
id = "compare"
host = { panels = { right = "outline" }, focus = "main" }   # the host's own patch; opaque
narration = { title = "Two views, one document", text = "The outline on the right follows the text you edit on the left." }

advance = { command = "move.word_right" }   # before the [[step.layers]] tables, or TOML puts it in the last one

[[step.layers]]                      # id "compare/0"
anchor = { find = "## 2 · Move" }
kind = "hint"
data = { text = "You edit here…" }
place = { connector = true }

[[step.layers]]
id = "outline-entry"                 # given, instead of "compare/1"
anchor = { find = "## 2 · Move", in = "panel:outline" }
kind = "hint"
data = { text = "…and the outline shows the same heading." }
```

**Jumping** (`tour.step {to}`, back, a branch's `goto`) is deterministic: the tour applies
the target step's `host` value (the host's patch, as authored, whatever the steps between
would have set), then replaces the guide's layers with the target step's. Nothing of the
step it left stays, so a jump to a step looks the same as arriving there in order.

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

### 6.2 State and replay

- **State:** walkthrough progress lives in `view.ext["tour"]`: `{tour: {…the whole tour…}, step:
  "move", since_ms, counts, seen}`. `tour.start` puts the whole tour in the message, so a trace is
  self-contained and replays with no tour file.
- **Recording:** traces record ops (`Msg::Ext { key, op }`). Each step change is derived inside
  `observe` from the message that satisfied the predicate, so it adds no trace lines. Starts, stops
  and manual next and back are `Msg::Ext` lines. Each segment's `state` line carries the `ext`
  values as they stand at its start, so a segment replays on its own.
- **Drawing:** the tour registers a `LayerSource` with `caretline-layers`. The current step's
  layers are made at draw time; they aren't copied into `ext["layers"]`, so there's no second
  copy to keep in step. Its `narration` and `host` value are read from the current step by
  the host, never drawn by the crates.
- **Replay** of a trace with a guide reproduces the same frames, guide included, with the same
  crates and renderers registered.

### 6.3 Seen-state is the host's

- **Loading:** the engine and the crates never touch storage. A host loads seen-state into the
  initial state (`ext["tour"].seen = {"caretline.guide": {"version": 1, "done": true}}`), so the
  trace's first `state` line carries it.
- **Saving:** changes leave as `Effect::Host { name: "tour.seen", data }`, and the host persists
  them. The standalone editor keeps `guides.json` in its state directory (`$XDG_STATE_HOME/caretline`,
  or `~/Library/Application Support/caretline`).
- **Policy (enforced by the reducer):**
  - Stopping is final for that version; a later version re-offers the guide only if `reoffer = true`.
  - A walkthrough never starts by itself. The standalone editor starts the guide only on
    `caretline demo guide` or `guide.restart`.

## 7. Pluggability

### 7.1 How a host plugs in

A host registers a renderer per content kind, then picks a mode.

```rust
let host = caretline_layers::install(host, LayersConfig::default()
    .renderer("hint", HintRenderer::new(&theme), KindInfo { agent: true })
    .renderer("cli.guide", GuideRenderer::new(&theme), KindInfo { agent: false }));
```

| Integration | When | What the host does |
|---|---|---|
| **A · In-frame** | The screen is a caretline frame (the standalone editor, a panel) | `install` registers the `layers` ext reducer, the frame pass, the ops and the catalog. The frame pass resolves, places and calls each renderer's `draw` into the `Frame`. Add `caretline_tour::install(host, tours)` for walkthroughs |
| **B · Screen-level** | The host draws more than caretline (lists, tabs, panes around editors) | The host keeps `Layers` in its own state and calls `apply` and `observe` from its reducer, passing each message's ChangeSet. While drawing it records an `AnchorMap` (`anchors.put(AnchorKey::host("row", key), rect)`) and adds each embedded editor's frame and offset. Then it calls `place` and draws the placements its own way. Mode B needs no engine change |

Either mode can add pixels: the host's renderers produce `Picture`s, and the runtime calls
`KittyState::frame` and writes `Output.bytes` before ending the synchronized update.

**Extending:**
- **Content kinds:** `renderer(kind, impl Renderer, KindInfo)`. Host kinds are namespaced
  (`<host>.<name>`).
- **Anchor kinds:** `LayersConfig::anchor_kind("row", fn(&Ctx, &Frame, &Value) -> Resolved)` for
  mode A, or the `AnchorMap` for mode B.
- **Layer sources:** `LayersConfig::source(name, fn(&Ctx) -> Vec<Layer>)`, used by
  `caretline-tour` and, later, by a lint-style plugin that rings text.

### 7.2 The `hint` kind

`hint` is the one kind every host is expected to render, so an agent can point at something in any
host without knowing how that host looks.

- **Data:** `{"title"?: string, "text": string}`, with `{{key:…}}` and `{{code:…}}` in `text`.
- **What a host must do:** draw the text (all of it, or the first lines with the rest one key away)
  near the placement's rect and give the box a region. Marking agent layers as an agent's is
  recommended, and the host's choice (7.5). A connector and a ring are optional.
- **Where it comes from:** `hint.show` over the protocol, the MCP `show_hint` tool, or the host
  itself.

### 7.3 The command catalog

`install` adds `HostCommandInfo` entries (E5), so they appear in F1 help, `caretline keys`,
`commands.list` and the MCP `commands` tool, tagged with their source:

| Id | Name | Default keys | Message |
|---|---|---|---|
| `guide.next` | Guide: next / skip | `F2` | `ext tour next` |
| `guide.back` | Guide: back | `⇧F2` | `ext tour back` |
| `guide.stop` | Guide: stop | `F3` | `ext tour stop` |
| `guide.more` | Guide: full text | `⇧F3` | `ext tour more` |
| `guide.restart` | Guide: start again | (none) | `ext tour restart` |
| `guide.reveal` | Show what the hint points at | (none) | `ext layers reveal` |
| `hint.dismiss` | Dismiss the newest hint | `F4` | `ext layers pop_newest` |
| `layers.toggle` | Hide or show all hints | `⇧F4` | `ext layers toggle` |

Defaults bind only if free; the host's table and the person's remaps win. F-keys pass through SSH
where `Alt` chords often don't. A renderer can offer each command as a chip, so a mouse works too.
On macOS laptop keyboards the F-keys need `fn` unless "Use F1, F2, etc. keys as standard function
keys" is on; the CLI's chips and strip name the key as bound.

### 7.4 Protocol ops and MCP tools

All ops take an optional `view`, defaulting to the person's view (0): showing a hint on the
person's screen is the point. Each becomes `Msg::Ext` lines in the trace, with the client as
`actor`.

```json
{"id":0,"op":"hello"}
{"id":0,"result":{"proto":1,"layer_kinds":[{"kind":"hint","agent":true},{"kind":"cli.guide","agent":false}],…}}

{"id":1,"op":"layer.push","actor":"claude","layer":{"anchor":[{"block":7}],"content":{"kind":"hint","data":{"text":"This block moved."}},"ttl_ms":8000}}
{"id":1,"result":{"rev":42,"layer":"L-4","resolved":{"rects":[{"x":0,"y":19,"w":15,"h":1}]}}}

{"id":2,"op":"layer.pop","layer":"L-4"}            // or {"owner":"agent:claude"} or {"all":true}
{"id":2,"result":{"rev":43,"popped":["L-4"]}}

{"id":3,"op":"layer.list"}
{"id":3,"result":{"rev":43,"layers":[{"id":"L-5","owner":"agent:claude","z":20,"since_ms":1000,"ttl_ms":8000,"anchor":[{"caret":true}],"content":{"kind":"hint","data":{"text":"Here."}}}],"hidden":false}}

{"id":4,"op":"hint.show","actor":"claude","anchor":{"text":{"from":412,"to":421}},"title":"Jump by word","text":"⌥← and ⌥→ move one word.","ttl_ms":8000,"place":["below","above"]}
{"id":4,"result":{"rev":44,"layer":"L-6","resolved":{"rects":[{"x":17,"y":13,"w":4,"h":1}]}}}
{"id":4,"result":{"rev":44,"layer":"L-6","resolved":{"off":"below"}}}                  // not visible
{"id":4,"result":{"rev":44,"layer":"L-6","resolved":null,"reason":"not_found"}}     // no anchor resolved

{"id":5,"op":"hint.show","actor":"claude","anchor":{"text":{"from":4,"to":9},"in":"panel:2"},"text":"In the side panel."}
{"id":5,"result":{"rev":45,"layer":"L-7","resolved":{"rects":[{"x":66,"y":3,"w":5,"h":1}],"in":"panel:2"}}}

{"id":6,"op":"hint.hide","layer":"L-6"}            // or {"all":true}: this actor's hints only
{"id":6,"result":{"rev":46,"popped":["L-6"]}}
{"id":6,"error":{"reason":"not_allowed","detail":"layer \"L-6\" isn't this actor's"}}   // under a host's policy

{"id":7,"op":"tour.start","tour":"caretline.guide"}  // or "tour": {…inline TOML-shaped JSON…}, optional "at": "move"
{"id":7,"result":{"rev":47,"step":"type","of":11}}

{"id":8,"op":"tour.step","to":"next"}             // "back" | "stop" | {"id":"fold"}
{"id":8,"result":{"rev":48,"step":"move","of":11}}

{"id":9,"op":"render","format":"layers"}          // the placements, as JSON
{"id":9,"result":{"rev":48,"w":80,"h":24,"placements":[…]}}
```

As built (12), the `hint.*` and `layer.*` lines above are `caretline_layers::ops`: a host
passes a request to `ops::parse` and answers with `ops::reply`, `ops::list` or `ops::error`
inside its own envelope (`id`, `rev`). `ops::schema()` is their JSON Schema (draft
2020-12), and a test checks every such line here against it.

`hint.show` is `layer.push` with `content: {kind: "hint", data: {title, text}}`. A push of a kind
the host doesn't list is refused with `unknown_kind`.

**What agents see.** Agents drive layers over the protocol and MCP, unchanged by pixels:
- `render` (`text`, `ansi`, `cells`) returns the host's cell composite, with any `dim` and `ring`
  flags in `cells`.
- `render` with `format: "layers"` returns the placements: rects, sides, anchor rects, off-screen
  state, routes and holes.
- Pixels are drawn only by the process that owns the terminal. No op returns images or kitty bytes.

The `caretline-mcp` tools take MCP's 1-based `{line, col}` positions and convert them to chars:

| Tool | Arguments | Op |
|---|---|---|
| `show_hint` | `at: {line, col, to_line?, to_col?}` or `search: "text"` (must be unique, as `edit` requires), `text`, `title?`, `ttl_ms?` | `hint.show` |
| `hide_hint` | `id?` or `all` | `hint.hide` |
| `start_guide` | `guide` (an id) or `steps` (inline) | `tour.start` |
| `guide_step` | `to` | `tour.step` |
| `list_hints` | | `layer.list` |

The tools are annotated non-destructive. `resolved` tells the agent whether the hint landed, and
`read {render: true}` shows it what the person sees, in cells. The hint tools work under
`caretline-mcp --read-only` (hints are view-only); `--no-hints` turns them off.

### 7.5 Safety limits for agent hints

**As built (12): the layer level imposes no restrictions; hosts decide policy.** With
`Limits::default()` the reducer refuses only malformed input, and agents may do what the person
and the host can. The limits below are an opt-in policy, `Limits::agent_defaults()`, that a
host passes to `apply` (field by field: each limit is an `Option`); enabled, they behave as
described, deterministically. The CLI's choice is its own (decision 6).

- **Cap:** at most 3 agent layers per view (the host sets the number). A
  fourth replaces that actor's oldest one.
- **Expiry:** `ttl_ms` defaults to 8 s and is clamped to 1–60 s. No permanent agent layers.
- **Rate:** at most 2 pushes per second per actor, measured by `now_ms`, so it stays deterministic.
  Excess is refused with `rate_limited`.
- **Kinds:** agents may push `hint` and kinds the host registered with `agent: true`.
- **No focus steal:**
  - agents can't use `capture`;
  - agent layers never move the caret or scroll (an off-screen anchor gets an `edge`; the person
    reveals it);
  - an agent's box never covers the caret or the person's selection (protected rects).
- **No spotlight without consent:** agents can't set `place.spotlight` unless the person allowed
  it (`agent_dim = "never" | "always"`: `never` in the opt-in policy, `always` with none), or
  the person started the walkthrough the agent is stepping.
- **Size:** a `hint`'s text at most 280 characters and 6 lines; title at most 40. Other kinds'
  data at most 1 KB (`Limits::content_bytes`).
- **Attribution:** every placed layer carries its owner (`agent:<actor>`, `Owner::actor`), and
  the crate writes nothing into content. Drawing an agent's layers so they can't pass as the
  host's own is recommended, in the host's own style (the CLI titles them `◆ <actor>` in its
  agent hue); it is the host's choice, not a crate rule.
- **Dismissal:** the person always wins: `hint.dismiss` and `layers.toggle` act on agent layers
  regardless of owner.

### 7.6 Plugin tiers

| Tier | Now or later | Fit |
|---|---|---|
| 1 · Rust crates on `Host` | Now | `caretline-layers` and `caretline-tour` are the first real tier-1 plugins and the reason for E1–E6. Host renderers are tier-1 code |
| 2 · Out of process (protocol) | Phase 2 | Agents and tools push `layer.*`, `hint.*` and `tour.*` ops with registered kinds. Their outputs (messages) are recorded, so replay needs no plugin. They can't register renderers or anchor kinds (that's code) |
| 3 · WASM | Later | Both crates are pure and already build for `wasm32`. A sandboxed module could add a renderer or an anchor kind with the same determinism as tier 1 |

## 8. API sketch

### 8.1 Engine hooks (`caretline`)

```rust
// state.rs
pub struct View { /* … */ #[serde(default, skip_serializing_if = "BTreeMap::is_empty")] pub ext: BTreeMap<String, Value>,
                  #[serde(default, skip_serializing_if = "Option::is_none")] pub cell_px: Option<CellPx>, /* … */ }   // as built: in View, not Viewport
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct CellPx { pub w: u16, pub h: u16 }   // device pixels

// msg.rs
#[non_exhaustive]   // as built
pub enum Msg { /* … */
    Resize { width: u16, height: u16, #[serde(default, skip_serializing_if = "Option::is_none")] cell_px: Option<CellPx> },   // E6
    Ext { key: String, #[serde(default)] op: Value } }   // passive

// host.rs
pub struct ExtOut { pub value: Option<Value>, pub effects: Vec<(String, Value)>, pub status: Option<String>, pub frame_clock: Option<u16> }
pub struct Observed<'a> { pub msg: &'a Msg, pub effects: &'a [Effect], pub changes: Option<&'a ChangeSet>, pub acting: bool }
pub type ExtApplyFn   = dyn Fn(&Ctx, Option<&Value>, &Value) -> Result<ExtOut, String> + Send + Sync;
pub type ExtObserveFn = dyn Fn(&Ctx, &Value, &Observed) -> Option<ExtOut> + Send + Sync;
pub type FramePassFn  = dyn Fn(&Ctx, &mut Frame) + Send + Sync;
pub struct HostCommandInfo { pub id: String, pub name: String, pub description: String, pub category: String, pub keys: Vec<String>, pub msg: Msg }
pub struct OpFns { pub to_msgs: Arc<dyn Fn(&Ctx, &Value) -> Result<Vec<Msg>, String> + Send + Sync>,
                   pub reply: Option<Arc<dyn Fn(&Ctx, &Frame, &Value) -> Value + Send + Sync>> }
impl Host {
    pub fn ext(self, key: &str, fns: ExtFns) -> Host;   // ExtFns::new(apply).with_observe(observe)
    pub fn frame_pass(self, name: &str, f: impl Fn(&Ctx, &mut Frame) + Send + Sync + 'static) -> Host;
    pub fn catalog(self, entries: Vec<HostCommandInfo>) -> Host;
    pub fn op(self, name: &str, fns: OpFns) -> Host;
}

// view.rs
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct CellFlags(u8);
impl CellFlags { pub const DIM: CellFlags = CellFlags(1); pub const RING: CellFlags = CellFlags(2); }
pub struct Cell { pub symbol: Tendril, pub role: Role, pub char_idx: Option<u32>, pub flags: CellFlags }
pub struct Region { pub x: u16, pub y: u16, pub w: u16, pub h: u16, pub id: String }
impl Frame {
    pub fn set(&mut self, x: u16, y: u16, grapheme: &str, role: Role) -> u16;   // width written; never splits a wide grapheme
    pub fn restyle(&mut self, x: u16, y: u16, role: Role);
    pub fn flag(&mut self, x: u16, y: u16, flags: CellFlags);                  // ORs flags in
    pub fn role(&mut self, name: &str) -> Role;
    pub fn region_at(&self, x: u16, y: u16) -> Option<&Region>;
}
pub struct Frame { /* … */ pub regions: Vec<Region>, pub cell_px: Option<CellPx> }
pub enum Locate { At { x: u16, y: u16 }, Above, Below, Left, Right, Folded { block: MarkId } }
pub fn locate(doc: &Document, view: &View, pos: usize) -> Locate;
pub fn render_plain(doc: &Document, view: &View) -> Frame;                     // without frame passes
pub fn render_skipping(doc: &Document, view: &View, skip: &[&str]) -> Frame;   // the document's host
```

**The `cells` wire format** gains an optional `flags` field per row, runs of `[x, len, "dim"]`,
`[x, len, "ring"]` or `[x, len, "dim ring"]`, left out when empty. It is additive, so it stays
within proto 1; clients that don't know it ignore it.

```json
{"text":"The caret is already at the end of this paragraph, so just start typing, and    ",
 "flags":[[77,3,"dim"]]}
```

### 8.2 `caretline-layers`

```rust
#[derive(Serialize, Deserialize, Clone, PartialEq)] #[serde(deny_unknown_fields)]
pub struct Layer { pub id: String, pub owner: Owner, pub z: i16, pub since_ms: u64, pub ttl_ms: Option<u64>,
                   pub anchor: Vec<Anchor>, pub place: PlaceReq, pub content: Content, pub capture: bool }
pub struct Content { pub kind: String, pub data: Value }   // opaque to the crate
pub struct PlaceReq { pub sides: Vec<Side>, pub max_w: Option<u16>, pub connector: bool, pub spotlight: bool,
                      pub hide_off_screen: bool, pub screen: Option<ScreenPos> }
pub enum Side { Below, Above, Right, Left }
pub enum Owner { Person, Host, Guide, Agent(String) }
pub enum Anchor { Text { from: usize, to: usize }, BlockText { block: MarkId, from: usize, to: usize },
                  Block(MarkId), Caret, Cells(Rect), Screen(ScreenPos), Find(String), Host { kind: String, key: Value } }
pub struct Layers { pub layers: Vec<Layer>, pub next: u64, pub hidden: bool }
pub enum LayerOp { Push(Layer), Update(Layer), Pop(Selector), PopNewest, Toggle, Reveal, Clear }

pub fn apply(layers: &mut Layers, op: LayerOp, actor: Option<&str>, now_ms: u64, kinds: &Kinds, limits: &Limits) -> Result<(), Refusal>;
pub fn observe(layers: &mut Layers, changes: Option<&ChangeSet>, now_ms: u64) -> bool;   // map anchors, expire

// Host renderers: one per content kind. Both methods must be pure (4).
pub struct Size { pub w: u16, pub h: u16 }
pub enum Fit { Box, Strip }
pub trait Renderer: Send + Sync {
    fn measure(&self, data: &Value, avail: Size, fit: Fit) -> Size;
    fn draw(&self, cx: &DrawCx, frame: &mut Frame) -> Vec<Region> { Vec::new() }   // in-frame (mode A) only
}
pub struct DrawCx<'a> { pub layer: &'a Layer, pub at: &'a Placement, pub surface: Surface, pub now_ms: u64, pub keys: &'a dyn KeyNames }
pub enum Surface { Cells, TextOnly }   // TextOnly: pixel mode, the host draws the rest as images
pub struct KindInfo { pub agent: bool }

// Placement: derived every frame, serializable, never stored.
pub enum Off { Above, Below, Left, Right, Missing }
pub struct Resolved { pub rects: Vec<Rect>, pub off: Option<Off> }
pub struct Route { pub points: Vec<(u16, u16)>, pub from: Side, pub head: Side }   // cell points; no glyphs
pub struct Placement {
    pub layer: String,
    pub fit: Option<Fit>,        // None: hidden (off-screen with hide_off_screen, or layers hidden)
    pub rect: Option<Rect>,      // the box (or the strip row)
    pub side: Option<Side>,
    pub anchor: Resolved,        // anchor rects, or off-screen state
    pub edge: Option<(u16, u16)>,// where an off-screen marker goes
    pub route: Option<Route>,
    pub holes: Vec<Rect>,
}
pub struct Grid<'a> { pub area: Rect, pub caret: Option<(u16, u16)>, pub protect: Vec<Rect>, pub cells: &'a dyn CellKinds }
pub trait Resolve { fn resolve(&self, a: &Anchor) -> Option<Resolved>; }   // FrameResolver<'a> (mode A), AnchorMap (mode B)
pub fn place(layers: &[Layer], anchors: &dyn Resolve, kinds: &Kinds, grid: &Grid) -> Vec<Placement>;   // pure
pub fn hit(placements: &[Placement], regions: &[Region], x: u16, y: u16) -> Option<Hit>;
pub fn keymap(view: &View, key: &Key) -> Option<Msg>;   // capture steps only

pub struct LayersConfig { pub limits: Limits, pub agent_dim: AgentDim, /* renderers, anchor kinds, sources */ }
pub fn install(host: Host, cfg: LayersConfig) -> Host;   // ext "layers", frame pass, ops, catalog

#[cfg(feature = "kitty")]
pub mod kitty {   // as built in 1b
    pub struct CellPx { pub w: u16, pub h: u16 }                  // device pixels, a value the host passes
    pub struct Image { pub w: u32, pub h: u32, pub rgba: Arc<[u8]> }   // straight alpha; Image::new checks the length
    pub enum Z { Below, Above }                                   // under the text (above cell backgrounds) or over it
    pub struct Cells { pub x: i32, pub y: i32, pub w: u16, pub h: u16 }   // may reach past the screen
    pub struct Picture { pub layer: String, pub part: String, pub key: u64,   // key: the shape key
                         pub z: Z, pub at: Cells, pub clip: Option<Rect>,
                         pub image: Option<Image> }               // None when the terminal holds the key
    pub enum Transport { Direct, File }                           // t=d zlib, base64 chunks | t=t
    pub struct Options { pub transport: Transport, pub chunk: usize, pub level: u8 }   // default Direct, 4096, 6
    pub trait TempFiles { fn write(&mut self, name: &str, data: &[u8]) -> Option<String>; }   // the host's I/O
    pub struct Output { pub bytes: Vec<u8>, pub sent: Vec<u32>, pub placed: usize,
                        pub deleted: usize, pub missing: Vec<(String, String)> }
    pub struct KittyState { /* the renderer cache: images held, (layer, part) placements */ }
    impl KittyState {
        pub fn new(opts: Options) -> KittyState;
        pub fn options(&self) -> Options;
        pub fn set_options(&mut self, opts: Options);             // later images only
        pub fn holds(&self, key: u64, cell: CellPx) -> bool;      // no need to rasterise
        pub fn image_id(&self, key: u64, cell: CellPx) -> u32;
        pub fn is_empty(&self) -> bool;
        pub fn frame(&mut self, plan: &Plan, pictures: &[Picture], cell: CellPx,
                     files: Option<&mut dyn TempFiles>) -> Output;   // APC bytes, no I/O
        pub fn clear(&mut self) -> Vec<u8>;                       // delete every id placed, and forget
        pub fn reset(&mut self);                                  // forget; the next frame re-sends
    }
    pub fn shape_key(parts: &[&[u8]]) -> u64;                     // FNV-1a: stable across machines
    pub fn base64(bytes: &[u8]) -> String;
}

#[cfg(feature = "kitty")]
pub mod probe {
    pub const QUERY_ID: u32 = 31;
    pub fn request() -> Vec<u8>;                                  // a=q, XTVERSION, CSI 16 t, DA1
    pub fn cell_size_request() -> &'static [u8];                  // CSI 16 t alone
    pub enum Reply { Graphics { id: Option<u32>, ok: bool, message: String }, Version(String),
                     CellSize(CellPx), WindowSize { w: u32, h: u32 }, Da1(Vec<u32>) }
    pub enum Scan { Reply(Reply, usize), Partial, No }            // No: a key, mouse, paste or text
    pub fn scan(input: &[u8]) -> Scan;                            // for the runtime's input parser
    pub struct Probe { pub graphics: Option<bool>, pub version: Option<String>,
                       pub cell: Option<CellPx>, pub fenced: bool }
    impl Probe { pub fn add(&mut self, r: &Reply) -> bool; pub fn terminal(&self) -> Option<String>;
                 pub fn graphics_ok(&self) -> bool; }
}
```

### 8.3 `caretline-tour`

```rust
pub struct Tour { pub id: String, pub version: u32, pub title: String, pub kind: String, pub steps: Vec<Step> }
pub struct Step { pub id: String, pub anchor: Vec<Anchor>, pub kind: Option<String>, pub data: Value, pub place: PlaceReq,
                  pub capture: bool, pub advance: Option<Pred>, pub skip_if: Option<Pred>,
                  pub nudge: Option<Nudge>, pub next: Vec<Branch> }
pub enum Pred { Msg { kind: String, count: u32 }, Command(String), Effect(String), HostEffect(String), CaretIn(AnchorRef),
                Selection, Changed(AnchorRef), Folded(AnchorRef), Ext { key: String, test: ExtTest }, AfterMs(u64),
                Any(Vec<Pred>), All(Vec<Pred>), Not(Box<Pred>) }
pub fn parse_toml(src: &str) -> Result<Tour, String>;
pub fn install(host: Host, built_in: Vec<Tour>) -> Host;   // ext "tour", layer source, tour.* ops, guide.* catalog
```

## 9. Phases

### 9.1 Phases

| Phase | Scope | Size |
|---|---|---|
| **1a · `caretline-layers` core** (in progress) | For hosts that draw their own screen, with no engine change: `Layer`, `Layers`, `LayerOp`, `apply` and `observe` as plain functions; content kinds and the `Renderer` trait (`measure`); built-in anchors and `AnchorMap`; ChangeSet mapping and off-screen state; placement (sides, flip, shift, clamp, collision, protected rects, the sliver rule, strip fallback); routes; holes; regions and `hit`; agent limits; placements as JSON; scope test; placement goldens and a bench | ~1,600 LOC: **1.5 weeks** |
| **1b · Kitty plumbing and the CLI's renderers** (built, 12.2; `cli.guide`, pulses and the `t=t` probe deferred) | The spike is done (3.6). The `kitty` feature: shape-key ids, transmit, re-place with crops, place-then-delete swaps, deletes, `t=d` and `t=t`, probe bytes and reply parsing, byte goldens. In `caretline-cli`: the `hint` and `cli.guide` renderers in cells (rungs A–C) and pixels (`tiny-skia`: panel, curve, ring, veil, pulse variants), the theme, the probe, `CARETLINE_LAYERS`, PNG goldens | Plumbing ~600 LOC; renderers ~1,100 LOC: **2 weeks** |
| **1c · Engine hooks and the guide** (the hooks are built, 12.5; mode A is next) | E1–E6 with tests: `View.ext`, `Msg::Ext`, frame passes, `CellFlags` and the `cells` wire field, `locate`, catalog and ops, and **`cell_px` in `Msg::Resize`** (a golden and a CHANGELOG line); the scope words; in-frame mode (`install`, the frame pass, `FrameResolver`, region hits); `caretline-tour` (TOML, predicates, branching, the reducer, the layer source, seen-state effects); `caretline-cli`: `caretline demo guide` (the 11 `## N ·` sections of `tour.md` as `demo/guide.toml`), `demo tour` kept as an alias, the hand-written `tour_hint` match deleted, F-keys, `guides.json`, the runtime's input parser turning terminal replies into messages; replay goldens, pixels included | Engine ~700 LOC (4 days); mode A ~400 LOC (2 days); tour ~800 LOC (4 days); CLI ~500 LOC (3 days); goldens (2 days): **about 3 weeks** |
| **2 · Agents** | `layer_kinds` in `hello`; `hint.*`, `layer.*` and `tour.*` routed through `Host::op`; `render` with `format: "layers"`; agent limits end to end; the MCP tools, `--no-hints`; `docs/protocol.md`, `docs/mcp.md`; `caretline demo agent` shows a hint as it edits | ~700 LOC: **1 week** |
| **3 · Host adoption** | Hosts register their `hint` renderer and their own kinds; host anchor kinds and `LayerSource` for mode B hosts; a "Layers and guides" section in `docs/embedding.md`; the site playground runs the guide with cell renderers; tier-2 manifests; a shared renderer crate only if a second host wants the CLI's look | **1–2 weeks**, then separate designs |

Phase 1 is about 6.5–7 engineer-weeks in all: 1b's plumbing can run in parallel with 1a once
`Placement` is fixed.

### 9.2 Tests

- **Placement goldens:** placements JSON for each mockup at 80×24, 120×32, 44×16 and 20×8.
- **CLI goldens:** each mockup as text and in the `cells` format (role spans and the `flags`
  field), with rounded and ASCII glyphs.
- **Pixel goldens:** PNGs of the CLI's raster and kitty byte sequences, at the `cell_px` in the
  trace (3.6). The same trace gives the same bytes on every machine.
- **Replay:** a trace that starts the guide, does every step's action and finishes replays to
  identical frames, step by step, pixels included. So does a trace with an agent `hint.show` in
  the middle of typing, with `--replay … --snapshot`. A replay with the kitty cache dropped midway
  sends one full re-send, then the same bytes as before.
- **Properties** (the fuzzer, random sizes 20×6 to 300×80, random edits and scrolls):
  - a box never covers its anchor, a hole, another box, the status row, or (for agents) the caret;
  - no rect edge or route point lands on the second half of a wide grapheme;
  - a mapped text anchor equals the anchor recomputed from scratch;
  - placement is the same after a resize there and back;
  - `measure` called twice gives the same size (renderer purity);
  - the kitty plumbing never leaves an id placed that the current frame doesn't have.
- **Guide lint:** every `find` in built-in tours resolves in `tour.md`, every `command` predicate
  names a catalog id, and every step's kind is registered.
- **Protocol:** under a host's policy, limits are refused with reasons (`rate_limited`,
  `dim_not_allowed`, `capture_not_allowed`, `unknown_kind`); `resolved` reports `off` or
  `missing`; `if_rev` works.
- **Performance guard** (`caretline bench` and a CI threshold test):
  - `render` 100×40 with no layers within 2% of the baseline;
  - a hint ≤ +30 µs and a spotlight ≤ +60 µs, the CLI's cell renderers included;
  - typing in a 5,000-block outline with a hint showing ≤ +3% per key;
  - typing with pixels on sends 0 graphics bytes.
- **By hand in Ghostty 1.3.1:** the mockups in pixels at 1× and 2×, a font-size change, scroll
  under a hint, reduced motion.

### 9.3 Risks

| Risk | Mitigation |
|---|---|
| `View.ext` becomes a junk drawer, a second state | One key per installed crate; a size cap (64 KB per key); strict schemas (`deny_unknown_fields`) inside the crates |
| Replay without the crates or renderers registered skips `Msg::Ext` or measures differently | The CLI and MCP always register them; `replay_trace_with`; `hello` lists the kinds; each segment's state line carries the `ext` values |
| An impure `measure` moves boxes on replay | Documented as a contract; the property test calls it twice; the CLI's renderers are golden-tested |
| Styling creeps into the crate | The crate's scope test bans styling words and raster crates; the API takes sizes and gives rects |
| Hosts render `hint` too differently for agents to rely on | A small required behaviour (7.2); `render` with `format: "layers"` tells the agent where it landed, whatever it looks like |
| Ambiguous-width glyphs render wide in some setups and break boxes | The CLI's ASCII glyphs and `CARETLINE_GLYPHS`; goldens in both glyph sets |
| Dimming is invisible without colour | Hosts keep the meaning in the box and anchor; spotlight is decoration |
| `observe` adds per-message cost | Runs only when the key is present; bench threshold |
| A box over text hides what the person is reading | Placement scoring, the sliver rule, `layers.toggle`, short default TTLs |
| The pixel path is tested in one terminal | Cells stay the golden source and the fallback; the probe requires every answer; `CARETLINE_LAYERS=cells` |
| Ghostty changes kitty behaviour (animation lands, limits change) | Pin the facts in 3.6 to a version; re-check on each Ghostty release; pulses don't depend on animation |
| The kitty cache disagrees with the terminal | It is a cache: `reset` and re-send. Content-hash ids make stale ones harmless to re-place |
| Images left on screen after a crash | The runtime clears by id on exit and on start |
| F-key conflicts in some terminals | Bindings are data; remappable; chips and palette commands too |
| Screen readers read box text mid-line | Strip mode via config; layers off |

## 10. Mockups: the CLI's renderers (80×24, over real `caretline` frames)

Each is `caretline crates/caretline-cli/src/demo/tour.md --snapshot 80x24` (plain or `--outline`)
with the CLI's renderers drawing on the cell rung. They show one host's look, not the layer
model: another host draws the same placements its own way. Roles and flags are listed under each,
since plain text can't show colour. In Ghostty the CLI draws the same placements with pixel panels,
curves and veils.

### 10.1 A callout with an arrow at a word

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

The anchor is "word" on row 13 (the `ring` flag). Below was the first choice, but it would cover
text, so the box flipped above. The route runs only through the blank row 12. The sliver rule
pushed the box to the right edge.

### 10.2 A spotlight with dimmed surroundings

```text
# Welcome to caretline

A terminal editor wher╭────────────────────────────────────────────────────────╮
replay. Work down this│ Lines wrap at words                                    │
section the caret is i│ ↑ and ↓ move by the rows you see and keep the column.  │
                      │ Try it in the lit paragraph.                           │
## 1 · Type           ╰───────┬────────────────────────────────────────────────╯
                              ▼
The caret is already at the end of this paragraph, so just start typing, and
keep going past the edge of the window.

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

The anchor is the paragraph on rows 8–9. The box sits above it, and the sliver rule took it to
the right edge. The route is one head in the blank row 7, so it crosses no words.

The map is what the `cells` golden checks: `.` is the `dim` flag the CLI set outside the holes,
`#` is `layer.callout*`, `>` is `layer.arrow`, a blank is lit (unchanged), and `s` is the status
bar (never dimmed). The holes are each anchor row's text plus one cell, and the box.

```text
................................................................................
................................................................................
......................##########################################################
......................##########################################################
......................##########################################################
......................##########################################################
......................##########################################################
..............................>.................................................
                                                                             ...
                                        ........................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
................................................................................
ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss
```

### 10.3 An agent hint

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

A `hint` from an agent, anchored to a block (`{"block": 31}`), placed to the right. The CLI draws
it in its `layer.agent.*` roles. No spotlight (agents may not ask for one by default); it expires
after 8 s, and `F4` dismisses it. The box avoids the caret and never covers the status row.

### 10.4 A walkthrough step with dots

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

A `cli.guide` step. This is a "do" step: it advances when `view.fold_toggle` runs, by any key
bound to it. The "Put the caret…" block and its children (rows 8–11) are inside the holes, with
the `ring` flag on row 8; the CLI sets `dim` on every other cell. `⌃O` is a
`{{key:view.fold_toggle}}` badge (`layer.key`). The F-key chips are regions the renderer declared.

### 10.5 Narrow widths (44×16)

The anchor is off-screen below, so the layer gets a strip, and the CLI draws an edge chip at the
placement's `edge` cell:

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

`⇧F3` (`guide.more`) shows the full box on rows the anchor doesn't use.

## 11. Decisions

1. **View state:** a generic `View.ext` (E1), not a typed `View.layers` in core.
2. **Dim and ring are host renderer choices.** The crate supplies holes and anchor rects. The
   engine keeps generic `CellFlags::DIM` and `RING` (E3) and the `flags` field in the `cells` wire
   format (additive, proto 1) for renderers that use them; the CLI does in cells, and draws a veil
   and ring images in pixels.
3. **Traces:** record ops (`Msg::Ext { key, op }`); each segment's `state` line carries the `ext`
   values at its start.
4. **Keys:** F2 next, ⇧F2 back, F3 stop, ⇧F3 more, F4 dismiss, ⇧F4 toggle; remappable, plus
   chips. Capture steps use Enter, ⇧Tab and Esc. macOS laptop keyboards need `fn` unless the
   standard-function-keys setting is on.
5. **Frame passes** run in every `render`, so agents and snapshots see hints.
6. **Agent policy is the host's.** The crate imposes none (`Limits::default()`); 7.5's values
   (cap 3, 8 s TTL with 60 s maximum, 2 pushes per second, `agent_dim = never`) are the opt-in
   `Limits::agent_defaults()`, no longer crate defaults. Hint tools work under `--read-only`;
   `--no-hints` turns them off.
7. **`demo guide`** replaces `demo tour` (the alias is kept); the status-bar hint becomes the
   narrow-mode strip.
8. **No autostart:** the guide starts only on request.
9. **Sub-cell glyphs** are opt-in in the CLI; pixels cover smoothness in Ghostty. Octants stay
   opt-in.
10. **Text-range rects** come from `Frame`; off-screen anchors use `locate`.
11. **`caretline-tour`** stays a separate crate, and stays generic: steps name anchors, a content
    kind and data, and predicates.
12. **Placement, tracking and lifecycle only.** `caretline-layers` owns where a layer goes, how it
    follows the text, and when it goes away. Hosts own all styling and drawing. No raster, theme,
    glyph or role in a caretline crate.
13. **Content is `{kind, data}`**, opaque to the crate. Hosts register a renderer per kind (a pure
    `measure`, plus drawing) and list the kinds in `hello`, like commands.
14. **One conventional kind, `hint`** (`{title?, text}`), rendered by every host in its own style,
    so agents can point at things in any host. Other kinds are host-specific.
15. **The rename:** `caretline-overlay` is `caretline-layers`; the ext key is `layers`, the toggle
    command `layers.toggle`, the override `CARETLINE_LAYERS`.
16. **Kitty plumbing in the crate, behind `kitty`.** Transmit, re-place, place-then-delete swaps in
    one 2026 update, delete, content-hash ids, `t=d` zlib 6 by default and `t=t` when local and
    probed, re-probe of the cell size. Hosts hand it RGBA; the crate never rasterises.
17. **`cell_px` is a message** (E6): `Msg::Resize` gains an optional `cell_px`, so pixel output is a
    pure function of the state. The kitty memory is a renderer cache, never state.
18. **The CLI's renderers live in `caretline-cli`**, not a library. A shared renderer crate can come
    later, if a second host wants the same look; it never changes the layer model.

**Still open:**
- Whether pulses move to kitty animation once a Ghostty release ships it.
- `t=s` (shared memory), if `t=t` proves not enough locally.
- When a second host wants the CLI's look: the shape of a shared renderer crate.

## 12. As built

### 12.1 Step 1a (`caretline-layers`)

The first step built the screen-level half (mode B) as its own crate, with
the scope narrowed. Where it differs from the sections above:

- **Name and scope.** `caretline-overlay` became **`caretline-layers`**: placement, tracking
  and lifecycle only. It draws nothing: no `Theme`, glyph sets, roles, `compose` or
  `CellGrid`. `plan(&layers, &anchors, &grid, &renderers) -> Plan` returns geometry (each
  box or strip, the edge chip, the arrow's route as cells with directions, ring cells,
  spotlight holes, click regions) and the host draws it. The engine is unchanged; E1–E6 are
  step 1c.
- **Content is opaque.** The typed `Item` became layer fields: `content: {kind, data}`
  (measured by a `Renderer` the host registers per kind), `arrow: bool`, `ring`, `spotlight`.
  The conventional kind is `hint` (`{"title"?, "text"}`); `{{key:…}}` markup, key badges and
  step dots are a host's content now.
- **Dimming is a flag, not a role prefix** (decision 2): a host asks `Plan::dimmed(x,
  y)` or reads `Plan.spots`.
- **No screen-cell anchors.** `{"cells": …}` was dropped: anchors store chars, mark ids
  and host keys only.
- **Owners and bands.** `person` layers take z 30–39, above agents.
- **Routing costs.** A text cell costs 16 (not 6) and a blank cell between words 6, so an
  arrow goes round words when a blank way exists; placement adds 300 per word cell an arrow
  would cross. The arrow may leave the box anywhere within 10 cells of the anchor's middle.
  The third-bend penalty is added to the finished route, not searched.
- **Ops** are protocol-neutral: `ops::parse` and `ops::reply` serve any host's JSON
  protocol; `layer.list` parses to a listing, not an op.
- **Performance:** `plan` at 100×40, release build, `tests/bench.rs` with `BENCH_N=50000`:

  | Case | Before | After | With views and per-side measure | Budget |
  |---|---|---|---|---|
  | A hint box | 10 µs | 4.3 µs | 5.6 µs | |
  | A hint with an arrow and a ring | 94 µs | 27 µs | 26.7 µs | 30 µs |
  | A spotlight with an arrow | 154 µs | 44 µs | 36.6 µs | 60 µs |

  The last column is the least of ten runs of five rounds each, run alternately with the
  build before it (26.8, 4.2 and 42.7 µs there), as `tests/bench.rs` now reports: a shared
  machine's load moves single runs by half.

  How, with every golden and property test unchanged:
  - The routing cost of every cell is built **once per plan**, on the first arrow, and kept in
    step as layers claim boxes, chips and holes or dim cells; the box-scoring sums are built
    once too and rebuilt only when a spotlight dims. A candidate box's field is that table
    with its own box and the anchor blocked.
  - Placement routes only what scoring needs. One search back from the anchor per side
    (`ToGoal`, the least cost on from each cell, boxes and bends left out) gives every
    candidate box a lower bound on its arrow; the candidates are routed cheapest bound first,
    one whose bound can't beat the best so far isn't routed, and a route's search stops once
    it can't win. The same table is the router's heuristic, so a route expands little more
    than its own cells.
  - The winning box's route is reused, not routed again (unless the sliver rule moved the
    box).
  - The router reads each corridor cell's cost once and queues by bucket (keys are small
    integers), popping in the same order as before: least estimate, then first pushed.
- **Unchanged inputs, unchanged plan.** `plan` is a pure function of the layers, what the
  resolver answers, the grid and the measured sizes, and keeps no cache of its own. A host
  that redraws often keeps the last plan with the inputs it came from and calls `plan` only
  when one of them changed (an edit, a scroll, a resize, a layer op): the layers and grid
  compare with `==`, and the resolver's answers are the frame's (a new frame, a new plan).
- **Policy is the host's** (owner decision). `Limits::default()` restricts nothing: no agent
  cap, rate, forced or clamped TTL, size limits, and agents may spotlight, capture and touch
  any layer; refusals are for malformed input only (no anchor, nothing to show, a duplicate or
  unknown id, a bad actor name). The old values are the opt-in `Limits::agent_defaults()`,
  and the refusal tests run against it. An agent's layer is still owned by it
  (`agent:<actor>`) and kept in its z band.
- **No attribution glyph.** `apply` no longer writes `◆ <actor>` into an agent's hint title
  (styling is the host's, decision 12). `Planned.owner` and `Owner::actor()` expose who a
  layer is for; attributing agents' layers is recommended to hosts, not enforced.
- **Layers keep apart.** Every layer's anchor is resolved before any is placed, and no box,
  strip or chip covers another layer's box, chip, anchor or arrow; later arrows don't cross
  earlier ones. Edge chips on one edge slide along it to the nearest free place.
- **Docked boxes.** A box for an off-screen anchor sits flush against its edge chip (no gap:
  there's no arrow to make room for) and `Planned.dock` (`Attach { edge, offset }`) says where
  the chip meets its border, so a host joins them there. A docked layer that asked for an arrow
  reports `no_arrow: docked`: the chip itself points the way.
- **Arrows route, or say why not.** Placement ranks every box whose arrow routes above any box
  whose arrow can't (it used to charge a missing route 600, which a route crossing two words
  outbid), trying further candidates when none of the first few routes. When the facing side
  of the anchor can't be reached (a box under an anchor at the area's left edge), the arrow may
  end beside the anchor on another side, pointing at it. `Planned.no_arrow` is `docked`,
  `screen`, `no_box` or `no_way`. Every arrowed layer at an on-screen anchor of the test page
  now routes (it was 146 of the page's chars at 80×24).
- **Where an arrow attaches.** `Route.attach` gives the edge and offset of the junction; on a
  left or right edge the arrow never leaves beside the title row (the first inside the border)
  of a box taller than three rows, so a host's title has the row to itself.
- **Chips sized by the host.** `Renderer::chip(data, anchor, off)` gets the anchor that lies off
  screen and which way, so a label such as "↓ 2/11 here" can be measured to fit.
- **Each message's changes come from the engine.** `caretline::update_with_changes`
  (`update_doc_with_changes`, `Session::apply_with_changes`) returns the message's
  `ChangeSet`; `observe` and `map_anchors` map text anchors through it. The stopgap that
  diffed two texts (`changes_between`) is gone.

### 12.2 Step 1b (the kitty plumbing and the CLI's `hint` renderer)

The second step built the pixel half for one kind, `hint`, and a demo to see it. Where it
differs from the sections above:

- **The API** is the one in 8.2 (as built): `KittyState::frame(&plan, &pictures, cell_px, files)
  -> Output` instead of `Kitty::frame(&parts, &images)`. A `Picture` carries its own pixels
  (`image: Option<Image>`, `None` when `holds` says the terminal has the key), so there is no
  `missing` call before the frame; `Output.missing` reports a picture that needed pixels and had
  none. `forget` is `reset`. `Z::Below | Above` replaces a raw z: the crate numbers parts from
  -1,048,575 up below the text and from 2 up above it, in the plan's draw order.
- **What it sends.** Image ids hash the shape key and the cell size; placement ids hash (layer,
  part); a collision rehashes with a fixed salt. A move is `a=p` with the same ids; a change
  transmits, places, then deletes the old image (`a=d,d=I`); a part gone is deleted by id; a
  picture past the screen or its `clip` is placed with a source rect. The bytes are wrapped in
  a cursor save and restore. `t=d` is zlib 6 and base64 in 4096-byte chunks; `t=t` goes through
  the host's `TempFiles` writer (falling back to `t=d` for a file it couldn't write), with
  `tty-graphics-protocol` in the file's name. `clear` deletes everything placed, by id.
- **The probe** is `probe::request` (the four questions of 3.6), `cell_size_request` for a font
  change, and `scan`, a pure parser a runtime calls on its input: a reply with its length,
  `Partial`, or `No` for a key. `Probe` gathers the answers; `graphics_ok` needs the graphics
  OK and the cell size, and which terminals to trust is the host's call.
- **The CLI.** `caretline demo layers` puts one hint (an arrow, a ring and a spotlight) at a
  word of the tour. Its renderer draws in cells everywhere (and in `--snapshot` goldens) and in
  pixels where the probe allows: a panel with a soft shadow under the words, an anti-aliased
  arrow, ring and a veil with feathered holes over them, rasterised with tiny-skia. The veil is a
  quarter of a cell's pixels per cell, three times the text area's height, so a scroll re-crops
  it. The runtime probes at startup (200 ms at most, fenced by DA1), trusts Ghostty and kitty,
  stays in cells inside tmux or screen, honours `CARETLINE_LAYERS`, and asks for the cell size
  again when the window's pixels stop matching cells × cell size. Images are deleted by id on
  exit and while the keys overlay shows.
- **The cell size lives in the CLI's runtime** (`Gfx.cell_px`), from the probe and later
  `CSI 16 t` answers: E6 is phase 1c, and until then pixel output does not replay from a trace.
- **CI** checks that `caretline-layers --features kitty` pulls in no terminal, async or raster
  crate, and runs clippy on it without the `caretline` feature too.

**Measured** in Ghostty 1.3.1 (release build, `caretline demo layers` with the spotlight on;
the status bar shows each frame's bytes, rasters and time):

| Situation | Cost |
|---|---|
| First frame (panel, arrow, ring, veil) | 17.2 KB with `t=d`, 1.1 KB with `t=t`; 14 ms and 7 ms, raster included |
| A scroll step | about 300 bytes: re-places and the veil's re-crop |
| An unchanged frame (the caret moving, a repaint) | 0 bytes |
| Spotlight toggled off / on | 154 bytes (deletes) / 5.7 KB (the veil sent again) |

**Deferred:**
- **E6** (`cell_px` as a message) and with it pixel replay goldens: phase 1c.
- **Raw input parsing** runs only in demos that want pixels (`Demo::wants_pixels`); the editor
  itself still reads keys through crossterm. It moves to the runtime for every session with the
  engine hooks (1c).
- **Pulse variants** (pre-rendered rings swapped on the frame clock): not built; rings are still.
- **The `cli.guide` renderer** and `caretline demo guide`: with `caretline-tour` in 1c.
- **A `t=t` probe:** `t` switches transports by hand in the demo; nothing yet checks that the
  terminal can read the temporary files before choosing `t=t` for a local session.

### 12.3 Views, measuring per side and the ops schema

After 1b, for hosts that show one document in several views. Where it differs from the sections above:

- **Views.** A host can show one document in several views (a main editor and side panels),
  each drawn at its own offset and clipped to its own rect. Each gets a `FrameResolver` with
  a stable id (the host's names: `main`, `panel:2`), `.at(x, y)` and `.clip(rect)`; the
  focused one is marked `.focused()`, and they go in one `Chain`.
  - Cells outside a view's clip aren't visible. An anchor whose cells are all clipped away
    lies off screen the way they are (`above`/`below` with the column, `left`/`right` with
    the row); a direction the frame gives (scrolled out) is pulled inside the clip, so the
    edge chip sits by that view.
  - A text, block or caret anchor can be scoped: `{"text": {"from": 4, "to": 9}, "in":
    "panel:2"}` (`Anchor::In`, `Anchor::scoped`). It resolves only in that view, and never
    falls back to another. Screen and host anchors can't be scoped (a host key names its own
    place); an empty view, a second scope or any other key beside the target is refused by
    serde and by `apply`.
  - An unscoped anchor resolves in a defined order (`Chain`): the focused view if it shows
    it; else the first view that shows it; else which way it lies from the focused view;
    else the first direction any view gives.
  - `Resolved.view` (wire `in`) says which view answered; `Planned.anchor` and
    `ops::resolved` carry it.
  - Every view shows the same document, so an edit through any of them (the `ChangeSet` of
    `update_doc_with_changes`) maps scoped and unscoped anchors alike.
- **Measured per side.** `Renderer::measure(data, avail)` is called once per candidate side,
  with that side's room: below and above, the rows between the anchor (plus the arrow's gap)
  and the area's edge, at most the width cap wide; right and left, the columns past the gap,
  at most the cap. A renderer can give a narrow, tall box where only a narrow side is free;
  a side with no room isn't measured. A box that keeps its size whatever room it gets fits
  where it did before.
- **The least box, not the first few.** With an arrow, placement takes the least-scoring box
  of every candidate (it used to take the best of the first six by a guess, which a
  side newly in reach could crowd out). Boxes are routed a side at a time, the best guess's
  side first; a box that couldn't win even with the cheapest arrow (one blank cell per cell
  of gap) is left out, a side with none left gets no search, and a side's search covers
  only the boxes still in it. Every golden plan is unchanged.
- **Docked boxes touch their chips.** A docked box sits next to its own chip and shares part
  of its edge; `Planned.dock` names a cell of that shared edge. When chip-avoidance slid the
  chip along the edge, the box follows it; when no box can touch it, the layer is a strip.
  (It used to accept any box along the edge, the one at the area's far side winning where
  it covered less text, and clamp `dock` to its border.)
- **A schema for the ops.** `ops::schema()` is a JSON Schema (draft 2020-12) for every
  request `ops::parse` accepts (`$defs/request`) and every reply (`reply`, `resolved`,
  `list`, `error`), with `anchor` (and `in`), `layer`, `content`, `hint` and `owner`. A test
  validates the requests `parse` accepts (and fails the ones it refuses), real replies, and
  the protocol examples in 7.4 against it with a small validator for the keywords it uses
  (no new dependency).

### 12.4 As built: `caretline-tour`

The walkthrough crate, built before the engine hooks (E1–E6), so a host keeps the state
itself. It reads the format of 6.1 (`{id, version, title, kind, step[]}`; `layers` or the
single-layer shorthand; `narration`; the opaque `host`). Where it differs from 6 and 8.3:

- **`meta` (added after the first host read its files).** A walkthrough may carry an opaque
  top-level `meta` (author, dates, audience…), carried and never read, like a step's `host`.
  Host-only presentation (a line one host shows in one mode) goes in the step's `host`, not
  in `narration`, which stays `{title?, text}`.
- **No `install`, no `LayerSource`.** The host keeps a `TourState` in its own single state and
  calls `apply(&mut state, op, now_ms)` and `observe(&mut state, &host, now_ms)`. Both return
  effects in order: `Host { patch }`, `Layers { layers }` (replace the `guide` layers;
  `replace_guide` does it), `Step { tour, step, at, of, narration }` and `Ended { tour, seen }`.
  `apply` returns `Result<_, TourError>` (`invalid`, `not_found`, `not_running`, `no_back`);
  `apply_with` lets the host answer branches and `skip_if` met by an op. The engine is a
  feature (`caretline`, default), not a dependency.
- **Predicates are the host's answers.** A `TourHost` says whether an action ran, a message
  kind was applied, a host event happened, and gives its state by key; `caret_in`, `changed`,
  `folded` and `selection` are asked of it too. With the feature, `Editor` answers those from a
  caretline document, view, message, effects and `ChangeSet`, and passes the rest on.
  `effect` and `host_effect` read as `event`; `ext` reads as `state` (`{key, present?,
  match?}`); `count` goes with `msg`, `command` and `event`, and counts since the step began,
  so `all = [{command = a}, {command = b}]` holds once both ran. A predicate's anchor name is
  `"anchor"` (the first layer's) or a layer id.
- **`find` is resolved once, by the host, before `tour.start`** (`Tour::resolve_finds`, and
  `find_in` for a caretline document), not by the reducer on entering a step: the started tour
  and its trace hold stable anchors. A `find` never resolved is left out of its layer's
  fallbacks; `check` warns of a `find` with no `in`.
- **Placement fields are the layer's.** `place` is `{sides, max_width, arrow, ring, spotlight,
  hide_off_screen}` (`max_w` and `connector` are read as `max_width` and `arrow`; `ring` and
  `spotlight` take `true` or their object) or a list of sides. A screen position is the
  `{screen}` anchor.
- **`at` and `of`** are added only to data of kinds other than `hint`, whose data stays
  `{title?, text}`.
- **Seen-state** is `TourState.seen` (`{version, end: finished | stopped}`), and an end emits
  `Ended` for the host to persist (not `Effect::Host { "tour.seen" }`). `TourState::offer` is
  the 6.3 policy for a host's prompt; an explicit start is never refused for it.
- **Moving.** Going forward (start, restart, `next`, `advance`) follows `next` branches and
  passes over steps whose `skip_if` holds; `to` and `back` enter their step exactly. `back`
  returns to the step left last (skipped steps aren't in the history). `to` also works after a
  walkthrough ended, entering that step of the last one. A nudge is given once per visit.
- **Ops.** `tour.start {tour}` takes the whole tour or the id of one in the host's library
  (`id` is the host's request id); `tour.step {to: id | "next" | "back"}`, `tour.restart`,
  `tour.stop`, `tour.list`, and `ops::schema()`. Step ids `next`, `back` and `end` are
  reserved.
- **Checking at several sizes.** `plan_steps(&tour, sizes, &renderers, scene)` plans every
  step with the host's scene per size (its grid and anchor resolver), and reports anchors
  found nowhere, kinds with no renderer, arrows with no way, anchors off screen, `find`s
  never resolved and layers the model refuses.
- **Approximations in `Editor`.** A block runs from its mark to the next. A started tour's
  text anchors aren't mapped through edits (the pushed layers are, by `caretline-layers`'
  `observe`); block anchors need nothing.
- **Still to come** on the engine hooks (built since, 12.5): `ext["tour"]` in the view (E1),
  `Msg::Ext` and an ext reducer so a trace records tour ops (E2), the host catalog for `command` (E5;
  `Editor` matches the engine's catalog), capture routing, and `caretline demo guide`.

### 12.5 The engine hooks (E1–E6)

Built in the engine in phase 1c, part 1, with `tests/hooks.rs`; documented for hosts in
[embedding.md](../embedding.md#extending-the-engine), [messages.md](../messages.md) and
[protocol.md](../protocol.md#host-ops-and-catalog-entries). Where they differ from 1.3 and 8.1:

- **`cell_px` lives in `View`, not `Viewport`** (E6). `Viewport` stays `{width, height}`, so
  every struct literal of it still builds; the view keeps `cell_px` (`"cell_px"` at the state's
  top level, left out while unknown) and every frame copies it. A resize without `cell_px`
  keeps the view's, so a runtime that learns the pixels later sends them then.
  `Msg::resize(width, height)` builds a resize without them.
- **`render_skipping(doc, view, skip)`** takes no `host`: the passes are the document's host's,
  as for every other extension.
- **`Msg` is `#[non_exhaustive]`** (breaking, once): a `match` outside the crate ends with a
  `_` arm, so `Msg::Ext`, `Msg::Drag` and later kinds aren't breaking. The variants' fields are
  not non-exhaustive (hosts build them with struct literals); a field added to a variant is
  still breaking and takes `#[serde(default)]`.
- **`Host::ext(key, ExtFns)`**, with `ExtFns::new(apply).with_observe(observe)`, instead of two
  closure arguments. `Observed` also says whether the message went through the observed view
  (`acting`). `ExtOut`, `ExtFns`, `OpFns` and `Observed` are `#[non_exhaustive]` with
  constructors and `with_` setters.
- **Host ops** (E5) run their messages through the request's `view` (0 when absent), after
  `if_rev` and `now_ms` as for `msgs`, and record them, so a trace holds only messages. A
  refusal is the new `op_failed` error; `hello` lists the host's ops in `ops` and `host_ops` and
  its catalog in `catalog`. A request never reaches a host op named like one of the protocol's.
- **A view the protocol opens for a client starts with no `ext` values**: they are the
  person's view's (what it shows them).
- **Cost.** With no frame pass registered rendering costs the same (100×40: 143.7 µs before,
  143.9 µs after); with no observer, `update` composes no changes it wasn't asked for.

**Next: mode A in `caretline-layers`.** The hooks are generic and nothing uses them in a host
yet. The next step is `caretline-layers`' `install(host)` adapter (in-frame mode, 2.5 and 7.1):
the layer set in `View::ext["layers"]` with a reducer that applies `LayerOp`s and maps anchors
through each message's changes, a frame pass that draws the `hint` kind in cells,
`FrameResolver` over `view::locate`, region hits through `Frame::region_at`, and the `layer.*`
and `hint.*` ops through `Host::op`. Then `caretline-tour` keeps `TourState` in
`View::ext["tour"]` the same way (12.4), and the CLI's editor and `demo guide` adopt both.

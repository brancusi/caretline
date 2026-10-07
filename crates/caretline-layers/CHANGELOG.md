# Changelog

## Unreleased

### Added

- The crate: placement, tracking and lifecycle for layers (hints, callouts, arrows, rings,
  spotlights) over a caretline host's screen. The host draws them.
- `Layers`, `Layer`, `LayerOp` and `apply(layers, op, actor, now_ms, &Limits)`: serializable
  state changed only by ops; `expire` and `observe` for time and edits. No clock, randomness
  or I/O.
- Policy is the host's: `Limits::default()` (also `Limits::none()`) restricts nothing, and
  `apply` then refuses only malformed input (no anchor, nothing to show, a duplicate or
  unknown id, a bad actor name). `Limits::agent_defaults()` is an opt-in policy with the
  design's values: 3 agent layers per view (a fourth replaces that actor's oldest), a TTL of
  8 s clamped to 1–60 s, 2 pushes a second per actor by `now_ms`, no spotlight
  (`AgentDim::Never`), no capture (`no_agent_capture`), size limits, and only an agent's own
  layers (`own_layers_only`). Each limit is an `Option` a host sets alone. Refusals carry a
  stable `Reason`. An agent's layer is always owned by it (`agent:<actor>`), in its z band.
- No attribution is written into content: `Planned.owner` and `Owner::actor()` expose who a
  layer is for, and how to show it is the host's (recommended for agents' layers).
- `Content { kind, data }`: opaque content a host renders, with one conventional kind,
  `hint` (`{"title"?, "text"}`).
- Anchors: text ranges, text within a block, blocks, the caret, a screen position and host
  kinds (`{"host": {"kind": "row", "key": "…"}}`); never screen cells. `AnchorMap` for a
  host's own anchors, `FrameResolver` for a caretline frame (feature `caretline`, default),
  `Chain` for both. `map_anchors` and `observe` move text anchors through the `ChangeSet` an
  editor message made, as caretline's `update_with_changes` (and `update_doc_with_changes`,
  `Session::apply_with_changes`) returns it. Needs the caretline release that adds them.
- `plan(layers, anchors, grid, renderers) -> Plan`: boxes placed by side order with flip,
  shift, the sliver rule and clamp, around other layers, holes, protected cells and (for
  agents) the caret, never splitting a wide grapheme; a one-row strip on narrow areas or when
  nothing fits; edge chips for off-screen anchors; arrow routes as cells (A*, round words);
  ring cells; spotlight holes; click regions and `Plan::hit`. Pure and serializable.
  At 100×40 (release), a box costs about 6 µs, a box with its arrow about 27 µs, and a
  spotlight with an arrow about 37 µs (with per-side measuring, below): routing costs are
  built once per plan, candidate boxes are routed only when a lower bound says they can win,
  and the winner's route is reused.
- Layers placed together keep apart: no box, chip or strip covers another layer's box, chip,
  anchor or arrow (every layer's anchor is known before any box is placed), and no arrow
  runs under a box; chips on one edge slide along it to a free place.
- A box for an off-screen anchor docks flush against its edge chip; `Planned.dock` says where
  on the box's border the chip touches it.
- Arrows: placement prefers any box whose arrow routes over one whose arrow can't; when the
  facing side of the anchor can't be reached the arrow may end beside it on another side,
  pointing at it; `Planned.no_arrow` (`docked`, `screen`, `no_box`, `no_way`) says why a
  layer that asked for an arrow has none. `Route.attach` (`Attach { edge, offset }`) says
  where the arrow leaves the box's border, and on a left or right edge it never leaves
  beside the title row.
- `Renderer` and `Renderers`: a host's measure per content kind. `Renderer::chip(data,
  anchor, off)` sizes the edge chip knowing which anchor lies off screen and which way.
- `ops`: protocol-neutral `hint.show`, `hint.hide`, `layer.push`, `layer.update`,
  `layer.pop` and `layer.list` requests (`parse`) and replies (`reply`, `list`, `error`).
- Feature `kitty` (off by default; adds only `miniz_oxide`): pixel plumbing for the kitty
  graphics protocol. The host rasterises one straight-alpha RGBA `Image` per layer part and
  hands over `Picture`s (layer, part, shape key, `Z::Below` or `Z::Above` the text, the cells
  it covers, an optional clip); `KittyState::frame(&plan, &pictures, cell_px, files)` returns
  the APC bytes to write after the text frame inside the same synchronized update: it
  transmits what the terminal lacks (`t=d`, zlib level 6, base64 in 4096-byte chunks; or
  `t=t` through the host's `TempFiles` writer), re-places what moved with the same ids,
  places a changed part before deleting its old image (`a=d,d=I`), deletes what's gone by id
  (never `d=A`), and crops a picture that reaches past the screen or its clip with a source
  rect. Image ids hash the shape key and the cell size, placement ids the (layer, part),
  never counters: the same frames give the same bytes. `KittyState` is a renderer cache,
  never layer state: `holds` says which pictures need no pixels, `clear` deletes everything
  placed, `reset` forgets it (one full re-send). The cell size is a value the host passes; the
  crate never asks the terminal.
- `probe` (feature `kitty`): the probe's bytes (`request`: a graphics query, XTVERSION,
  `CSI 16 t`, DA1 as the fence; `cell_size_request`) and a pure parser for the replies
  (`scan` → `Reply::{Graphics, Version, CellSize, WindowSize, Da1}`, or `Partial`, or `No`
  for keys), so a runtime turns them into messages instead of keys. `Probe` gathers them.
- View-scoped anchors, for one document shown in several views. `FrameResolver` gains
  `.id("panel:2")`, `.clip(rect)` (cells outside it aren't visible; an anchor clipped away
  lies off screen the way its cells are, with its column or row) and `.focused()`. A text,
  block or caret anchor can name its view: `{"text": {"from": 4, "to": 9}, "in": "panel:2"}`
  (`Anchor::In { view, anchor }`, built with `Anchor::scoped`); it resolves only there.
  `in` on a screen or host anchor, an empty view or a second scope is refused by serde and
  by `apply`. `Resolved.view` (wire `in`) and so `Planned.anchor` and `ops::resolved` say
  which view an anchor resolved in. `map_anchors` maps scoped anchors too, whichever view
  the edit came through.
- `Resolve::is_focused` (default `false`).
- `ops::schema() -> serde_json::Value`: a JSON Schema (draft 2020-12) for every request
  `ops::parse` accepts (`$defs/request`, one per op, with `op`, `id`, `view` and `actor`)
  and every reply (`$defs/reply`, `resolved`, `list`, `error`), with `anchor` (and its
  `in`), `layer`, `content`, `hint` and `owner`. A test checks the ops tests' requests (those
  `parse` refuses must fail it too), real replies, and the design's protocol examples
  against it.
- `Renderer::measure(data, avail)` is called once per candidate side with that side's real
  room (below and above: the rows past the arrow's gap, at most the width cap; right and
  left: the columns past the gap, at most the cap), so a renderer can return a narrow, tall
  box for a narrow side. A side with no room isn't measured.

### Changed (breaking within 0.1.0's development)

- `Anchor` has a new variant, `In`: exhaustive matches need an arm. `Resolved` has a new
  public field, `view`: struct literals need it (or use `Resolved::at` / `Resolved::off`).
- `Chain` no longer takes the first answer: of its resolvers' answers it takes the focused
  one's if its cells show, else the first whose cells show, else the focused one's
  off-screen direction, else the first. A resolver that answered "off screen" no longer
  hides a later one that shows the anchor.
- Placement with an arrow picks the least-scoring box of every candidate, not of the first
  few that route: the result no longer depends on how a first guess ranked them. Boxes are
  routed a side at a time, the best guess's side first; a side none of whose boxes could
  win even with the cheapest arrow (one blank cell per cell of gap) gets no search at all,
  and the search for a side covers only the boxes that could. Every golden plan is
  unchanged. At 100×40 (release, `tests/bench.rs`, the least of ten runs of five rounds,
  alternating with the previous build): a box with its arrow 26.7 µs (was 26.8), a
  spotlight with an arrow 36.6 µs (was 42.7), a box alone 5.6 µs (was 4.2: four measures,
  one per side, with the test host's measure).

### Fixed

- A docked box (its anchor off screen) now always touches its own edge chip: it sits next
  to the chip and shares part of its edge, and `Planned.dock` names a cell of that shared
  edge. It used to take any candidate along the edge (one at the area's far side won where
  it covered less text, or one a few rows away), and `dock` was clamped to the box's
  border, touching nothing. When no box can touch the chip, the layer is a strip.
- `Planned.owner`'s documentation said a host must attribute agents' layers; attributing
  them is recommended, and the host's choice.

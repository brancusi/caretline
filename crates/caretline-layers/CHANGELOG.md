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
  At 100×40 (release), a box costs about 4 µs, a box with its arrow about 27 µs, and a
  spotlight with an arrow about 44 µs: routing costs are built once per plan, candidate boxes
  are routed only when a lower bound says they can win, and the winner's route is reused.
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

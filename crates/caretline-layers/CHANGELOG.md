# Changelog

## Unreleased

### Added

- The crate: placement, tracking and lifecycle for layers (hints, callouts, arrows, rings,
  spotlights) over a caretline host's screen. The host draws them.
- `Layers`, `Layer`, `LayerOp` and `apply(layers, op, actor, now_ms, &Limits)`: serializable
  state changed only by ops; `expire` and `observe` for time and edits. No clock, randomness
  or I/O.
- Agent policy in `apply`: at most 3 agent layers per view (a host may lower it, never past
  8; a fourth replaces that actor's oldest), a TTL of 8 s clamped to 1–60 s, 2 pushes a second
  per actor by `now_ms`, no spotlight without consent (`AgentDim`), no capture, size limits,
  `◆ <actor>` titles on hints, z bands per owner. Refusals carry a stable `Reason`.
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
- `Renderer` and `Renderers`: a host's measure per content kind.
- `ops`: protocol-neutral `hint.show`, `hint.hide`, `layer.push`, `layer.update`,
  `layer.pop` and `layer.list` requests (`parse`) and replies (`reply`, `list`, `error`).

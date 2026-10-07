# caretline-layers

Placement, tracking and lifecycle for layers drawn over a [caretline](../../docs/README.md)
host's screen: hints, callouts, arrows, rings and spotlights. The crate decides where things
go and keeps them pointing at the right text as it scrolls, wraps and changes; the host draws
them.

- **Layers are data.** `apply(&mut layers, op, actor, now_ms, &limits)` is the only way they
  change, so a trace of ops replays exactly. Agents are held to limits (a cap, expiry, a rate,
  no dimming or focus stealing, attribution).
- **Anchors are stable keys**: chars, block ids, the caret, or a host's own keys (a table row
  by id, a diff line). They resolve to cells every frame, from a caretline frame
  (`FrameResolver`) or what the host recorded while drawing (`AnchorMap`).
- **Content is the host's**: `{kind, data}`, measured by a `Renderer` the host registers.
  Every host should render the `hint` kind (`{"title"?, "text"}`).
- **Placement is pure**: `plan(&layers, &anchors, &grid, &renderers)` returns where each
  box, strip, edge chip, arrow, ring and spotlight hole goes, and the click regions.
- **`ops`** parses `hint.show`, `layer.push` and friends from any JSON protocol.

`cargo run -p caretline-layers --example layers_ratatui` is a ratatui host that takes an
agent's `hint.show`, plans it, and draws the hint itself.

Pure: no clock, randomness, I/O, terminal or async. The `caretline` feature (default) adds
`FrameResolver` and anchor mapping through edits.

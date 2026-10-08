# caretline-layers

Placement, tracking and lifecycle for layers drawn over a [caretline](../../docs/README.md)
host's screen: hints, callouts, arrows, rings and spotlights. The crate decides where things
go and keeps them pointing at the right text as it scrolls, wraps and changes; the host draws
them.

- **Layers are data.** `apply(&mut layers, op, actor, now_ms, &limits)` is the only way they
  change, so a trace of ops replays exactly.
- **Policy is the host's.** The crate restricts nothing: with `Limits::default()` an agent may
  do what the person and the host can, and only malformed input is refused. A host that wants
  limits sets them, or starts from `Limits::agent_defaults()` (a cap of 3 per view, 8 s
  lifetimes, 2 pushes a second, size limits, no dimming or capture, only its own layers).
  Each placed layer carries its `owner` (`agent:<actor>`); attributing an agent's layers so
  they can't pass as the host's own is recommended, in the host's own style.
- **Anchors are stable keys**: chars, block ids, the caret, or a host's own keys (a table row
  by id, a diff line). They resolve to cells every frame, from a caretline frame
  (`FrameResolver`) or what the host recorded while drawing (`AnchorMap`).
- **Views**: one document in several views gets a `FrameResolver` per view, each with an
  id, offset and clip, the focused one marked. An anchor scoped to a view (`"in":
  "panel:2"`) resolves only there; an unscoped one in the focused view first, and the answer
  says which view it came from.
- **Content is the host's**: `{kind, data}`, measured by a `Renderer` the host registers,
  once per candidate side with that side's room and the layer's owner (`MeasureCtx`), so an
  agent's attribution can be sized into its box.
  Every host should render the `hint` kind (`{"title"?, "text"}`).
- **Placement is pure**: `plan(&layers, &anchors, &grid, &renderers)` returns where each
  box, strip, edge chip, arrow, ring and spotlight hole goes, and the click regions. Layers
  keep off each other's boxes, chips, anchors and arrows; an arrow says where it attaches to
  its box, or why there is none (`no_arrow`), and its head is never on text; a box for an
  off-screen anchor docks against its edge chip and touches it (`dock`).
- **Edits come from the engine**: `caretline::update_with_changes` returns each message's
  `ChangeSet`, and `observe` maps text anchors through it: all of them for a host with one
  document (`Edited::All`), or only those in the views that show the edited one
  (`Edited::Views`).
- **`ops`** parses `hint.show`, `layer.push` and friends from any JSON protocol, and
  `ops::schema()` is a JSON Schema for its requests and replies.
- **Pixels** (feature `kitty`, off by default): `kitty::KittyState` turns a plan and the
  host's own RGBA images into kitty graphics bytes (transmit, re-place on scroll, swap, crop,
  delete by id), and `probe` builds the terminal probe and parses its replies. The host
  rasterises, writes the bytes and reads the replies. Targets Ghostty.

`cargo run -p caretline-layers --example layers_ratatui` is a ratatui host that takes an
agent's `hint.show`, plans it, and draws the hint itself.

Pure: no clock, randomness, I/O, terminal or async. The `caretline` feature (default) adds
`FrameResolver` and anchor mapping through edits; `kitty` adds only `miniz_oxide`.

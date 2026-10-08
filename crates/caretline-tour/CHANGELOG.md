# Changelog

## Unreleased (0.1.0)

### Added

- `StepLayer.avoid` (and `avoid` as a single-layer field on a step): anchors the layer's box
  and arrow keep off (caretline-layers' `Layer.avoid`), one or a list, `find` included
  (`Tour::resolve_finds` resolves them); in `ops::schema()`. Left out of JSON when empty.
- A step layer with no `kind` and no `data` has no content: a ring or a spotlight alone, no
  box (`Layer.content` is `None`, as caretline-layers models one). `check` no longer reports
  `hint_data` for it, and a nudge doesn't give it content.
- `Tour.meta`: the host's own metadata about a walkthrough (author, dates, audience…),
  opaque like a step's `host`; left out when absent. Other unknown top-level fields are
  still refused.
- The crate: walkthroughs over a caretline host's screen, as data. Pure: no clock,
  randomness, I/O, terminal or async.
- The format: `Tour { id, version, title, kind, reoffer, steps, meta }` (wire `step`, TOML's
  `[[step]]`), `Step { id, layers, narration, host, advance, skip_if, nudge, next }`,
  `StepLayer { id, anchor, kind, data, place, capture, avoid }`, `Narration { title?, text }`,
  `Branch { if?, goto }`, `Nudge { after_ms, data }`, `Place`. `parse_toml` and `parse_json`
  are strict: unknown fields are refused everywhere but the opaque `data` and `host`. The
  single-layer shorthand on a step (`anchor`, `kind`, `data`, `place`, `capture`) reads as
  `layers[0]`, and both together is an error; a layer's id defaults to `<step id>/<index>`.
  Anchors are `caretline_layers::Anchor`s, one or a list, or `{find, in?}`, resolved once by
  the host with `Tour::resolve_finds`. `place` is an object (`sides`, `max_width`, `arrow`,
  `ring`, `spotlight`, `hide_off_screen`; `max_w` and `connector` are read too) or a list of
  sides.
- `check(&Tour) -> Vec<Problem>`: errors (no steps, duplicate step or layer ids, reserved ids,
  an unknown `goto`, an anchor a predicate names that isn't there, malformed `hint` data) and
  warnings (empty steps, an unscoped `find`, a nudge with nothing to merge into, `count = 0`,
  an unreachable branch).
- The reducer: `TourState { tour, step, since_ms, counts, history, nudged, seen }`,
  `TourOp` (`Start`, `Stop`, `Next`, `Back`, `To`, `Restart`) and
  `apply(&mut TourState, op, now_ms) -> Result<Vec<TourEffect>, TourError>` (`apply_with` to
  let a host answer branches and `skip_if`). `TourEffect`: `Host { patch }`,
  `Layers { layers }`, `Step { tour, step, at, of, narration }`, `Ended { tour, seen }`.
  Entering a step is exact: the same effects however it was reached. `TourState::offer` says
  whether to offer a walkthrough given what was seen and `reoffer`.
- Predicates: `Pred` (`msg`, `command`, `event` with `count`; `state` subset tests; `caret_in`,
  `selection`, `changed`, `folded`; `after_ms`; `any`, `all`, `not`) answered by the host's
  `TourHost`; `observe(&mut TourState, &dyn TourHost, now_ms)` counts what happened, gives a
  step's nudge, and moves on when `advance` holds.
- Feature `caretline` (default): `Editor` answers the editor's predicates from a caretline
  `Document`, `View`, message, effects and `ChangeSet`, and passes the rest to the host;
  `find_in` resolves a `find` text in a document.
- Layers: `step_layers` (owner `guide`, z `GUIDE_Z`; `at` and `of` added to non-`hint` data),
  `replace_guide`, and `plan_steps`, which plans every step at several sizes with the host's
  resolver and renderers and reports what didn't work.
- `ops::parse`, `ops::reply`, `ops::list`, `ops::error` and `ops::schema()` for `tour.start`,
  `tour.step {to}`, `tour.restart`, `tour.stop` and `tour.list`.

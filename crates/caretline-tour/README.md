# caretline-tour

Walkthroughs over a [caretline](../../docs/README.md) host's screen. A walkthrough is a list
of steps; each step sets the host's scene (`host`, an opaque patch), points at things in it
(`layers`, placed by [`caretline-layers`](../caretline-layers)) and says what it is about
(`narration`). The crate keeps the progress as data and tells the host what to do; the host
keeps the state and draws.

- **A strict format.** `parse_toml` and `parse_json` read `{id, version, title, kind,
  step[]}`. A step has `layers`, or the single-layer shorthand (`anchor`, `kind`, `data`,
  `place`, `capture`) that reads as `layers[0]`; a layer's id defaults to
  `<step id>/<index>`. Unknown fields are refused everywhere but `data` and `host`. `check`
  lints what parses but can't work (duplicate ids, an unknown `goto`, empty steps, an
  unscoped `find`, …).
- **A pure reducer.** `apply(&mut state, op, now_ms)` with `TourOp` (start, stop, next,
  back, `to`, restart) returns `TourEffect`s: apply this `host` patch, replace the guide
  layers with these, the step changed (with its narration), the walkthrough ended (seen-state
  for the host to persist). Entering a step yields the same effects however it was reached,
  so a jump looks the same as arriving in order, and an op log replays to an equal state.
- **Predicates the host answers.** `advance`, `skip_if` and `next` branches ask a
  `TourHost` whether an action ran, a host event happened or a state subset matches, and
  check `after_ms`, with `any`, `all` and `not`. `observe(&mut state, &host, now_ms)` moves
  on when `advance` holds. Steps without predicates move only by ops: that is how pages are
  turned.
- **Editor predicates** (feature `caretline`, default): `Editor` answers message kinds and
  counts, commands, `caret_in`, `changed`, `folded` and `selection` from caretline types, and
  `find_in` resolves `find` anchors in a document.
- **Layers.** `step_layers` turns a step into `caretline_layers::Layer`s owned by `guide`,
  `replace_guide` applies them, and `plan_steps` plans every step at several sizes with the
  host's resolver and renderers, for hosts that check walkthroughs at several terminal sizes.
- **Ops.** `ops::parse` and `ops::reply` for `tour.start`, `tour.step {to}`,
  `tour.restart`, `tour.stop` and `tour.list` in a host's own protocol, and `ops::schema()`.

[`examples/tour.toml`](examples/tour.toml) is a four-step walkthrough over a table and an
editor.
The guide, with the format, the reducer's effects, predicates, `plan_steps` and the ops, is
[docs/tour.md](../../docs/tour.md).

Pure: no clock, randomness, I/O, terminal or async. Depends on serde, serde_json, toml and
caretline-layers (and caretline with the `caretline` feature).

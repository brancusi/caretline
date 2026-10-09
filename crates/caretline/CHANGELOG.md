# Changelog

## Unreleased

## 0.4.0 — 2026-10-08

### Demo

- `caretline demo showcase` (also `scripts/demo.sh`): twelve timed, interactive slides
  introducing the engine through real edits, Unicode, multiple carets, overlays with several
  highlights, shared views, guarded remote writes, folds and stable marks. Arrow keys or
  numbers select slides, Space pauses, and the local socket accepts updates and subscriptions.
  `--headless` runs 13 actual invariant checks; snapshot key scripts advance the same timeline
  without sleeping. `--seconds` sets the slide duration; `--socket` selects the listener.
- The showcase now follows the public brand's starlight/void/selection palette, sharp
  neutral panels and rings without glow. Its typography gallery rasterizes bundled,
  SIL-OFL-licensed Geist and Geist Mono outlines at three sizes; it does not change the
  terminal font. Existing ASCII scenes lead into fractional-device-pixel motion and the
  original warp-logo finale. `End`/`0` jumps to that finale; Space pauses it, and
  `--reduced-motion` freezes continuous motion. Terminal pastes cannot corrupt the deck;
  socket edits remain allowed. Continuous frame history is bounded to 512 trace lines.
- Layer demos share one pixel/cell renderer. Switching transport now queues image cleanup,
  so a rapid transport-then-cells switch or quit cannot leave forgotten images on screen.
- The layers demo explains its overlay controls inside the hint and keeps the render mode,
  quit, scroll, spotlight, mode and transport keys visible at 80 columns, ahead of pixel
  diagnostics. The Quickstart includes a build-from-checkout overlay walkthrough.

### Added

- Companion crates `caretline-layers` (placement, tracking, pixel transport and host
  conformance checks for overlays) and `caretline-tour` (walkthroughs as data, reducers,
  predicates and protocol ops). Both stay pure; hosts keep their state and draw.
- `OutlineConfig::nest_joins` (`with_nest_joins`, `"nest_joins": true` in JSON, left out while
  false): Tab closes the blank row above every block it nests directly under the block above
  it (that block becomes its parent), in the same undo step; Shift-Tab adds none back, so
  outdenting moves nothing vertically. Off by default: Tab and Shift-Tab keep every blank row,
  as before.

- `Edit::then_default` (and the constructor `Edit::then_default()`): an input rule's edit that
  adjusts what the engine does with the message rather than replacing it. The rule's changes,
  selection and marks are applied, then the engine handles the message as it would have (an
  outline's Enter splits the block with the same marker, number and tag; a key types), and the
  two are one undo step that restores the selection from before the message. A rule that
  wants Enter to drop the spaces after the caret deletes them and lets the engine split,
  instead of reimplementing the split. Left out of JSON while false; a command ignores it.
- `update_with_changes(&mut State, Msg) -> (Vec<Effect>, Option<ChangeSet>)`,
  `update_doc_with_changes(doc, views, acting, msg)` and `Session::apply_with_changes` /
  `Session::apply_on_with_changes`: what `update`, `update_doc` and `Session::apply` do, also
  returning the message's text changes as one `ChangeSet` from the text before it to the text
  after (typing, paste, undo and redo, host commands, input rules and `Msg::External`; several
  edits in one message composed). `None` when the text didn't change. A host maps positions
  of its own through it with `ChangeSet::map_pos` and an `Assoc`, both now also re-exported
  at the crate root. Nothing is kept in the state.
- `Layout::click_at`: where a click puts the caret.
- `ViewConfig::page_overlap`: rows of the previous screen a page motion keeps on screen (a page
  moves the text rows less this). Default 0, as before.
- `ViewConfig::scroll_past_end` (`with_scroll_past_end`) and `ScrollPastEnd`: how far the view
  may scroll past the document's last row, like an editor's "scroll beyond last line".
  `Off` (the default, as before), `Margin` (as many empty rows as `scrolloff`, so the caret
  keeps its margin while writing at the end of a long page), `Rows(n)` or `Half`. Following
  the caret, page motions, `Scroll` and `ScrollView` go that far past the end and no further;
  an edit that leaves the caret in place still never pulls the view back, a click still keeps
  it, and `Follow::Typewriter` is unchanged. In JSON `"scroll_past_end": "margin"`, `"half"`
  or `{"rows": 3}`, left out while off (`ConfigInput::scroll_past_end` reads it).
  `ScrollPastEnd::rows(h, scrolloff)` gives the count for a view.
- `Config::single_line` (`"config": {"single_line": true}`, left out of JSON while false): a
  one-line text field. The text never holds a line break: `InsertNewline` and `SoftBreak`
  change nothing; line breaks typed, pasted, edited in by a host or an input rule, or put in
  from elsewhere (`Msg::External`, `text.set`) are flattened where they land: each run (`\r\n`
  counted once) becomes one space, or nothing at the start or end of the line or next to
  whitespace, so a break never joins two words or doubles a space; lines never wrap; motion by
  a line, row or page goes to the start or the end. `State::sanitize` flattens line breaks
  already in the text and starts the history again when its undo or redo could bring one
  back. Ignored (and cleared) in an outline document. `Document::single_line()` says whether
  it applies.
- `with_` setters on `Config`, `ViewConfig`, `OutlineConfig` and `OutlineLayout`, one per field
  (`OutlineLayout::default().with_hang_glyphs(true)`).
- `History::transactions`: every revision's transaction and inversion.
- **View values** (`View::ext: BTreeMap<String, serde_json::Value>`): a host's own state per
  view, by key, the view-level twin of a mark's payload. Serialized at the state's top level
  as `"ext"` (left out when empty), so it goes through `state.get`/`state.set`, `view.open`,
  traces' `state` and `view_open` lines and replay. The engine never reads it.
- **Ext reducers**: `Host::ext(key, ExtFns::new(apply).with_observe(observe))` and
  `Msg::Ext { key, op }` (`{"msg":"ext","key":"…","op":…}`). `apply(ctx, current, op) ->
  Result<ExtOut, String>` runs for `Msg::Ext` on the acting view; `observe(ctx, value,
  &Observed { msg, effects, changes, acting }) -> Option<ExtOut>` runs after every message on
  each view whose `ext` holds the key, with the message's composed `ChangeSet` (the one
  `update_with_changes` returns). `ExtOut` (`value`, `effects`, `status`, `frame_clock`;
  `ExtOut::value(v)`, `ExtOut::remove()` and `with_` setters) sets or removes the value, emits
  `Effect::Host`s, sets the status and the frame clock. `Msg::Ext` is passive (it doesn't end
  a typing run or clear the status) and is accepted on read-only views; with no reducer for
  the key the status says so. Traces record it like any message and replay it with
  `trace::replay_trace_with(input, &host)`. `Host::ext_keys` lists the keys.
- **The cell pixel size is a message**: `CellPx { w, h }` (device pixels), `Msg::Resize`'s
  optional `cell_px` (`{"msg":"resize","width":80,"height":24,"cell_px":{"w":8,"h":16}}`), kept
  in `View::cell_px` (`"cell_px"` in the state's JSON, left out while unknown) and copied to
  `Frame::cell_px`. A resize without it keeps the view's. Whatever a host draws in pixels is
  then a function of the state, and replays the same on any machine. The `caretline` binary
  sends the size its terminal probe finds (and a later `CSI 16 t` answer) this way.
- `Msg::resize(width, height)`: a resize that keeps the cell pixel size.
- **Frame passes**: `Host::frame_pass(name, |ctx, frame| …)` draws over every frame
  `view::render` (and `view`, snapshots, the protocol's `render`) makes, in registration order;
  a pass registered again under its name is replaced in place. `view::render_plain` draws
  without any, `view::render_skipping(doc, view, &["name"])` without the named ones.
  `Host::frame_pass_names` lists them. With none registered rendering costs the same (100×40:
  143.7 µs before, 143.9 µs after).
- Grapheme-safe writers on `Frame` for passes: `Frame::new(w, h)` (now public),
  `set(x, y, grapheme, role) -> u16` (the cells written; writing over either half of a wide
  grapheme blanks its other half, a wide grapheme that doesn't fit is drawn as a space),
  `restyle(x, y, role)`, `flag(x, y, CellFlags)` and `role(name) -> Role` (a host's style
  name, as decorations use). `Frame::regions: Vec<Region { x, y, w, h, id }>` and
  `Frame::region_at(x, y)` (the last added wins) for what a pass makes clickable.
- `CellFlags` (`DIM`, `RING`, one byte) on every `Cell` (`Cell::flags`): marks a pass sets for
  its renderer. The engine never sets or reads them. The protocol's `cells` rows gain an
  optional `flags` list of runs, `[x, len, "dim" | "ring" | "dim ring"]` (left out when empty;
  additive, proto 1), and `Frame::to_ansi` draws them faint and underlined.
- `view::locate(doc, view, pos) -> Locate`: where a char position is on screen, the inverse
  of `view::hit` through the same layout: `At { x, y }`, or which way it lies, `Above`,
  `Below`, `Left`, `Right` (a line that doesn't wrap) or `Folded { block }`. Serialized as
  `{"kind": "at", "x": …, "y": …}`.
- `Cell`, `CellFlags`, `Region` and `Role` are re-exported at the crate root.
- **Host catalog entries**: `Host::catalog(vec![HostCommandInfo::new(id, name, msg)
  .with_description(…).with_category(…).with_keys(vec!["<f2>".into()])])`. The protocol lists
  them with `"source": "host"`: `hello` in a new `catalog` field, `commands.list` after the
  engine's commands, `keymap.get` one binding per key. `Host::catalog_entries` returns them.
- **Host ops**: `Host::op(name, OpFns::new(to_msgs).with_reply(reply))`. A request whose op
  isn't one of the protocol's own goes to the host's op of that name: `to_msgs(ctx, request)`
  gives the messages, applied through the request's `view` (0 when absent, after `if_rev` and
  `now_ms` as for `msgs`) and recorded in the trace; the reply is `{rev, view, msgs, effects}`,
  or `rev` and the fields `reply(ctx, frame, request)` returns. A refusal is an `op_failed`
  error; ops nobody knows are still `unknown_op`. `hello` adds the host's ops to `ops` and
  lists them in `host_ops`. `Host::op_names` returns them.
- `Msg::Drag { col, row }` (`{"msg":"drag","col":4,"row":0}`): the pointer dragged to a
  screen cell. It extends the selection as `Click` with `extend` does; on the first text row
  with text above the view it scrolls the view up one row, on the last text row (or past it)
  with text below down one row, and extends to the row brought in. The `caretline` editor
  sends its drags as this.

### Changed

- Faster row counts for soft-wrapped lines of printable ASCII (perf, no API change). A line
  whose drawn text is all U+0020 to U+007E is wrapped by word-wrap arithmetic instead of the
  grapheme formatter: `Layout::line_rows` and `text_rows_of`, the walks behind scrolling,
  paging and keeping the caret in view, and the wrap cache of long lines. The rows, row
  starts and caret places are the formatter's exactly; a line with a tab, a control
  character or anything outside ASCII is formatted as before. Counting the rows of 5,000
  wrapped outline blocks of ~75 characters at a new width went from 14 ms to 1.0 ms
  (release).

### Breaking

- `Edit` has a new public field, `then_default`: a struct literal that names every field adds
  `then_default: false` (or ends in `..Edit::default()`, as the docs' do).
- `Msg` is `#[non_exhaustive]` (as `Effect` is): a `match` on it outside the crate needs a
  wildcard arm, and new kinds of message are no longer breaking. Its variants' fields are not,
  so hosts still build messages with struct literals; a field added to a variant stays a
  breaking change (with `#[serde(default)]`, so recorded JSON still parses). This release
  adds `Msg::Ext` and `Msg::Drag`.
- `Msg::Resize` has a third field, `cell_px: Option<CellPx>`. Code that builds it writes
  `Msg::resize(width, height)` (or adds `cell_px: None`); a pattern that names its fields adds
  `..`. Its JSON is unchanged without pixels (`#[serde(default)]`, left out when `None`), so
  old traces and clients replay and parse as before.
- `Frame` has new public fields, `cell_px` and `regions`, and `Cell` one, `flags`: a struct
  literal adds `cell_px: None, regions: Vec::new()` (or starts from `Frame::new(w, h)`) and
  `flags: CellFlags::NONE`.

- `Config`, `ConfigInput`, `ViewConfig`, `OutlineConfig` and `OutlineLayout` are
  `#[non_exhaustive]`, so adding a setting is no longer a breaking change. Outside the crate
  they can't be built with a struct literal, not even with `..Default::default()`. Start from
  `Default` and chain the `with_` setters, or set the public fields on a default value:
  `OutlineConfig { tags: "ab".into(), ..OutlineConfig::default() }` becomes
  `OutlineConfig::default().with_tags("ab".into())`; for `ConfigInput` (no setters), take
  `ConfigInput::default()` and set its fields. This release also adds fields to them
  (`Config::single_line`, `ConfigInput::single_line`, `ViewConfig::page_overlap`), which broke
  struct literals anyway. This release bumps the minor version to 0.4.

### Fixed

- A blank row a host sets with `MarkOp::SetGap` is no longer reverted by the engine's gap
  pinning in the same message: an input rule's edit for Tab, Shift-Tab, typing, Backspace or
  Delete (with or without `then_default`), or a command's edit with `keep_gaps`. The gaps the
  engine kept were read before the edit and written back after it, over the host's own.
- `Document::take_touched` reports the blocks that changed, no more and no fewer. A host's
  edit (`Msg::Edit`, a host command or input rule's `Edit`) that rewrites more text than it
  changes, such as the whole text without an empty last block, used to touch every block it
  spanned and drop their marks, so their ids changed too: it is now applied as the least
  change, the way `Msg::External`'s `replace` already was, and blocks it leaves as they were
  keep their ids. On a 5,000-block page, dropping the empty last block now touches one block,
  not the page. A touched range never counts text a change puts back as it was. In an outline
  document the range is widened to whole blocks and takes in every block that changed without
  a change to its own chars, which it used to miss: a mark's blank row or payload set from
  elsewhere (`set_gap`, `set_data`) or by a pin, a new mark, the block before a line that
  started or stopped starting a block, the next block's default blank row, and the lines a
  code fence takes in or lets go. `enable_outline` touches everything.
- A click (`Msg::Click`, `view::hit`) on the right half of a wide grapheme puts the caret after
  it, not before.
- The engine no longer enables serde_json's `preserve_order` for every crate that depends on it
  (it changed `serde_json::Map`'s key order across a host's whole build). Protocol responses
  are built from structs, so their key order is the same with or without the feature; the
  engine's tests also run with `arbitrary_precision` on.
- Soft-wrapped rows never exceed the column when a wide grapheme (an emoji, a CJK character, a
  tab) meets the wrap edge: it used to overflow by one cell; it now starts the next row. This
  holds in plain and prose (outline) wrapping, for long words and word boundaries alike. Only
  a grapheme wider than the whole column still overflows, alone on its row (the view clips
  it), and spaces that hang past a prose row's end are unchanged.
- An edit that shortens the document (Backspace on an empty last line) no longer pulls the view
  up when the caret stays in it: the view keeps its top and shows empty rows below the end.
  The view is kept off them only when it moves to bring the caret into sight (a jump, a page,
  a resize that hides the caret), never back above where it was while following the caret
  down. `ensure_caret_visible` follows the same rule.
- A caret placed by the pointer (`Msg::Click`, with or without `extend`, `SelectWordAt`,
  `SelectBlock`) inside the view no longer scrolls it by `scrolloff` (or re-centres a
  `Follow::Typewriter` view): the text stays under the pointer. The next key follows the caret
  as before, and a click below the text rows still scrolls to the caret. Selecting by dragging
  past an edge scrolls with the new `Msg::Drag`.
- A passive message no longer moves the view. A `tick` (which the `caretline` editor, the
  protocol server and hosts send before every batch), `ext`, `show_status`, `frame`,
  `frame_clock`, `saved`, `save_failed`, and `copy`, `save` and `quit` leave the scroll where it
  is; they used to follow the caret by `scrolloff`, so a click in the margin rows, or a `drag`
  on an edge row, scrolled again one message later (three edge drags with ticks between them
  scrolled 7 rows, not 3). The view follows the caret only after a key that moves it or edits,
  a scroll, a resize, a fold, or a host's `edit`, `command` or `insert_blocks` that moved a
  caret or changed the text. A host that sets a view's selection or rows itself, then relied on
  a `tick` to bring the caret into sight, calls `layout::ensure_caret_visible` instead.
- In prose (outline) wrapping, the end of the text or a line end right after a space that
  hangs past the column stays on that row. Typing a space after a word that fills the row no
  longer adds an empty row for the caret alone (so the page no longer grew a row while typing
  and shrank one when a save dropped the trailing space); the next character still starts the
  next row. The caret is drawn right after the hanging space when the view has a cell there,
  else in the view's last column on the same row; `view::locate` says the same cell, and a
  click in that last cell (`view::hit`, `Msg::Click`) puts the caret at the row's end. The new
  `Layout::hangs(line)` says whether a line wraps this way.

## 0.3.0 (2026-10-07)

caretline is a text-editing engine only. What a line *means* (a task, a status) belongs in the
host, which now adds it through extension points; the engine names no host concept.

### Added

- **Host extensions** (`Host`, set with `Document::set_host`): named commands run by
  `Msg::Command { name, args }` as one transaction and one undo step (`Edit`, `MarkOp`),
  recorded in traces and replayed with `trace::replay_trace_with`; input rules that take an
  editing message before the engine; a decorator that draws text under host-named roles in a
  block's hang and gutter.
- `Effect::Host { name, data }`: a host command's own effects.
- **Mark payloads**: `MarkAttrs.data`, any JSON value, carried through edits, cut and paste,
  undo and redo, changes from elsewhere (`ExtChange::SetData`) and JSON. `Marks::set_gap`,
  `Marks::set_data`.
- **Decorations**: `Decoration`, `Deco`, `Role::Named`, `Frame::roles`, `Frame::role_name`; hits
  report the decoration's id.
- **Tags**: `OutlineConfig::tags` and `new_tag`, `BlockInfo::tag`, `NewBlock::tag`,
  `ExtChange::SetShape::tag`: a bullet's meaning-free `[c]`.
- **The command catalog and the keymap as data**: `commands()`, `command()`, `command_msg()`,
  `default_keymap()`, `command_for()`, `Binding`, `CommandInfo`, `Category`, `Platform`.
- Protocol ops `commands.list` and `keymap.get`; `hello` lists the host's commands; `cells`
  spans carry host role names. `PROTO` stays 1.

### Breaking

1. `Msg::TaskCycle` and `Msg::SetStatus` removed (use host commands).
2. `Effect::Completed` and `Effect::Restored` removed; `Effect::Host` added.
3. `outline::Kind::Task`, `BlockInfo::status`, `NewBlock::status`, `Hang::Task` removed.
4. `OutlineConfig::task_markers`, `OutlineConfig::cycle`, `TaskMarker` and `is_task_char`
   removed. By default a bullet's `[x]` is text; set `tags` to make it part of the marker.
5. `ExtChange::SetShape`'s `status` is now `tag`.
6. `BlockAttrs` is renamed `MarkAttrs` and gains `data`; `Mark`, `MarkAttrs` and `ClipMark` are
   no longer `Copy`, and `Mark` is no longer `Hash`.
7. `OutlineLayout::marks` is renamed `gutter` (JSON still reads `marks`); `Hit::Marks` is
   `Hit::Gutter`; `Hit::Hang` and `Hit::Gutter` gain `deco`; `Hit` is no longer `Copy`.
8. `view::Role` gains `Named(u16)`: exhaustive matches need an arm.
9. `outline_keymap` no longer binds Ctrl-T. `keymap` is a lookup in the default keymap: chords
   the old function took by ignoring a modifier (Ctrl-Enter, Alt-Home, Ctrl-PageUp…) are unbound
   unless a host binds them.
10. `markdown::parse_markdown` takes the `OutlineConfig` (for tags); `parse_markdown_with` is
    gone.
11. A 0.2 trace with `task_cycle` or `set_status` messages no longer parses. 0.2 states read
    (`task_markers` and `cycle` are ignored).

# Changelog

## Unreleased

### Added

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

### Breaking

- `Msg` is `#[non_exhaustive]` (as `Effect` is): a `match` on it outside the crate needs a
  wildcard arm, and new kinds of message are no longer breaking. Its variants' fields are not,
  so hosts still build messages with struct literals; a field added to a variant stays a
  breaking change (with `#[serde(default)]`, so recorded JSON still parses). This release
  adds `Msg::Ext`.
- `Msg::Resize` has a third field, `cell_px: Option<CellPx>`. Code that builds it writes
  `Msg::resize(width, height)` (or adds `cell_px: None`); a pattern that names its fields adds
  `..`. Its JSON is unchanged without pixels (`#[serde(default)]`, left out when `None`), so
  old traces and clients replay and parse as before.
- `Frame` has a new public field, `cell_px`: a `Frame` built with a struct literal adds
  `cell_px: None`.

- `Config`, `ConfigInput`, `ViewConfig`, `OutlineConfig` and `OutlineLayout` are
  `#[non_exhaustive]`, so adding a setting is no longer a breaking change. Outside the crate
  they can't be built with a struct literal, not even with `..Default::default()`. Start from
  `Default` and chain the `with_` setters, or set the public fields on a default value:
  `OutlineConfig { tags: "ab".into(), ..OutlineConfig::default() }` becomes
  `OutlineConfig::default().with_tags("ab".into())`; for `ConfigInput` (no setters), take
  `ConfigInput::default()` and set its fields. This release also adds fields to them
  (`Config::single_line`, `ConfigInput::single_line`, `ViewConfig::page_overlap`), which broke
  struct literals anyway. The next release is a minor bump (0.4).

### Fixed

- A click (`Msg::Click`, `view::hit`) on the right half of a wide grapheme puts the caret after
  it, not before.
- The engine no longer enables serde_json's `preserve_order` for every crate that depends on it
  (it changed `serde_json::Map`'s key order across a host's whole build). Protocol responses
  are built from structs, so their key order is the same with or without the feature; the
  engine's tests also run with `arbitrary_precision` on.

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

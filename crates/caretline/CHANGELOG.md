# Changelog

## Unreleased

### Added

- `Layout::click_at`: where a click puts the caret.
- `ViewConfig::page_overlap`: rows of the previous screen a page motion keeps on screen (a page
  moves the text rows less this). Default 0, as before.
- `Config::single_line` (`"config": {"single_line": true}`, left out of JSON while false): a
  one-line text field. The text never holds a line break: `InsertNewline` and `SoftBreak`
  change nothing; line breaks typed, pasted, edited in by a host or an input rule, or put in
  from elsewhere (`Msg::External`, `text.set`) become spaces, `\r\n` counted once, and those at
  the end of the inserted text are dropped; lines never wrap; motion by a line, row or page goes
  to the start or the end. `State::sanitize` flattens line breaks already in the text and starts
  the history again when its undo or redo could bring one back. Ignored (and cleared) in an
  outline document. `Document::single_line()` says whether it applies.
- `History::transactions`: every revision's transaction and inversion.

### Breaking

- `Config` and `ConfigInput` gain a public field (`single_line`); neither is
  `#[non_exhaustive]`, so a struct literal that names every field needs `single_line` (or
  `..Default::default()`). The next release is a minor bump (0.4).

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

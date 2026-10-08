# Vendored Helix core

These files come from [Helix](https://github.com/helix-editor/helix) at commit
`ba40e547426b0f9896c8bdc699a4ab11f2b37dbc` and are licensed under the Mozilla Public License 2.0 (`LICENSE-MPL-2.0`).
MPL-2.0 applies per file: the rest of `caretline` is MIT.

| File here | Upstream |
|---|---|
| `selection.rs` | `helix-core/src/selection.rs` |
| `transaction.rs` | `helix-core/src/transaction.rs` |
| `history.rs` | `helix-core/src/history.rs` |
| `graphemes.rs` | `helix-core/src/graphemes.rs` |
| `movement.rs` | `helix-core/src/movement.rs` |
| `position.rs` | `helix-core/src/position.rs` |
| `doc_formatter.rs`, `doc_formatter/test.rs` | `helix-core/src/doc_formatter.rs` and its test module |
| `line_ending.rs` | `helix-core/src/line_ending.rs` |
| `chars.rs` | `helix-core/src/chars.rs` |
| `text_annotations.rs` | `helix-core/src/text_annotations.rs` (needed by the formatter) |
| `test.rs` | `helix-core/src/test.rs` (test helpers, compiled for tests only) |
| `stdx/rope.rs`, `stdx/range.rs` | `helix-stdx/src/rope.rs`, `helix-stdx/src/range.rs` |

Each file's header lists what changed from upstream. In short:

- Module paths point at `crate::helix` instead of `helix_core` and `helix_stdx`.
- Tree-sitter, textobject and regex code is removed, so no syntax or loader crates are needed.
- `History` takes caller-supplied millisecond timestamps instead of reading
  `std::time::Instant`, so editing stays a pure function of its inputs. It also gains
  `amend_current_revision`, which folds a run of typing into one undo step, and
  `current_transaction` / `current_inversion`, which the block marks read to record what each
  revision did to them, and `transactions`, which a
  one-line document checks for line breaks an undo could bring back.
- `DocumentFormatter` gains `resume_at_row` and `indent_level`, so layout can restart inside a
  long soft-wrapped line at a row it already knows.
- `DocumentFormatter`'s soft wrap checks a grapheme's start column plus its width against the
  viewport width, so a wide grapheme (an emoji, a CJK character, a tab) that would cross a
  row's end starts the next row instead of overflowing it by a cell. A word that starts a row
  breaks there instead of moving to a fresh row, and a grapheme wider than the whole row is
  placed alone on its row. `hang_spaces` adds prose wrapping: whitespace after a word that
  reaches the row's end hangs past it, and words end at whitespace only.
- The selection, transaction and history types derive serde, so editor state serializes.
  `Range::old_visual_position` and `Selection::primary_index` may be left out when
  deserializing.

`mod.rs` and this README are not from Helix.

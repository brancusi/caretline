// Vendored from Helix (https://github.com/helix-editor/helix), `helix-core/src/doc_formatter/test.rs`
// at commit ba40e547426b0f9896c8bdc699a4ab11f2b37dbc.
// SPDX-License-Identifier: MPL-2.0. This file is under the Mozilla Public License 2.0;
// see LICENSE-MPL-2.0 in the `helix` directory.
// Changes from upstream: module paths, and the `hang_spaces` field (off); caretline's tests
// for wide graphemes at the wrap edge (the section at the end) are added.

use crate::helix::doc_formatter::{DocumentFormatter, TextFormat};
use crate::helix::text_annotations::{InlineAnnotation, Overlay, TextAnnotations};

impl TextFormat {
    fn new_test(softwrap: bool) -> Self {
        TextFormat {
            soft_wrap: softwrap,
            tab_width: 2,
            max_wrap: 3,
            max_indent_retain: 4,
            wrap_indicator: ".".into(),
            wrap_indicator_highlight: None,
            // use a prime number to allow lining up too often with repeat
            viewport_width: 17,
            hang_spaces: false,
            soft_wrap_at_text_width: false,
        }
    }
}

impl<'t> DocumentFormatter<'t> {
    fn collect_to_str(&mut self) -> String {
        use std::fmt::Write;
        let mut res = String::new();
        let viewport_width = self.text_fmt.viewport_width;
        let soft_wrap_at_text_width = self.text_fmt.soft_wrap_at_text_width;
        let mut line = 0;

        for grapheme in self {
            if grapheme.visual_pos.row != line {
                line += 1;
                assert_eq!(grapheme.visual_pos.row, line);
                write!(res, "\n{}", ".".repeat(grapheme.visual_pos.col)).unwrap();
            }
            if !soft_wrap_at_text_width {
                assert!(
                    grapheme.visual_pos.col <= viewport_width as usize,
                    "softwrapped failed {}<={viewport_width}",
                    grapheme.visual_pos.col
                );
            }
            write!(res, "{}", grapheme.raw).unwrap();
        }

        res
    }
}

fn softwrap_text(text: &str) -> String {
    DocumentFormatter::new_at_prev_checkpoint(
        text.into(),
        &TextFormat::new_test(true),
        &TextAnnotations::default(),
        0,
    )
    .collect_to_str()
}

#[test]
fn basic_softwrap() {
    assert_eq!(
        softwrap_text(&"foo ".repeat(10)),
        "foo foo foo foo \n.foo foo foo foo \n.foo foo  "
    );
    assert_eq!(
        softwrap_text(&"fooo ".repeat(10)),
        "fooo fooo fooo \n.fooo fooo fooo \n.fooo fooo fooo \n.fooo  "
    );

    // check that we don't wrap unnecessarily
    assert_eq!(softwrap_text("\t\txxxx1xxxx2xx\n"), "    xxxx1xxxx2xx \n ");
}

#[test]
fn softwrap_indentation() {
    assert_eq!(
        softwrap_text("\t\tfoo1 foo2 foo3 foo4 foo5 foo6\n"),
        "    foo1 foo2 \n.....foo3 foo4 \n.....foo5 foo6 \n "
    );
    assert_eq!(
        softwrap_text("\t\t\tfoo1 foo2 foo3 foo4 foo5 foo6\n"),
        "      foo1 foo2 \n.foo3 foo4 foo5 \n.foo6 \n "
    );
}

#[test]
fn long_word_softwrap() {
    assert_eq!(
        softwrap_text("\t\txxxx1xxxx2xxxx3xxxx4xxxx5xxxx6xxxx7xxxx8xxxx9xxx\n"),
        "    xxxx1xxxx2xxx\n.....x3xxxx4xxxx5\n.....xxxx6xxxx7xx\n.....xx8xxxx9xxx \n "
    );
    assert_eq!(
        softwrap_text("xxxxxxxx1xxxx2xxx\n"),
        "xxxxxxxx1xxxx2xxx\n. \n "
    );
    assert_eq!(
        softwrap_text("\t\txxxx1xxxx 2xxxx3xxxx4xxxx5xxxx6xxxx7xxxx8xxxx9xxx\n"),
        "    xxxx1xxxx \n.....2xxxx3xxxx4x\n.....xxx5xxxx6xxx\n.....x7xxxx8xxxx9\n.....xxx \n "
    );
    assert_eq!(
        softwrap_text("\t\txxxx1xxx 2xxxx3xxxx4xxxx5xxxx6xxxx7xxxx8xxxx9xxx\n"),
        "    xxxx1xxx 2xxx\n.....x3xxxx4xxxx5\n.....xxxx6xxxx7xx\n.....xx8xxxx9xxx \n "
    );
}

#[test]
fn softwrap_multichar_grapheme() {
    assert_eq!(
        softwrap_text("xxxx xxxx xxx a\u{0301}bc\n"),
        "xxxx xxxx xxx \n.ábc \n "
    )
}

fn softwrap_text_at_text_width(text: &str) -> String {
    let mut text_fmt = TextFormat::new_test(true);
    text_fmt.soft_wrap_at_text_width = true;
    let annotations = TextAnnotations::default();
    let mut formatter =
        DocumentFormatter::new_at_prev_checkpoint(text.into(), &text_fmt, &annotations, 0);
    formatter.collect_to_str()
}
#[test]
fn long_word_softwrap_text_width() {
    assert_eq!(
        softwrap_text_at_text_width("xxxxxxxx1xxxx2xxx\nxxxxxxxx1xxxx2xxx"),
        "xxxxxxxx1xxxx2xxx \nxxxxxxxx1xxxx2xxx "
    );
}

fn overlay_text(text: &str, char_pos: usize, softwrap: bool, overlays: &[Overlay]) -> String {
    DocumentFormatter::new_at_prev_checkpoint(
        text.into(),
        &TextFormat::new_test(softwrap),
        TextAnnotations::default().add_overlay(overlays, None),
        char_pos,
    )
    .collect_to_str()
}

#[test]
fn overlay() {
    assert_eq!(
        overlay_text(
            "foobar",
            0,
            false,
            &[Overlay::new(0, "X"), Overlay::new(2, "\t")],
        ),
        "Xo  bar "
    );
    assert_eq!(
        overlay_text(
            &"foo ".repeat(10),
            0,
            true,
            &[
                Overlay::new(2, "\t"),
                Overlay::new(5, "\t"),
                Overlay::new(16, "X"),
            ]
        ),
        "fo   f  o foo \n.foo Xoo foo foo \n.foo foo foo  "
    );
}

fn annotate_text(text: &str, softwrap: bool, annotations: &[InlineAnnotation]) -> String {
    DocumentFormatter::new_at_prev_checkpoint(
        text.into(),
        &TextFormat::new_test(softwrap),
        TextAnnotations::default().add_inline_annotations(annotations, None),
        0,
    )
    .collect_to_str()
}

#[test]
fn annotation() {
    assert_eq!(
        annotate_text("bar", false, &[InlineAnnotation::new(0, "foo")]),
        "foobar "
    );
    assert_eq!(
        annotate_text(
            &"foo ".repeat(10),
            true,
            &[InlineAnnotation::new(0, "foo ")]
        ),
        "foo foo foo foo \n.foo foo foo foo \n.foo foo foo  "
    );
}

#[test]
fn annotation_and_overlay() {
    let annotations = [InlineAnnotation {
        char_idx: 0,
        text: "fooo".into(),
    }];
    let overlay = [Overlay {
        char_idx: 0,
        grapheme: "\t".into(),
    }];
    assert_eq!(
        DocumentFormatter::new_at_prev_checkpoint(
            "bbar".into(),
            &TextFormat::new_test(false),
            TextAnnotations::default()
                .add_inline_annotations(annotations.as_slice(), None)
                .add_overlay(overlay.as_slice(), None),
            0,
        )
        .collect_to_str(),
        "fooo  bar "
    );
}

// caretline: soft wrap never lets a wide grapheme cross the row's end.

/// A wrap format as caretline's layout makes one: no wrap indicator, tabs of 4.
fn wrap_fmt(width: u16, max_wrap: u16, hang: bool) -> TextFormat {
    TextFormat {
        soft_wrap: true,
        tab_width: 4,
        max_wrap,
        max_indent_retain: 40.min(width * 2 / 5),
        wrap_indicator: "".into(),
        wrap_indicator_highlight: None,
        viewport_width: width,
        soft_wrap_at_text_width: hang,
        hang_spaces: hang,
    }
}

/// One formatted row: its first column, its document text, and its graphemes as
/// (column, width, is whitespace, is a line end or the end of the text).
struct Row {
    start: usize,
    text: String,
    cells: Vec<(usize, usize, bool, bool)>,
}

fn format_rows(text: &str, fmt: &TextFormat) -> Vec<Row> {
    let rope = crate::helix::Rope::from(text);
    let annotations = TextAnnotations::default();
    let mut rows: Vec<Row> = Vec::new();
    for g in DocumentFormatter::new_at_prev_checkpoint(rope.slice(..), fmt, &annotations, 0) {
        let row = g.visual_pos.row;
        assert!(row <= rows.len(), "rows skipped at {row}");
        if row == rows.len() {
            rows.push(Row {
                start: g.visual_pos.col,
                text: String::new(),
                cells: Vec::new(),
            });
        }
        let r = &mut rows[row];
        let end = g.source.is_eof() || g.raw == crate::helix::graphemes::Grapheme::Newline;
        r.cells
            .push((g.visual_pos.col, g.width(), g.is_whitespace(), end));
        let chars = g.doc_chars();
        r.text
            .push_str(&rope.slice(g.char_idx..g.char_idx + chars).to_string());
    }
    rows
}

/// The rows' texts.
fn wrap(text: &str, fmt: &TextFormat) -> Vec<String> {
    format_rows(text, fmt).into_iter().map(|r| r.text).collect()
}

/// The invariants: the rows give back the text; no row is wider than the column, except
/// for whitespace that hangs (`hang_spaces`) and a single grapheme wider than the room left
/// on an otherwise empty row; line ends and the end of the text sit inside the column (or
/// right at its end with `soft_wrap_at_text_width`).
fn check_rows(text: &str, fmt: &TextFormat) {
    let width = fmt.viewport_width as usize;
    let rows = format_rows(text, fmt);
    let joined: String = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, text, "{text:?} at {width}");
    for (i, r) in rows.iter().enumerate() {
        let ctx = || {
            format!(
                "{text:?} at {width} hang {}: row {i} {:?}",
                fmt.hang_spaces, r.text
            )
        };
        let mut cells = r.cells.as_slice();
        // Trailing whitespace may hang past the column.
        if fmt.hang_spaces {
            while let [rest @ .., (_, _, true, false)] = cells {
                cells = rest;
            }
        }
        for (k, &(col, w, _, end)) in cells.iter().enumerate() {
            if end {
                let limit = if fmt.soft_wrap_at_text_width {
                    width
                } else {
                    width - 1
                };
                assert!(col <= limit, "{}: line end at {col}", ctx());
            } else if col + w > width {
                assert!(
                    k == 0 && col == r.start,
                    "{}: grapheme at {col} of width {w} overflows",
                    ctx()
                );
            }
        }
    }
}

#[test]
fn wide_grapheme_at_the_row_end_starts_the_next_row_hang() {
    // Host request #5's minimal case: a long word (max_wrap 1) in prose.
    let fmt = wrap_fmt(4, 1, true);
    assert_eq!(wrap("abc🙂d", &fmt), ["abc", "🙂d"]);
    check_rows("abc🙂d", &fmt);
    // The host's case: 71 `a`, an emoji and more words in a 72-cell column.
    let text = format!("{}🙂 tail words here", "a".repeat(71));
    let fmt = wrap_fmt(72, 72, true);
    let rows = wrap(&text, &fmt);
    assert_eq!(rows[0], "a".repeat(71));
    assert_eq!(rows[1], "🙂 tail words here");
    check_rows(&text, &fmt);
    // With 70 `a` the emoji fits exactly.
    let text = format!("{}🙂 tail", "a".repeat(70));
    assert_eq!(wrap(&text, &fmt)[0], format!("{}🙂 ", "a".repeat(70)));
}

#[test]
fn wide_grapheme_at_the_row_end_plain_wrap() {
    // A short word moves whole to the next row; a CJK char that would cross the end with it.
    let fmt = wrap_fmt(12, 3, false);
    assert_eq!(wrap("abcd efg 日本", &fmt), ["abcd efg ", "日本"]);
    assert_eq!(wrap("abcdefghij 日", &fmt), ["abcdefghij ", "日"]);
    // A long word breaks before the wide grapheme instead of after it.
    assert_eq!(wrap("abcdefghijk日本語", &fmt), ["abcdefghijk", "日本語"]);
    assert_eq!(wrap("x abcdefghij日本", &fmt), ["x abcdefghij", "日本"]);
    for t in [
        "abcd efg 日本",
        "abcdefghij 日",
        "abcdefghijk日本語",
        "x abcdefghij日本",
    ] {
        check_rows(t, &fmt);
    }
}

#[test]
fn wide_grapheme_at_a_word_boundary() {
    // Code wrap breaks after punctuation: the emoji after the comma starts the next row.
    let fmt = wrap_fmt(6, 1, false);
    assert_eq!(wrap("abcd,🙂x", &fmt), ["abcd,", "🙂x"]);
    // Prose: the word with the emoji moves whole.
    let fmt = wrap_fmt(6, 6, true);
    assert_eq!(wrap("ab cd🙂", &fmt), ["ab ", "cd🙂"]);
    check_rows("abcd,🙂x", &wrap_fmt(6, 1, false));
    check_rows("ab cd🙂", &fmt);
}

#[test]
fn a_tab_that_crosses_the_row_end() {
    // Without hang, a tab that would cross the end starts the next row.
    let fmt = wrap_fmt(6, 1, false);
    assert_eq!(wrap("abcde\tx", &fmt), ["abcde", "\tx"]);
    check_rows("abcde\tx", &fmt);
    // With hang, the tab is whitespace: it hangs past the end.
    let fmt = wrap_fmt(6, 6, true);
    assert_eq!(wrap("ab cde\tx", &fmt), ["ab cde\t", "x"]);
    check_rows("ab cde\tx", &fmt);
}

#[test]
fn a_grapheme_wider_than_the_row_is_placed_alone() {
    // Two cells in a one-cell column: each emoji gets its own row, and wrapping ends.
    let fmt = wrap_fmt(1, 1, false);
    // (The end of the text needs a cell of its own.)
    assert_eq!(wrap("🙂🙂a", &fmt), ["🙂", "🙂", "a", ""]);
    check_rows("🙂🙂a", &fmt);
    let fmt = wrap_fmt(1, 1, true);
    assert_eq!(wrap("🙂🙂 a", &fmt), ["🙂", "🙂 ", "a"]);
    check_rows("🙂🙂 a", &fmt);
}

#[test]
fn a_wide_grapheme_ending_a_line_or_the_text() {
    // The end of the text after a word that fills the row exactly.
    let fmt = wrap_fmt(36, 9, false);
    let text = "word- Leaveword word word 日本xx日本";
    assert_eq!(
        wrap(text, &fmt),
        ["word- Leaveword word word 日本xx日本", ""]
    );
    check_rows(text, &fmt);
    let fmt = wrap_fmt(5, 5, true);
    assert_eq!(wrap("ab 日本\nc", &fmt), ["ab ", "日本\n", "c"]);
    check_rows("ab 日本\nc", &fmt);
}

/// Random text mixing ASCII, CJK, emoji, tabs, combining marks and line breaks, at random
/// widths: every row fits its column (the exceptions in `check_rows`) and the rows give
/// back the text.
#[test]
fn wrapped_rows_never_exceed_the_column() {
    const PIECES: &[&str] = &[
        "a",
        "b",
        "word",
        "lorem",
        "x",
        " ",
        " ",
        "  ",
        "\t",
        "\n",
        "日",
        "本語",
        "🙂",
        "👍🏽",
        "👨‍👩‍👧",
        "🇫🇷",
        "e\u{301}",
        "a\u{308}\u{301}",
        "-",
        ",",
        "1️⃣",
        "\u{200b}",
        "ｗｉｄｅ",
    ];
    let mut seed: u32 = 0x9e37_79b9;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for _ in 0..3000 {
        let len = (next() % 40) as usize;
        let text: String = (0..len)
            .map(|_| PIECES[next() as usize % PIECES.len()])
            .collect();
        let width = 2 + (next() % 39) as u16;
        let hang = next() % 2 == 0;
        let max_wrap = if hang { width } else { 20.min(width / 4) };
        check_rows(&text, &wrap_fmt(width, max_wrap, hang));
    }
}

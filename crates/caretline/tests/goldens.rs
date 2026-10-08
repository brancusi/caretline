//! Behaviour goldens: a starting text and selection, keys or messages, and the exact
//! expected text, selection and (where it matters) frame.
//!
//! The baseline is a macOS text field. Cases are grouped by area; the plain-text cases of
//! the editing behaviour contract are numbered `e01`… in the order of that contract.

mod common;

use caretline::{Effect, Msg};
use common::*;

// ---------------------------------------------------------------------------------------
// Selection collapse: a motion without Shift collapses a selection instead of moving from
// the caret; with Shift the anchor stays and the caret moves.

#[test]
fn e01_left_collapses_to_start() {
    golden("Hello ⟦wor▮⟧ld", "<left>", "Hello ▮world");
}

#[test]
fn e02_right_collapses_to_end() {
    golden("Hello ⟦wor▮⟧ld", "<right>", "Hello wor▮ld");
}

#[test]
fn e03_collapse_ignores_which_end_the_caret_is_at() {
    golden("Hello ⟦▮wor⟧ld", "<right>", "Hello wor▮ld");
    golden("Hello ⟦▮wor⟧ld", "<left>", "Hello ▮world");
}

#[test]
fn e04_shift_left_shrinks_from_the_caret() {
    golden("Hello ⟦wor▮⟧ld", "<s-left>", "Hello ⟦wo▮⟧rld");
}

#[test]
fn e05_shift_right_grows_from_the_caret() {
    golden("Hello ⟦wor▮⟧ld", "<s-right>", "Hello ⟦worl▮⟧d");
}

#[test]
fn e06_caret_back_on_anchor_means_no_selection() {
    golden("ab ⟦▮c⟧ d", "<s-right>", "ab c▮ d");
}

#[test]
fn e07_collapse_never_jumps_rows() {
    golden(
        "First line\nSec⟦ond li▮⟧ne",
        "<left>",
        "First line\nSec▮ond line",
    );
}

#[test]
fn e08_up_with_selection_starts_from_its_start() {
    golden(
        "Line one\nLine ⟦two and▮⟧ more",
        "<up>",
        "Line ▮one\nLine two and more",
    );
}

#[test]
fn e09_down_with_selection_starts_from_its_end() {
    golden("Line ⟦one▮⟧\nLine two", "<down>", "Line one\nLine two▮");
}

#[test]
fn e10_word_left_with_selection_starts_from_its_start() {
    golden("one two ⟦thr▮⟧ee", "<a-left>", "one ▮two three");
}

#[test]
fn e11_word_right_with_selection_starts_from_its_end() {
    golden("one ⟦tw▮⟧o three", "<a-right>", "one two▮ three");
}

#[test]
fn e12_escape_collapses_to_the_caret() {
    golden("ab ⟦cd▮⟧ ef", "<esc>", "ab cd▮ ef");
    golden("ab ⟦▮cd⟧ ef", "<esc>", "ab ▮cd ef");
}

#[test]
fn e13_shift_click_keeps_the_anchor() {
    let mut s = state("Hello ⟦wor▮⟧ld");
    send(
        &mut s,
        [Msg::Click {
            col: 8,
            row: 0,
            extend: true,
        }],
    );
    assert_eq!(show(&s), "Hello ⟦wo▮⟧rld");
}

#[test]
fn e14_click_places_the_caret() {
    let mut s = state("⟦Hello▮⟧ world");
    send(
        &mut s,
        [Msg::Click {
            col: 9,
            row: 0,
            extend: false,
        }],
    );
    assert_eq!(show(&s), "Hello wor▮ld");
}

#[test]
fn collapse_rules_for_line_and_document_keys() {
    golden("ab ⟦cd▮⟧ ef\ngh", "<home>", "▮ab cd ef\ngh");
    golden("ab ⟦▮cd⟧ ef\ngh", "<end>", "ab cd ef▮\ngh");
    golden("ab ⟦cd▮⟧ ef\ngh", "<d-down>", "ab cd ef\ngh▮");
    golden("ab ⟦cd▮⟧ ef\ngh", "<d-up>", "▮ab cd ef\ngh");
}

// ---------------------------------------------------------------------------------------
// Edits with a selection

#[test]
fn e15_typing_replaces_the_selection() {
    golden("Hello ⟦wor▮⟧ld", "X", "Hello X▮ld");
}

#[test]
fn e16_backspace_deletes_only_the_selection() {
    golden("Hello ⟦wor▮⟧ld", "<bs>", "Hello ▮ld");
}

#[test]
fn e17_delete_deletes_only_the_selection() {
    golden("Hello ⟦wor▮⟧ld", "<del>", "Hello ▮ld");
}

#[test]
fn e18_word_and_line_deletes_never_extend_a_selection() {
    golden("one ⟦two thr▮⟧ee", "<a-bs>", "one ▮ee");
    golden("one ⟦two thr▮⟧ee", "<a-del>", "one ▮ee");
    golden("one ⟦two thr▮⟧ee", "<d-bs>", "one ▮ee");
    golden("one ⟦two thr▮⟧ee", "<d-del>", "one ▮ee");
    golden("one ⟦two thr▮⟧ee", "<c-k>", "one ▮ee");
    golden("one ⟦two thr▮⟧ee", "<c-w>", "one ▮ee");
    golden("one ⟦two thr▮⟧ee", "<c-u>", "one ▮ee");
}

#[test]
fn e19_typing_over_a_multi_line_selection() {
    golden("One ⟦two\nthree fo▮⟧ur", "X", "One X▮ur");
}

#[test]
fn e20_backspace_over_whole_lines() {
    golden("A⟦a\nBb\nC▮⟧c", "<bs>", "A▮c");
}

#[test]
fn e21_enter_replaces_the_selection_with_a_line_break() {
    golden("ab⟦cd▮⟧ef", "<cr>", "ab\n▮ef");
}

#[test]
fn e24_one_undo_restores_text_and_selection_after_typing_over_it() {
    golden("Hello ⟦wor▮⟧ld", "XY<c-z>", "Hello ⟦wor▮⟧ld");
    golden("Hello ⟦▮wor⟧ld", "XY<d-z>", "Hello ⟦▮wor⟧ld");
}

// ---------------------------------------------------------------------------------------
// Word and line deletes without a selection

#[test]
fn e25_word_delete_back_inside_a_word() {
    golden("one two thr▮ee", "<a-bs>", "one two ▮ee");
}

#[test]
fn e26_word_delete_back_skips_spaces_first() {
    golden("one two ▮three", "<a-bs>", "one ▮three");
    golden("one two ▮three", "<c-w>", "one ▮three");
}

#[test]
fn e27_word_delete_back_at_a_line_start_joins_like_backspace() {
    golden("First\n▮Second", "<a-bs>", "First▮Second");
    golden("First▮\nSecond", "<a-del>", "First▮Second");
}

#[test]
fn e28_word_delete_forward() {
    golden("one ▮two three", "<a-del>", "one ▮ three");
    golden("one ▮two three", "<a-d>", "one ▮ three");
}

#[test]
fn e29_delete_to_row_start() {
    golden("one two thr▮ee", "<d-bs>", "▮ee");
    golden("one two thr▮ee", "<c-u>", "▮ee");
    // At a row start it deletes the previous character (the line break).
    golden("one\n▮two", "<d-bs>", "one▮two");
}

#[test]
fn e30_kill_to_the_line_end() {
    golden("one ▮two\nthree", "<c-k>", "one ▮\nthree");
}

#[test]
fn e31_kill_at_the_line_end_removes_the_break() {
    golden("one▮\nthree", "<c-k>", "one▮three");
}

#[test]
fn e32_delete_to_row_end() {
    golden("one ▮two", "<d-del>", "one ▮");
}

// ---------------------------------------------------------------------------------------
// The clipboard

#[test]
fn e33_copy_changes_nothing_but_the_clipboard() {
    let mut s = state("Hello ⟦wor▮⟧ld");
    let before = s.clone();
    let fx = keys(&mut s, "<d-c>");
    assert_eq!(fx, vec![Effect::ClipboardSet { text: "wor".into() }]);
    assert_eq!(s.doc.clipboard, "wor");
    assert_eq!(show(&s), "Hello ⟦wor▮⟧ld");
    assert_eq!(s.doc.history, before.doc.history);
    assert_eq!(s.doc.dirty, before.doc.dirty);
    // The Ctrl twin does the same.
    let mut t = before.clone();
    assert_eq!(keys(&mut t, "<c-c>"), fx);
}

#[test]
fn e36_copy_with_nothing_selected_copies_nothing() {
    let mut s = state("Hello wor▮ld");
    s.doc.clipboard = "kept".into();
    let fx = keys(&mut s, "<c-c>");
    assert!(fx.is_empty());
    assert_eq!(s.doc.clipboard, "kept");
    assert_eq!(s.view.status.as_deref(), Some("nothing selected"));
    assert_eq!(show(&s), "Hello wor▮ld");
}

#[test]
fn e37_cut_is_one_undo_step() {
    let mut s = state("Hello ⟦wor▮⟧ld");
    let fx = keys(&mut s, "<c-x>");
    assert_eq!(fx, vec![Effect::ClipboardSet { text: "wor".into() }]);
    assert_eq!(show(&s), "Hello ▮ld");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "Hello ⟦wor▮⟧ld");
}

#[test]
fn e38_paste_replaces_the_selection() {
    let mut s = state("Hello ⟦wor▮⟧ld");
    s.doc.clipboard = "X".into();
    keys(&mut s, "<c-v>");
    assert_eq!(show(&s), "Hello X▮ld");
}

#[test]
fn e39_multi_line_paste_is_one_step() {
    let mut s = state("▮");
    send(
        &mut s,
        [Msg::Paste {
            text: Some("- a\n- b".into()),
        }],
    );
    assert_eq!(show(&s), "- a\n- b▮");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "▮");
}

#[test]
fn e40_cut_then_paste_in_place_is_the_identity() {
    let mut s = state("One ⟦two\nth▮⟧ree");
    keys(&mut s, "<c-x><c-v>");
    assert_eq!(show(&s), "One two\nth▮ree");
}

#[test]
fn e41_select_all_then_left() {
    let mut s = state("ab\ncd▮");
    keys(&mut s, "<d-a>");
    assert_eq!(show(&s), "⟦ab\ncd▮⟧");
    keys(&mut s, "<d-a>");
    assert_eq!(show(&s), "⟦ab\ncd▮⟧", "a second select-all changes nothing");
    keys(&mut s, "<left>");
    assert_eq!(show(&s), "▮ab\ncd");
    // Alt-A is the twin for terminals that don't forward Cmd.
    keys(&mut s, "<a-a>");
    assert_eq!(show(&s), "⟦ab\ncd▮⟧");
}

#[test]
fn e42_select_all_delete_then_undo() {
    let mut s = state("ab\ncd▮");
    keys(&mut s, "<d-a><bs>");
    assert_eq!(show(&s), "▮");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "⟦ab\ncd▮⟧");
}

#[test]
fn e43_redo_with_nothing_to_redo() {
    let mut s = state("Hello ⟦wor▮⟧ld");
    keys(&mut s, "X");
    keys(&mut s, "<c-s-z>");
    assert_eq!(show(&s), "Hello X▮ld");
    assert_eq!(s.view.status.as_deref(), Some("nothing to redo"));
    keys(&mut s, "<c-z>");
    assert_eq!(s.view.status, None);
    keys(&mut s, "<c-y>");
    assert_eq!(show(&s), "Hello X▮ld");
}

// ---------------------------------------------------------------------------------------
// Undo grouping

#[test]
fn typing_within_the_gap_is_one_step_and_a_pause_starts_another() {
    let mut s = state("▮");
    keys(&mut s, "ab<wait:1000>cd<wait:1600>ef");
    assert_eq!(show(&s), "abcdef▮");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "abcd▮");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "▮");
    keys(&mut s, "<c-s-z>");
    assert_eq!(show(&s), "abcd▮");
    keys(&mut s, "<c-s-z>");
    assert_eq!(show(&s), "abcdef▮");
}

#[test]
fn a_motion_ends_a_typing_run() {
    let mut s = state("▮");
    keys(&mut s, "ab<left>c");
    assert_eq!(show(&s), "ac▮b");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "a▮b");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "▮");
}

#[test]
fn every_command_is_its_own_step() {
    let mut s = state("one two three▮");
    keys(&mut s, "<a-bs><a-bs>");
    assert_eq!(show(&s), "one ▮");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "one two ▮");
}

#[test]
fn held_backspace_is_one_step() {
    let mut s = state("abcdef▮");
    keys(&mut s, "<bs><bs><bs>");
    assert_eq!(show(&s), "abc▮");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "abcdef▮");
}

#[test]
fn undo_and_redo_restore_exact_text_and_selection() {
    let mut s = state("alpha ⟦▮beta⟧ gamma");
    keys(&mut s, "<a-del>");
    assert_eq!(show(&s), "alpha ▮ gamma");
    keys(&mut s, "<right><wait:2000>X");
    assert_eq!(show(&s), "alpha  X▮gamma");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "alpha  ▮gamma");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "alpha ⟦▮beta⟧ gamma");
    keys(&mut s, "<c-z>");
    assert_eq!(s.view.status.as_deref(), Some("nothing to undo"));
    keys(&mut s, "<c-y>");
    assert_eq!(show(&s), "alpha ▮ gamma");
    keys(&mut s, "<c-y>");
    assert_eq!(show(&s), "alpha  X▮gamma");
}

#[test]
fn saving_marks_clean_and_editing_marks_dirty() {
    let mut s = state("ab▮");
    assert!(!s.doc.dirty);
    keys(&mut s, "c");
    assert!(s.doc.dirty);
    let fx = keys(&mut s, "<c-s>");
    assert_eq!(
        fx,
        vec![Effect::WriteFile {
            path: "test.md".into(),
            text: "abc".into()
        }]
    );
    send(&mut s, [Msg::Saved]);
    assert!(!s.doc.dirty);
    // Typing straight after a save starts a new step, so the save point stays exact.
    keys(&mut s, "d");
    assert!(s.doc.dirty);
    keys(&mut s, "<c-z>");
    assert!(!s.doc.dirty);
    assert_eq!(show(&s), "abc▮");
}

#[test]
fn quitting_with_unsaved_changes_asks_twice() {
    let mut s = state("ab▮");
    assert_eq!(keys(&mut s, "<c-q>"), vec![Effect::Quit]);
    keys(&mut s, "c");
    assert!(keys(&mut s, "<c-q>").is_empty());
    assert!(s.view.status.is_some());
    assert_eq!(keys(&mut s, "<c-q>"), vec![Effect::Quit]);
}

// ---------------------------------------------------------------------------------------
// Basic editing

#[test]
fn enter_in_mid_line_splits_it() {
    golden("hel▮lo", "<cr>", "hel\n▮lo");
}

#[test]
fn backspace_and_delete_join_lines() {
    golden("hel\n▮lo", "<bs>", "hel▮lo");
    golden("hel▮\nlo", "<del>", "hel▮lo");
}

#[test]
fn edges_of_the_document_are_safe() {
    golden("▮abc", "<bs><left><up><home><a-left>", "▮abc");
    golden("abc▮", "<del><right><down><end><a-right>", "abc▮");
}

#[test]
fn up_on_the_first_row_goes_to_the_start_and_down_on_the_last_to_the_end() {
    golden("ab▮c\ndef", "<up>", "▮abc\ndef");
    golden("abc\nd▮ef", "<down>", "abc\ndef▮");
}

#[test]
fn crlf_documents_stay_crlf() {
    let mut s = state("a▮\r\nb");
    keys(&mut s, "<cr>");
    assert_eq!(s.doc.text.to_string(), "a\r\n\r\nb");
    // A CRLF is one grapheme: one backspace removes both characters.
    keys(&mut s, "<bs>");
    assert_eq!(s.doc.text.to_string(), "a\r\nb");
    send(
        &mut s,
        [Msg::Paste {
            text: Some("x\ny".into()),
        }],
    );
    assert_eq!(s.doc.text.to_string(), "ax\r\ny\r\nb");
}

#[test]
fn shift_motions_build_selections() {
    golden("one ▮two three", "<s-a-right>", "one ⟦two▮⟧ three");
    golden("one two▮ three", "<s-a-left><s-a-left>", "⟦▮one two⟧ three");
    golden("one ▮two\nthree", "<s-down>", "one ⟦two\nthre▮⟧e");
    golden("one ▮two", "<s-end>", "one ⟦two▮⟧");
    golden("one ▮two", "<s-home>", "⟦▮one ⟧two");
}

#[test]
fn tab_inserts_a_tab_and_renders_to_the_stop() {
    let mut s = state_wh("a▮b", 20, 3);
    keys(&mut s, "<tab>");
    assert_eq!(show(&s), "a\t▮b");
    assert_eq!(frame(&s).lines().next().unwrap(), "a   b");
    assert_eq!(cursor(&s), Some((4, 0)));
}

// ---------------------------------------------------------------------------------------
// Word motion

#[test]
fn word_motion_moves_by_words_skipping_spaces_and_punctuation() {
    golden("▮Hello, wide world!", "<a-right>", "Hello▮, wide world!");
    golden(
        "▮Hello, wide world!",
        "<a-right><a-right>",
        "Hello, wide▮ world!",
    );
    golden(
        "▮Hello, wide world!",
        "<a-right><a-right><a-right><a-right>",
        "Hello, wide world!▮",
    );
    golden("Hello, wide world!▮", "<a-left>", "Hello, wide ▮world!");
    golden(
        "Hello, wide world!▮",
        "<a-left><a-left><a-left>",
        "▮Hello, wide world!",
    );
    golden("snake_case wo▮rd", "<a-left><a-left>", "▮snake_case word");
    // Emacs twins.
    golden("one ▮two", "<a-f>", "one two▮");
    golden("one two▮", "<a-b>", "one ▮two");
    // Ctrl-arrows move by word too.
    golden("one ▮two", "<c-right>", "one two▮");
}

#[test]
fn word_motion_crosses_lines() {
    golden("one▮\ntwo", "<a-right>", "one\ntwo▮");
    golden("one\n▮two", "<a-left>", "▮one\ntwo");
}

// ---------------------------------------------------------------------------------------
// Unicode: emoji, combining marks, wide characters

#[test]
fn emoji_with_a_modifier_is_one_step() {
    golden("a▮👍🏽b", "<right>", "a👍🏽▮b");
    golden("a👍🏽▮b", "<left>", "a▮👍🏽b");
    golden("a👍🏽▮b", "<bs>", "a▮b");
    golden("a▮👍🏽b", "<del>", "a▮b");
    golden("a▮👍🏽b", "<s-right>", "a⟦👍🏽▮⟧b");
}

#[test]
fn zwj_family_emoji_is_one_grapheme() {
    golden("▮👨‍👩‍👧x", "<right>", "👨‍👩‍👧▮x");
    golden("👨‍👩‍👧▮x", "<bs>", "▮x");
}

#[test]
fn combining_accents_move_with_their_base() {
    golden("cafe\u{301}▮ ok", "<left>", "caf▮e\u{301} ok");
    golden("caf▮e\u{301} ok", "<right>", "cafe\u{301}▮ ok");
    golden("cafe\u{301}▮ ok", "<bs>", "caf▮ ok");
    // The accented letter is part of the word.
    golden("▮cafe\u{301} ok", "<a-right>", "cafe\u{301}▮ ok");
}

#[test]
fn wide_characters_take_two_columns() {
    let s = state_wh("漢字ab▮c", 20, 3);
    assert_eq!(frame(&s).lines().next().unwrap(), "漢字abc");
    assert_eq!(cursor(&s), Some((6, 0)));
    // Vertical motion keeps the visual column across wide characters.
    golden("漢字▮abc\nabcdef", "<down>", "漢字abc\nabcd▮ef");
    golden("漢字abc\nab▮cdef", "<up>", "漢▮字abc\nabcdef");
    golden("漢字abc\nabc▮def", "<up>", "漢▮字abc\nabcdef");
}

#[test]
fn emoji_and_cjk_render_with_the_caret_on_the_right_cell() {
    let mut s = state_wh("▮", 20, 3);
    send(
        &mut s,
        [Msg::InsertText {
            text: "a👍🏽漢e\u{301}".into(),
        }],
    );
    assert_eq!(frame(&s).lines().next().unwrap(), "a👍🏽漢e\u{301}");
    // a(1) + emoji(2) + wide(2) + accented e(1).
    assert_eq!(cursor(&s), Some((6, 0)));
}

#[test]
fn a_keycap_emoji_takes_two_cells() {
    // `1️⃣` starts with an ASCII digit but is an emoji (2 cells), as terminals draw it.
    let mut s = state_wh("▮", 20, 3);
    send(
        &mut s,
        [Msg::InsertText {
            text: "a1\u{fe0f}\u{20e3}b".into(),
        }],
    );
    assert_eq!(cursor(&s), Some((4, 0)));
    golden("a▮1\u{fe0f}\u{20e3}b", "<right>", "a1\u{fe0f}\u{20e3}▮b");
}

// ---------------------------------------------------------------------------------------
// Soft wrap: visual rows, the goal column, Home and End

/// Three lines; at width 20 the first and last wrap:
/// row 0 `aaaa bbbb cccc dddd ` · row 1 `eeee ffff gggg` · row 2 `xy` ·
/// row 3 `hhhh iiii jjjj kkkk ` · row 4 `llll`.
const WRAPPED: &str = "aaaa bbbb cccc dddd eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll";

fn wrapped(at: &str) -> caretline::State {
    // `at` is WRAPPED with a caret mark in it.
    state_wh(at, 20, 7)
}

#[test]
fn wrapped_paragraph_frame() {
    let s = wrapped(&format!("▮{WRAPPED}"));
    assert_eq!(
        frame(&s),
        "aaaa bbbb cccc dddd\neeee ffff gggg\nxy\nhhhh iiii jjjj kkkk\nllll\n\n test.md        1:1\n"
    );
    assert_eq!(cursor(&s), Some((0, 0)));
}

#[test]
fn arrows_across_wrapped_rows_keep_the_goal_column() {
    let mut s = wrapped("aaaa bbbb cc▮cc dddd eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll");
    keys(&mut s, "<down>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gg▮gg\nxy\nhhhh iiii jjjj kkkk llll"
    );
    assert_eq!(cursor(&s), Some((12, 1)));
    keys(&mut s, "<down>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gggg\nxy▮\nhhhh iiii jjjj kkkk llll"
    );
    keys(&mut s, "<down>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gggg\nxy\nhhhh iiii jj▮jj kkkk llll"
    );
    keys(&mut s, "<down>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll▮"
    );
    keys(&mut s, "<up><up><up>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gg▮gg\nxy\nhhhh iiii jjjj kkkk llll"
    );
    // A horizontal move resets the goal column.
    keys(&mut s, "<left><down><down>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gggg\nxy\nhhhh iiii j▮jjj kkkk llll"
    );
}

#[test]
fn logical_line_motion_ignores_wrapping() {
    let mut s = wrapped("aaaa bbbb cc▮cc dddd eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll");
    send(
        &mut s,
        [Msg::Move {
            dir: caretline::Dir::Forward,
            by: caretline::By::Line,
            extend: false,
        }],
    );
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gggg\nxy▮\nhhhh iiii jjjj kkkk llll"
    );
}

#[test]
fn home_and_end_work_on_visual_rows() {
    // On the second row of the wrapped line.
    let mut s = wrapped("aaaa bbbb cccc dddd eeee ff▮ff gggg\nxy\nhhhh iiii jjjj kkkk llll");
    keys(&mut s, "<home>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd ▮eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll"
    );
    assert_eq!(cursor(&s), Some((0, 1)));
    keys(&mut s, "<end>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd eeee ffff gggg▮\nxy\nhhhh iiii jjjj kkkk llll"
    );
    // On the first row, End stops where the row wraps (before the wrapping space), so the
    // caret stays on that row.
    let mut s = wrapped("aaaa bb▮bb cccc dddd eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll");
    keys(&mut s, "<end>");
    assert_eq!(
        show(&s),
        "aaaa bbbb cccc dddd▮ eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll"
    );
    assert_eq!(cursor(&s), Some((19, 0)));
    keys(&mut s, "<home>");
    assert_eq!(
        show(&s),
        "▮aaaa bbbb cccc dddd eeee ffff gggg\nxy\nhhhh iiii jjjj kkkk llll"
    );
    // The Cmd and Ctrl twins.
    keys(&mut s, "<d-right>");
    assert_eq!(cursor(&s), Some((19, 0)));
    keys(&mut s, "<c-a>");
    assert_eq!(cursor(&s), Some((0, 0)));
    keys(&mut s, "<c-e>");
    assert_eq!(cursor(&s), Some((19, 0)));
}

#[test]
fn delete_to_row_start_on_a_wrapped_row() {
    let mut s = wrapped("aaaa bbbb cccc dddd eeee ff▮ff gggg\nxy");
    keys(&mut s, "<d-bs>");
    assert_eq!(show(&s), "aaaa bbbb cccc dddd ▮ff gggg\nxy");
}

#[test]
fn wrapped_continuation_rows_keep_the_indent() {
    let s = state_wh("▮  - item one two three four five six", 20, 4);
    assert_eq!(
        frame(&s),
        "  - item one two\n  three four five\n  six\n test.md        1:1\n"
    );
}

#[test]
fn narrow_viewports_turn_wrapping_off_and_scroll_sideways() {
    let mut s = state_wh("▮abcdefghijklmnop", 8, 2);
    assert_eq!(frame(&s).lines().next().unwrap(), "abcdefgh");
    keys(&mut s, "<end>");
    assert_eq!(frame(&s).lines().next().unwrap(), "jklmnop");
    assert_eq!(cursor(&s), Some((7, 0)));
}

#[test]
fn a_wide_grapheme_at_the_wrap_edge_starts_the_next_row() {
    // Width 12. `日` would start at column 11 and end at 13: it starts the next row (with
    // its short word), and so does the emoji after a long word that ends at column 11.
    // No row is wider than 12 cells.
    let mut s = state_wh("▮abcd efgh i日本 xy\nabcdefghijk🙂z", 12, 6);
    assert_eq!(
        frame(&s),
        "abcd efgh\ni日本 xy\nabcdefghijk\n🙂z\n\n test.m 1:1\n"
    );
    for row in frame(&s).lines() {
        assert!(caretline::view::display_width(row) <= 12, "{row:?}");
    }
    // Vertical motion walks the new rows: each wrapped grapheme starts its row.
    keys(&mut s, "<down>");
    assert_eq!(show(&s), "abcd efgh ▮i日本 xy\nabcdefghijk🙂z");
    assert_eq!(cursor(&s), Some((0, 1)));
    keys(&mut s, "<down><down>");
    assert_eq!(show(&s), "abcd efgh i日本 xy\nabcdefghijk▮🙂z");
    assert_eq!(cursor(&s), Some((0, 3)));
    keys(&mut s, "<right>");
    assert_eq!(cursor(&s), Some((2, 3)));
    // A click on the right half of the wrapped `日` (row 1, columns 1-2) lands after it.
    send(
        &mut s,
        [Msg::Click {
            col: 2,
            row: 1,
            extend: false,
        }],
    );
    assert_eq!(show(&s), "abcd efgh i日▮本 xy\nabcdefghijk🙂z");
    assert_eq!(cursor(&s), Some((3, 1)));
}

// ---------------------------------------------------------------------------------------
// Paste

#[test]
fn paste_of_multi_line_text_at_the_caret() {
    let mut s = state("x▮y");
    send(
        &mut s,
        [Msg::Paste {
            text: Some("one\ntwo\nthree".into()),
        }],
    );
    assert_eq!(show(&s), "xone\ntwo\nthree▮y");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "x▮y");
    keys(&mut s, "<c-y>");
    assert_eq!(show(&s), "xone\ntwo\nthree▮y");
}

#[test]
fn paste_normalizes_line_endings() {
    let mut s = state("▮");
    send(
        &mut s,
        [Msg::Paste {
            text: Some("a\r\nb\rc".into()),
        }],
    );
    assert_eq!(show(&s), "a\nb\nc▮");
}

// ---------------------------------------------------------------------------------------
// Scrolling and pages

fn numbered(n: usize) -> String {
    (1..=n)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn page_down_moves_a_screenful_and_keeps_the_column() {
    let mut s = state_wh(
        &format!("line▮ 1\n{}", &numbered(60)["line 1\n".len()..]),
        20,
        11,
    );
    keys(&mut s, "<pgdn>");
    assert_eq!(s.doc.text.char_to_line(s.caret()), 10);
    assert_eq!(cursor(&s).map(|c| c.0), Some(4));
    keys(&mut s, "<pgdn><pgup>");
    assert_eq!(s.doc.text.char_to_line(s.caret()), 10);
    keys(&mut s, "<pgup><pgup>");
    assert_eq!(s.caret(), 0);
}

#[test]
fn a_page_keeps_the_overlap_on_screen() {
    // Ten text rows, two kept: a page is eight rows, for the caret and the view alike.
    let mut s = state_wh(&numbered(60).replacen("line 5", "▮line 5", 1), 20, 11);
    s.view.config.page_overlap = 2;
    keys(&mut s, "<pgdn>");
    assert_eq!(
        s.doc.text.char_to_line(s.caret()),
        12,
        "the caret moved eight rows"
    );
    assert_eq!(
        s.view.scroll.line, 8,
        "lines 9 and 10, the last two before, are still on screen"
    );
    keys(&mut s, "<pgup>");
    assert_eq!(
        (s.doc.text.char_to_line(s.caret()), s.view.scroll.line),
        (4, 0)
    );
}

#[test]
fn the_view_follows_the_caret_with_a_margin() {
    let mut s = state_wh(&format!("▮{}", numbered(40)), 20, 11);
    for _ in 0..9 {
        keys(&mut s, "<down>");
    }
    // Ten text rows, a two-row margin: line 10 sits on row 7, so the view scrolled by 2.
    assert_eq!(s.view.scroll.line, 2);
    assert_eq!(cursor(&s), Some((0, 7)));
    keys(&mut s, "<d-down>");
    assert_eq!(frame(&s).lines().nth(9).unwrap(), "line 40");
    keys(&mut s, "<d-up>");
    assert_eq!(s.view.scroll.line, 0);
}

#[test]
fn wheel_scrolling_drags_the_caret_along() {
    let mut s = state_wh(&format!("▮{}", numbered(40)), 20, 11);
    send(&mut s, [Msg::Scroll { rows: 5 }]);
    assert_eq!(s.view.scroll.line, 5);
    // The caret moved down to stay two rows inside the view.
    assert_eq!(s.doc.text.char_to_line(s.caret()), 7);
    send(&mut s, [Msg::Scroll { rows: 100 }]);
    assert_eq!(
        s.view.scroll.line, 30,
        "stops with the last line at the bottom"
    );
}

#[test]
fn clicking_below_the_text_goes_to_the_end() {
    let mut s = state_wh("ab\n▮cd", 20, 10);
    send(
        &mut s,
        [Msg::Click {
            col: 1,
            row: 8,
            extend: false,
        }],
    );
    assert_eq!(show(&s), "ab\ncd▮");
    send(
        &mut s,
        [Msg::Click {
            col: 15,
            row: 0,
            extend: false,
        }],
    );
    assert_eq!(show(&s), "ab▮\ncd");
}

// ---------------------------------------------------------------------------------------
// The view stays: it moves only when asked (a scroll, a page) or when the caret would
// otherwise leave it. An edit that shortens the document leaves empty rows below the end;
// a click keeps the text under the pointer, whatever the margin.

use caretline::helix::Selection;
use caretline::outline::markdown;
use caretline::{
    update_doc, By, Dir, Document, Follow, OutlineConfig, OutlineLayout, Scroll, State, View,
    Viewport,
};

/// `l0`, `l1`, … `l{n-1}`, one per line.
fn short_lines(n: usize) -> String {
    (0..n)
        .map(|i| format!("l{i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn mv(dir: Dir, by: By) -> Msg {
    Msg::Move {
        dir,
        by,
        extend: false,
    }
}

fn click(row: u16) -> Msg {
    Msg::Click {
        col: 1,
        row,
        extend: false,
    }
}

fn line_of(s: &State) -> usize {
    s.doc.text.char_to_line(s.caret())
}

/// The caret at the start of line `line`, the view's top at line `top`.
fn place(s: &mut State, line: usize, top: usize) {
    s.view.selection = Selection::point(s.doc.text.line_to_char(line));
    s.view.scroll = Scroll {
        line: top,
        ..Scroll::default()
    };
}

thread_local! {
    /// Whether `sendv` and `send_doc` follow every message with passive ones.
    static BETWEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Messages that never move a view: the clock, a host value's operation (no reducer: it only
/// sets the status), a status line. A live runtime sends a `tick` before every batch.
fn passive() -> [Msg; 3] {
    [
        Msg::Tick { now_ms: 1 },
        Msg::Ext {
            key: "nobody".into(),
            op: serde_json::Value::Null,
        },
        Msg::ShowStatus { text: "hi".into() },
    ]
}

/// `send`, each message followed by the passive ones when `BETWEEN` is set.
fn sendv(s: &mut State, msgs: impl IntoIterator<Item = Msg>) {
    for msg in msgs {
        send(s, [msg]);
        if BETWEEN.get() {
            send(s, passive());
        }
    }
}

/// `update_doc`, followed by the passive messages through every view when `BETWEEN` is set.
fn send_doc(doc: &mut Document, views: &mut [View], acting: usize, msg: Msg) {
    update_doc(doc, views, acting, msg);
    if BETWEEN.get() {
        for v in 0..views.len() {
            for p in passive() {
                update_doc(doc, views, v, p);
            }
        }
    }
}

/// Runs a case as written, then again with passive messages after every message: they must
/// change nothing it checks.
fn twice(case: fn()) {
    case();
    BETWEEN.set(true);
    case();
    BETWEEN.set(false);
}

#[test]
fn v01_an_edit_that_shortens_the_document_keeps_the_view() {
    twice(|| {
        // Ten text rows, no margin, the caret at the end: the view shows lines 20 to 29.
        let mut s = state_wh(&format!("{}▮", short_lines(30)), 20, 11);
        s.view.config.scrolloff = 0;
        assert_eq!(s.view.scroll.line, 20);
        sendv(&mut s, (0..5).map(|_| Msg::InsertNewline));
        assert_eq!((line_of(&s), s.view.scroll.line), (34, 25));
        sendv(&mut s, (0..3).map(|_| mv(Dir::Backward, By::Line)));
        assert_eq!((line_of(&s), s.view.scroll.line), (31, 25));
        sendv(&mut s, [Msg::DeleteBackward]);
        assert_eq!(
            (line_of(&s), s.view.scroll.line),
            (30, 25),
            "an empty row below the end, as in VS Code and Sublime"
        );
        sendv(&mut s, (0..4).map(|_| Msg::DeleteBackward));
        assert_eq!(
            (line_of(&s), s.view.scroll.line),
            (29, 25),
            "nor does any further Backspace move it"
        );
        // A view moved to bring the caret into sight still keeps off empty rows.
        send(
            &mut s,
            [
                mv(Dir::Backward, By::DocStart),
                mv(Dir::Forward, By::DocEnd),
            ],
        );
        assert_eq!((line_of(&s), s.view.scroll.line), (32, 23));
    });
}

#[test]
fn v02_a_click_on_the_margin_rows_keeps_the_view() {
    twice(|| {
        // Twenty text rows, a two-row margin, the view from line 40.
        for (row, line) in [(0, 40), (1, 41), (18, 58), (19, 59)] {
            let mut s = state_wh(&format!("▮{}", numbered(100)), 20, 21);
            place(&mut s, 50, 40);
            sendv(&mut s, [click(row)]);
            assert_eq!(
                (line_of(&s), s.view.scroll.line),
                (line, 40),
                "a click on row {row}"
            );
        }
        // A key after it scrolls as the margin says.
        let mut s = state_wh(&format!("▮{}", numbered(100)), 20, 21);
        place(&mut s, 50, 40);
        sendv(&mut s, [click(0), mv(Dir::Backward, By::Line)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (39, 37));
        place(&mut s, 50, 40);
        sendv(&mut s, [click(19), mv(Dir::Forward, By::Line)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (60, 43));
    });
}

#[test]
fn v03_a_shift_click_double_click_or_click_in_a_free_view_keeps_the_view() {
    twice(|| {
        let mut s = state_wh(&format!("▮{}", numbered(100)), 20, 21);
        place(&mut s, 50, 40);
        let shift = |row| Msg::Click {
            col: 3,
            row,
            extend: true,
        };
        sendv(&mut s, [click(10), shift(19)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (59, 40));
        sendv(&mut s, [shift(0)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (40, 40));
        // Below the text rows, the view follows the caret as it always has.
        sendv(&mut s, [shift(20)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (60, 43));
        assert_eq!(
            s.view.selection.primary().anchor,
            s.doc.text.line_to_char(50) + 1
        );
        // A double-click on the top row.
        place(&mut s, 50, 40);
        let pos = s.doc.text.line_to_char(41) + 2;
        sendv(&mut s, [Msg::SelectWordAt { pos }]);
        assert_eq!((line_of(&s), s.view.scroll.line), (41, 40));
        // A view scrolled with the wheel (the caret left behind) takes a click where it is.
        sendv(&mut s, [Msg::ScrollView { rows: 30 }]);
        assert_eq!(s.view.scroll.line, 70);
        sendv(&mut s, [click(19)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (89, 70));
    });
}

/// Lines of 24 chars, two rows each at width 20.
fn wrapped_lines(n: usize) -> String {
    (0..n)
        .map(|i| format!("w{i:02} word word word word"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn v04_wrapped_rows_keep_the_view() {
    twice(|| {
        // The view starts on the second row of line 20.
        let mut s = state_wh(&format!("▮{}", wrapped_lines(60)), 20, 11);
        assert_eq!(caretline::layout::Layout::new(&s).line_rows(20), 2);
        for row in [0, 1, 8, 9] {
            place(&mut s, 23, 20);
            s.view.scroll.row = 1;
            let top = s.view.scroll;
            sendv(&mut s, [click(row)]);
            assert_eq!(s.view.scroll, top, "a click on row {row}");
            if row == 0 {
                assert_eq!(line_of(&s), 20);
            }
        }
        // An edit that shortens the document, at its end.
        let mut s = state_wh(&format!("{}▮", wrapped_lines(15)), 20, 11);
        s.view.config.scrolloff = 0;
        sendv(&mut s, (0..3).map(|_| Msg::InsertNewline));
        sendv(&mut s, [mv(Dir::Backward, By::Line)]);
        let top = s.view.scroll;
        assert_eq!(top.row, 1, "the view starts inside a wrapped line");
        sendv(&mut s, [Msg::DeleteBackward]);
        assert_eq!(s.view.scroll, top);
    });
}

/// An outline: a folded list, then forty paragraphs (a blank row before each), one with a
/// host's row after it, laid out in columns.
fn outline(w: u16, h: u16) -> State {
    let mut md = String::from("- parent\n  - child a\n  - child b\n\n");
    for i in 0..40 {
        md.push_str(&format!("Paragraph {i}\n\n"));
    }
    let mut s = markdown::load(
        &md,
        None,
        Viewport {
            width: w,
            height: h,
        },
        OutlineConfig::default(),
    );
    s.view.config.status_bar = false;
    let ids: Vec<_> = s
        .doc
        .blocks()
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.id)
        .collect();
    let mut layout = OutlineLayout::default();
    layout.extra_rows.insert(ids[30], 1);
    s.view.layout = Some(layout);
    sendv(&mut s, [Msg::Fold { id: ids[0] }]);
    sendv(&mut s, [Msg::resize(w, h)]);
    s
}

#[test]
fn v05_an_outline_keeps_the_view() {
    twice(|| {
        let mut s = outline(40, 20);
        let line = |s: &State, needle: &str| {
            let text = s.doc.text.to_string();
            s.doc.text.char_to_line(text.find(needle).unwrap())
        };
        let (caret, top) = (line(&s, "Paragraph 25"), line(&s, "Paragraph 20"));
        // Row 0 is Paragraph 20's blank row, 18 Paragraph 28's text (27 has a host's row).
        for row in [0, 1, 18] {
            place(&mut s, caret, top);
            sendv(&mut s, [click(row)]);
            assert_eq!(s.view.scroll.line, top, "a click on row {row}");
        }
        // Row 19 is the blank row before Paragraph 29, whose text is below the view: the caret
        // goes there and the view follows it.
        place(&mut s, caret, top);
        sendv(&mut s, [click(19)]);
        assert_eq!(line_of(&s), line(&s, "Paragraph 29"));
        assert_eq!(s.view.scroll.line, line(&s, "Paragraph 21"));
        // At the end, joining the last paragraph to the one before shortens the document.
        send(
            &mut s,
            [
                mv(Dir::Forward, By::DocEnd),
                mv(Dir::Backward, By::LineStart),
            ],
        );
        let top = s.view.scroll;
        let rows = caretline::layout::Layout::new(&s).end();
        sendv(&mut s, [Msg::DeleteBackward]);
        assert_ne!(caretline::layout::Layout::new(&s).end(), rows);
        assert_eq!(s.view.scroll, top);
    });
}

#[test]
fn v06_two_views_each_keep_their_own() {
    twice(|| {
        let mut doc = Document::new(&short_lines(30), None);
        let end = doc.text.len_chars();
        let mut views = [0, 1].map(|_| {
            let mut v = View::new(Viewport {
                width: 20,
                height: 11,
            });
            v.config.scrolloff = 0;
            v.selection = Selection::point(end);
            v
        });
        for i in 0..2 {
            send_doc(&mut doc, &mut views, i, Msg::resize(20, 11));
        }
        let send1 = |doc: &mut Document, views: &mut [View], msg| send_doc(doc, views, 1, msg);
        for _ in 0..5 {
            send1(&mut doc, &mut views, Msg::InsertNewline);
        }
        for _ in 0..3 {
            send1(&mut doc, &mut views, mv(Dir::Backward, By::Line));
        }
        assert_eq!(views[1].scroll.line, 25);
        send1(&mut doc, &mut views, Msg::DeleteBackward);
        assert_eq!(views[1].scroll.line, 25);
        // Clicks through view 0 keep view 0, and leave view 1 alone.
        let top0 = views[0].scroll;
        for row in [0, 9] {
            send_doc(&mut doc, &mut views, 0, click(row));
            assert_eq!(views[0].scroll, top0, "a click on row {row}");
        }
        assert_eq!(views[1].scroll.line, 25);
    });
}

#[test]
fn v07_typewriter_centres_on_keys_not_on_clicks() {
    twice(|| {
        // Twenty text rows, the caret's row at half: row 10.
        let mut s = state_wh(&format!("▮{}", numbered(100)), 20, 21);
        s.view.config.follow = Follow::Typewriter { percent: 50 };
        place(&mut s, 50, 40);
        sendv(&mut s, [click(0)]);
        assert_eq!(
            (line_of(&s), s.view.scroll.line),
            (40, 40),
            "the text stays under the pointer"
        );
        sendv(&mut s, [mv(Dir::Forward, By::Line)]);
        assert_eq!(
            (line_of(&s), s.view.scroll.line),
            (41, 31),
            "a key centres it"
        );
    });
}

fn drag(row: u16) -> Msg {
    Msg::Drag { col: 3, row }
}

#[test]
fn v08_a_drag_on_an_edge_row_scrolls_one_row_a_move() {
    twice(|| {
        let mut s = state_wh(&format!("▮{}", numbered(100)), 20, 21);
        place(&mut s, 50, 40);
        let anchor = s.doc.text.line_to_char(50) + 1;
        sendv(&mut s, [click(10), drag(5)]);
        assert_eq!(
            (line_of(&s), s.view.scroll.line),
            (45, 40),
            "inside: no scroll"
        );
        sendv(&mut s, [drag(0)]);
        assert_eq!(
            (line_of(&s), s.view.scroll.line),
            (39, 39),
            "up a row, to the row it shows"
        );
        sendv(&mut s, [drag(0), drag(0)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (37, 37));
        sendv(&mut s, [drag(19)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (57, 38), "down a row");
        sendv(&mut s, [drag(20), drag(25)]);
        assert_eq!(
            (line_of(&s), s.view.scroll.line),
            (59, 40),
            "past the last row, the same"
        );
        assert_eq!(s.view.selection.primary().anchor, anchor);
        // At the top and the end of the document there is nowhere to go.
        place(&mut s, 5, 0);
        sendv(&mut s, [drag(0)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (0, 0));
        place(&mut s, 90, 80);
        sendv(&mut s, [drag(19), drag(20)]);
        assert_eq!((line_of(&s), s.view.scroll.line), (99, 80));
    });
}

#[test]
fn v09_an_outline_drag_on_an_edge_row_scrolls_one_row() {
    twice(|| {
        let mut s = outline(40, 20);
        let line = |s: &State, needle: &str| {
            let text = s.doc.text.to_string();
            s.doc.text.char_to_line(text.find(needle).unwrap())
        };
        let (caret, top) = (line(&s, "Paragraph 25"), line(&s, "Paragraph 20"));
        place(&mut s, caret, top);
        // Row 0 is Paragraph 20's blank row: up one row shows Paragraph 19's text.
        sendv(&mut s, [click(5), drag(0)]);
        assert_eq!(line_of(&s), line(&s, "Paragraph 19"));
        assert_eq!(
            (s.view.scroll.line, s.view.scroll.row),
            (line(&s, "Paragraph 19"), 1)
        );
        place(&mut s, caret, top);
        // Row 19 is Paragraph 29's blank row: down one row shows its text.
        sendv(&mut s, [click(5), drag(19)]);
        assert_eq!(line_of(&s), line(&s, "Paragraph 29"));
        assert_eq!((s.view.scroll.line, s.view.scroll.row), (top, 1));
    });
}

fn tick(now_ms: u64) -> Msg {
    Msg::Tick { now_ms }
}

#[test]
fn v10_a_tick_after_a_click_in_the_margin_keeps_the_view() {
    // Five text rows, a two-row margin: row 4 is in the bottom margin.
    let mut s = state_wh(&format!("▮{}", numbered(100)), 20, 6);
    s.view.config.scrolloff = 2;
    place(&mut s, 42, 40);
    send(&mut s, [click(4)]);
    assert_eq!((line_of(&s), s.view.scroll.line), (44, 40));
    send(&mut s, [tick(10), tick(20)]);
    assert_eq!(
        (line_of(&s), s.view.scroll.line),
        (44, 40),
        "a tick never moves the view"
    );
    // Nor does a host value's operation, a status line, a save's result or a copy.
    send(
        &mut s,
        [
            Msg::Ext {
                key: "nobody".into(),
                op: serde_json::Value::Null,
            },
            Msg::ShowStatus { text: "hi".into() },
            Msg::Saved,
            Msg::Copy,
            Msg::FrameClock { fps: 60 },
            Msg::Frame { now_ms: 30 },
        ],
    );
    assert_eq!(s.view.scroll.line, 40);
    // A key does.
    send(&mut s, [mv(Dir::Forward, By::Grapheme)]);
    assert_eq!(s.view.scroll.line, 42);
}

#[test]
fn v11_drags_on_an_edge_row_with_ticks_between_scroll_one_row_each() {
    let mut s = state_wh(&format!("▮{}", numbered(100)), 20, 21);
    s.view.config.scrolloff = 2;
    place(&mut s, 50, 40);
    send(&mut s, [click(10), tick(0)]);
    for i in 1..=3 {
        send(&mut s, [tick(i * 16), drag(19), tick(i * 16 + 8)]);
    }
    assert_eq!((line_of(&s), s.view.scroll.line), (62, 43));
    for i in 4..=6 {
        send(&mut s, [tick(i * 16), drag(0), tick(i * 16 + 8)]);
    }
    assert_eq!((line_of(&s), s.view.scroll.line), (40, 40));
}

// ---------------------------------------------------------------------------------------
// Scrolling past the end (`ViewConfig::scroll_past_end`): the view may show empty rows below
// the document's last row, so the caret keeps its margin at the end too.

use caretline::layout::Layout;
use caretline::ScrollPastEnd;

/// `state_wh` with a margin and a `scroll_past_end`, settled around the caret.
fn past_end(notation: &str, w: u16, h: u16, so: u16, past: ScrollPastEnd) -> State {
    let mut s = state_wh(notation, w, h);
    s.view.config = s
        .view
        .config
        .clone()
        .with_scrolloff(so)
        .with_scroll_past_end(past);
    send(&mut s, [Msg::resize(w, h)]);
    s
}

/// The caret's row in the view, and the empty rows below the document's last row.
fn rows_below(s: &State) -> (isize, isize) {
    let layout = Layout::new(s);
    let h = s.text_rows();
    let top = layout.top(&s.view.scroll);
    let (caret, _) = layout.pos_coords(s.caret());
    let row = layout.rows_between(top, caret, h * 2);
    let last = layout.rows_between(top, layout.end(), h * 2);
    (row, h as isize - 1 - last)
}

#[test]
fn v12_writing_at_the_end_keeps_the_margin_with_scroll_past_end() {
    twice(|| {
        // A hundred lines, twenty text rows, a three-row margin, the caret at the end.
        let end = format!("{}▮", numbered(100));
        let mut off = past_end(&end, 20, 21, 3, ScrollPastEnd::Off);
        assert_eq!((off.view.scroll.line, rows_below(&off)), (80, (19, 0)));
        sendv(&mut off, (0..5).map(|_| Msg::InsertNewline));
        assert_eq!(
            (off.view.scroll.line, rows_below(&off)),
            (85, (19, 0)),
            "off: the caret on the last row, the page scrolling under it"
        );

        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Margin);
        assert_eq!((s.view.scroll.line, rows_below(&s)), (83, (16, 3)));
        for i in 1..=5 {
            sendv(&mut s, [Msg::InsertNewline]);
            assert_eq!(
                (s.view.scroll.line, rows_below(&s)),
                (83 + i, (16, 3)),
                "margin: one row a line, three empty rows below"
            );
        }
        assert_eq!(cursor(&s), Some((0, 16)));
        let f = frame(&s);
        let rows: Vec<&str> = f.lines().collect();
        assert_eq!(rows[11], "line 100");
        assert!(rows[12..20].iter().all(|r| r.trim().is_empty()));
        // A jump to the end, from the start, stops with the same margin.
        sendv(
            &mut s,
            [
                mv(Dir::Backward, By::DocStart),
                mv(Dir::Forward, By::DocEnd),
            ],
        );
        assert_eq!((s.view.scroll.line, rows_below(&s)), (88, (16, 3)));
    });
}

#[test]
fn v13_rows_and_half_bound_scrolling_past_the_end() {
    twice(|| {
        let end = format!("{}▮", numbered(100));
        // More rows than the margin: following keeps the margin, the wheel goes further.
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Rows(5));
        assert_eq!(rows_below(&s), (16, 3));
        sendv(&mut s, [Msg::InsertNewline]);
        assert_eq!(rows_below(&s), (16, 3));
        sendv(&mut s, [Msg::ScrollView { rows: 100 }]);
        assert_eq!(rows_below(&s), (14, 5), "the wheel stops five rows past");
        sendv(&mut s, [Msg::ScrollView { rows: 1 }]);
        assert_eq!(rows_below(&s), (14, 5), "and no further");
        // Fewer rows than the margin: the caret at the end sits that many rows up.
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Rows(1));
        assert_eq!(rows_below(&s), (18, 1));
        sendv(&mut s, [Msg::InsertNewline]);
        assert_eq!(rows_below(&s), (18, 1));
        // A huge count is capped: the last row stays in sight.
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Rows(500));
        sendv(&mut s, [Msg::ScrollView { rows: 500 }]);
        assert_eq!(rows_below(&s), (0, 19));
        // Half the view: ten rows. `Scroll` drags the caret to the end, a page stops there.
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Half);
        sendv(&mut s, [Msg::Scroll { rows: 100 }]);
        assert_eq!(rows_below(&s), (9, 10));
        let start = format!("▮{}", numbered(100));
        let mut s = past_end(&start, 20, 21, 3, ScrollPastEnd::Half);
        sendv(&mut s, (0..6).map(|_| mv(Dir::Forward, By::Page)));
        assert_eq!(line_of(&s), 99);
        assert_eq!(
            rows_below(&s).1,
            10,
            "a page goes ten rows past, no further"
        );
        sendv(&mut s, [mv(Dir::Forward, By::Page)]);
        assert_eq!(rows_below(&s).1, 10);
        // Off, the wheel stops with the last row at the bottom, as before.
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Off);
        sendv(&mut s, [Msg::ScrollView { rows: 100 }]);
        assert_eq!(rows_below(&s), (19, 0));
    });
}

#[test]
fn v14_typewriter_ignores_scroll_past_end() {
    twice(|| {
        let end = format!("{}▮", numbered(100));
        let mut tops = vec![];
        for past in [
            ScrollPastEnd::Off,
            ScrollPastEnd::Margin,
            ScrollPastEnd::Half,
        ] {
            let mut s = past_end(&end, 20, 21, 3, past);
            s.view.config.follow = Follow::Typewriter { percent: 50 };
            sendv(&mut s, [Msg::InsertNewline, mv(Dir::Backward, By::Line)]);
            sendv(&mut s, (0..3).map(|_| Msg::InsertNewline));
            tops.push((s.view.scroll, rows_below(&s).0));
        }
        assert!(tops.iter().all(|t| *t == tops[0]), "{tops:?}");
        assert_eq!(tops[0].1, 10);
    });
}

#[test]
fn v15_a_resize_keeps_the_caret_and_the_margin() {
    twice(|| {
        let end = format!("{}▮", numbered(100));
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Margin);
        assert_eq!((s.view.scroll.line, rows_below(&s)), (83, (16, 3)));
        // Taller: the caret is in place, the view stays (more empty rows, none taken away).
        sendv(&mut s, [Msg::resize(20, 25)]);
        assert_eq!((s.view.scroll.line, rows_below(&s)), (83, (16, 7)));
        // Shorter: the caret would sit in the margin, the view follows it, three rows past.
        sendv(&mut s, [Msg::resize(20, 16)]);
        assert_eq!((s.view.scroll.line, rows_below(&s)), (88, (11, 3)));
    });
}

#[test]
fn v16_wrapped_rows_keep_the_margin_past_the_end() {
    twice(|| {
        // Lines two rows each, ten text rows, a two-row margin.
        let end = format!("{}▮", wrapped_lines(30));
        let mut s = past_end(&end, 20, 11, 2, ScrollPastEnd::Margin);
        assert_eq!(rows_below(&s), (7, 2));
        // Typing wraps the last line onto a third row: the view follows by one row.
        let top = s.view.scroll;
        sendv(
            &mut s,
            [Msg::InsertText {
                text: " and some more words".into(),
            }],
        );
        assert_eq!(Layout::new(&s).line_rows(29), 3);
        assert_eq!(rows_below(&s), (7, 2));
        assert_ne!(s.view.scroll, top);
        sendv(&mut s, [Msg::InsertNewline]);
        assert_eq!(rows_below(&s), (7, 2));
        // Backspace pulls nothing back.
        let top = s.view.scroll;
        sendv(&mut s, [Msg::DeleteBackward]);
        assert_eq!(s.view.scroll, top);
        assert_eq!(rows_below(&s), (6, 3));
    });
}

#[test]
fn v17_an_outline_keeps_the_margin_past_the_end() {
    twice(|| {
        // Twenty text rows, the default two-row margin, a fold and a host's row above.
        let mut s = outline(40, 20);
        s.view.config.scroll_past_end = ScrollPastEnd::Margin;
        sendv(&mut s, [mv(Dir::Forward, By::DocEnd)]);
        assert_eq!(rows_below(&s), (17, 2));
        // A new paragraph (a blank row before it): the view follows, the margin stays.
        sendv(&mut s, [Msg::InsertNewline]);
        assert_eq!(rows_below(&s), (17, 2));
        let top = s.view.scroll;
        sendv(&mut s, [Msg::DeleteBackward]);
        assert_eq!(s.view.scroll, top, "joining it back pulls nothing back");
        // Off, the same document's caret sits on the last row.
        let mut s = outline(40, 20);
        sendv(&mut s, [mv(Dir::Forward, By::DocEnd)]);
        assert_eq!(rows_below(&s), (19, 0));
    });
}

#[test]
fn v18_backspace_at_the_end_past_it_keeps_the_view() {
    twice(|| {
        let end = format!("{}▮", numbered(100));
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Margin);
        sendv(&mut s, (0..5).map(|_| Msg::InsertNewline));
        assert_eq!((s.view.scroll.line, rows_below(&s)), (88, (16, 3)));
        for i in 1..=5 {
            sendv(&mut s, [Msg::DeleteBackward]);
            assert_eq!(
                (s.view.scroll.line, rows_below(&s)),
                (88, (16 - i, 3 + i)),
                "Backspace {i} pulls nothing back"
            );
        }
        // Typing again on the same line doesn't move it either.
        sendv(&mut s, [Msg::InsertText { text: "x".into() }]);
        assert_eq!((s.view.scroll.line, rows_below(&s)), (88, (11, 8)));
    });
}

#[test]
fn v19_clicks_and_ticks_past_the_end_keep_the_view() {
    twice(|| {
        let end = format!("{}▮", numbered(100));
        let mut s = past_end(&end, 20, 21, 3, ScrollPastEnd::Margin);
        assert_eq!(s.view.scroll.line, 83);
        // On text in the margins, and on the empty rows past the end (the caret goes to the
        // end).
        for row in [0, 1, 15, 16, 17, 19] {
            sendv(&mut s, [click(row)]);
            assert_eq!(s.view.scroll.line, 83, "a click on row {row}");
            send(&mut s, [tick(5), tick(10)]);
            assert_eq!(s.view.scroll.line, 83, "a tick after a click on row {row}");
        }
        assert_eq!(line_of(&s), 99);
        // A key that leaves the caret in place keeps it too.
        sendv(&mut s, [mv(Dir::Backward, By::Grapheme)]);
        assert_eq!(s.view.scroll.line, 83);
    });
}

#[test]
fn v20_scroll_past_end_in_json() {
    let mut s = state_wh("a▮", 20, 5);
    let json = serde_json::to_value(&s).unwrap();
    assert!(
        json["config"].get("scroll_past_end").is_none(),
        "off is left out"
    );
    for (past, value) in [
        (ScrollPastEnd::Margin, serde_json::json!("margin")),
        (ScrollPastEnd::Half, serde_json::json!("half")),
        (ScrollPastEnd::Rows(3), serde_json::json!({"rows": 3})),
    ] {
        s.view.config.scroll_past_end = past;
        let mut json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["config"]["scroll_past_end"], value);
        let back: State = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back.view.config.scroll_past_end, past);
        json["config"]
            .as_object_mut()
            .unwrap()
            .remove("scroll_past_end");
        let back: State = serde_json::from_value(json).unwrap();
        assert_eq!(back.view.config.scroll_past_end, ScrollPastEnd::Off);
    }
}

// ---------------------------------------------------------------------------------------
// Status bar

#[test]
fn status_bar_shows_name_dirty_marker_and_position() {
    let mut s = state_wh("ab\nc▮d", 30, 3);
    assert_eq!(
        frame(&s).lines().nth(2).unwrap(),
        " test.md                  2:2"
    );
    keys(&mut s, "x");
    assert_eq!(
        frame(&s).lines().nth(2).unwrap(),
        " test.md [+]              2:3"
    );
    keys(&mut s, "<s-left><s-left>");
    assert_eq!(
        frame(&s).lines().nth(2).unwrap(),
        " test.md [+]       2 sel  2:1"
    );
    keys(&mut s, "<c-c>");
    assert_eq!(
        frame(&s).lines().nth(2).unwrap(),
        " test.md [+]  copi 2 sel  2:1"
    );
}

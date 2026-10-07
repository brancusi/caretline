//! One-line documents (`config.single_line`): a text field. The text never holds a line
//! break: Enter changes nothing; a run of breaks typed, pasted, edited in or put in from
//! elsewhere becomes one space where it lands, or nothing at the line's start or end or next
//! to whitespace; lines never wrap; Up and Down go to the start and the end.

mod common;

use caretline::helix::{Range, Selection};
use caretline::outline::markdown;
use caretline::{update, Edit, ExtChange, Host, Msg, OutlineConfig, Session, State, Viewport};
use common::*;
use serde_json::{json, Value};

/// A one-line field from notation, `width` columns, one row, no status bar.
fn field_wh(notation: &str, width: u16) -> State {
    let mut s = state_wh(notation, width, 1);
    s.doc.config.single_line = true;
    s.view.config.status_bar = false;
    s.sanitize();
    update(&mut s, Msg::Resize { width, height: 1 });
    s
}

fn field(notation: &str) -> State {
    field_wh(notation, 40)
}

#[track_caller]
fn field_golden(before: &str, script: &str, after: &str) {
    let mut s = field(before);
    keys(&mut s, script);
    assert_eq!(show(&s), after, "{before:?} · {script:?}");
}

fn text(s: &State) -> String {
    s.doc.text.to_string()
}

// ---------------------------------------------------------------------------------------
// Enter

#[test]
fn s01_enter_changes_nothing() {
    let mut s = field("ab▮");
    keys(&mut s, "c<cr>");
    assert_eq!(show(&s), "abc▮");
    // Nothing happened: the typing run is still one undo step.
    keys(&mut s, "d<c-z>");
    assert_eq!(show(&s), "ab▮");
}

#[test]
fn s02_enter_keeps_a_selection() {
    field_golden("a⟦bc▮⟧d", "<cr>", "a⟦bc▮⟧d");
    let mut s = field("a⟦bc▮⟧d");
    send(&mut s, [Msg::SoftBreak, Msg::InsertNewline]);
    assert_eq!(show(&s), "a⟦bc▮⟧d");
    assert!(!s.doc.dirty);
}

// ---------------------------------------------------------------------------------------
// Typed and pasted text

#[test]
fn s03_pasted_lines_join_with_spaces() {
    // Before more text, the last break is a space too: a break never joins two words.
    let mut s = field("a ▮b");
    send(&mut s, [Msg::Paste { text: Some("one\ntwo\nthree\n".into()) }]);
    assert_eq!(show(&s), "a one two three ▮b");
    // At the end of the line, it is dropped.
    let mut s = field("x ▮");
    send(&mut s, [Msg::Paste { text: Some("foo\n".into()) }]);
    assert_eq!(show(&s), "x foo▮");
    // At the start of the line, a leading break too.
    let mut s = field("▮one");
    send(&mut s, [Msg::Paste { text: Some("\nand\n".into()) }]);
    assert_eq!(show(&s), "and ▮one");
}

#[test]
fn s04_crlf_counts_once() {
    let mut s = field("▮");
    send(&mut s, [Msg::Paste { text: Some("x\r\ny\rz\r\n\r\n".into()) }]);
    assert_eq!(show(&s), "x y z▮");
    send(&mut s, [Msg::InsertText { text: " p\r\nq".into() }]);
    assert_eq!(show(&s), "x y z p q▮");
    // Blank lines inside: a run of breaks is one space.
    send(&mut s, [Msg::InsertText { text: "\n\nr".into() }]);
    assert_eq!(show(&s), "x y z p q r▮");
    // Next to a space, a break adds none: no doubled spaces.
    send(&mut s, [Msg::InsertText { text: " \ns\n ".into() }]);
    assert_eq!(show(&s), "x y z p q r s ▮");
}

#[test]
fn s05_pasting_only_line_breaks() {
    // At the end of the line: nothing.
    let mut s = field("ab▮");
    send(&mut s, [Msg::Paste { text: Some("\r\n\n".into()) }, Msg::InsertText { text: "\n".into() }]);
    assert_eq!(show(&s), "ab▮");
    assert!(!s.doc.dirty);
    // Between two words: one space, over the selection like any paste.
    let mut s = field("a⟦b▮⟧c");
    send(&mut s, [Msg::Paste { text: Some("\r\n\n".into()) }]);
    assert_eq!(show(&s), "a ▮c");
    // Next to that space: nothing more.
    send(&mut s, [Msg::InsertText { text: "\n".into() }]);
    assert_eq!(show(&s), "a ▮c");
}

#[test]
fn s06_undo_and_redo_across_a_flattening_paste() {
    let mut s = field("say ▮");
    keys(&mut s, "hi<wait:2000>");
    send(&mut s, [Msg::Paste { text: Some(" there\nfriend\n".into()) }]);
    assert_eq!(show(&s), "say hi there friend▮");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "say hi▮");
    keys(&mut s, "<c-s-z>");
    assert_eq!(show(&s), "say hi there friend▮");
    keys(&mut s, "<c-z><c-z>");
    assert_eq!(show(&s), "say ▮");
    assert!(!s.doc.dirty);
}

#[test]
fn s07_multi_cursor_typing_on_one_line() {
    let mut s = field("ab▮cd");
    s.view.selection = Selection::new([Range::point(1), Range::point(3)].into_iter().collect(), 0);
    keys(&mut s, "x<cr>y");
    assert_eq!(text(&s), "axybcxyd");
    send(&mut s, [Msg::Paste { text: Some("1\n2\n".into()) }]);
    assert_eq!(text(&s), "axy1 2 bcxy1 2 d");
    let carets: Vec<usize> = s.view.selection.iter().map(|r| r.head).collect();
    assert_eq!(carets, vec![7, 15]);
}

#[test]
fn s08_a_hosts_edit_and_input_rule_are_flattened_too() {
    let mut s = field("ab▮");
    send(&mut s, [Msg::Edit { changes: vec![(1, 1, "\nX\r\n".into())], join: false }]);
    assert_eq!(text(&s), "a X b");
    // An input rule that puts a line break in, with its own selection after it.
    s.doc.set_host(Host::new().input_rule("test.lines", |ctx, msg| {
        let Msg::InsertText { text } = msg else { return None };
        let p = ctx.caret();
        (text == "|").then(|| Edit {
            changes: vec![(p, p, "1\r\n2".into())],
            selection: Some(Selection::point(p + 4)),
            ..Edit::default()
        })
    }));
    keys(&mut s, "<end>|");
    assert_eq!(show(&s), "a X b1 2▮");
}

// ---------------------------------------------------------------------------------------
// Changes from elsewhere and loading

#[test]
fn s09_an_external_change_with_line_breaks_is_flattened_outside_undo() {
    let mut s = field("one▮");
    keys(&mut s, " two");
    send(&mut s, [Msg::External { changes: vec![ExtChange::Replace { from: 0, to: 0, text: "zero\r\nand\n".into() }] }]);
    // The break before "one" is a space: it never joins two words.
    assert_eq!(show(&s), "zero and one two▮");
    // Undo takes back only the local typing.
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "zero and one▮");
    // A break put in at the end of the line is dropped.
    send(&mut s, [Msg::External { changes: vec![ExtChange::Replace { from: 12, to: 12, text: "!\n".into() }] }]);
    assert_eq!(text(&s), "zero and one!");

    // text.set: the whole new text is flattened, then diffed.
    let mut session = Session::new(field("abc▮"));
    session.set_text("a\nb\r\nc\n");
    assert_eq!(text(session.state()), "a b c");
}

#[test]
fn s10_a_state_with_line_breaks_loads_on_one_line() {
    let json = r#"{"text": "first\r\nsecond\nthird\n", "selection": {"ranges": [{"anchor": 8, "head": 8}]},
                  "config": {"single_line": true}}"#;
    let s = State::from_json(json).unwrap();
    assert_eq!(text(&s), "first second third");
    // The caret stays on its char ("c" of "second", one char earlier for the CRLF).
    assert_eq!(s.caret(), 7);
    assert!(s.doc.dirty, "the text differs from the saved one");
    assert!(s.doc.history.is_empty());
    // Repaired once: loading the repaired state changes nothing.
    assert_eq!(State::from_json(&s.to_json()).unwrap(), s);
}

#[test]
fn s11_turning_it_on_flattens_and_drops_an_undo_that_could_break_the_line() {
    let mut s = state("ab▮");
    keys(&mut s, "<cr>c<c-z><c-z>");
    assert_eq!(text(&s), "ab");
    // The history can redo a line break: it starts again.
    s.doc.config.single_line = true;
    s.sanitize();
    assert!(s.doc.history.is_empty());
    keys(&mut s, "<c-s-z>");
    assert_eq!(text(&s), "ab");
    assert!(!s.doc.dirty, "the text did not change: still clean");
}

#[test]
fn s12_an_outline_document_is_never_one_line() {
    let mut s = markdown::load("- one\n- two\n", None, Viewport { width: 30, height: 5 }, OutlineConfig::default());
    let before = text(&s);
    s.doc.config.single_line = true;
    s.sanitize();
    assert!(!s.doc.config.single_line);
    assert_eq!(text(&s), before);
    assert!(before.contains('\n'));
}

// ---------------------------------------------------------------------------------------
// Motion

#[test]
fn s13_up_and_down_go_to_the_start_and_end() {
    field_golden("ab▮cd", "<up>", "▮abcd");
    field_golden("ab▮cd", "<down>", "abcd▮");
    field_golden("ab▮cd", "<s-up>", "⟦▮ab⟧cd");
    field_golden("ab▮cd", "<s-down>", "ab⟦cd▮⟧");
    field_golden("ab▮cd", "<pgup>", "▮abcd");
    field_golden("ab▮cd", "<pgdn>", "abcd▮");
    field_golden("ab▮cd", "<s-pgdn>", "ab⟦cd▮⟧");
    // A selection: Up goes to the start (not the selection's edge), as in a text field.
    field_golden("a⟦bc▮⟧d", "<up>", "▮abcd");
    field_golden("a⟦bc▮⟧d", "<s-up>", "⟦▮a⟧bcd");
}

// ---------------------------------------------------------------------------------------
// Layout

#[test]
fn s14_a_long_line_scrolls_sideways_and_never_wraps() {
    let mut s = field_wh("▮", 12);
    keys(&mut s, "the quick brown fox");
    assert_eq!(frame(&s).trim_end(), "k brown fox");
    assert_eq!(s.view.scroll.col, 8);
    assert_eq!(cursor(&s), Some((11, 0)));
    keys(&mut s, "<home>");
    assert_eq!(frame(&s).trim_end(), "the quick br");
    assert_eq!(cursor(&s), Some((0, 0)));

    // Soft wrap is on in the config, but a one-line document draws one row.
    let mut s = field_wh("▮", 20);
    s.view.viewport.height = 3;
    keys(&mut s, "a line that is longer than twenty columns");
    let f = frame(&s);
    let rows: Vec<&str> = f.lines().collect();
    assert_eq!(rows[0].trim_end(), "than twenty columns");
    assert_eq!(rows[1].trim(), "");
    assert!(s.doc.config.soft_wrap);
}

// ---------------------------------------------------------------------------------------
// The setting itself

#[test]
fn s15_the_setting_serializes_only_when_on() {
    let s = field("x▮");
    let v: Value = serde_json::from_str(&s.to_json()).unwrap();
    assert_eq!(v["config"]["single_line"], true);
    let plain: Value = serde_json::from_str(&state("x▮").to_json()).unwrap();
    assert!(plain["config"].get("single_line").is_none());
    assert_eq!(State::from_json(&s.to_json()).unwrap(), s);
}

#[test]
fn s16_over_the_protocol() {
    let mut s = Session::new(State::new("", None, Viewport { width: 20, height: 1 }));
    let r = s.handle(
        &json!({"op": "state.set", "state": {"text": "a\nb", "config": {"single_line": true, "status_bar": false}}}).to_string(),
        None,
    );
    let r: Value = serde_json::from_str(&r.response).unwrap();
    assert!(r["result"]["rev"].is_u64(), "{r}");
    assert_eq!(text(s.state()), "a b");
    let r = s.handle(
        &json!({"op": "msgs", "msgs": [
            {"msg": "move", "dir": "forward", "by": "doc_end"},
            {"msg": "insert_newline"},
            {"msg": "insert_text", "text": "\nc\r\nd\n"},
            {"msg": "move", "dir": "backward", "by": "line", "extend": true}
        ]})
        .to_string(),
        None,
    );
    let r: Value = serde_json::from_str(&r.response).unwrap();
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(text(s.state()), "a b c d");
    assert_eq!(show(s.state()), "⟦▮a b c d⟧");
    let st = s.handle(&json!({"op": "state.get"}).to_string(), None);
    let st: Value = serde_json::from_str(&st.response).unwrap();
    assert_eq!(st["result"]["state"]["config"]["single_line"], true, "{st}");
}

// ---------------------------------------------------------------------------------------
// Property

#[test]
fn s17_random_sessions_never_break_the_line() {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    for seed in 0..8 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut s = State::new(&gen::text(&mut rng), None, Viewport { width: 30, height: 3 });
        s.doc.config.single_line = true;
        s.sanitize();
        for step in 0..400 {
            if rng.random_bool(0.03) {
                s.view.selection = gen::multi_selection(&mut rng, &s);
            }
            let msg = if rng.random_bool(0.05) {
                let len = s.doc.text.len_chars();
                let from = rng.random_range(0..=len);
                Msg::External { changes: vec![ExtChange::Replace { from, to: from, text: gen::text(&mut rng) }] }
            } else {
                gen::msg(&mut rng, &s)
            };
            update(&mut s, msg.clone());
            let t = text(&s);
            assert!(!t.contains(['\n', '\r']), "seed {seed} step {step}: {msg:?} left {t:?}");
        }
        // Undo all the way back never brings a line break back.
        while s.doc.history.current_revision() > 0 {
            update(&mut s, Msg::Undo);
            assert!(!text(&s).contains(['\n', '\r']), "seed {seed}: undo");
        }
    }
}

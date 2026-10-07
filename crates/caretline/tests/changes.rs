//! Each message's text changes, handed out by `update_with_changes`, `update_doc_with_changes`
//! and `Session::apply_with_changes`: one `ChangeSet` from the text before the message to the
//! text after, so a host maps its own positions through it.

mod common;

use caretline::helix::{Assoc, ChangeSet, Rope, Selection};
use caretline::outline::markdown;
use caretline::{
    update_doc_with_changes, update_with_changes, By, Dir, Document, Edit, Effect, ExtChange, Host, MarkAttrs,
    MarkOp, Msg, Session, State, View, Viewport,
};
use serde_json::json;

fn vp() -> Viewport {
    Viewport { width: 40, height: 8 }
}

fn state(text: &str) -> State {
    let mut s = State::new(text, None, vp());
    s.doc.set_host(host());
    s
}

/// A made-up host: a command that puts `>> ` at the start, one that only pins a mark, and an
/// input rule that turns `->` into an arrow.
fn host() -> Host {
    Host::new()
        .command("test.prefix", |_, _| Ok(Edit { changes: vec![(0, 0, ">> ".into())], ..Edit::default() }))
        .command("test.pin", |_, _| {
            let op = MarkOp::Mint { pos: 0, attrs: MarkAttrs { gap: None, data: Some(json!(1)) } };
            Ok(Edit { marks: vec![op], ..Edit::default() })
        })
        .input_rule("test.arrow", |ctx, msg| {
            let Msg::InsertText { text } = msg else { return None };
            let p = ctx.caret();
            (text == ">" && p > 0 && ctx.text().char(p - 1) == '-')
                .then(|| Edit { changes: vec![(p - 1, p, "→".into())], selection: Some(Selection::point(p)), ..Edit::default() })
        })
}

/// Applies `msg` and checks the changes it returns against the texts: they start from the
/// old text's length and turn it into the new text; `None` exactly when the text stayed.
fn send(s: &mut State, msg: Msg) -> Option<ChangeSet> {
    let old = s.doc.text.clone();
    let (_, changes) = update_with_changes(s, msg.clone());
    check(&old, &s.doc.text, changes.as_ref(), &msg);
    changes
}

fn check(old: &Rope, new: &Rope, changes: Option<&ChangeSet>, msg: &Msg) {
    match changes {
        Some(cs) => {
            assert_eq!(cs.len(), old.len_chars(), "{msg:?}: the changes start from the old text");
            let mut text = old.clone();
            assert!(cs.apply(&mut text), "{msg:?}");
            assert_eq!(text, *new, "{msg:?}: the changes make the new text");
        }
        None => assert_eq!(old, new, "{msg:?}: the text changed but no changes came back"),
    }
}

fn end(s: &mut State) {
    let n = s.doc.text.len_chars();
    s.view.selection = Selection::point(n);
}

#[test]
fn typing_maps_positions_after_the_caret() {
    let mut s = state("hello world");
    s.view.selection = Selection::point(6);
    let cs = send(&mut s, Msg::InsertText { text: "big ".into() }).unwrap();
    assert_eq!(s.doc.text.to_string(), "hello big world");
    assert_eq!(cs.map_pos(0, Assoc::Before), 0);
    assert_eq!(cs.map_pos(6, Assoc::Before), 6, "before the insertion");
    assert_eq!(cs.map_pos(6, Assoc::After), 10, "after the insertion");
    assert_eq!(cs.map_pos(11, Assoc::Before), 15);
}

#[test]
fn deleting_and_pasting() {
    let mut s = state("abcdef");
    end(&mut s);
    let cs = send(&mut s, Msg::DeleteBackward).unwrap();
    assert_eq!(cs.map_pos(6, Assoc::Before), 5);
    s.view.selection = Selection::point(0);
    let cs = send(&mut s, Msg::Paste { text: Some("XY".into()) }).unwrap();
    assert_eq!(s.doc.text.to_string(), "XYabcde");
    assert_eq!(cs.map_pos(0, Assoc::After), 2);
    assert_eq!(cs.map_pos(3, Assoc::Before), 5);
}

#[test]
fn undo_and_redo_return_their_changes() {
    let mut s = state("hello");
    end(&mut s);
    for c in ["a", "b", "c"] {
        send(&mut s, Msg::InsertText { text: c.into() });
    }
    assert_eq!(s.doc.text.to_string(), "helloabc");
    let undo = send(&mut s, Msg::Undo).unwrap();
    assert_eq!(s.doc.text.to_string(), "hello");
    assert_eq!(undo.map_pos(8, Assoc::Before), 5, "the end follows the undone typing back");
    assert_eq!(undo.map_pos(2, Assoc::Before), 2);
    let redo = send(&mut s, Msg::Redo).unwrap();
    assert_eq!(s.doc.text.to_string(), "helloabc");
    assert_eq!(redo.map_pos(5, Assoc::After), 8);
    // Nothing left to redo: no change.
    assert!(send(&mut s, Msg::Redo).is_none());
}

#[test]
fn a_change_from_elsewhere() {
    let mut s = state("the end");
    let cs = send(&mut s, Msg::External { changes: vec![ExtChange::Replace { from: 0, to: 0, text: "say ".into() }] })
        .unwrap();
    assert_eq!(s.doc.text.to_string(), "say the end");
    assert_eq!(cs.map_pos(4, Assoc::Before), 8);
    // Several in one message compose into one.
    let cs = send(
        &mut s,
        Msg::External {
            changes: vec![
                ExtChange::Replace { from: 0, to: 4, text: String::new() },
                ExtChange::Replace { from: 7, to: 7, text: "!".into() },
            ],
        },
    )
    .unwrap();
    assert_eq!(s.doc.text.to_string(), "the end!");
    assert_eq!(cs.map_pos(8, Assoc::Before), 4, "\"end\" moves left by the deleted \"say \"");
}

#[test]
fn host_commands_and_input_rules() {
    let mut s = state("one-");
    let cs = send(&mut s, Msg::Command { name: "test.prefix".into(), args: json!(null) }).unwrap();
    assert_eq!(s.doc.text.to_string(), ">> one-");
    assert_eq!(cs.map_pos(0, Assoc::After), 3);
    end(&mut s);
    let cs = send(&mut s, Msg::InsertText { text: ">".into() }).unwrap();
    assert_eq!(s.doc.text.to_string(), ">> one→");
    assert_eq!(cs.map_pos(3, Assoc::Before), 3);
    // A command that only changes marks changes no text.
    assert!(send(&mut s, Msg::Command { name: "test.pin".into(), args: json!(null) }).is_none());
}

#[test]
fn messages_that_change_no_text_return_none() {
    let mut s = state("abc");
    assert!(send(&mut s, Msg::Move { dir: Dir::Forward, by: By::Grapheme, extend: false }).is_none());
    assert!(send(&mut s, Msg::Undo).is_none());
    assert!(send(&mut s, Msg::Tick { now_ms: 5 }).is_none());
    assert!(send(&mut s, Msg::External { changes: vec![ExtChange::Replace { from: 1, to: 1, text: String::new() }] }).is_none());
    s.view.read_only = true;
    let (fx, cs) = update_with_changes(&mut s, Msg::InsertText { text: "x".into() });
    assert_eq!(fx, vec![Effect::Refused]);
    assert!(cs.is_none());
}

#[test]
fn an_edit_through_another_view_maps_for_the_document() {
    let mut doc = Document::new("left right", None);
    let mut views = vec![View::new(vp()), View::new(vp())];
    views[1].selection = Selection::point(4);
    let old = doc.text.clone();
    let msg = Msg::InsertText { text: " middle".into() };
    let (_, cs) = update_doc_with_changes(&mut doc, &mut views, 1, msg.clone());
    check(&old, &doc.text, cs.as_ref(), &msg);
    let cs = cs.unwrap();
    assert_eq!(doc.text.to_string(), "left middle right");
    assert_eq!(cs.map_pos(5, Assoc::Before), 12, "\"right\" moves past what the other view typed");
    // An external change through either view is the document's.
    let msg = Msg::External { changes: vec![ExtChange::Replace { from: 0, to: 5, text: String::new() }] };
    let old = doc.text.clone();
    let (_, cs) = update_doc_with_changes(&mut doc, &mut views, 0, msg.clone());
    check(&old, &doc.text, cs.as_ref(), &msg);
    assert_eq!(cs.unwrap().map_pos(5, Assoc::Before), 0);
}

#[test]
fn a_session_hands_them_out_too() {
    let mut session = Session::new(state("abc"));
    let (_, cs) = session.apply_with_changes(Msg::InsertText { text: "x".into() });
    assert_eq!(cs.unwrap().map_pos(0, Assoc::After), 1);
    let id = session.open_view(View::new(vp()));
    let (_, cs) = session.apply_on_with_changes(id, Msg::InsertText { text: "y".into() });
    assert_eq!(session.state().doc.text.to_string(), "yxabc");
    assert_eq!(cs.unwrap().map_pos(1, Assoc::Before), 2);
    let (_, cs) = session.apply_on_with_changes(99, Msg::InsertText { text: "z".into() });
    assert!(cs.is_none(), "no such view");
    assert_eq!(session.apply_with_changes(Msg::Undo).1.unwrap().len(), 5);
}

/// Every message of a long outline session: the changes it returns turn the old text into the
/// new one, whatever path the edit took (outline rules, folds, block moves, undo).
#[test]
fn every_message_returns_exactly_its_changes() {
    let sample = "# Title\n\n- one\n  - two\n- three\n\nA paragraph that runs on.\n";
    let mut s = markdown::load(sample, None, vp(), common::tagged_cfg());
    s.doc.set_host(common::retag_host());
    let msgs = [
        Msg::Move { dir: Dir::Forward, by: By::Line, extend: false },
        Msg::Move { dir: Dir::Forward, by: By::Line, extend: false },
        Msg::InsertText { text: "x".into() },
        Msg::InsertNewline,
        Msg::InsertText { text: "new".into() },
        Msg::DeleteBackward,
        Msg::Move { dir: Dir::Backward, by: By::Line, extend: true },
        Msg::Cut,
        Msg::Paste { text: None },
        Msg::Paste { text: Some("a\nb\n\nc".into()) },
        Msg::Undo,
        Msg::Undo,
        Msg::Redo,
        Msg::External { changes: vec![ExtChange::Replace { from: 0, to: 2, text: "## ".into() }] },
        Msg::Undo,
        Msg::Move { dir: Dir::Forward, by: By::DocEnd, extend: false },
        Msg::DeleteBackward,
        Msg::InsertNewline,
        Msg::InsertNewline,
    ];
    let mut changed = 0;
    for msg in msgs {
        changed += usize::from(send(&mut s, msg).is_some());
    }
    assert!(changed >= 10, "{changed}");
}

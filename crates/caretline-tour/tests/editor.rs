//! The `caretline` feature: `Editor` answers the editor's predicates from a real caretline
//! state, and `find_in` resolves `find` anchors in a document.

#![cfg(feature = "caretline")]

use caretline::{Dir, MarkId, Msg, State, Viewport, command_msg, update_with_changes};
use caretline_layers::Anchor;
use caretline_tour::*;
use serde_json::{Value, json};

const TEXT: &str = "# Title\n\nSome words here.\n## Second\nMore words.\n";

fn state() -> State {
    let mut s = State::new(
        TEXT,
        None,
        Viewport {
            width: 40,
            height: 10,
        },
    );
    s.doc.marks.mint(0);
    s.doc.marks.mint(TEXT.find("## Second").unwrap());
    s
}

/// Sends `msg`, then observes the walkthrough with the editor's answers.
fn send(ed: &mut State, tour: &mut TourState, msg: Msg, now_ms: u64) -> Vec<TourEffect> {
    let (effects, changes) = update_with_changes(ed, msg.clone());
    let on = Editor::new(&ed.doc, &ed.view).message(&msg, &effects, changes.as_ref());
    observe(tour, &on, now_ms)
}

#[test]
fn find_in_gives_text_within_the_block_it_starts_in() {
    let ed = state();
    let second = ed
        .doc
        .marks
        .at_or_before(TEXT.find("More").unwrap())
        .unwrap();
    assert_eq!(
        find_in(&ed.doc, "More words"),
        Some(Anchor::BlockText {
            block: second.id.0,
            from: 10,
            to: 20
        })
    );
    assert_eq!(find_in(&ed.doc, "absent"), None);
    let plain = State::new(
        "abc def",
        None,
        Viewport {
            width: 20,
            height: 3,
        },
    );
    assert_eq!(
        find_in(&plain.doc, "def"),
        Some(Anchor::Text { from: 4, to: 7 })
    );
    // Through Tour::resolve_finds, with the view kept.
    let mut t = parse_toml(
        "id = \"t\"\n[[step]]\nid = \"a\"\nanchor = [{ find = \"Some\", in = \"main\" }, { find = \"zzz\" }]\ndata = { text = \"x\" }\n",
    )
    .unwrap();
    let missing = t.resolve_finds(|text, _| find_in(&ed.doc, text));
    assert_eq!(missing, vec![("a".into(), "a/0".into(), "zzz".into())]);
    assert_eq!(
        t.steps[0].layers[0].anchor[0],
        StepAnchor::At(Anchor::scoped(
            "main",
            Anchor::BlockText {
                block: 0,
                from: 9,
                to: 13
            }
        ))
    );
    assert!(t.has_finds());
    // The unresolved fallback is left out of the layer.
    let layers = step_layers(&t, 0, false);
    assert_eq!(layers[0].anchor.len(), 1);
}

#[test]
fn message_kinds_and_commands() {
    let mut ed = state();
    let mut tour = TourState::default();
    let t = parse_toml(
        r#"
id = "t"
[[step]]
id = "type"
narration = { text = "Type two letters." }
advance = { msg = "insert_text", count = 2 }
[[step]]
id = "word"
narration = { text = "Jump a word." }
advance = { command = "move.word_right" }
[[step]]
id = "last"
narration = { text = "The end." }
"#,
    )
    .unwrap();
    apply(&mut tour, TourOp::Start(Box::new(t)), 0).unwrap();
    send(&mut ed, &mut tour, Msg::InsertText { text: "a".into() }, 1);
    send(
        &mut ed,
        &mut tour,
        Msg::Move {
            dir: Dir::Forward,
            by: caretline::By::Grapheme,
            extend: false,
        },
        2,
    );
    assert_eq!(tour.step.as_deref(), Some("type"));
    send(&mut ed, &mut tour, Msg::InsertText { text: "b".into() }, 3);
    assert_eq!(tour.step.as_deref(), Some("word"));
    send(
        &mut ed,
        &mut tour,
        Msg::Move {
            dir: Dir::Forward,
            by: caretline::By::Grapheme,
            extend: false,
        },
        4,
    );
    assert_eq!(tour.step.as_deref(), Some("word"));
    send(
        &mut ed,
        &mut tour,
        command_msg("move.word_right", None).unwrap(),
        5,
    );
    assert_eq!(tour.step.as_deref(), Some("last"));
}

#[test]
fn caret_in_changed_selection_and_folded() {
    let mut ed = state();
    let second = MarkId(1);
    let words = Anchor::Text { from: 9, to: 13 }; // "Some"
    let block = Anchor::Block(second.0);

    let on = Editor::new(&ed.doc, &ed.view);
    assert!(!on.caret_in(std::slice::from_ref(&words)));
    assert!(!on.selection());
    assert!(!on.folded(std::slice::from_ref(&block)));
    assert!(
        on.caret_in(&[Anchor::Block(0)]),
        "the caret is in the first block"
    );

    ed.view.selection = caretline::helix::Selection::single(9, 11);
    let on = Editor::new(&ed.doc, &ed.view);
    assert!(on.caret_in(std::slice::from_ref(&words)));
    assert!(on.selection());

    let msg = Msg::InsertText { text: "x".into() };
    let (effects, changes) = update_with_changes(&mut ed, msg.clone());
    let on = Editor::new(&ed.doc, &ed.view).message(&msg, &effects, changes.as_ref());
    assert!(on.changed(std::slice::from_ref(&words)));
    assert!(!on.changed(std::slice::from_ref(&block)));

    ed.view.folds.insert(second);
    let on = Editor::new(&ed.doc, &ed.view);
    assert!(on.folded(std::slice::from_ref(&block)));
    let in_second = Anchor::BlockText {
        block: second.0,
        from: 0,
        to: 2,
    };
    assert!(on.folded(&[in_second]));
}

/// A host that knows one event and one state key.
struct Mine;

impl TourHost for Mine {
    fn event(&self, name: &str) -> bool {
        name == "panel.opened"
    }
    fn state(&self, key: &str) -> Option<Value> {
        (key == "panel").then(|| json!({"open": true}))
    }
}

#[test]
fn what_the_editor_cant_answer_goes_to_the_host() {
    let ed = state();
    let on = Editor::new(&ed.doc, &ed.view).with_host(&Mine);
    assert!(on.event("panel.opened"));
    assert!(!on.event("other"));
    assert_eq!(on.state("panel"), Some(json!({"open": true})));
    let effects = vec![caretline::Effect::Host {
        name: "custom".into(),
        data: Value::Null,
    }];
    let msg = Msg::InsertText { text: "x".into() };
    let on = Editor::new(&ed.doc, &ed.view).message(&msg, &effects, None);
    assert!(on.event("custom"), "a host effect by name");
}

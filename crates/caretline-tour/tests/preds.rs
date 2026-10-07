//! Predicates with a fake host: `advance` after counts, events, state and time; `skip_if`;
//! `next` branches; nudges; and steps without predicates moving only by ops.

use std::cell::RefCell;
use std::collections::BTreeMap;

use caretline_layers::Anchor;
use caretline_tour::*;
use serde_json::{Value, json};

/// A host that answers from what the test says happened in this message.
#[derive(Default)]
struct Fake {
    ran: Vec<&'static str>,
    msgs: Vec<&'static str>,
    events: Vec<&'static str>,
    state: BTreeMap<String, Value>,
    caret_in: bool,
    asked: RefCell<Vec<Vec<Anchor>>>,
}

impl TourHost for Fake {
    fn ran(&self, c: &str) -> bool {
        self.ran.contains(&c)
    }
    fn msg(&self, k: &str) -> bool {
        self.msgs.contains(&k)
    }
    fn event(&self, n: &str) -> bool {
        self.events.contains(&n)
    }
    fn state(&self, k: &str) -> Option<Value> {
        self.state.get(k).cloned()
    }
    fn caret_in(&self, a: &[Anchor]) -> bool {
        self.asked.borrow_mut().push(a.to_vec());
        self.caret_in
    }
}

fn msg(k: &'static str) -> Fake {
    Fake {
        msgs: vec![k],
        ..Fake::default()
    }
}

fn started(src: &str) -> TourState {
    let mut s = TourState::default();
    apply(&mut s, TourOp::Start(Box::new(parse_toml(src).unwrap())), 0).unwrap();
    s
}

fn at(s: &TourState) -> &str {
    s.step.as_deref().unwrap_or("(ended)")
}

#[test]
fn advance_waits_for_its_count() {
    let mut s = started(
        r#"
id = "t"
[[step]]
id = "a"
narration = { text = "type" }
advance = { msg = "insert_text", count = 3 }
[[step]]
id = "b"
narration = { text = "ok" }
"#,
    );
    for n in 1..=2 {
        assert!(observe(&mut s, &msg("insert_text"), n).is_empty());
        assert!(observe(&mut s, &msg("move"), n).is_empty());
    }
    assert_eq!(s.counts["msg:insert_text"], 2);
    let fx = observe(&mut s, &msg("insert_text"), 3);
    assert_eq!(at(&s), "b");
    assert!(
        fx.iter()
            .any(|e| matches!(e, TourEffect::Step { step, .. } if step == "b"))
    );
    assert!(s.counts.is_empty(), "counts start again on each step");
    assert_eq!(s.history, vec!["a".to_string()]);
}

#[test]
fn all_of_two_commands_counts_each_since_the_step_began() {
    let mut s = started(
        r#"
id = "t"
[[step]]
id = "a"
narration = { text = "x" }
advance = { all = [{ command = "left" }, { command = "right" }] }
[[step]]
id = "b"
narration = { text = "y" }
"#,
    );
    let left = Fake {
        ran: vec!["left"],
        ..Fake::default()
    };
    let right = Fake {
        ran: vec!["right"],
        ..Fake::default()
    };
    observe(&mut s, &left, 1);
    observe(&mut s, &left, 2);
    assert_eq!(at(&s), "a");
    observe(&mut s, &right, 3);
    assert_eq!(at(&s), "b");
}

#[test]
fn events_state_and_time() {
    let src = r#"
id = "t"
[[step]]
id = "a"
narration = { text = "x" }
advance = { any = [{ event = "saved" }, { state = { key = "panel", match = { open = true } } }, { after_ms = 5000 }] }
[[step]]
id = "b"
narration = { text = "y" }
"#;
    let mut s = started(src);
    observe(&mut s, &Fake::default(), 4999);
    assert_eq!(at(&s), "a");
    observe(&mut s, &Fake::default(), 5000);
    assert_eq!(at(&s), "b", "after_ms from the step's start");

    let mut s = started(src);
    let saved = Fake {
        events: vec!["saved"],
        ..Fake::default()
    };
    observe(&mut s, &saved, 1);
    assert_eq!(at(&s), "b");

    let mut s = started(src);
    let mut h = Fake::default();
    h.state
        .insert("panel".into(), json!({"open": false, "w": 3}));
    observe(&mut s, &h, 1);
    assert_eq!(at(&s), "a");
    h.state
        .insert("panel".into(), json!({"open": true, "w": 3}));
    observe(&mut s, &h, 2);
    assert_eq!(at(&s), "b", "a subset of the host's state");
}

#[test]
fn not_and_state_presence() {
    let mut s = started(
        r#"
id = "t"
[[step]]
id = "a"
narration = { text = "x" }
advance = { all = [{ not = { state = { key = "busy" } } }, { state = { key = "ready", present = true } }] }
[[step]]
id = "b"
narration = { text = "y" }
"#,
    );
    let mut h = Fake::default();
    h.state.insert("busy".into(), json!(true));
    h.state.insert("ready".into(), json!(1));
    observe(&mut s, &h, 1);
    assert_eq!(at(&s), "a");
    h.state.remove("busy");
    observe(&mut s, &h, 2);
    assert_eq!(at(&s), "b");
}

#[test]
fn caret_in_asks_the_host_with_the_named_layers_anchors() {
    let mut s = started(
        r#"
id = "t"
[[step]]
id = "a"
advance = { all = [{ caret_in = "anchor" }, { caret_in = "second" }] }
[[step.layers]]
anchor = [{ block = 4 }, { screen = "top" }]
data = { text = "x" }
[[step.layers]]
id = "second"
anchor = { text = { from = 1, to = 2 } }
data = { text = "y" }
[[step]]
id = "b"
narration = { text = "y" }
"#,
    );
    let h = Fake {
        caret_in: true,
        ..Fake::default()
    };
    observe(&mut s, &h, 1);
    assert_eq!(at(&s), "b");
    let asked = h.asked.borrow();
    assert_eq!(
        asked[0],
        vec![
            Anchor::Block(4),
            Anchor::Screen(caretline_layers::ScreenPos::Top)
        ]
    );
    assert_eq!(asked[1], vec![Anchor::Text { from: 1, to: 2 }]);
}

#[test]
fn steps_without_predicates_move_only_by_ops() {
    let mut s = started(
        r#"
id = "pages"
[[step]]
id = "p1"
narration = { text = "one" }
[[step]]
id = "p2"
narration = { text = "two" }
"#,
    );
    let busy = Fake {
        ran: vec!["anything"],
        msgs: vec!["insert_text"],
        events: vec!["e"],
        caret_in: true,
        ..Fake::default()
    };
    for t in [1, 10_000, 1_000_000] {
        assert!(observe(&mut s, &busy, t).is_empty());
    }
    assert_eq!(at(&s), "p1");
    apply(&mut s, TourOp::Next, 2_000_000).unwrap();
    assert_eq!(at(&s), "p2");
}

const BRANCHY: &str = r#"
id = "t"
[[step]]
id = "a"
narration = { text = "x" }
advance = { event = "go" }
next = [{ if = { state = { key = "wide" } }, goto = "wide" }, { if = { event = "quit" }, goto = "end" }]
[[step]]
id = "narrow"
narration = { text = "n" }
[[step]]
id = "wide"
narration = { text = "w" }
skip_if = { state = { key = "seen_wide" } }
[[step]]
id = "last"
narration = { text = "l" }
"#;

#[test]
fn branches_take_the_first_that_holds_else_the_next_step() {
    let go = |extra: &[(&str, Value)], events: Vec<&'static str>| {
        let mut h = Fake {
            events,
            ..Fake::default()
        };
        for (k, v) in extra {
            h.state.insert(k.to_string(), v.clone());
        }
        h
    };
    let mut s = started(BRANCHY);
    observe(&mut s, &go(&[], vec!["go"]), 1);
    assert_eq!(at(&s), "narrow");

    let mut s = started(BRANCHY);
    observe(&mut s, &go(&[("wide", json!(true))], vec!["go"]), 1);
    assert_eq!(at(&s), "wide");

    let mut s = started(BRANCHY);
    let fx = observe(&mut s, &go(&[], vec!["go", "quit"]), 1);
    assert!(!s.running());
    assert!(matches!(
        fx.last(),
        Some(TourEffect::Ended {
            seen: Seen {
                end: End::Finished,
                ..
            },
            ..
        })
    ));
}

#[test]
fn skip_if_passes_over_a_step_going_forward_but_not_on_a_jump() {
    let both = {
        let mut h = Fake {
            events: vec!["go"],
            ..Fake::default()
        };
        h.state.insert("wide".into(), json!(true));
        h.state.insert("seen_wide".into(), json!(true));
        h
    };
    let mut s = started(BRANCHY);
    observe(&mut s, &both, 1);
    assert_eq!(at(&s), "last", "wide was skipped");
    assert_eq!(s.history, vec!["a".to_string()], "and isn't in the history");
    // A jump enters it exactly, skip_if or not.
    apply_with(&mut s, TourOp::To("wide".into()), 2, &both).unwrap();
    assert_eq!(at(&s), "wide");
    // `apply_with` going forward asks the host too.
    let mut s = started(BRANCHY);
    apply_with(&mut s, TourOp::Next, 3, &both).unwrap();
    assert_eq!(at(&s), "last");
    // Plain `apply` asks no host: no branch holds, no skip.
    let mut s = started(BRANCHY);
    apply(&mut s, TourOp::Next, 3).unwrap();
    assert_eq!(at(&s), "narrow");
}

#[test]
fn a_nudge_merges_its_data_once() {
    let mut s = started(
        r#"
id = "t"
kind = "host.box"
[[step]]
id = "a"
anchor = { caret = true }
data = { title = "T", text = "first" }
nudge = { after_ms = 3000, data = { text = "still here?" } }
advance = { event = "never" }
"#,
    );
    assert!(observe(&mut s, &Fake::default(), 2999).is_empty());
    let fx = observe(&mut s, &Fake::default(), 3000);
    let [TourEffect::Layers { layers }] = fx.as_slice() else {
        panic!("{fx:?}")
    };
    assert_eq!(
        layers[0].content.as_ref().unwrap().data,
        json!({"title": "T", "text": "still here?", "at": 1, "of": 1})
    );
    assert!(s.nudged);
    assert_eq!(s.layers(), *layers);
    assert!(observe(&mut s, &Fake::default(), 9000).is_empty(), "once");
}

#[test]
fn subset_matches_nested_objects() {
    assert!(subset(
        &json!({"a": {"b": 1, "c": 2}, "d": 3}),
        &json!({"a": {"b": 1}})
    ));
    assert!(!subset(&json!({"a": {"b": 1}}), &json!({"a": {"b": 2}})));
    assert!(!subset(&json!({"a": 1}), &json!({"z": 1})));
    assert!(subset(&json!([1, 2]), &json!([1, 2])));
}

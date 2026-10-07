//! The reducer: next, back, `to`, restart and stop; jumps enter a step exactly; an op log
//! replays to an equal state; the state round-trips through JSON.

use caretline_layers::{Layers, Owner};
use caretline_tour::*;
use serde_json::json;

fn tour() -> Tour {
    parse_toml(
        r#"
id = "demo"
version = 3
title = "Demo"
kind = "host.box"

[[step]]
id = "one"
host = { open = "left" }
narration = { title = "One", text = "First." }
anchor = { block = 1 }
data = { title = "One" }

[[step]]
id = "two"
host = { open = "right" }
narration = { text = "Second." }
[[step.layers]]
anchor = { block = 2 }
data = { title = "Two" }
[[step.layers]]
id = "side"
anchor = { block = 3, in = "side" }
kind = "hint"
data = { text = "beside" }

[[step]]
id = "three"
narration = { text = "Third." }

[[step]]
id = "four"
host = { open = "none" }
anchor = { screen = "center" }
data = { title = "Bye" }
"#,
    )
    .unwrap()
}

fn start(s: &mut TourState) -> Vec<TourEffect> {
    apply(s, TourOp::Start(Box::new(tour())), 0).unwrap()
}

fn step_of(fx: &[TourEffect]) -> Option<(String, usize, usize)> {
    fx.iter().find_map(|e| match e {
        TourEffect::Step { step, at, of, .. } => Some((step.clone(), *at, *of)),
        _ => None,
    })
}

#[test]
fn start_enters_the_first_step_with_its_host_patch_layers_and_narration() {
    let mut s = TourState::default();
    let fx = start(&mut s);
    assert_eq!(fx.len(), 3);
    assert_eq!(
        fx[0],
        TourEffect::Host {
            patch: json!({"open": "left"})
        }
    );
    let TourEffect::Layers { layers } = &fx[1] else {
        panic!("{fx:?}")
    };
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].id, "one/0");
    assert_eq!(layers[0].owner, Owner::Guide);
    let c = layers[0].content.as_ref().unwrap();
    assert_eq!(c.kind, "host.box");
    assert_eq!(c.data, json!({"title": "One", "at": 1, "of": 4}));
    assert_eq!(
        fx[2],
        TourEffect::Step {
            tour: "demo".into(),
            step: "one".into(),
            at: 1,
            of: 4,
            narration: Some(Narration {
                title: Some("One".into()),
                text: "First.".into()
            })
        }
    );
    assert!(s.running());
    assert_eq!(s.narration().unwrap().text, "First.");
}

#[test]
fn next_back_and_hint_layers_without_at_and_of() {
    let mut s = TourState::default();
    start(&mut s);
    let fx = apply(&mut s, TourOp::Next, 10).unwrap();
    assert_eq!(step_of(&fx), Some(("two".into(), 2, 4)));
    let TourEffect::Layers { layers } = &fx[1] else {
        panic!()
    };
    assert_eq!(layers[1].id, "side");
    // A hint's data stays {title?, text}: no step dots added.
    assert_eq!(
        layers[1].content.as_ref().unwrap().data,
        json!({"text": "beside"})
    );
    assert_eq!(s.since_ms, 10);
    let fx = apply(&mut s, TourOp::Back, 20).unwrap();
    assert_eq!(step_of(&fx), Some(("one".into(), 1, 4)));
    assert_eq!(
        apply(&mut s, TourOp::Back, 30).unwrap_err().reason,
        TourReason::NoBack
    );
}

#[test]
fn a_step_without_layers_clears_them_and_without_host_sends_no_patch() {
    let mut s = TourState::default();
    start(&mut s);
    apply(&mut s, TourOp::Next, 1).unwrap();
    let fx = apply(&mut s, TourOp::Next, 2).unwrap();
    assert_eq!(fx[0], TourEffect::Layers { layers: vec![] });
    assert_eq!(step_of(&fx), Some(("three".into(), 3, 4)));
}

#[test]
fn a_jump_enters_its_step_exactly_as_arriving_in_order() {
    let mut a = TourState::default();
    start(&mut a);
    for _ in 0..3 {
        apply(&mut a, TourOp::Next, 5).unwrap();
    }
    let mut b = TourState::default();
    start(&mut b);
    // Arriving at "four" in order, and jumping there from "one", give the same effects.
    let mut c = TourState::default();
    start(&mut c);
    apply(&mut c, TourOp::Next, 5).unwrap();
    apply(&mut c, TourOp::Next, 5).unwrap();
    let in_order = apply(&mut c, TourOp::Next, 5).unwrap();
    let jumped = apply(&mut b, TourOp::To("four".into()), 5).unwrap();
    assert_eq!(in_order, jumped);
    assert_eq!(
        in_order[0],
        TourEffect::Host {
            patch: json!({"open": "none"})
        }
    );
    // Jumping back to "two" from "four" looks like arriving at "two".
    let mut d = TourState::default();
    start(&mut d);
    let two = apply(&mut d, TourOp::Next, 7).unwrap();
    assert_eq!(apply(&mut a, TourOp::To("two".into()), 7).unwrap(), two);
    // And back from a jump returns where it jumped from.
    let fx = apply(&mut a, TourOp::Back, 8).unwrap();
    assert_eq!(step_of(&fx), Some(("four".into(), 4, 4)));
    assert_eq!(
        apply(&mut a, TourOp::To("nine".into()), 9)
            .unwrap_err()
            .reason,
        TourReason::NotFound
    );
}

#[test]
fn back_re_enters_the_earlier_step_identically() {
    let mut s = TourState::default();
    start(&mut s);
    let two = apply(&mut s, TourOp::Next, 4).unwrap();
    apply(&mut s, TourOp::Next, 4).unwrap();
    assert_eq!(apply(&mut s, TourOp::Back, 4).unwrap(), two);
}

#[test]
fn the_end_finishes_and_stop_stops_with_seen_state() {
    let mut s = TourState::default();
    start(&mut s);
    for _ in 0..3 {
        apply(&mut s, TourOp::Next, 1).unwrap();
    }
    let fx = apply(&mut s, TourOp::Next, 2).unwrap();
    assert_eq!(
        fx,
        vec![
            TourEffect::Layers { layers: vec![] },
            TourEffect::Ended {
                tour: "demo".into(),
                seen: Seen {
                    version: 3,
                    end: End::Finished
                }
            }
        ]
    );
    assert!(!s.running());
    assert_eq!(
        apply(&mut s, TourOp::Next, 3).unwrap_err().reason,
        TourReason::NotRunning
    );
    // Restart plays the last one again.
    let fx = apply(&mut s, TourOp::Restart, 4).unwrap();
    assert_eq!(step_of(&fx), Some(("one".into(), 1, 4)));
    let fx = apply(&mut s, TourOp::Stop, 5).unwrap();
    assert!(matches!(
        fx[1],
        TourEffect::Ended {
            seen: Seen {
                end: End::Stopped,
                ..
            },
            ..
        }
    ));
    assert_eq!(s.seen["demo"].end, End::Stopped);
}

#[test]
fn restart_starts_over_and_forgets_history() {
    let mut s = TourState::default();
    let first = start(&mut s);
    apply(&mut s, TourOp::Next, 1).unwrap();
    apply(&mut s, TourOp::Next, 1).unwrap();
    let again = apply(&mut s, TourOp::Restart, 0).unwrap();
    assert_eq!(again, first);
    assert!(s.history.is_empty());
    assert_eq!(
        apply(&mut TourState::default(), TourOp::Restart, 0)
            .unwrap_err()
            .reason,
        TourReason::NotRunning
    );
}

#[test]
fn offer_follows_seen_and_reoffer() {
    let mut s = TourState::default();
    let mut t = tour();
    assert!(s.offer(&t));
    s.seen.insert(
        "demo".into(),
        Seen {
            version: 3,
            end: End::Stopped,
        },
    );
    assert!(!s.offer(&t));
    t.version = 4;
    assert!(!s.offer(&t), "a later version only with reoffer");
    t.reoffer = true;
    assert!(s.offer(&t));
}

#[test]
fn a_tour_with_errors_is_refused_at_start() {
    let mut bad = tour();
    bad.steps[1].id = "one".into();
    let e = apply(&mut TourState::default(), TourOp::Start(Box::new(bad)), 0).unwrap_err();
    assert_eq!(e.reason, TourReason::Invalid);
    assert!(e.detail.starts_with("duplicate_step"), "{e}");
}

fn ops() -> Vec<(TourOp, u64)> {
    vec![
        (TourOp::Start(Box::new(tour())), 0),
        (TourOp::Next, 100),
        (TourOp::To("four".into()), 250),
        (TourOp::Back, 300),
        (TourOp::Next, 400),
        (TourOp::Back, 500),
        (TourOp::Restart, 600),
        (TourOp::Next, 700),
    ]
}

#[test]
fn replaying_an_op_log_gives_an_equal_state_and_the_same_effects() {
    let log: Vec<String> = ops()
        .iter()
        .map(|(op, t)| json!({"op": op, "now_ms": t}).to_string())
        .collect();
    let run = |lines: &[String]| {
        let mut s = TourState::default();
        let mut all = Vec::new();
        for line in lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let op: TourOp = serde_json::from_value(v["op"].clone()).unwrap();
            all.extend(apply(&mut s, op, v["now_ms"].as_u64().unwrap()).unwrap());
        }
        (s, all)
    };
    let (a, fa) = run(&log);
    let (b, fb) = run(&log);
    assert_eq!(a, b);
    assert_eq!(fa, fb);
    // And the direct run agrees with the replayed one.
    let mut c = TourState::default();
    for (op, t) in ops() {
        apply(&mut c, op, t).unwrap();
    }
    assert_eq!(a, c);
    assert_eq!(a.step.as_deref(), Some("two"));
}

#[test]
fn the_state_round_trips_through_json() {
    let mut s = TourState::default();
    for (op, t) in ops() {
        apply(&mut s, op, t).unwrap();
    }
    s.counts.insert("msg:insert_text".into(), 2);
    s.seen.insert(
        "other".into(),
        Seen {
            version: 1,
            end: End::Finished,
        },
    );
    let j = serde_json::to_string(&s).unwrap();
    let back: TourState = serde_json::from_str(&j).unwrap();
    assert_eq!(back, s);
    // Unknown fields are refused in the state too.
    let mut v: serde_json::Value = serde_json::from_str(&j).unwrap();
    v["extra"] = json!(1);
    assert!(serde_json::from_value::<TourState>(v).is_err());
    // The ops and effects serialize as data.
    let fx = apply(&mut s, TourOp::To("four".into()), 900).unwrap();
    let fj = serde_json::to_value(&fx).unwrap();
    assert_eq!(fj[0], json!({"effect": "host", "patch": {"open": "none"}}));
    assert_eq!(fj[2]["effect"], "step");
    let back: Vec<TourEffect> = serde_json::from_value(fj).unwrap();
    assert_eq!(back, fx);
    assert_eq!(
        serde_json::to_value(TourOp::To("x".into())).unwrap(),
        json!({"to": "x"})
    );
    assert_eq!(serde_json::to_value(TourOp::Next).unwrap(), json!("next"));
}

#[test]
fn the_layers_effect_replaces_the_guide_layers() {
    let mut layers = Layers::default();
    caretline_layers::apply(
        &mut layers,
        caretline_layers::LayerOp::Push(
            caretline_layers::Layer::new(caretline_layers::Anchor::Caret)
                .with_content(caretline_layers::Content::hint(None, "the host's")),
        ),
        None,
        0,
        &caretline_layers::Limits::default(),
    )
    .unwrap();
    let mut s = TourState::default();
    for fx in start(&mut s)
        .into_iter()
        .chain(apply(&mut s, TourOp::Next, 5).unwrap())
    {
        if let TourEffect::Layers { layers: l } = fx {
            replace_guide(&mut layers, l, 5).unwrap();
        }
    }
    let ids: Vec<&str> = layers.layers.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, vec!["L-1", "two/0", "side"]);
    assert!(layers.layers[1..].iter().all(|l| l.z == GUIDE_Z));
    for fx in apply(&mut s, TourOp::Stop, 6).unwrap() {
        if let TourEffect::Layers { layers: l } = fx {
            replace_guide(&mut layers, l, 6).unwrap();
        }
    }
    assert_eq!(layers.layers.len(), 1, "only the host's own layer is left");
    assert_eq!(s.layers(), vec![]);
}

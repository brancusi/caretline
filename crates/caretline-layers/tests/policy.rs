//! The layer lifecycle and the agent policy `apply` enforces: one test per refusal, the
//! limits that rewrite rather than refuse (cap, ttl, attribution, bands), and the Elm
//! promises: state changes only through ops, and a recorded op sequence replays exactly.

use caretline_layers::*;
use serde_json::json;

fn hint(text: &str) -> Layer {
    Layer::new(Anchor::Text { from: 0, to: 4 }).with_content(Content::hint(None, text))
}

fn agent(layers: &mut Layers, layer: Layer, now: u64) -> Result<Applied, Refusal> {
    apply(
        layers,
        LayerOp::Push(layer),
        Some("helper"),
        now,
        &Limits::default(),
    )
}

fn reason(r: Result<Applied, Refusal>) -> Reason {
    r.expect_err("refused").reason
}

#[test]
fn rate_limited_past_two_pushes_a_second_per_actor() {
    let mut l = Layers::default();
    agent(&mut l, hint("a"), 1_000).unwrap();
    agent(&mut l, hint("b"), 1_400).unwrap();
    assert_eq!(reason(agent(&mut l, hint("c"), 1_900)), Reason::RateLimited);
    // Another actor has its own budget; the window slides by now_ms.
    apply(
        &mut l,
        LayerOp::Push(hint("d")),
        Some("other"),
        1_900,
        &Limits::default(),
    )
    .unwrap();
    agent(&mut l, hint("e"), 2_000).unwrap();
    // The person is never rate limited.
    for t in 0..5 {
        apply(
            &mut l,
            LayerOp::Push(hint("p")),
            None,
            2_000 + t,
            &Limits::default(),
        )
        .unwrap();
    }
}

#[test]
fn dim_not_allowed_for_agents_without_consent() {
    let mut l = Layers::default();
    let spot = hint("x").with_spotlight();
    assert_eq!(
        reason(agent(&mut l, spot.clone(), 0)),
        Reason::DimNotAllowed
    );
    let allow = Limits {
        agent_dim: AgentDim::Always,
        ..Limits::default()
    };
    apply(&mut l, LayerOp::Push(spot), Some("helper"), 0, &allow).unwrap();
}

#[test]
fn capture_not_allowed_for_agents() {
    let mut l = Layers::default();
    let mut c = hint("x");
    c.capture = true;
    assert_eq!(
        reason(agent(&mut l, c.clone(), 0)),
        Reason::CaptureNotAllowed
    );
    apply(&mut l, LayerOp::Push(c), None, 0, &Limits::default()).unwrap();
}

#[test]
fn too_long_hints_titles_and_content() {
    let mut l = Layers::default();
    assert_eq!(
        reason(agent(&mut l, hint(&"x".repeat(281)), 0)),
        Reason::TooLong
    );
    assert_eq!(
        reason(agent(&mut l, hint("1\n2\n3\n4\n5\n6\n7"), 0)),
        Reason::TooLong
    );
    let titled = Layer::new(Anchor::Caret).with_content(Content::hint(Some(&"t".repeat(41)), "x"));
    assert_eq!(reason(agent(&mut l, titled, 0)), Reason::TooLong);
    let big = Layer::new(Anchor::Caret)
        .with_content(Content::new("card", json!({"text": "y".repeat(2_000)})));
    assert_eq!(reason(agent(&mut l, big, 0)), Reason::TooLong);
    // Exactly at the limits is fine.
    agent(&mut l, hint(&"x".repeat(280)), 0).unwrap();
}

#[test]
fn not_found_for_unknown_layers() {
    let mut l = Layers::default();
    let lim = Limits::default();
    assert_eq!(
        reason(apply(
            &mut l,
            LayerOp::Pop(Selector::Layer("L-9".into())),
            None,
            0,
            &lim
        )),
        Reason::NotFound
    );
    let mut u = hint("x");
    u.id = "L-9".into();
    assert_eq!(
        reason(apply(&mut l, LayerOp::Update(u), None, 0, &lim)),
        Reason::NotFound
    );
}

#[test]
fn not_allowed_to_touch_others_layers_or_the_persons_controls() {
    let mut l = Layers::default();
    let lim = Limits::default();
    let mine = apply(&mut l, LayerOp::Push(hint("host")), None, 0, &lim)
        .unwrap()
        .layer
        .unwrap();
    let as_agent = |l: &mut Layers, op| apply(l, op, Some("helper"), 0, &lim);
    assert_eq!(
        reason(as_agent(
            &mut l,
            LayerOp::Pop(Selector::Layer(mine.clone()))
        )),
        Reason::NotAllowed
    );
    assert_eq!(
        reason(as_agent(&mut l, LayerOp::Pop(Selector::Owner(Owner::Host)))),
        Reason::NotAllowed
    );
    assert_eq!(
        reason(as_agent(&mut l, LayerOp::PopNewest)),
        Reason::NotAllowed
    );
    assert_eq!(
        reason(as_agent(&mut l, LayerOp::Toggle)),
        Reason::NotAllowed
    );
    assert_eq!(reason(as_agent(&mut l, LayerOp::Clear)), Reason::NotAllowed);
    let mut u = hint("rewritten");
    u.id = mine.clone();
    assert_eq!(
        reason(as_agent(&mut l, LayerOp::Update(u))),
        Reason::NotAllowed
    );
    // `all` from an agent pops only its own.
    let own = agent(&mut l, hint("a"), 0).unwrap().layer.unwrap();
    let r = apply(
        &mut l,
        LayerOp::Pop(Selector::All(true)),
        Some("helper"),
        0,
        &lim,
    )
    .unwrap();
    assert_eq!(r.popped, vec![own]);
    assert!(l.get(&mine).is_some());
}

#[test]
fn too_many_when_other_agents_fill_the_cap() {
    let mut l = Layers::default();
    let lim = Limits::default();
    for (k, a) in ["a", "b", "c"].iter().enumerate() {
        apply(&mut l, LayerOp::Push(hint("x")), Some(a), k as u64, &lim).unwrap();
    }
    assert_eq!(reason(agent(&mut l, hint("x"), 10)), Reason::TooMany);
}

#[test]
fn invalid_layers() {
    let mut l = Layers::default();
    let lim = Limits::default();
    let mut push = |layer: Layer| reason(apply(&mut l, LayerOp::Push(layer), None, 0, &lim));
    let mut none = hint("x");
    none.anchor.clear();
    assert_eq!(push(none), Reason::Invalid);
    assert_eq!(push(Layer::new(Anchor::Caret)), Reason::Invalid);
    assert_eq!(
        push(Layer::new(Anchor::Caret).with_ring().with_arrow()),
        Reason::Invalid
    );
    assert_eq!(
        push(Layer::new(Anchor::Caret).with_content(Content::new("", json!({})))),
        Reason::Invalid
    );
    assert_eq!(
        push(Layer::new(Anchor::Caret).with_content(Content::new(HINT, json!({"txt": "typo"})))),
        Reason::Invalid
    );
    let mut named = hint("x");
    named.id = "same".into();
    apply(&mut l, LayerOp::Push(named.clone()), None, 0, &lim).unwrap();
    assert_eq!(
        reason(apply(&mut l, LayerOp::Push(named), None, 0, &lim)),
        Reason::Invalid
    );
    // A ring alone is fine.
    apply(
        &mut l,
        LayerOp::Push(Layer::new(Anchor::Caret).with_ring()),
        None,
        0,
        &lim,
    )
    .unwrap();
}

#[test]
fn a_fourth_agent_hint_replaces_that_actors_oldest() {
    let mut l = Layers::default();
    let lim = Limits::default();
    let first = agent(&mut l, hint("1"), 0).unwrap().layer.unwrap();
    agent(&mut l, hint("2"), 600).unwrap();
    agent(&mut l, hint("3"), 1_200).unwrap();
    let r = agent(&mut l, hint("4"), 1_800).unwrap();
    assert_eq!(r.popped, vec![first]);
    assert_eq!(l.layers.len(), 3);
    // A host may lower the cap, never raise it past 8.
    assert_eq!(
        Limits {
            agent_layers: 50,
            ..lim.clone()
        }
        .agent_cap(),
        8
    );
    assert_eq!(
        Limits {
            agent_layers: 1,
            ..lim
        }
        .agent_cap(),
        1
    );
}

#[test]
fn agent_layers_expire_are_attributed_and_stay_in_their_band() {
    let mut l = Layers::default();
    let mut h = Layer::new(Anchor::Caret).with_content(Content::hint(Some("tip"), "body"));
    h.owner = Owner::Host; // claims to be the host
    h.z = 3;
    let id = agent(&mut l, h, 1_000).unwrap().layer.unwrap();
    let got = l.get(&id).unwrap();
    assert_eq!(got.owner, Owner::Agent("helper".into()));
    assert_eq!(got.ttl_ms, Some(8_000));
    assert_eq!(got.z, 20);
    assert_eq!(
        got.content
            .as_ref()
            .unwrap()
            .as_hint()
            .unwrap()
            .title
            .as_deref(),
        Some("◆ helper · tip")
    );
    assert!(!expire(&mut l, 8_999));
    assert!(expire(&mut l, 9_000));

    let mut forever = hint("x");
    forever.ttl_ms = Some(10_000_000);
    let id = agent(&mut l, forever, 20_000).unwrap().layer.unwrap();
    assert_eq!(l.get(&id).unwrap().ttl_ms, Some(60_000));
    let mut brief = hint("x");
    brief.ttl_ms = Some(1);
    let id = agent(&mut l, brief, 21_000).unwrap().layer.unwrap();
    assert_eq!(l.get(&id).unwrap().ttl_ms, Some(1_000));
    assert_eq!(
        l.get(&id)
            .unwrap()
            .content
            .as_ref()
            .unwrap()
            .as_hint()
            .unwrap()
            .title
            .as_deref(),
        Some("◆ helper")
    );
}

#[test]
fn bands_ids_and_the_persons_controls() {
    let mut l = Layers::default();
    let lim = Limits::default();
    let mut g = hint("guide");
    g.owner = Owner::Guide;
    g.z = 99;
    let gid = apply(&mut l, LayerOp::Push(g), None, 5, &lim)
        .unwrap()
        .layer
        .unwrap();
    assert_eq!(l.get(&gid).unwrap().z, 19);
    let aid = agent(&mut l, hint("a"), 6).unwrap().layer.unwrap();
    assert_eq!((gid.as_str(), aid.as_str()), ("L-1", "L-2"));
    let order: Vec<&str> = l.in_order().iter().map(|x| x.id.as_str()).collect();
    assert_eq!(order, vec!["L-1", "L-2"]);
    // The person dismisses the newest, whoever owns it.
    let r = apply(&mut l, LayerOp::PopNewest, None, 7, &lim).unwrap();
    assert_eq!(r.popped, vec![aid]);
    apply(&mut l, LayerOp::Toggle, None, 7, &lim).unwrap();
    assert!(l.hidden);
    let r = apply(&mut l, LayerOp::Clear, None, 7, &lim).unwrap();
    assert_eq!(r.popped, vec![gid]);
}

#[test]
fn a_recorded_op_sequence_replays_to_the_live_layers() {
    let lim = Limits::default();
    // The live session: ops applied as they come, refusals included, expiry by now_ms.
    let ops: Vec<(LayerOp, Option<&str>, u64)> = vec![
        (LayerOp::Push(hint("one")), None, 100),
        (LayerOp::Push(hint("two")), Some("helper"), 200),
        (LayerOp::Push(hint("three")), Some("helper"), 300),
        (LayerOp::Push(hint("refused")), Some("helper"), 400),
        (LayerOp::Pop(Selector::Layer("L-1".into())), None, 500),
        (LayerOp::Toggle, None, 600),
        (LayerOp::Toggle, None, 700),
    ];
    let mut live = Layers::default();
    let mut trace = Vec::new();
    for (op, actor, now) in ops {
        let _ = apply(&mut live, op.clone(), actor, now, &lim);
        trace.push(serde_json::to_string(&(op, actor, now)).unwrap());
    }
    expire(&mut live, 8_250);
    // Replay from the recorded JSON lines into a fresh value.
    let mut again = Layers::default();
    for line in &trace {
        let (op, actor, now): (LayerOp, Option<String>, u64) = serde_json::from_str(line).unwrap();
        let _ = apply(&mut again, op, actor.as_deref(), now, &lim);
    }
    expire(&mut again, 8_250);
    assert_eq!(again, live);
    assert_eq!(live.layers.len(), 1, "{live:?}");
    let json = serde_json::to_string(&live).unwrap();
    assert_eq!(serde_json::from_str::<Layers>(&json).unwrap(), live);
}

#[test]
fn update_keeps_since() {
    let lim = Limits::default();
    let mut l = Layers::default();
    agent(&mut l, hint("two"), 200).unwrap();
    let mut u = hint("changed");
    u.id = "L-1".into();
    apply(&mut l, LayerOp::Update(u), Some("helper"), 900, &lim).unwrap();
    assert_eq!(l.get("L-1").unwrap().since_ms, 200);
}

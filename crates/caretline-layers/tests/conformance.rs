#![cfg(all(feature = "conformance", feature = "caretline"))]
//! The conformance kit: it passes the crate's own plans and catches broken ones (each
//! invariant by a plan tampered with), replays, mapping across documents, the contract against
//! a host bridge, snapshots, and the inspector.

mod common;

use caretline::{Document, Msg, View, Viewport, update_doc_with_changes};
use caretline_layers::conformance::contract::{self, Fixture};
use caretline_layers::conformance::*;
use caretline_layers::ops::{self, Request};
use caretline_layers::*;
use common::*;
use serde_json::{Value, json};

/// A table host: rows with stable keys, as a host draws them.
fn table(size: Size) -> (Grid, AnchorMap) {
    let mut grid = Grid::new(size.w, size.h).with_area(Rect::new(0, 0, size.w, size.h - 1));
    let mut m = AnchorMap::new();
    for (i, (k, name)) in [("a", "Alpha row"), ("b", "Beta row"), ("c", "Gamma row")]
        .iter()
        .enumerate()
    {
        let y = 2 + i as u16 * 2;
        grid.mark_text(4, y, name);
        m.put(
            AnchorKey::host("row", k),
            Rect::new(4, y, name.len() as u16, 1),
        );
    }
    m.put_off(AnchorKey::host("row", "z"), Off::Below { x: Some(4) });
    (grid, m)
}

fn row(k: &str) -> Anchor {
    Anchor::Host {
        kind: "row".into(),
        key: k.into(),
    }
}

fn table_layers() -> Layers {
    let mut l = Layers::default();
    push(
        &mut l,
        Layer::new(row("a"))
            .with_content(hint("Alpha", "The first row."))
            .with_arrow()
            .with_ring(),
        None,
    );
    push(
        &mut l,
        Layer::new(row("c"))
            .with_content(hint("Gamma", "An agent's note on the last row."))
            .with_arrow(),
        Some("helper"),
    );
    push(
        &mut l,
        Layer::new(row("z")).with_content(hint("Below", "Off screen.")),
        None,
    );
    push(
        &mut l,
        Layer::new(row("gone")).with_content(hint("Gone", "Nowhere.")),
        None,
    );
    l
}

#[test]
fn the_kit_passes_a_table_host_at_every_size() {
    let r = renderers();
    let report = check_sizes(
        &|size| {
            let (grid, m) = table(size);
            Scene::new(table_layers(), m, grid, &r)
        },
        &[
            Size::new(140, 40),
            Size::new(100, 30),
            Size::new(80, 24),
            Size::new(44, 16),
        ],
    );
    assert!(report.ok(), "{report}");
    let narrow = &report.sizes[3];
    assert_eq!(narrow.missing, ["L-4"]);
    assert_eq!(narrow.off_screen, ["L-3"]);
    assert!(!narrow.strips.is_empty(), "{report}");
    // The table prints a row per size.
    let text = report.to_string();
    assert!(text.contains("44x16") && text.contains("140x40"), "{text}");
    // And survives JSON.
    let back: Report = serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
    assert_eq!(back, report);
}

/// A plan tampered with one way: the kit names the invariant broken.
fn caught(tamper: impl Fn(&mut Plan), kind: Kind) {
    let r = renderers();
    let (grid, m) = table(Size::new(80, 24));
    let scene = Scene::new(table_layers(), m, grid, &r);
    let mut p = scene.plan();
    assert!(check_plan(&p, &scene).is_empty());
    tamper(&mut p);
    let v = check_plan(&p, &scene);
    assert!(
        v.iter().any(|v| v.kind == kind),
        "{kind:?} not caught: {v:#?}"
    );
}

#[test]
fn each_invariant_catches_a_plan_that_breaks_it() {
    let boxed = |p: &mut Plan| -> usize {
        p.layers
            .iter()
            .position(|l| l.mode == Some(Mode::Box) && l.route.is_some())
            .unwrap()
    };
    caught(
        |p| {
            let i = boxed(p);
            let a = p.layers[i].anchor.clone().unwrap().rects[0];
            p.layers[i].rect = Some(Rect::new(a.x, a.y, 10, 3));
        },
        Kind::CoversAnchor,
    );
    caught(
        |p| {
            let i = boxed(p);
            let r = p.layers[i].rect.unwrap();
            p.layers[i].rect = Some(Rect::new(r.x, 21, r.w, 3));
        },
        Kind::OutsideArea,
    );
    caught(
        |p| {
            let (a, b) = (p.layers[0].rect.unwrap(), 1);
            p.layers[b].rect = Some(a);
        },
        Kind::Overlap,
    );
    caught(
        |p| {
            let i = boxed(p);
            p.layers[i].route.as_mut().unwrap().steps.remove(0);
        },
        Kind::RouteStart,
    );
    caught(
        |p| {
            let i = boxed(p);
            let s = p.layers[i].route.as_mut().unwrap();
            let last = s.steps.len() - 1;
            s.steps[last].leave = Dir::Up;
            s.steps[last].enter = Dir::Up;
            if s.steps.len() > 1 {
                s.steps.truncate(1);
            }
        },
        Kind::RouteEnd,
    );
    caught(
        |p| {
            let i = boxed(p);
            p.layers[i].route = None;
        },
        Kind::NoArrowReason,
    );
    caught(
        |p| {
            let i = boxed(p);
            p.layers[i].covers_avoid = 3;
        },
        Kind::AvoidCovered,
    );
    caught(
        |p| {
            p.missing.clear();
        },
        Kind::Dropped,
    );
    caught(
        |p| {
            let id = p.layers[0].id.clone();
            p.layers.remove(0);
            p.missing.push(id);
        },
        Kind::Missing,
    );
    caught(
        |p| {
            p.regions.clear();
        },
        Kind::Region,
    );
    caught(
        |p| {
            let i = p.layers.iter().position(|l| l.chip.is_some()).unwrap();
            p.layers[i].dock = None;
        },
        Kind::DockApart,
    );
}

#[test]
fn a_box_over_avoid_cells_with_a_clear_one_in_reach_is_caught() {
    // The plan's box is clear; claim its cells are avoided and the kit re-plans to find the
    // clear one placement would have taken.
    let r = renderers();
    let (grid, m) = table(Size::new(80, 24));
    let layers = table_layers();
    let p = plan(&layers, &m, &grid, &r);
    let b = p.layers[0].rect.unwrap();
    let mut avoided = grid.clone();
    avoided.avoid(b, AVOID);
    let scene = Scene::new(layers, m, avoided, &r);
    let mut tampered = p.clone();
    tampered.layers[0].covers_avoid = b.w * b.h;
    let v = check_plan(&tampered, &scene);
    assert!(
        v.iter()
            .any(|v| v.kind == Kind::AvoidCovered && v.detail.contains("in reach")),
        "{v:#?}"
    );
}

#[test]
fn replay_matches_the_live_layers() {
    let ops: Vec<(LayerOp, Option<&str>, u64)> = vec![
        (
            LayerOp::Push(Layer::new(row("a")).with_content(hint("A", "a"))),
            None,
            0,
        ),
        (
            LayerOp::Push(Layer::new(row("b")).with_content(hint("B", "b"))),
            Some("helper"),
            10,
        ),
        (LayerOp::Pop(Selector::Layer("L-1".into())), None, 20),
        (LayerOp::Toggle, None, 30),
    ];
    let limits = Limits::agent_defaults();
    let mut live = Layers::default();
    for (op, actor, now) in &ops {
        let _ = apply(&mut live, op.clone(), *actor, *now, &limits);
    }
    assert!(check_replay(&live, &ops, &limits).is_empty());
    // A live state that drifted (a layer changed outside the log) is caught.
    let mut drifted = live.clone();
    drifted.layers[0].z = 25;
    let v = check_replay(&drifted, &ops, &limits);
    assert_eq!(v[0].kind, Kind::Replay, "{v:#?}");
}

/// Two documents, each in its view: an edit through one moves only the anchors in it.
#[test]
fn mapping_moves_only_the_edited_documents_anchors() {
    let vp = Viewport {
        width: 60,
        height: 10,
    };
    let mut a = Document::new("alpha beta gamma", None);
    let b = Document::new("one two three", None);
    let mut views = vec![View::new(vp)];
    let mut before = Layers::default();
    for anchor in [
        Anchor::scoped("main", Anchor::Text { from: 6, to: 10 }),
        Anchor::scoped("panel", Anchor::Text { from: 4, to: 7 }),
        Anchor::Text { from: 11, to: 16 },
    ] {
        push(
            &mut before,
            Layer::new(anchor).with_content(hint("x", "y")),
            None,
        );
    }
    let (_, changes) = update_doc_with_changes(
        &mut a,
        &mut views,
        0,
        Msg::InsertText {
            text: ">>> ".into(),
        },
    );
    let changes = changes.unwrap();
    let pairs = [("main", "a"), ("panel", "b")];
    let m = Mapping {
        views: &pairs,
        focused: "main",
        edited: "a",
        changes: &changes,
    };
    // The host that says which views show the edited document.
    let mut right = before.clone();
    let edited = m.edited_views();
    observe(
        &mut right,
        Edited::Views {
            views: &edited,
            unscoped: m.unscoped(),
        },
        Some(&changes),
        0,
    );
    assert!(check_mapping(&before, &right, &m).is_empty());
    assert_eq!(
        right.layers[0].anchor[0].unscoped(),
        &Anchor::Text { from: 10, to: 14 }
    );
    assert_eq!(right.layers[1].anchor, before.layers[1].anchor);
    // The host that maps everything moves the panel's anchor into the wrong document.
    let mut wrong = before.clone();
    observe(&mut wrong, Edited::All, Some(&changes), 0);
    let v = check_mapping(&before, &wrong, &m);
    assert_eq!(v.len(), 1, "{v:#?}");
    assert_eq!(v[0].layer.as_deref(), Some("L-2"));
    // Focused on the panel, an edit in `a` leaves unscoped anchors put.
    let m2 = Mapping {
        focused: "panel",
        ..m
    };
    assert!(!m2.unscoped());
    assert_eq!(
        m2.expected(&before).layers[2].anchor,
        before.layers[2].anchor
    );
    let _ = b;
}

/// The test host's bridge: parse, apply, plan, reply, as a protocol handler would.
fn bridge<'a>(
    layers: &'a std::cell::RefCell<Layers>,
    anchors: &'a AnchorMap,
    grid: &'a Grid,
    r: &'a Renderers,
) -> impl FnMut(&Value) -> Value + 'a {
    move |req: &Value| {
        let op = req["op"].as_str().unwrap_or_default();
        let (parsed, actor) = match ops::parse(op, req) {
            Ok(x) => x,
            Err(e) => return ops::error(&e),
        };
        let mut l = layers.borrow_mut();
        match parsed {
            Request::List => ops::list(&l),
            Request::Apply(o) => match apply(&mut l, o, actor.as_deref(), 0, &Limits::default()) {
                Ok(a) => ops::reply(&a, Some(&plan(&l, anchors, grid, r))),
                Err(e) => ops::error(&e),
            },
        }
    }
}

#[test]
fn the_contract_passes_a_bridge_that_answers_right() {
    let r = renderers();
    let (grid, m) = table(Size::new(80, 24));
    let layers = std::cell::RefCell::new(Layers::default());
    let fx = Fixture::new(AnchorKey::host("row", "b"), &m);
    let v = contract::run(&mut bridge(&layers, &m, &grid, &r), &fx);
    assert!(v.is_empty(), "{v:#?}");
    assert!(layers.borrow().layers.is_empty(), "the contract cleans up");
    assert!(contract::cases(&fx).len() >= 15);
    // An agent's requests, under no limits, pass too.
    let fx = fx.with_actor("conformance");
    assert!(contract::run(&mut bridge(&layers, &m, &grid, &r), &fx).is_empty());
}

#[test]
fn the_contract_catches_a_bridge_that_answers_wrong() {
    let r = renderers();
    let (grid, m) = table(Size::new(80, 24));
    let layers = std::cell::RefCell::new(Layers::default());
    // Plans with a stale map: row b is somewhere else.
    let mut stale = AnchorMap::new();
    stale.put(AnchorKey::host("row", "b"), Rect::new(0, 10, 3, 1));
    let fx = Fixture::new(AnchorKey::host("row", "b"), &m);
    let mut inner = bridge(&layers, &stale, &grid, &r);
    let v = contract::run(&mut inner, &fx);
    assert!(
        v.iter()
            .any(|v| v.kind == Kind::Contract && v.detail.contains("resolver's rects")),
        "{v:#?}"
    );
    // Without the plan: no `resolved` at all.
    let mut no_plan = |req: &Value| {
        let mut reply = bridge(&layers, &m, &grid, &r)(req);
        if let Some(o) = reply.as_object_mut() {
            o.remove("resolved");
            o.remove("reason");
        }
        reply
    };
    let v = contract::run(&mut no_plan, &fx);
    assert!(v.iter().any(|v| v.kind == Kind::Contract), "{v:#?}");
    // A reply off the schema.
    let mut wrapped = |req: &Value| json!({"result": bridge(&layers, &m, &grid, &r)(req)});
    let v = contract::run(&mut wrapped, &fx);
    assert!(v.iter().all(|v| v.kind == Kind::Schema), "{v:#?}");
}

#[test]
fn a_snapshot_is_stable_and_shows_each_part() {
    let r = renderers();
    let (grid, m) = table(Size::new(80, 24));
    let layers = table_layers();
    let p = plan(&layers, &m, &grid, &r);
    let frame = "  Documents   \n\n    Alpha row\n";
    let a = snapshot(&p, frame);
    assert_eq!(a, snapshot(&p, frame));
    assert!(a.starts_with("--- frame 80x24\n  Documents\n"), "{a}");
    for c in ['#', '>', '*', 'c'] {
        assert!(
            a.split("--- plan").next().unwrap().contains(c),
            "{c} in {a}"
        );
    }
    // The plan's keys are sorted, whatever serde_json's features.
    let json = &a[a.find("--- plan\n").unwrap() + 9..];
    let v: Value = serde_json::from_str(json).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}

#[test]
fn the_inspector_says_why_the_winner_won() {
    let r = renderers();
    let (grid, m) = table(Size::new(80, 24));
    let layers = table_layers();
    let (p, e) = plan_explained(&layers, &m, &grid, &r);
    assert_eq!(p, plan(&layers, &m, &grid, &r));
    let first = e.get("L-1").unwrap();
    assert_eq!(first.used, Some(0));
    assert!(first.candidates.len() > 1);
    let w = &first.candidates[first.winner.unwrap()];
    assert_eq!(Some(w.side), p.layers[0].side);
    assert!(matches!(w.arrow, Some(Arrow::Routed { .. })));
    assert!(first.why.starts_with("won: "), "{}", first.why);
    let missing = e.get("L-4").unwrap();
    assert_eq!(missing.used, None);
    assert!(missing.why.contains("missing"));
    let text = e.to_string();
    assert!(
        text.contains("candidates") && text.contains("*anchor 1 of 1"),
        "{text}"
    );
    // The plan alone, in words.
    let short = p.explain();
    assert!(
        short.contains("L-1 (host, z 0)") && short.contains("L-4: missing"),
        "{short}"
    );
    // And the explanation survives JSON.
    let back: Explanation = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
    assert_eq!(back, e);
}

#[test]
fn every_golden_scene_passes_check_sizes_over_a_caretline_frame() {
    let r = renderers();
    let report = check_sizes(
        &|size| {
            let s = state(size.w, size.h);
            let frame = caretline::view(&s);
            let grid = Grid::from_frame(&frame);
            let mut layers = Layers::default();
            push(
                &mut layers,
                Layer::new(find("word at a time", 0))
                    .with_content(hint("Jump", "Option and an arrow key."))
                    .with_arrow()
                    .with_ring(),
                None,
            );
            push(
                &mut layers,
                Layer::new(find("Control S", 0)).with_content(hint("Save", "Saves.")),
                Some("helper"),
            );
            let views =
                OwnedViews::new(AnchorMap::new()).view(OwnedView::new(frame).with_doc(s.doc));
            Scene::new(layers, views, grid, &r)
        },
        &[Size::new(140, 40), Size::new(80, 24), Size::new(44, 16)],
    );
    assert!(report.ok(), "{report}");
}

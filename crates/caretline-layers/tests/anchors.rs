#![cfg(feature = "caretline")]
//! Anchors: resolving from a caretline frame and a host's anchor map, mapping through edits,
//! the wire forms, and click regions.

mod common;

use caretline::helix::Selection;
use caretline::{
    ExtChange, Msg, OutlineConfig, State, Viewport, update, update_with_changes, view,
};
use caretline_layers::*;
use common::*;

#[test]
fn a_text_range_resolves_to_its_cells_and_follows_the_frame_offset() {
    let s = state(80, 24);
    let frame = view(&s);
    let r = FrameResolver::new(&frame)
        .resolve(&find("Getting", 0))
        .unwrap();
    assert_eq!(r.rects, vec![Rect::new(2, 0, 7, 1)]);
    let r = FrameResolver::new(&frame)
        .at(10, 3)
        .resolve(&find("Getting", 0))
        .unwrap();
    assert_eq!(r.rects, vec![Rect::new(12, 3, 7, 1)]);
}

#[test]
fn a_wrapped_range_resolves_to_one_rect_per_row() {
    let s = state(44, 16);
    let frame = view(&s);
    let r = FrameResolver::new(&frame)
        .resolve(&find("jump a word", 0))
        .unwrap();
    assert_eq!(r.rects.len(), 2, "{r:?}");
    assert_eq!(r.rects[1].x, 0);
}

#[test]
fn an_anchor_out_of_view_says_which_way() {
    let mut s = state(80, 10);
    let frame = view(&s);
    let below = FrameResolver::new(&frame)
        .resolve(&find("Control S", 0))
        .unwrap();
    assert_eq!(below.off, Some(Off::Below { x: None }));
    update(&mut s, Msg::ScrollView { rows: 12 });
    let frame = view(&s);
    let above = FrameResolver::new(&frame)
        .resolve(&find("Getting", 0))
        .unwrap();
    assert_eq!(above.off, Some(Off::Above { x: None }));
}

#[test]
fn the_caret_resolves_and_host_kinds_are_left_to_the_host() {
    let s = state(80, 24);
    let frame = view(&s);
    let res = FrameResolver::new(&frame).at(1, 1);
    assert_eq!(
        res.resolve(&Anchor::Caret).unwrap().rects,
        vec![Rect::new(1, 1, 1, 1)]
    );
    assert_eq!(
        res.resolve(&Anchor::Host {
            kind: "row".into(),
            key: "a".into()
        }),
        None
    );
}

fn outline() -> State {
    let mut s = State::new(
        "first block\nsecond block\nthird block\n",
        Some("o.md".into()),
        Viewport {
            width: 40,
            height: 10,
        },
    );
    for p in [0, 12, 25] {
        s.doc.marks.mint(p);
    }
    s.enable_outline(OutlineConfig::default());
    update(&mut s, Msg::resize(40, 10));
    s
}

#[test]
fn a_block_and_text_within_it_resolve_from_the_rows() {
    let s = outline();
    let frame = view(&s);
    let res = FrameResolver::new(&frame).with_doc(&s.doc);
    let ids: Vec<u64> = s.blocks().unwrap().blocks.iter().map(|b| b.id.0).collect();
    let second = ids[1];
    let block = res.resolve(&Anchor::Block(second)).unwrap();
    assert_eq!(block.rects.len(), 1, "{block:?}");
    let row = block.rects[0].y;
    let word = res
        .resolve(&Anchor::BlockText {
            block: second,
            from: 7,
            to: 12,
        })
        .unwrap();
    assert_eq!(word.rects.len(), 1);
    assert_eq!(word.rects[0].y, row);
    assert_eq!(word.rects[0].w, 5);
    let res = FrameResolver::new(&frame);
    assert_eq!(
        res.resolve(&Anchor::BlockText {
            block: second,
            from: 7,
            to: 12
        })
        .unwrap(),
        word
    );
    assert_eq!(res.resolve(&Anchor::Block(9999)), None);
}

#[test]
fn an_anchor_map_resolves_host_kinds_and_chain_asks_each_in_turn() {
    let mut map = AnchorMap::new();
    map.put(AnchorKey::host("row", "r1"), Rect::new(0, 2, 10, 1));
    map.put_off(AnchorKey::host("row", "r9"), Off::Below { x: Some(4) });
    map.put(
        AnchorKey::host("diff", "src/main.rs:42"),
        Rect::new(3, 7, 20, 1),
    );
    let s = state(80, 24);
    let frame = view(&s);
    let editor = FrameResolver::new(&frame).at(20, 0);
    let both = Chain(vec![&map, &editor]);
    let host = |k: &str, key: &str| Anchor::Host {
        kind: k.into(),
        key: key.into(),
    };
    assert_eq!(
        both.resolve(&host("row", "r1")).unwrap().rects,
        vec![Rect::new(0, 2, 10, 1)]
    );
    assert_eq!(
        both.resolve(&host("row", "r9")).unwrap().off,
        Some(Off::Below { x: Some(4) })
    );
    assert_eq!(
        both.resolve(&host("diff", "src/main.rs:42")).unwrap().rects,
        vec![Rect::new(3, 7, 20, 1)]
    );
    assert_eq!(
        both.resolve(&find("Getting", 0)).unwrap().rects,
        vec![Rect::new(22, 0, 7, 1)]
    );
    assert_eq!(both.resolve(&host("row", "nope")), None);
}

#[test]
fn the_first_anchor_that_resolves_is_used_and_none_is_missing() {
    let s = state(80, 24);
    let mut layers = Layers::default();
    let mut layer = Layer::new(Anchor::Host {
        kind: "row".into(),
        key: "gone".into(),
    })
    .with_content(hint("Fallback", "This one."));
    layer.anchor.push(find("Getting", 0));
    push(&mut layers, layer, None);
    let (_, p) = plan_over(&s, &layers);
    assert_eq!(
        p.layers[0].anchor.as_ref().unwrap().rects,
        vec![Rect::new(2, 0, 7, 1)]
    );
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(Anchor::Host {
            kind: "row".into(),
            key: "gone".into(),
        })
        .with_content(hint("x", "y")),
        None,
    );
    let (_, p) = plan_over(&s, &layers);
    assert!(p.layers.is_empty());
    assert_eq!(p.missing, vec!["L-1".to_string()]);
}

/// Applies `msg` with the caret at `at` and returns the changes the editor made.
fn edit(s: &mut State, at: usize, msg: Msg) -> caretline::ChangeSet {
    s.view.selection = Selection::point(at);
    update_with_changes(s, msg).1.expect("an edit")
}

fn typed(text: &str) -> Msg {
    Msg::InsertText { text: text.into() }
}

fn replace(from: usize, to: usize, text: &str) -> Msg {
    Msg::External {
        changes: vec![ExtChange::Replace {
            from,
            to,
            text: text.into(),
        }],
    }
}

#[test]
fn text_anchors_follow_edits_and_a_deleted_range_falls_to_the_next() {
    let mut s = State::new(
        "alpha beta gamma",
        None,
        Viewport {
            width: 80,
            height: 24,
        },
    );
    let mut layers = Layers::default();
    let mut layer = Layer::new(Anchor::Text { from: 6, to: 10 }).with_content(hint("t", "b"));
    layer.anchor.push(Anchor::Block(3));
    push(&mut layers, layer, None);

    assert!(map_anchors(
        &mut layers,
        Edited::All,
        &edit(&mut s, 0, typed("XX "))
    ));
    assert_eq!(s.doc.text.to_string(), "XX alpha beta gamma");
    assert_eq!(layers.layers[0].anchor[0], Anchor::Text { from: 9, to: 13 });
    // Undo and redo move it back and forth.
    let n = s.doc.text.len_chars();
    map_anchors(&mut layers, Edited::All, &edit(&mut s, n, Msg::Undo));
    assert_eq!(layers.layers[0].anchor[0], Anchor::Text { from: 6, to: 10 });
    map_anchors(&mut layers, Edited::All, &edit(&mut s, 0, Msg::Redo));
    assert_eq!(layers.layers[0].anchor[0], Anchor::Text { from: 9, to: 13 });
    // An insertion at its start or end stays outside it.
    map_anchors(&mut layers, Edited::All, &edit(&mut s, 9, typed("YY")));
    assert_eq!(s.doc.text.to_string(), "XX alpha YYbeta gamma");
    assert_eq!(
        layers.layers[0].anchor[0],
        Anchor::Text { from: 11, to: 15 }
    );
    map_anchors(&mut layers, Edited::All, &edit(&mut s, 15, typed("ZZ")));
    assert_eq!(
        layers.layers[0].anchor[0],
        Anchor::Text { from: 11, to: 15 }
    );
    // Deleting its text (here, a change from elsewhere) collapses it: the block anchor takes
    // over.
    assert!(map_anchors(
        &mut layers,
        Edited::All,
        &edit(&mut s, 0, replace(11, 15, ""))
    ));
    assert_eq!(s.doc.text.to_string(), "XX alpha YYZZ gamma");
    assert_eq!(layers.layers[0].anchor, vec![Anchor::Block(3)]);

    let mut s = State::new(
        "ab cd",
        None,
        Viewport {
            width: 80,
            height: 24,
        },
    );
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(Anchor::Text { from: 0, to: 2 }).with_content(hint("t", "b")),
        None,
    );
    // A message that changes no text has no changes to map.
    let (_, none) = update_with_changes(&mut s, Msg::Tick { now_ms: 0 });
    assert!(!observe(&mut layers, Edited::All, none.as_ref(), 0));
    let (_, changes) = update_with_changes(&mut s, replace(0, 2, ""));
    assert!(observe(&mut layers, Edited::All, changes.as_ref(), 0));
    assert!(layers.layers.is_empty());
}

#[test]
fn observe_expires_layers_by_now_ms() {
    let mut layers = Layers::default();
    let mut l = Layer::new(Anchor::Caret).with_content(hint("t", "b"));
    l.ttl_ms = Some(500);
    apply(
        &mut layers,
        LayerOp::Push(l),
        None,
        1_000,
        &Limits::default(),
    )
    .unwrap();
    assert!(!observe(&mut layers, Edited::All, None, 1_499));
    assert!(observe(&mut layers, Edited::All, None, 1_500));
    assert!(layers.layers.is_empty());
}

#[test]
fn anchors_and_layers_have_strict_wire_forms() {
    let a: Vec<Anchor> = serde_json::from_str(
        r#"[{"text":{"from":4,"to":9}},{"text":{"block":7,"from":1,"to":2}},{"block":7},{"caret":true},{"screen":"center"},{"host":{"kind":"row","key":"k"}}]"#,
    )
    .unwrap();
    assert_eq!(
        a,
        vec![
            Anchor::Text { from: 4, to: 9 },
            Anchor::BlockText {
                block: 7,
                from: 1,
                to: 2
            },
            Anchor::Block(7),
            Anchor::Caret,
            Anchor::Screen(ScreenPos::Center),
            Anchor::Host {
                kind: "row".into(),
                key: "k".into()
            },
        ]
    );
    let back: Vec<Anchor> = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    assert_eq!(back, a);
    assert!(serde_json::from_str::<Anchor>(r#"{"text":{"from":1,"to":2,"extra":1}}"#).is_err());
    assert!(serde_json::from_str::<Anchor>(r#"{"caret":false}"#).is_err());
    // Screen positions are never anchors.
    assert!(serde_json::from_str::<Anchor>(r#"{"cells":{"x":1,"y":2,"w":3,"h":1}}"#).is_err());

    let layer: Layer = serde_json::from_str(
        r#"{"id":"guide:2","owner":"guide","z":10,"anchor":[{"text":{"from":4,"to":9}}],
            "content":{"kind":"hint","data":{"title":"T","text":"B"}},"arrow":true,
            "ring":{"pulse":{"period_ms":800,"cycles":2}},"spotlight":{"holes":["anchor","box"]},
            "place":["below","above"],"max_width":40}"#,
    )
    .unwrap();
    assert_eq!(layer.owner, Owner::Guide);
    assert_eq!(
        layer
            .content
            .as_ref()
            .unwrap()
            .as_hint()
            .unwrap()
            .title
            .as_deref(),
        Some("T")
    );
    assert!(serde_json::from_str::<Layer>(r#"{"anchor":[],"colour":"red"}"#).is_err());
    let o: Owner = serde_json::from_str(r#""agent:helper""#).unwrap();
    assert_eq!(o, Owner::Agent("helper".into()));
    assert!(serde_json::from_str::<Owner>(r#""agent:""#).is_err());
    let layers = Layers {
        layers: vec![layer],
        next: 3,
        hidden: false,
        recent: Default::default(),
    };
    let json = serde_json::to_string(&layers).unwrap();
    assert_eq!(serde_json::from_str::<Layers>(&json).unwrap(), layers);
    let op: LayerOp = serde_json::from_str(r#"{"pop":{"layer":"L-4"}}"#).unwrap();
    assert_eq!(op, LayerOp::Pop(Selector::Layer("L-4".into())));
}

#[test]
fn regions_hit_boxes_and_chips() {
    let s = state(80, 10);
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(find("Undo steps", 0)).with_content(hint("Undo", "One step at a time.")),
        None,
    );
    push(
        &mut layers,
        Layer::new(find("Getting", 0)).with_content(hint("Top", "The title.")),
        None,
    );
    let (_, p) = plan_over(&s, &layers);
    let chip = p.layers[0].chip.expect("an edge chip");
    assert_eq!(p.hit(chip.x, chip.y).unwrap().id, "L-1/reveal");
    let r = p.layers[1].rect.unwrap();
    assert_eq!(p.hit(r.x, r.y).unwrap().id, "L-2");
    assert_eq!(p.hit(0, 9), None);
}

#[test]
fn hidden_layers_place_nothing() {
    let s = state(80, 24);
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(find("Getting", 0)).with_content(hint("t", "b")),
        None,
    );
    apply(&mut layers, LayerOp::Toggle, None, 0, &Limits::default()).unwrap();
    let (_, p) = plan_over(&s, &layers);
    assert!(p.layers.is_empty() && p.regions.is_empty());
}

#[test]
fn a_screen_anchor_centres_the_box_without_an_arrow() {
    let s = state(80, 24);
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(Anchor::Screen(ScreenPos::Center))
            .with_content(hint("Welcome", "A short tour."))
            .with_arrow(),
        None,
    );
    let (_, p) = plan_over(&s, &layers);
    let l = &p.layers[0];
    assert!(l.route.is_none());
    let r = l.rect.unwrap();
    assert!((79..=80).contains(&(r.x * 2 + r.w)), "{r:?}");
}

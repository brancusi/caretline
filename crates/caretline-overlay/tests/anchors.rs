//! Anchors: resolving from a caretline frame and a host's anchor map, mapping through edits,
//! the wire forms, and click regions.

mod common;

use caretline::{Msg, OutlineConfig, State, Viewport, update, view};
use caretline_overlay::*;
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
fn the_caret_and_cells_resolve() {
    let s = state(80, 24);
    let frame = view(&s);
    let res = FrameResolver::new(&frame).at(1, 1);
    assert_eq!(
        res.resolve(&Anchor::Caret).unwrap().rects,
        vec![Rect::new(1, 1, 1, 1)]
    );
    assert_eq!(
        res.resolve(&Anchor::Cells(Rect::new(3, 4, 5, 1)))
            .unwrap()
            .rects,
        vec![Rect::new(3, 4, 5, 1)]
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
    update(
        &mut s,
        Msg::Resize {
            width: 40,
            height: 10,
        },
    );
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
    // Without the document, the block's first visible row gives its start.
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
    let s = state(80, 24);
    let frame = view(&s);
    let editor = FrameResolver::new(&frame).at(20, 0);
    let both = Chain(vec![&map, &editor]);
    assert_eq!(
        both.resolve(&Anchor::Host {
            kind: "row".into(),
            key: "r1".into()
        })
        .unwrap()
        .rects,
        vec![Rect::new(0, 2, 10, 1)]
    );
    assert_eq!(
        both.resolve(&Anchor::Host {
            kind: "row".into(),
            key: "r9".into()
        })
        .unwrap()
        .off,
        Some(Off::Below { x: Some(4) })
    );
    assert_eq!(
        both.resolve(&find("Getting", 0)).unwrap().rects,
        vec![Rect::new(22, 0, 7, 1)]
    );
    assert_eq!(
        both.resolve(&Anchor::Host {
            kind: "row".into(),
            key: "nope".into()
        }),
        None
    );
}

#[test]
fn the_first_anchor_that_resolves_is_used() {
    let s = state(80, 24);
    let mut layers = Layers::default();
    let mut layer = Layer::new(
        Anchor::Host {
            kind: "row".into(),
            key: "gone".into(),
        },
        vec![callout("Fallback", "This one.")],
    );
    layer.anchor.push(find("Getting", 0));
    push(&mut layers, layer, None);
    let (_, scene) = draw(&s, &layers, Glyphs::Rounded);
    assert_eq!(
        scene.layers[0].anchor.as_ref().unwrap().rects,
        vec![Rect::new(2, 0, 7, 1)]
    );
    // None resolves: reported missing, nothing drawn.
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(
            Anchor::Host {
                kind: "row".into(),
                key: "gone".into(),
            },
            vec![callout("x", "y")],
        ),
        None,
    );
    let (_, scene) = draw(&s, &layers, Glyphs::Rounded);
    assert!(scene.layers.is_empty());
    assert_eq!(scene.missing, vec!["L-1".to_string()]);
}

#[test]
fn text_anchors_follow_edits_and_a_deleted_range_falls_to_the_next() {
    let old = "alpha beta gamma";
    let mut layers = Layers::default();
    let mut layer = Layer::new(Anchor::Text { from: 6, to: 10 }, vec![callout("t", "b")]);
    layer.anchor.push(Anchor::Block(3));
    push(&mut layers, layer, None);

    // An insertion before it shifts it; one at its start stays outside it.
    let cs = changes_between(old, "XX alpha beta gamma");
    assert!(map_anchors(&mut layers, &cs));
    assert_eq!(layers.layers[0].anchor[0], Anchor::Text { from: 9, to: 13 });
    let cs = changes_between("XX alpha beta gamma", "XX alpha YYbeta gamma");
    map_anchors(&mut layers, &cs);
    assert_eq!(
        layers.layers[0].anchor[0],
        Anchor::Text { from: 11, to: 15 }
    );
    // At its end, too.
    let cs = changes_between("XX alpha YYbeta gamma", "XX alpha YYbetaZZ gamma");
    map_anchors(&mut layers, &cs);
    assert_eq!(
        layers.layers[0].anchor[0],
        Anchor::Text { from: 11, to: 15 }
    );

    // Deleting its text collapses it: the block anchor takes over.
    let cs = changes_between("XX alpha YYbetaZZ gamma", "XX alpha YYZZ gamma");
    assert!(map_anchors(&mut layers, &cs));
    assert_eq!(layers.layers[0].anchor, vec![Anchor::Block(3)]);

    // A layer with only a text anchor goes when its text does.
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(Anchor::Text { from: 0, to: 2 }, vec![callout("t", "b")]),
        None,
    );
    assert!(observe(
        &mut layers,
        Some(&changes_between("ab cd", " cd")),
        0
    ));
    assert!(layers.layers.is_empty());
}

#[test]
fn observe_expires_layers_by_now_ms() {
    let mut layers = Layers::default();
    let mut l = Layer::new(Anchor::Caret, vec![callout("t", "b")]);
    l.ttl_ms = Some(500);
    apply(
        &mut layers,
        LayerOp::Push(l),
        None,
        1_000,
        &Limits::default(),
    )
    .unwrap();
    assert!(!observe(&mut layers, None, 1_499));
    assert!(observe(&mut layers, None, 1_500));
    assert!(layers.layers.is_empty());
}

#[test]
fn anchors_and_layers_have_strict_wire_forms() {
    let a: Vec<Anchor> =
        serde_json::from_str(r#"[{"text":{"from":4,"to":9}},{"text":{"block":7,"from":1,"to":2}},{"block":7},{"caret":true},{"cells":{"x":1,"y":2,"w":3,"h":1}},{"screen":"center"},{"host":{"kind":"row","key":"k"}}]"#).unwrap();
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
            Anchor::Cells(Rect::new(1, 2, 3, 1)),
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

    let layer: Layer = serde_json::from_str(
        r#"{"id":"guide:2","owner":"guide","z":10,"anchor":[{"text":{"from":4,"to":9}}],"dim":true,
            "items":[{"spotlight":{"holes":["anchor","callout"]}},{"callout":{"title":"T","body":"B","place":["below","above"],"max_width":40}},
                     {"arrow":{"from":"callout","to":"anchor"}},{"ring":{"around":"anchor","pulse":{"period_ms":800,"cycles":2}}},{"steps":{"of":11,"at":2}},{"keys":["a.b"]}]}"#,
    )
    .unwrap();
    assert_eq!(layer.owner, Owner::Guide);
    assert!(layer.dims());
    assert!(serde_json::from_str::<Layer>(r#"{"anchor":[],"items":[],"colour":"red"}"#).is_err());
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
fn regions_hit_callouts_and_their_chips() {
    let s = state(80, 24);
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(
            find("Undo steps", 0),
            vec![Item::Callout(Callout {
                title: Some("Undo".into()),
                body: "One step at a time.".into(),
                chips: vec![Chip {
                    id: "close".into(),
                    text: "{{key:hint.dismiss}} close".into(),
                }],
                ..Callout::default()
            })],
        ),
        None,
    );
    let (_, scene) = draw(&s, &layers, Glyphs::Rounded);
    let callout = scene.layers[0].panels[0].rect;
    let chip = scene
        .regions
        .iter()
        .find(|r| r.id == "L-1/chip/close")
        .expect("a chip region")
        .rect;
    assert!(callout.intersection(&chip) == chip);
    assert_eq!(hit(&scene, chip.x, chip.y).unwrap().id, "L-1/chip/close");
    assert_eq!(hit(&scene, callout.x, callout.y).unwrap().id, "L-1");
    assert_eq!(hit(&scene, 0, 23), None);
}

#[test]
fn hidden_layers_draw_nothing() {
    let s = state(80, 24);
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(find("Getting", 0), vec![callout("t", "b")]),
        None,
    );
    apply(&mut layers, LayerOp::Toggle, None, 0, &Limits::default()).unwrap();
    let (grid, scene) = draw(&s, &layers, Glyphs::Rounded);
    assert!(scene.is_empty());
    assert_eq!(grid.to_text(), TestGrid::from_frame(&view(&s)).to_text());
}

#[test]
fn a_screen_anchor_centres_the_callout_without_an_arrow() {
    let s = state(80, 24);
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(
            Anchor::Screen(ScreenPos::Center),
            vec![
                callout("Welcome", "A short tour."),
                Item::Arrow(Arrow::default()),
            ],
        ),
        None,
    );
    let (_, scene) = draw(&s, &layers, Glyphs::Rounded);
    let p = &scene.layers[0];
    assert!(p.arrow.is_none());
    let r = p.panels[0].rect;
    assert!((79..=80).contains(&(r.x * 2 + r.w)), "{r:?}");
}

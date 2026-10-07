#![cfg(feature = "caretline")]
//! Placement and tracking with sizes the host's renderer gives: sides, flip, shift, clamp,
//! collisions, strips, edge chips, routes, holes and regions, on synthetic screens.

mod common;

use caretline_layers::*;
use serde_json::{Value, json};

/// A host whose `card` content is a fixed size.
fn sized(w: u16, h: u16) -> Renderers {
    Renderers::new().register("card", move |_: &Value, avail: Size| {
        Size::new(w.min(avail.w), h)
    })
}

fn card() -> Content {
    Content::new("card", json!({"text": "hello"}))
}

fn at(anchors: &mut AnchorMap, key: &str, r: Rect) -> Anchor {
    anchors.put(AnchorKey::host("row", key), r);
    Anchor::Host {
        kind: "row".into(),
        key: key.into(),
    }
}

fn one(layer: Layer) -> Layers {
    let mut l = Layers::default();
    apply(&mut l, LayerOp::Push(layer), None, 0, &Limits::default()).unwrap();
    l
}

#[test]
fn below_first_then_flips_above_near_the_bottom() {
    let grid = Grid::new(80, 24).with_area(Rect::new(0, 0, 80, 23));
    let mut m = AnchorMap::new();
    let a = at(&mut m, "top", Rect::new(10, 2, 6, 1));
    let p = plan(
        &one(Layer::new(a).with_content(card())),
        &m,
        &grid,
        &sized(20, 4),
    );
    assert_eq!(p.layers[0].side, Some(Side::Below));
    assert!(p.layers[0].rect.unwrap().y >= 4);

    let b = at(&mut m, "low", Rect::new(10, 21, 6, 1));
    let p = plan(
        &one(Layer::new(b).with_content(card())),
        &m,
        &grid,
        &sized(20, 4),
    );
    assert_eq!(p.layers[0].side, Some(Side::Above));
    assert!(p.layers[0].rect.unwrap().bottom() <= 21);
}

#[test]
fn the_box_shifts_to_stay_inside_and_honours_the_layers_sides() {
    let grid = Grid::new(60, 20);
    let mut m = AnchorMap::new();
    let a = at(&mut m, "edge", Rect::new(56, 5, 4, 1));
    let p = plan(
        &one(Layer::new(a.clone()).with_content(card())),
        &m,
        &grid,
        &sized(30, 3),
    );
    let r = p.layers[0].rect.unwrap();
    assert!(r.right() <= 60 && r.x <= 56, "{r:?}");

    let mut left = Layer::new(a).with_content(card());
    left.place = vec![Side::Left];
    let p = plan(&one(left), &m, &grid, &sized(30, 3));
    assert_eq!(p.layers[0].side, Some(Side::Left));
    assert!(p.layers[0].rect.unwrap().right() < 56);
}

#[test]
fn boxes_avoid_protected_cells_the_caret_for_agents_and_earlier_boxes() {
    let grid = Grid::new(80, 24).with_protect(vec![Rect::new(0, 6, 80, 6)]);
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(30, 5, 4, 1));
    let p = plan(
        &one(Layer::new(a.clone()).with_content(card())),
        &m,
        &grid,
        &sized(20, 4),
    );
    assert!(
        !p.layers[0]
            .rect
            .unwrap()
            .intersects(&Rect::new(0, 6, 80, 6))
    );

    let mut l = Layers::default();
    for _ in 0..2 {
        apply(
            &mut l,
            LayerOp::Push(Layer::new(a.clone()).with_content(card())),
            None,
            0,
            &Limits::default(),
        )
        .unwrap();
    }
    let p = plan(&l, &m, &Grid::new(80, 24), &sized(20, 4));
    let (r1, r2) = (p.layers[0].rect.unwrap(), p.layers[1].rect.unwrap());
    assert!(!r1.intersects(&r2), "{r1:?} {r2:?}");

    let caret = (31, 7);
    let grid = Grid::new(80, 24).with_caret(Some(caret));
    let mut l = Layers::default();
    apply(
        &mut l,
        LayerOp::Push(Layer::new(a).with_content(card())),
        Some("helper"),
        0,
        &Limits::default(),
    )
    .unwrap();
    let p = plan(&l, &m, &grid, &sized(20, 4));
    assert!(p.layers[0].agent);
    assert!(!p.layers[0].rect.unwrap().contains(caret.0, caret.1));
}

#[test]
fn box_edges_never_fall_inside_a_wide_grapheme() {
    let mut grid = Grid::new(40, 20);
    for y in 0..20 {
        grid.mark_text(0, y, "漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字");
    }
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(10, 3, 4, 1));
    let p = plan(
        &one(Layer::new(a).with_content(card())),
        &m,
        &grid,
        &sized(13, 3),
    );
    let r = p.layers[0].rect.unwrap();
    assert!(!grid.splits(&r), "{r:?}");
    assert_eq!(r.x % 2, 0);
}

#[test]
fn narrow_areas_and_boxes_that_dont_fit_become_strips() {
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(5, 6, 4, 1));
    let narrow = Grid::new(40, 16).with_area(Rect::new(0, 0, 40, 15));
    let p = plan(
        &one(Layer::new(a.clone()).with_content(card())),
        &m,
        &narrow,
        &sized(20, 4),
    );
    assert_eq!(p.layers[0].mode, Some(Mode::Strip));
    assert_eq!(p.layers[0].rect, Some(Rect::new(0, 0, 40, 1)));
    let top = at(&mut m, "top", Rect::new(5, 0, 4, 1));
    let p = plan(
        &one(Layer::new(top).with_content(card())),
        &m,
        &narrow,
        &sized(20, 4),
    );
    assert_eq!(p.layers[0].rect, Some(Rect::new(0, 14, 40, 1)));
    // A box that keeps its size whatever room it's given fits on no side.
    let mid = at(&mut m, "mid", Rect::new(30, 6, 4, 1));
    let fixed = Renderers::new().register("card", |_: &Value, _: Size| Size::new(50, 30));
    let p = plan(
        &one(Layer::new(mid.clone()).with_content(card())),
        &m,
        &Grid::new(80, 24),
        &fixed,
    );
    assert_eq!(p.layers[0].mode, Some(Mode::Strip));
    assert_eq!(p.regions[0].id, "L-1");
    // One that narrows to the room it's given fits on the right (44 columns there).
    let p = plan(
        &one(Layer::new(mid).with_content(card())),
        &m,
        &Grid::new(80, 24),
        &sized(50, 30),
    );
    assert_eq!(p.layers[0].mode, Some(Mode::Box));
    assert_eq!(p.layers[0].side, Some(Side::Right));
    assert_eq!(p.layers[0].rect, Some(Rect::new(36, 0, 44, 24)));
}

#[test]
fn an_off_screen_anchor_gets_a_chip_and_the_box_docks_beside_it() {
    let grid = Grid::new(80, 24).with_area(Rect::new(0, 0, 80, 23));
    let mut m = AnchorMap::new();
    m.put_off(AnchorKey::host("row", "far"), Off::Below { x: Some(12) });
    let a = Anchor::Host {
        kind: "row".into(),
        key: "far".into(),
    };
    let mut layer = Layer::new(a).with_content(card()).with_arrow().with_ring();
    let p = plan(&one(layer.clone()), &m, &grid, &sized(20, 3));
    let l = &p.layers[0];
    assert_eq!(l.chip, Some(Rect::new(12, 22, 8, 1)));
    assert_eq!(l.side, Some(Side::Above));
    assert!(l.rect.unwrap().bottom() <= 22);
    assert!(l.route.is_none() && l.ring.is_empty());
    assert_eq!(p.hit(13, 22).unwrap().id, "L-1/reveal");
    layer.hide_off_screen = true;
    let p = plan(&one(layer), &m, &grid, &sized(20, 3));
    assert!(p.layers[0].rect.is_none() && p.layers[0].chip.is_some());
}

#[test]
fn the_route_runs_from_the_border_to_beside_the_anchor_round_words() {
    let mut grid = Grid::new(80, 24);
    for x in 0..80 {
        if x != 40 {
            grid.set_kind(x, 12, CellKind::Text);
        }
    }
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(38, 15, 4, 1));
    let mut layer = Layer::new(a).with_content(card()).with_arrow();
    layer.place = vec![Side::Above];
    let p = plan(&one(layer), &m, &grid, &sized(24, 4));
    let l = &p.layers[0];
    let r = l.rect.unwrap();
    let route = l.route.as_ref().expect("a route");
    let first = route.steps[0];
    let last = *route.steps.last().unwrap();
    assert_eq!(route.junction, (first.x, r.bottom() - 1));
    assert_eq!((first.y, first.enter), (r.bottom(), Dir::Down));
    assert_eq!((last.y, last.leave), (14, Dir::Down));
    assert!((38..42).contains(&last.x));
    for s in &route.steps {
        assert_ne!(grid.kind(s.x, s.y), CellKind::Text, "crossed text at {s:?}");
    }
    for w in route.steps.windows(2) {
        assert_eq!(w[0].x.abs_diff(w[1].x) + w[0].y.abs_diff(w[1].y), 1);
        assert_eq!(w[0].leave, w[1].enter);
    }
}

#[test]
fn a_spotlight_leaves_holes_for_the_anchor_and_the_box() {
    let grid = Grid::new(80, 24).with_area(Rect::new(0, 0, 80, 23));
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(10, 5, 6, 1));
    let p = plan(
        &one(Layer::new(a).with_content(card()).with_spotlight()),
        &m,
        &grid,
        &sized(20, 3),
    );
    let s = &p.spots[0];
    assert_eq!(s.area, Rect::new(0, 0, 80, 23));
    assert_eq!(s.holes[0], Rect::new(9, 5, 8, 1));
    assert_eq!(s.holes[1], p.layers[0].rect.unwrap());
    assert!(p.dimmed(0, 0) && !p.dimmed(9, 5) && !p.dimmed(0, 23));
}

#[test]
fn the_renderer_measures_and_unknown_kinds_get_no_box() {
    let grid = Grid::new(80, 24);
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(10, 5, 6, 1));
    let layer = Layer::new(a).with_content(card()).with_ring();
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = asked.clone();
    let r = Renderers::new().register("card", move |data: &Value, avail: Size| {
        seen.borrow_mut().push(avail);
        Size::new(data["text"].as_str().unwrap().len() as u16 + 4, 3)
    });
    let p = plan(&one(layer.clone()), &m, &grid, &r);
    assert_eq!(p.layers[0].rect.map(|r| (r.w, r.h)), Some((9, 3)));
    // Once per side, in order, with that side's room: below (a row's gap for an arrow),
    // above, right and left (two columns' gap), at most 52 wide.
    assert_eq!(
        *asked.borrow(),
        vec![
            Size::new(52, 17),
            Size::new(52, 4),
            Size::new(52, 24),
            Size::new(8, 24)
        ]
    );
    assert_eq!(p.layers[0].ring, vec![Rect::new(10, 5, 6, 1)]);
    let p = plan(&one(layer), &m, &grid, &Renderers::new());
    assert!(p.layers[0].rect.is_none() && p.regions.is_empty());
    assert_eq!(p.unrendered, vec!["L-1".to_string()]);
}

#[test]
fn placement_is_a_pure_function_of_its_inputs() {
    let s = common::state(80, 24);
    let mut l = Layers::default();
    let layer = Layer::new(common::find("word at a time", 0))
        .with_content(card())
        .with_arrow()
        .with_ring();
    apply(&mut l, LayerOp::Push(layer), None, 0, &Limits::default()).unwrap();
    let frame = caretline::view(&s);
    let res = FrameResolver::new(&frame);
    let grid = Grid::from_frame(&frame);
    let a = plan(&l, &res, &grid, &sized(30, 4));
    // The same inputs, rebuilt from scratch, give the same plan.
    let frame2 = caretline::view(&s);
    let b = plan(
        &l.clone(),
        &FrameResolver::new(&frame2),
        &Grid::from_frame(&frame2),
        &sized(30, 4),
    );
    assert_eq!(a, b);
    // Resolved anchors are all placement reads: a map holding the same rects gives the same boxes.
    let mut m = AnchorMap::new();
    m.put(
        AnchorKey::host("t", "w"),
        a.layers[0].anchor.as_ref().unwrap().rects[0],
    );
    let mut moved = l.clone();
    moved.layers[0].anchor = vec![Anchor::Host {
        kind: "t".into(),
        key: "w".into(),
    }];
    let c = plan(&moved, &m, &grid, &sized(30, 4));
    assert_eq!(c.layers[0].rect, a.layers[0].rect);
    assert_eq!(c.layers[0].route, a.layers[0].route);
}

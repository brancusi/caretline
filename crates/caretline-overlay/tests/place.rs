//! Placement and tracking with sizes the host gives: sides, flip, shift, clamp, collisions,
//! strips, edge chips, routes, holes and regions, on synthetic screens.

mod common;

use caretline_overlay::*;
use serde_json::json;

/// A host whose content is a fixed size.
fn sized(w: u16, h: u16) -> impl Fn(&Layer, (u16, u16)) -> Option<(u16, u16)> {
    move |_: &Layer, max: (u16, u16)| Some((w.min(max.0), h))
}

fn card() -> Item {
    Item::Content {
        kind: "card".into(),
        data: json!({"text": "hello"}),
    }
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
    let p = plan(&one(Layer::new(a, vec![card()])), &m, &grid, &sized(20, 4));
    assert_eq!(p.layers[0].side, Some(Side::Below));
    let r = p.layers[0].rect.unwrap();
    assert!(r.y >= 4, "{r:?}");

    let b = at(&mut m, "low", Rect::new(10, 21, 6, 1));
    let p = plan(&one(Layer::new(b, vec![card()])), &m, &grid, &sized(20, 4));
    assert_eq!(p.layers[0].side, Some(Side::Above));
    assert!(p.layers[0].rect.unwrap().bottom() <= 21);
}

#[test]
fn the_box_shifts_to_stay_inside_and_honours_the_layers_sides() {
    let grid = Grid::new(60, 20);
    let mut m = AnchorMap::new();
    let a = at(&mut m, "edge", Rect::new(56, 5, 4, 1));
    let p = plan(
        &one(Layer::new(a.clone(), vec![card()])),
        &m,
        &grid,
        &sized(30, 3),
    );
    let r = p.layers[0].rect.unwrap();
    assert!(r.right() <= 60 && r.x <= 56, "{r:?}");

    let mut left = Layer::new(a, vec![card()]);
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
        &one(Layer::new(a.clone(), vec![card()])),
        &m,
        &grid,
        &sized(20, 4),
    );
    let r = p.layers[0].rect.unwrap();
    assert!(!r.intersects(&Rect::new(0, 6, 80, 6)), "{r:?}");

    // Two layers at the same anchor never overlap.
    let mut l = Layers::default();
    apply(
        &mut l,
        LayerOp::Push(Layer::new(a.clone(), vec![card()])),
        None,
        0,
        &Limits::default(),
    )
    .unwrap();
    apply(
        &mut l,
        LayerOp::Push(Layer::new(a.clone(), vec![card()])),
        None,
        0,
        &Limits::default(),
    )
    .unwrap();
    let p = plan(&l, &m, &Grid::new(80, 24), &sized(20, 4));
    let (r1, r2) = (p.layers[0].rect.unwrap(), p.layers[1].rect.unwrap());
    assert!(!r1.intersects(&r2), "{r1:?} {r2:?}");

    // An agent's box never covers the caret.
    let caret = (31, 7);
    let grid = Grid::new(80, 24).with_caret(Some(caret));
    let mut l = Layers::default();
    apply(
        &mut l,
        LayerOp::Push(Layer::new(a, vec![card()])),
        Some("helper"),
        0,
        &Limits::default(),
    )
    .unwrap();
    let p = plan(&l, &m, &grid, &sized(20, 4));
    assert!(!p.layers[0].rect.unwrap().contains(caret.0, caret.1));
}

#[test]
fn narrow_areas_and_boxes_that_dont_fit_become_strips() {
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(5, 6, 4, 1));
    let narrow = Grid::new(40, 16).with_area(Rect::new(0, 0, 40, 15));
    let p = plan(
        &one(Layer::new(a.clone(), vec![card()])),
        &m,
        &narrow,
        &sized(20, 4),
    );
    assert_eq!(p.layers[0].mode, Some(Mode::Strip));
    assert_eq!(p.layers[0].rect, Some(Rect::new(0, 0, 40, 1)));
    // On the top row, the strip goes to the bottom of the area.
    let top = at(&mut m, "top", Rect::new(5, 0, 4, 1));
    let p = plan(
        &one(Layer::new(top, vec![card()])),
        &m,
        &narrow,
        &sized(20, 4),
    );
    assert_eq!(p.layers[0].rect, Some(Rect::new(0, 14, 40, 1)));
    // Too big for any side of a wide area.
    let wide = Grid::new(80, 24);
    let p = plan(&one(Layer::new(a, vec![card()])), &m, &wide, &sized(50, 30));
    assert_eq!(p.layers[0].mode, Some(Mode::Strip));
    assert_eq!(p.regions[0].id, "L-1");
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
    let mut layer = Layer::new(a, vec![card(), Item::Arrow(Arrow::default())]);
    layer.items.push(Item::Ring(Ring::default()));
    let p = plan(&one(layer.clone()), &m, &grid, &sized(20, 3));
    let l = &p.layers[0];
    assert_eq!(l.chip, Some(Rect::new(12, 22, 10, 1)));
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
    // A wall of text between the box's likely place and the anchor, with one gap at x = 40.
    let mut grid = Grid::new(80, 24);
    for x in 0..80 {
        if x != 40 {
            grid.set_kind(x, 12, CellKind::Text);
        }
    }
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(38, 15, 4, 1));
    let mut layer = Layer::new(a, vec![card(), Item::Arrow(Arrow::default())]);
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
    // Consecutive steps are neighbours.
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
        &one(Layer::new(
            a,
            vec![card(), Item::Spotlight(Spotlight::default())],
        )),
        &m,
        &grid,
        &sized(20, 3),
    );
    let s = &p.spots[0];
    assert_eq!(s.area, Rect::new(0, 0, 80, 23));
    assert_eq!(s.holes[0], Rect::new(9, 5, 8, 1));
    assert_eq!(s.holes[1], p.layers[0].rect.unwrap());
}

#[test]
fn the_host_measures_its_content_and_none_means_no_box() {
    let grid = Grid::new(80, 24);
    let mut m = AnchorMap::new();
    let a = at(&mut m, "a", Rect::new(10, 5, 6, 1));
    let layer = Layer::new(a, vec![card(), Item::Ring(Ring::default())]);
    let measure = |l: &Layer, max: (u16, u16)| {
        assert_eq!(max, (52, 24));
        match &l.items[0] {
            Item::Content { kind, .. } if kind == "card" => Some((12, 3)),
            _ => None,
        }
    };
    let p = plan(&one(layer.clone()), &m, &grid, &measure);
    assert_eq!(p.layers[0].rect.map(|r| (r.w, r.h)), Some((12, 3)));
    assert_eq!(p.layers[0].ring, vec![Rect::new(10, 5, 6, 1)]);
    let p = plan(&one(layer), &m, &grid, &|_: &Layer, _: (u16, u16)| None);
    assert!(p.layers[0].rect.is_none());
    assert!(p.regions.is_empty());
}

#[test]
fn plan_json_golden() {
    let s = common::state(80, 24);
    let frame = caretline::view(&s);
    let res = FrameResolver::new(&frame);
    let cells = TestGrid::from_frame(&frame);
    let grid = Grid::scan(&cells).with_area(Rect::new(0, 0, 80, 23));
    let mut l = Layers::default();
    let mut layer = Layer::new(
        common::find("word at a time", 0),
        vec![
            card(),
            Item::Arrow(Arrow::default()),
            Item::Ring(Ring::default()),
            Item::Spotlight(Spotlight::default()),
        ],
    );
    layer.owner = Owner::Guide;
    apply(&mut l, LayerOp::Push(layer), None, 0, &Limits::default()).unwrap();
    let p = plan(&l, &res, &grid, &sized(30, 4));
    common::golden(
        "plan.80x24.json",
        &(serde_json::to_string_pretty(&p).unwrap() + "\n"),
    );
}

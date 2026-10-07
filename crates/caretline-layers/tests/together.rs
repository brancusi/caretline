#![cfg(feature = "caretline")]
//! Layers placed together, and what a host needs from a plan to draw it: chips that keep
//! apart, boxes off every layer's anchor and arrow, docked boxes against their chips, arrows
//! that route (or say why not), chips sized by the host, attribution and where arrows attach.

mod common;

use caretline_layers::*;
use common::*;
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

fn host(m: &mut AnchorMap, key: &str, r: Rect) -> Anchor {
    m.put(AnchorKey::host("row", key), r);
    Anchor::Host {
        kind: "row".into(),
        key: key.into(),
    }
}

fn off(m: &mut AnchorMap, key: &str, o: Off) -> Anchor {
    m.put_off(AnchorKey::host("row", key), o);
    Anchor::Host {
        kind: "row".into(),
        key: key.into(),
    }
}

fn all(layers: Vec<Layer>) -> Layers {
    let mut l = Layers::default();
    for layer in layers {
        apply(&mut l, LayerOp::Push(layer), None, 0, &Limits::default()).unwrap();
    }
    l
}

/// Nothing of one layer sits on another's: boxes, strips and chips apart, none over another
/// layer's anchor, no arrow cell under a box or chip.
fn apart(p: &Plan) {
    for (i, a) in p.layers.iter().enumerate() {
        let mine: Vec<Rect> = a.rect.iter().chain(&a.chip).copied().collect();
        for (j, b) in p.layers.iter().enumerate() {
            if i == j {
                continue;
            }
            let theirs: Vec<Rect> = b.rect.iter().chain(&b.chip).copied().collect();
            for r in &mine {
                for o in &theirs {
                    assert!(!r.intersects(o), "{} {r:?} meets {} {o:?}", a.id, b.id);
                }
                for o in b.anchor.iter().flat_map(|x| &x.rects) {
                    assert!(
                        !r.intersects(o),
                        "{} {r:?} covers {}'s anchor {o:?}",
                        a.id,
                        b.id
                    );
                }
                for s in b.route.iter().flat_map(|x| &x.steps) {
                    assert!(
                        !r.contains(s.x, s.y),
                        "{}'s arrow runs under {} at {s:?}",
                        b.id,
                        a.id
                    );
                }
            }
        }
    }
}

#[test]
fn chips_on_the_same_edge_keep_apart() {
    let grid = Grid::new(80, 24).with_area(Rect::new(0, 0, 80, 23));
    let mut m = AnchorMap::new();
    let a = off(&mut m, "a", Off::Below { x: None });
    let b = off(&mut m, "b", Off::Below { x: None });
    let c = off(&mut m, "c", Off::Above { x: Some(30) });
    let d = off(&mut m, "d", Off::Above { x: Some(30) });
    let mut layers = Vec::new();
    for anchor in [a, b, c, d] {
        let mut l = Layer::new(anchor).with_content(card());
        l.hide_off_screen = true;
        layers.push(l);
    }
    let p = plan(&all(layers), &m, &grid, &sized(20, 3));
    let chips: Vec<Rect> = p.layers.iter().map(|l| l.chip.unwrap()).collect();
    assert_eq!(chips[0], Rect::new(72, 22, 8, 1));
    assert_eq!(chips[1].y, 22, "the same edge");
    assert_eq!(chips[2], Rect::new(30, 0, 8, 1));
    assert_eq!(chips[3].y, 0);
    apart(&p);
}

#[test]
fn a_box_never_covers_another_layers_anchor_or_arrow() {
    let grid = Grid::new(80, 24);
    let mut m = AnchorMap::new();
    // The first layer's box would go just below its anchor, on the second's.
    let a = host(&mut m, "a", Rect::new(10, 5, 6, 1));
    let b = host(&mut m, "b", Rect::new(12, 7, 4, 1));
    let c = host(&mut m, "c", Rect::new(40, 3, 4, 1));
    let p = plan(
        &all(vec![
            Layer::new(a).with_content(card()).with_arrow(),
            Layer::new(b).with_content(card()).with_arrow(),
            Layer::new(c).with_content(card()).with_arrow(),
        ]),
        &m,
        &grid,
        &sized(20, 3),
    );
    assert!(p.layers.iter().all(|l| l.rect.is_some()));
    apart(&p);
}

#[test]
fn layers_over_a_real_page_stay_apart() {
    for (w, h) in [(80, 24), (100, 40)] {
        let s = state(w, h);
        let mut layers = Layers::default();
        for (needle, arrow) in [
            ("word at a time", true),
            ("stops at the start", true),
            ("Hold Shift", false),
            ("Every change", true),
            ("Control S", true),
        ] {
            let mut layer =
                Layer::new(find(needle, 0)).with_content(hint("Tip", "A word or two about this."));
            layer.arrow = arrow;
            push(&mut layers, layer, None);
        }
        let (_, p) = plan_over(&s, &layers);
        apart(&p);
    }
}

#[test]
fn a_docked_box_sits_against_its_chip_and_says_where() {
    let grid = Grid::new(80, 24).with_area(Rect::new(0, 0, 80, 23));
    let mut m = AnchorMap::new();
    let a = off(&mut m, "far", Off::Below { x: Some(12) });
    let p = plan(
        &all(vec![Layer::new(a).with_content(card()).with_arrow()]),
        &m,
        &grid,
        &sized(20, 3),
    );
    let l = &p.layers[0];
    let (r, chip) = (l.rect.unwrap(), l.chip.unwrap());
    assert_eq!(chip, Rect::new(12, 22, 8, 1));
    assert_eq!(r.bottom(), chip.y, "flush against the chip");
    assert!(r.x <= chip.x && chip.right() <= r.right(), "over the chip");
    assert_eq!(
        l.dock,
        Some(Attach {
            edge: Edge::Bottom,
            offset: chip.x + chip.w / 2 - r.x
        })
    );
    assert!(l.route.is_none());
    assert_eq!(l.no_arrow, Some(NoArrow::Docked));
}

#[test]
fn every_arrow_at_an_anchor_on_screen_routes() {
    // Every char of the page as an anchor: a box placed beside it always gets its arrow, even
    // at the area's edge where the facing side can't be reached from any box.
    for (w, h) in [(80u16, 24u16), (100, 40)] {
        let s = state(w, h);
        let n = DOC.chars().count();
        let mut boxes = 0;
        for from in (0..n).step_by(3) {
            let mut l = Layers::default();
            push(
                &mut l,
                Layer::new(Anchor::Text { from, to: from + 1 })
                    .with_content(hint("t", "Some words here for a box."))
                    .with_arrow(),
                None,
            );
            let (_, p) = plan_over(&s, &l);
            let Some(pl) = p.layers.first() else { continue };
            let on_screen = pl.anchor.as_ref().is_some_and(|a| !a.rects.is_empty());
            if pl.mode == Some(Mode::Box) && on_screen {
                boxes += 1;
                let route = pl
                    .route
                    .as_ref()
                    .unwrap_or_else(|| panic!("{w}x{h} char {from}: {:?}", pl.no_arrow));
                let head = route.steps.last().unwrap();
                let a = &pl.anchor.as_ref().unwrap().rects;
                let (hx, hy) = match head.leave {
                    Dir::Up => (head.x, head.y.wrapping_sub(1)),
                    Dir::Down => (head.x, head.y + 1),
                    Dir::Left => (head.x.wrapping_sub(1), head.y),
                    Dir::Right => (head.x + 1, head.y),
                };
                assert!(
                    a.iter().any(|r| r.contains(hx, hy)),
                    "{w}x{h} char {from}: the head points at the anchor"
                );
            } else {
                assert!(pl.route.is_some() || pl.no_arrow.is_some(), "a reason");
            }
        }
        assert!(boxes > 100, "{boxes}");
    }
}

#[test]
fn the_anchor_at_the_left_edge_gets_its_arrow() {
    let s = state(80, 24);
    let mut l = Layers::default();
    push(
        &mut l,
        Layer::new(find("Hold Option", 0))
            .with_content(hint("t", "Some words here for a box."))
            .with_arrow(),
        None,
    );
    let (_, p) = plan_over(&s, &l);
    let pl = &p.layers[0];
    assert_eq!(pl.anchor.as_ref().unwrap().rects[0].x, 0);
    assert!(pl.route.is_some(), "{:?}", pl.no_arrow);
    assert_eq!(pl.no_arrow, None);
}

#[test]
fn reasons_for_no_arrow() {
    let grid = Grid::new(80, 24);
    let mut m = AnchorMap::new();
    let a = host(&mut m, "a", Rect::new(10, 5, 6, 1));
    // No box: a kind nobody renders.
    let mut l = Layer::new(a.clone()).with_arrow();
    l.content = Some(Content::new("nobody", json!({})));
    let p = plan(&all(vec![l]), &m, &grid, &sized(20, 3));
    assert_eq!(p.layers[0].no_arrow, Some(NoArrow::NoBox));
    // A screen position.
    let l = Layer::new(Anchor::Screen(ScreenPos::Center))
        .with_content(card())
        .with_arrow();
    let p = plan(&all(vec![l]), &m, &grid, &sized(20, 3));
    assert_eq!(p.layers[0].no_arrow, Some(NoArrow::Screen));
    // No arrow asked for: no reason.
    let p = plan(
        &all(vec![Layer::new(a).with_content(card())]),
        &m,
        &grid,
        &sized(20, 3),
    );
    assert_eq!(p.layers[0].no_arrow, None);
}

/// A host that labels its chips with the anchor's key and an arrow for the way.
struct Labelled;

impl Renderer for Labelled {
    fn measure(&self, _: &Value, avail: Size) -> Size {
        Size::new(20.min(avail.w), 3)
    }
    fn chip(&self, _: &Value, anchor: &Anchor, off: Off) -> Size {
        let key = match anchor {
            Anchor::Host { key, .. } => key.chars().count() as u16,
            _ => 0,
        };
        let arrow = match off {
            Off::Above { .. } | Off::Below { .. } => 2,
            Off::Left { .. } | Off::Right { .. } => 4,
        };
        Size::new(key + arrow, 1)
    }
}

#[test]
fn the_host_sizes_a_chip_from_its_anchor_and_direction() {
    let grid = Grid::new(80, 24);
    let mut m = AnchorMap::new();
    let a = off(&mut m, "chapter-twelve", Off::Below { x: Some(3) });
    let b = off(&mut m, "x", Off::Right { y: 4 });
    let r = Renderers::new().register("card", Labelled);
    let p = plan(
        &all(vec![
            Layer::new(a).with_content(card()),
            Layer::new(b).with_content(card()),
        ]),
        &m,
        &grid,
        &r,
    );
    assert_eq!(p.layers[0].chip.unwrap().w, 16);
    assert_eq!(p.layers[1].chip.unwrap().w, 5);
}

#[test]
fn an_agents_layer_carries_its_owner_and_its_own_title() {
    let s = state(80, 24);
    let mut l = Layers::default();
    push(
        &mut l,
        Layer::new(find("Every change can be undone.", 0))
            .with_content(hint("tip", "Each undo step is one word."))
            .with_arrow(),
        Some("helper"),
    );
    let (_, p) = plan_over(&s, &l);
    let pl = &p.layers[0];
    assert!(pl.agent);
    assert_eq!(pl.owner, Owner::Agent("helper".into()));
    assert_eq!(pl.owner.actor(), Some("helper"));
    let json = serde_json::to_value(pl).unwrap();
    assert_eq!(json["owner"], "agent:helper");
    // The content is the agent's, unchanged: the host draws the attribution.
    let h = l.layers[0].content.as_ref().unwrap().as_hint().unwrap();
    assert_eq!(h.title.as_deref(), Some("tip"));
}

#[test]
fn an_arrow_says_where_it_attaches_and_never_beside_the_title_row() {
    let grid = Grid::new(80, 24);
    for (side, edge) in [
        (Side::Right, Edge::Left),
        (Side::Left, Edge::Right),
        (Side::Below, Edge::Top),
        (Side::Above, Edge::Bottom),
    ] {
        for y in [3u16, 8, 12, 16, 20] {
            let mut m = AnchorMap::new();
            let a = host(&mut m, "a", Rect::new(36, y, 6, 1));
            let mut l = Layer::new(a).with_content(card()).with_arrow();
            l.place = vec![side];
            let p = plan(&all(vec![l]), &m, &grid, &sized(20, 6));
            let pl = &p.layers[0];
            let (Some(r), Some(route)) = (pl.rect, pl.route.as_ref()) else {
                continue;
            };
            assert_eq!(pl.side, Some(side));
            let at = route.attach;
            assert_eq!(at.edge, edge);
            let (jx, jy) = route.junction;
            match edge {
                Edge::Top => assert_eq!((jx - r.x, jy), (at.offset, r.y)),
                Edge::Bottom => assert_eq!((jx - r.x, jy), (at.offset, r.bottom() - 1)),
                Edge::Left | Edge::Right => {
                    assert_eq!(jy - r.y, at.offset);
                    assert!(at.offset >= 2, "beside the title row: {at:?}");
                    assert!(at.offset < r.h - 1, "not on a corner");
                }
            }
        }
    }
}

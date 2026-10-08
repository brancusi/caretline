#![cfg(feature = "caretline")]
//! Arrow heads at table rows: a head never points at a neighbouring row when the host
//! requests OnAnchorRows, and a blocked row gives a box with an explicit reason.

mod common;

use caretline::{Frame, Role};
use caretline_layers::*;
use common::*;

fn table() -> (Frame, Grid, AnchorMap, Anchor) {
    let mut frame = Frame::new(80, 24);
    frame.rows = vec![caretline::view::RowInfo::Past; 24];
    frame.rows[23] = caretline::view::RowInfo::Status;
    let mut grid = Grid::new(80, 24).with_area(Rect::new(0, 0, 80, 23));
    let mut anchors = AnchorMap::new();
    for (y, name, amount) in [
        (6, "Alpha", "12.00"),
        (8, "Beta", "24.00"),
        (10, "Gamma", "36.00"),
    ] {
        for (x, text) in [(20, name), (48, amount)] {
            grid.mark_text(x, y, text);
            for (i, ch) in text.chars().enumerate() {
                frame.set(x + i as u16, y, &ch.to_string(), Role::Text);
            }
        }
    }
    anchors.put(AnchorKey::host("row", "beta"), Rect::new(20, 8, 4, 1));
    let anchor = Anchor::Host {
        kind: "row".into(),
        key: "beta".into(),
    };
    (frame, grid, anchors, anchor)
}

#[test]
fn a_table_arrow_ends_beside_the_row_instead_of_above_or_below_it() {
    let (frame, grid, anchors, anchor) = table();
    let r = renderers();
    for (name, head) in [
        ("any", HeadRule::Any),
        ("on-anchor-rows", HeadRule::OnAnchorRows),
    ] {
        let mut layers = Layers::default();
        let mut layer = Layer::new(anchor.clone())
            .with_content(hint("Beta", "This note belongs to Beta, not the next row."))
            .with_arrow()
            .with_ring()
            .with_head(head);
        layer.place = vec![Side::Below];
        push(&mut layers, layer, None);
        let p = plan(&layers, &anchors, &grid, &r);
        conform(&layers, &anchors, &grid, &r, &p);
        let arrow = p.layers[0]
            .route
            .as_ref()
            .expect("a clear route beside the row");
        let tip = arrow.steps.last().unwrap();
        if head == HeadRule::OnAnchorRows {
            assert_eq!(tip.y, 8, "{}", p.explain());
            assert!(matches!(tip.leave, Dir::Left | Dir::Right));
        } else {
            assert_ne!(tip.y, 8, "the fixture reproduces a head on an adjacent row");
        }
        golden(
            &format!("table-head.{name}.80x24.txt"),
            &picture(&frame, &p, &layers),
        );
        golden(
            &format!("table-head.{name}.80x24.json"),
            &(serde_json::to_string_pretty(&p).unwrap() + "\n"),
        );
    }
}

#[test]
fn when_only_an_adjacent_row_is_clear_the_box_explains_the_missing_arrow() {
    let (_, mut grid, anchors, anchor) = table();
    grid.protect
        .extend([Rect::new(0, 8, 20, 1), Rect::new(24, 8, 56, 1)]);
    let r = renderers();
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(anchor)
            .with_content(hint("Beta", "A blocked row still gets its note."))
            .with_arrow()
            .with_head(HeadRule::OnAnchorRows),
        None,
    );
    let p = plan(&layers, &anchors, &grid, &r);
    conform(&layers, &anchors, &grid, &r, &p);
    assert_eq!(p.layers[0].mode, Some(Mode::Box));
    assert!(p.layers[0].route.is_none());
    assert_eq!(
        p.layers[0].no_arrow,
        Some(NoArrow::HeadOffAnchorRows),
        "{}",
        p.explain()
    );
}

#[cfg(feature = "conformance")]
#[test]
fn conformance_catches_a_head_on_the_wrong_row() {
    use caretline_layers::conformance::{Kind, Scene, check_plan};
    let (_, grid, anchors, anchor) = table();
    let r = renderers();
    let mut layers = Layers::default();
    let mut layer = Layer::new(anchor)
        .with_content(hint("Beta", "This note belongs to Beta, not the next row."))
        .with_arrow();
    layer.place = vec![Side::Below];
    push(&mut layers, layer, None);
    let p = plan(&layers, &anchors, &grid, &r);
    layers.layers[0].head = HeadRule::OnAnchorRows;
    let scene = Scene::new(layers, anchors, grid, &r);
    assert!(
        check_plan(&p, &scene)
            .iter()
            .any(|v| v.kind == Kind::HeadOffAnchorRows)
    );
}

//! The protocol-neutral ops: request shapes in, layer ops out, and the replies.

use caretline_layers::ops::{self, Request};
use caretline_layers::*;
use serde_json::{Value, json};

fn hint_size(_: &Value, avail: Size) -> Size {
    Size::new(30.min(avail.w), 4)
}

/// A host screen: a table of rows by stable id, recorded while drawing.
fn table() -> (AnchorMap, Grid) {
    let mut m = AnchorMap::new();
    let mut g = Grid::new(80, 24).with_area(Rect::new(0, 0, 80, 23));
    for (i, id) in ["abc", "def", "ghi"].iter().enumerate() {
        let y = 2 + i as u16 * 2;
        g.mark_text(2, y, "a row of the table");
        m.put(AnchorKey::host("row", id), Rect::new(2, y, 18, 1));
    }
    m.put_off(AnchorKey::host("row", "zzz"), Off::Below { x: None });
    m.put(
        AnchorKey::host("diff", "src/main.rs:42"),
        Rect::new(2, 12, 30, 1),
    );
    (m, g)
}

fn run(layers: &mut Layers, op: &str, req: Value, now: u64) -> Value {
    let (m, g) = table();
    let renderers = Renderers::new().register(HINT, hint_size);
    match ops::parse(op, &req) {
        Err(r) => ops::error(&r),
        Ok((Request::List, _)) => ops::list(layers),
        Ok((Request::Apply(o), actor)) => {
            match apply(layers, o, actor.as_deref(), now, &Limits::default()) {
                Err(r) => ops::error(&r),
                Ok(a) => ops::reply(&a, Some(&plan(layers, &m, &g, &renderers))),
            }
        }
    }
}

#[test]
fn hint_show_on_a_host_row() {
    let mut l = Layers::default();
    let r = run(
        &mut l,
        "hint.show",
        json!({"id": 4, "op": "hint.show", "actor": "claude", "anchor": {"host": {"kind": "row", "key": "def"}},
               "title": "Stale", "text": "This row hasn't synced.", "ttl_ms": 8000, "place": ["right", "below"]}),
        1_000,
    );
    assert_eq!(
        r,
        json!({"layer": "L-1", "resolved": {"rects": [{"x": 2, "y": 4, "w": 18, "h": 1}]}})
    );
    let layer = l.get("L-1").unwrap();
    assert_eq!(layer.owner, Owner::Agent("claude".into()));
    assert!(layer.arrow && layer.ring.is_some());
    assert_eq!(layer.place, vec![Side::Right, Side::Below]);
    assert_eq!(
        layer
            .content
            .as_ref()
            .unwrap()
            .as_hint()
            .unwrap()
            .title
            .as_deref(),
        Some("◆ claude · Stale")
    );
}

#[test]
fn hint_show_off_screen_missing_and_on_a_diff_line() {
    let mut l = Layers::default();
    let r = run(
        &mut l,
        "hint.show",
        json!({"actor": "a", "anchor": {"host": {"kind": "row", "key": "zzz"}}, "text": "Below."}),
        0,
    );
    assert_eq!(r, json!({"layer": "L-1", "resolved": {"off": "below"}}));
    let r = run(
        &mut l,
        "hint.show",
        json!({"actor": "b", "anchor": {"host": {"kind": "row", "key": "nope"}}, "text": "Gone."}),
        0,
    );
    assert_eq!(
        r,
        json!({"layer": "L-2", "resolved": null, "reason": "not_found"})
    );
    let r = run(
        &mut l,
        "hint.show",
        json!({"actor": "c", "anchor": [{"host": {"kind": "diff", "key": "src/main.rs:42"}}, {"caret": true}], "text": "This line."}),
        0,
    );
    assert_eq!(
        r["resolved"],
        json!({"rects": [{"x": 2, "y": 12, "w": 30, "h": 1}]})
    );
}

#[test]
fn hide_pop_push_update_and_list() {
    let mut l = Layers::default();
    run(
        &mut l,
        "hint.show",
        json!({"actor": "a", "anchor": {"host": {"kind": "row", "key": "abc"}}, "text": "One."}),
        0,
    );
    let r = run(
        &mut l,
        "layer.push",
        json!({"layer": {"anchor": [{"host": {"kind": "row", "key": "ghi"}}], "content": {"kind": "hint", "data": {"text": "Host's."}}}}),
        0,
    );
    assert_eq!(r["layer"], json!("L-2"));
    let r = run(
        &mut l,
        "layer.update",
        json!({"layer": {"id": "L-2", "anchor": [{"host": {"kind": "row", "key": "def"}}], "content": {"kind": "hint", "data": {"text": "Moved."}}}}),
        0,
    );
    assert_eq!(r["resolved"]["rects"][0]["y"], json!(4));
    let r = run(&mut l, "layer.list", json!({}), 0);
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    // An agent hides only its own.
    let r = run(
        &mut l,
        "hint.hide",
        json!({"actor": "a", "layer": "L-2"}),
        0,
    );
    assert_eq!(r["error"]["reason"], json!("not_allowed"));
    let r = run(&mut l, "hint.hide", json!({"actor": "a", "all": true}), 0);
    assert_eq!(r, json!({"popped": ["L-1"]}));
    let r = run(&mut l, "layer.pop", json!({"owner": "host"}), 0);
    assert_eq!(r, json!({"popped": ["L-2"]}));
}

#[test]
fn malformed_requests_are_invalid_and_policy_refusals_pass_through() {
    let mut l = Layers::default();
    for (op, req) in [
        ("hint.show", json!({"anchor": {"caret": true}})),
        (
            "hint.show",
            json!({"anchor": {"cells": {"x": 1, "y": 1, "w": 1, "h": 1}}, "text": "x"}),
        ),
        (
            "hint.show",
            json!({"anchor": {"caret": true}, "text": "x", "colour": "red"}),
        ),
        ("hint.hide", json!({})),
        ("hint.hide", json!({"owner": "host"})),
        ("layer.pop", json!({"layer": "L-1", "all": true})),
        ("layer.frobnicate", json!({})),
    ] {
        let r = run(&mut l, op, req.clone(), 0);
        assert_eq!(r["error"]["reason"], json!("invalid"), "{op} {req}");
    }
    let spot = json!({"actor": "a", "layer": {"anchor": [{"caret": true}], "spotlight": {}}});
    assert_eq!(
        run(&mut l, "layer.push", spot, 0)["error"]["reason"],
        json!("dim_not_allowed")
    );
    let long = json!({"actor": "a", "anchor": {"caret": true}, "text": "x".repeat(300)});
    assert_eq!(
        run(&mut l, "hint.show", long, 0)["error"]["reason"],
        json!("too_long")
    );
}

//! `plan_steps`: every step planned at several sizes with the host's resolver and renderers,
//! and what didn't work reported per step and size.

use caretline_layers::{AnchorKey, AnchorMap, Grid, Off, Rect, Renderers, Size};
use caretline_tour::*;
use serde_json::Value;

fn tour() -> Tour {
    parse_toml(
        r#"
id = "t"
[[step]]
id = "intro"
narration = { text = "Hello." }

[[step]]
id = "row"
anchor = { host = { kind = "row", key = "r1" } }
data = { text = "This row." }
place = { arrow = true }

[[step]]
id = "far"
anchor = { host = { kind = "row", key = "r9" } }
data = { text = "A row further down." }

[[step]]
id = "two"
[[step.layers]]
anchor = { host = { kind = "row", key = "gone" } }
data = { text = "Missing." }
[[step.layers]]
id = "odd"
anchor = { screen = "center" }
kind = "host.unknown"
data = { x = 1 }
[[step.layers]]
id = "found"
anchor = { find = "never", in = "main" }
data = { text = "x" }
"#,
    )
    .unwrap()
}

fn renderers() -> Renderers {
    Renderers::new().register(caretline_layers::HINT, |_: &Value, avail: Size| {
        Size::new(avail.w.min(24), avail.h.min(3))
    })
}

#[test]
fn every_step_at_every_size() {
    let t = tour();
    let mut scenes = Vec::new();
    let plans = plan_steps(
        &t,
        &[(80, 24), (44, 16)],
        &renderers(),
        &mut |step, w, h, go| {
            scenes.push((step.id.clone(), w, h));
            // The host draws its table: row r1 on row 2; r9 scrolled out below.
            let mut anchors = AnchorMap::new();
            anchors.put(AnchorKey::host("row", "r1"), Rect::new(2, 2, 20, 1));
            anchors.put_off(AnchorKey::host("row", "r9"), Off::Below { x: Some(4) });
            let mut grid = Grid::new(w, h);
            grid.mark_text(2, 2, "r1 the first row");
            go(&grid, &anchors)
        },
    );
    assert_eq!(plans.len(), 8);
    assert_eq!(scenes.len(), 8);
    assert_eq!(scenes[0], ("intro".into(), 80, 24));
    assert_eq!(scenes[1], ("intro".into(), 44, 16));

    let at = |id: &str, w: u16| plans.iter().find(|p| p.step == id && p.width == w).unwrap();
    assert!(at("intro", 80).plan.layers.is_empty());
    assert!(at("intro", 80).problems.is_empty());

    let row = at("row", 80);
    assert!(row.problems.is_empty(), "{:?}", row.problems);
    let l = &row.plan.layers[0];
    assert_eq!(l.id, "row/0");
    assert!(l.rect.is_some());
    assert!(l.route.is_some(), "the arrow routes");

    let far = at("far", 44);
    assert!(
        far.problems
            .iter()
            .any(|p| p.code == "off_screen" && p.level == Level::Warning)
    );

    let two = at("two", 80);
    let codes: Vec<&str> = two.problems.iter().map(|p| p.code.as_str()).collect();
    assert!(codes.contains(&"not_found"), "{codes:?}");
    assert!(codes.contains(&"unrendered"), "{codes:?}");
    assert!(codes.contains(&"unresolved_find"), "{codes:?}");
    let missing = two.problems.iter().find(|p| p.code == "not_found").unwrap();
    assert_eq!(missing.layer.as_deref(), Some("two/0"));
    assert_eq!(missing.step.as_deref(), Some("two"));
}

#[test]
fn a_scene_that_draws_nothing_plans_nothing() {
    let t = tour();
    let plans = plan_steps(&t, &[(80, 24)], &renderers(), &mut |_, _, _, _| {});
    assert!(plans.is_empty());
}

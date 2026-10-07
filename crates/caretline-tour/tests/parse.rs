//! The format: TOML and JSON, the single-layer shorthand and `layers[]`, `narration`, the
//! opaque `host`, predicates, and strictness. Goldens are the normalised tour as JSON
//! (`TOUR_GOLDENS=update` rewrites them).

use caretline_layers::{Anchor, ScreenPos, Side};
use caretline_tour::*;
use serde_json::{Value, json};

fn golden(name: &str, tour: &Tour) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(format!("{name}.json"));
    let got = serde_json::to_string_pretty(tour).unwrap() + "\n";
    if std::env::var("TOUR_GOLDENS").as_deref() == Ok("update") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("no golden {name} (TOUR_GOLDENS=update writes it)"));
    assert_eq!(got, want, "golden {name}");
}

fn example() -> Tour {
    let src = include_str!("../examples/tour.toml");
    parse_toml(src).unwrap()
}

#[test]
fn the_example_parses_to_its_golden_and_checks_clean_of_errors() {
    let t = example();
    golden("example", &t);
    assert_eq!(t.steps.len(), 4);
    let errors: Vec<_> = check(&t)
        .into_iter()
        .filter(|p| p.level == Level::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_shorthand_is_layers_0_and_ids_default_to_step_slash_index() {
    let t = example();
    let rows = t.step("rows").unwrap();
    assert_eq!(rows.layers.len(), 1);
    let l = &rows.layers[0];
    assert_eq!(l.id, "rows/0");
    assert_eq!(
        l.anchor,
        vec![StepAnchor::At(Anchor::Host {
            kind: "row".into(),
            key: "first".into()
        })]
    );
    assert!(l.place.arrow);
    assert!(l.place.ring.is_some());
    assert_eq!(l.place.sides, vec![Side::Right, Side::Below]);
    let edit = t.step("edit").unwrap();
    assert_eq!(edit.layers[0].id, "edit/0");
    assert_eq!(edit.layers[1].id, "outline-entry");
    assert_eq!(
        edit.layers[1].anchor,
        vec![
            StepAnchor::Find {
                text: "Summary".into(),
                view: Some("outline".into())
            },
            StepAnchor::At(Anchor::Screen(ScreenPos::Top)),
        ]
    );
    let fin = t.step("finish").unwrap();
    assert!(fin.layers[0].capture);
}

#[test]
fn narration_and_host_are_carried_untouched() {
    let t = example();
    let w = t.step("welcome").unwrap();
    assert!(w.layers.is_empty());
    assert_eq!(
        w.narration,
        Some(Narration {
            title: Some("Welcome".into()),
            text: "On the left, a table of rows; on the right, the row you open, as text.".into()
        })
    );
    assert_eq!(
        w.host,
        Some(json!({"panels": {"left": "table", "right": "editor"}, "focus": "table"}))
    );
}

#[test]
fn json_reads_the_same_tour_and_round_trips() {
    let t = example();
    let j = serde_json::to_string(&t).unwrap();
    assert_eq!(parse_json(&j).unwrap(), t);
    // The canonical form is idempotent.
    let again = serde_json::to_value(parse_json(&j).unwrap()).unwrap();
    assert_eq!(again, serde_json::to_value(&t).unwrap());
}

/// A host's file as authored in JSON: shorthand, `layers[]`, narration, host, a jump branch.
const HOST_JSON: &str = r#"{
  "id": "host.intro", "version": 2, "title": "Intro", "kind": "hint",
  "step": [
    {"id": "one", "anchor": {"host": {"kind": "panel", "key": "left"}},
     "data": {"text": "The list."}, "place": {"connector": true, "max_w": 40},
     "narration": {"text": "Start here."}, "host": {"open": ["left"]}},
    {"id": "two", "narration": {"title": "Two", "text": "Both panels."},
     "host": {"open": ["left", "right"]},
     "layers": [
       {"anchor": [{"text": {"from": 4, "to": 9}, "in": "main"}, {"screen": "center"}], "data": {"text": "a"}},
       {"id": "r", "anchor": {"block": 7, "in": "side"}, "kind": "host.callout", "data": {"n": 1},
        "place": ["left"], "capture": true}
     ],
     "next": [{"if": {"ext": {"key": "mode", "match": {"wide": true}}}, "goto": "one"}, {"goto": "end"}]}
  ]
}"#;

#[test]
fn a_host_json_file_parses_to_its_golden() {
    let t = parse_json(HOST_JSON).unwrap();
    golden("host", &t);
    assert_eq!(t.version, 2);
    let one = &t.steps[0];
    assert_eq!(one.layers[0].id, "one/0");
    assert!(one.layers[0].place.arrow, "connector reads as arrow");
    assert_eq!(one.layers[0].place.max_width, Some(40));
    let two = &t.steps[1];
    assert_eq!(two.layers[0].id, "two/0");
    assert_eq!(
        two.layers[0].anchor[0],
        StepAnchor::At(Anchor::scoped("main", Anchor::Text { from: 4, to: 9 }))
    );
    assert_eq!(two.layers[1].place.sides, vec![Side::Left]);
    assert_eq!(
        two.next[0].when,
        Some(Pred::State {
            key: "mode".into(),
            test: StateTest::Match(json!({"wide": true}))
        })
    );
    assert!(check(&t).iter().all(|p| p.level != Level::Error));
}

#[test]
fn predicates_read_every_form() {
    let src = r#"
id = "p"
[[step]]
id = "a"
narration = { text = "x" }
advance = { all = [
  { msg = "insert_text", count = 5 },
  { command = "move.word_right" },
  { effect = "clipboard_set" },
  { host_effect = "saved", count = 2 },
  { state = { key = "k" } },
  { ext = { key = "k", present = false } },
  { selection = "nonempty" },
  { after_ms = 1500 },
  { not = { any = [{ event = "e" }] } },
] }
"#;
    let t = parse_toml(src).unwrap();
    let Some(Pred::All(v)) = &t.steps[0].advance else {
        panic!()
    };
    assert_eq!(
        v[0],
        Pred::Msg {
            kind: "insert_text".into(),
            count: 5
        }
    );
    assert_eq!(
        v[2],
        Pred::Event {
            name: "clipboard_set".into(),
            count: 1
        }
    );
    assert_eq!(
        v[4],
        Pred::State {
            key: "k".into(),
            test: StateTest::Present(true)
        }
    );
    assert_eq!(v[6], Pred::Selection);
    assert_eq!(v[7], Pred::AfterMs(1500));
    // Round trip through the canonical form.
    let j = serde_json::to_value(&t).unwrap();
    assert_eq!(
        j["step"][0]["advance"]["all"][2],
        json!({"event": "clipboard_set"})
    );
    assert_eq!(parse_json(&j.to_string()).unwrap(), t);
}

fn err(src: &str) -> String {
    parse_toml(src).expect_err("should be refused")
}

#[test]
fn unknown_fields_are_refused_everywhere_but_data_and_host() {
    let base = "id = \"t\"\n";
    for (src, why) in [
        (format!("{base}colour = \"red\"\n"), "tour"),
        (
            format!("{base}[[step]]\nid = \"a\"\nnarration = {{ text = \"x\" }}\nspeed = 2\n"),
            "step",
        ),
        (
            format!(
                "{base}[[step]]\nid = \"a\"\nnarration = {{ text = \"x\", tone = \"warm\" }}\n"
            ),
            "narration",
        ),
        (
            format!(
                "{base}[[step]]\nid = \"a\"\n[[step.layers]]\nanchor = {{ caret = true }}\nshade = 1\n"
            ),
            "layer",
        ),
        (
            format!(
                "{base}[[step]]\nid = \"a\"\nanchor = {{ caret = true }}\nplace = {{ glow = true }}\n"
            ),
            "place",
        ),
        (
            format!("{base}[[step]]\nid = \"a\"\nanchor = {{ caret = true, near = 1 }}\n"),
            "anchor",
        ),
        (
            format!("{base}[[step]]\nid = \"a\"\nanchor = {{ find = \"x\", near = 1 }}\n"),
            "find",
        ),
        (
            format!(
                "{base}[[step]]\nid = \"a\"\nnarration = {{ text = \"x\" }}\nadvance = {{ blink = 1 }}\n"
            ),
            "pred",
        ),
        (
            format!(
                "{base}[[step]]\nid = \"a\"\nnarration = {{ text = \"x\" }}\nnext = [{{ goto = \"end\", when = 1 }}]\n"
            ),
            "branch",
        ),
        (
            format!(
                "{base}[[step]]\nid = \"a\"\nnarration = {{ text = \"x\" }}\nnudge = {{ after_ms = 1, data = {{}}, again = true }}\n"
            ),
            "nudge",
        ),
    ] {
        let e = err(&src);
        assert!(!e.is_empty(), "{why}");
    }
    // `data` and `host` are opaque: anything goes.
    let ok = format!(
        "{base}[[step]]\nid = \"a\"\nanchor = {{ caret = true }}\nkind = \"x.y\"\ndata = {{ any = {{ thing = [1, 2] }} }}\nhost = {{ whatever = true }}\n"
    );
    parse_toml(&ok).unwrap();
}

#[test]
fn layers_and_the_shorthand_together_are_an_error() {
    let e = err(r#"
id = "t"
[[step]]
id = "a"
anchor = { caret = true }
data = { text = "x" }
[[step.layers]]
anchor = { caret = true }
data = { text = "y" }
"#);
    assert!(e.contains("not both"), "{e}");
    let e = err("id = \"t\"\n[[step]]\nid = \"a\"\ndata = { text = \"x\" }\n");
    assert!(e.contains("need an `anchor`"), "{e}");
}

#[test]
fn malformed_predicates_are_refused() {
    let step = |p: &str| {
        format!("id = \"t\"\n[[step]]\nid = \"a\"\nnarration = {{ text = \"x\" }}\nadvance = {p}\n")
    };
    for p in [
        "{ msg = \"a\", command = \"b\" }",
        "{ count = 2 }",
        "{ after_ms = 5, count = 2 }",
        "{ selection = \"empty\" }",
        "{ state = { key = \"k\", present = true, match = 1 } }",
        "{ effect = \"a\", event = \"b\" }",
        "{}",
    ] {
        assert!(parse_toml(&step(p)).is_err(), "{p} should be refused");
    }
}

#[test]
fn place_takes_a_list_of_sides_or_flags() {
    let t = parse_json(
        r#"{"id": "t", "step": [
          {"id": "a", "anchor": {"caret": true}, "data": {"text": "x"}, "place": ["above"]},
          {"id": "b", "anchor": {"caret": true}, "data": {"text": "x"},
           "place": {"spotlight": {"holes": ["anchor"]}, "ring": false, "hide_off_screen": true}}
        ]}"#,
    )
    .unwrap();
    assert_eq!(t.steps[0].layers[0].place.sides, vec![Side::Above]);
    let p = &t.steps[1].layers[0].place;
    assert!(p.ring.is_none());
    assert_eq!(
        p.spotlight.as_ref().unwrap().holes,
        vec![caretline_layers::Part::Anchor]
    );
    assert!(p.hide_off_screen);
    assert!(
        parse_json(
            r#"{"id": "t", "step": [{"id": "a", "anchor": {"caret": true}, "place": {"ring": 3}}]}"#
        )
        .is_err()
    );
}

#[test]
fn the_lint_finds_what_parses_but_cant_work() {
    let t = parse_toml(
        r#"
id = "t"
[[step]]
id = "a"
anchor = { find = "x" }
data = { text = "x" }
advance = { caret_in = "nope" }
next = [{ goto = "b" }, { goto = "zzz" }]
[[step]]
id = "a"
[[step]]
id = "next"
anchor = { caret = true }
data = { nottext = 1 }
"#,
    )
    .unwrap();
    let codes: Vec<(Level, String)> = check(&t).into_iter().map(|p| (p.level, p.code)).collect();
    for want in [
        (Level::Warning, "unscoped_find"),
        (Level::Error, "unknown_anchor"),
        (Level::Error, "unknown_goto"),
        (Level::Warning, "unreachable_branch"),
        (Level::Error, "duplicate_step"),
        (Level::Warning, "empty_step"),
        (Level::Error, "reserved_id"),
        (Level::Error, "hint_data"),
    ] {
        assert!(
            codes.contains(&(want.0, want.1.to_string())),
            "{want:?} in {codes:?}"
        );
    }
    let empty = parse_toml("id = \"t\"\n").unwrap();
    assert!(check(&empty).iter().any(|p| p.code == "no_steps"));
    let _: Value = serde_json::to_value(check(&t)).unwrap();
}

#[test]
fn meta_is_opaque_and_round_trips() {
    let toml = r#"
id = "m"
title = "M"
meta = { author = "someone", created = "2026-10-08", record = 4, audience = "lay" }

[[step]]
id = "a"
narration = { text = "Hi." }
"#;
    let t = parse_toml(toml).unwrap();
    assert_eq!(
        t.meta,
        Some(json!({"author": "someone", "created": "2026-10-08", "record": 4, "audience": "lay"}))
    );
    let back: Tour = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
    assert_eq!(back, t);
    let j = parse_json(r#"{"id":"m","meta":{"any":["shape",1]},"step":[{"id":"a"}]}"#).unwrap();
    assert_eq!(j.meta, Some(json!({"any": ["shape", 1]})));
    // Without meta, nothing is written.
    let plain = parse_json(r#"{"id":"p","step":[{"id":"a"}]}"#).unwrap();
    assert!(serde_json::to_value(&plain).unwrap().get("meta").is_none());
    // Other unknown top-level fields are still refused.
    assert!(parse_json(r#"{"id":"m","author":"x","step":[{"id":"a"}]}"#).is_err());
}

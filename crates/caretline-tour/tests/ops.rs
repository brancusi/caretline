//! `ops`: what `ops::parse` accepts and refuses, the replies, and `ops::schema()`: the
//! requests `parse` accepts, the replies and the example walkthroughs match it, and the ones
//! it refuses don't. The check is the small validator of caretline-layers' `tests/schema.rs`,
//! for the subset of JSON Schema the schema uses; a test makes sure it uses no other keyword.

use caretline_tour::ops::{self, Request};
use caretline_tour::*;
use serde_json::{Map, Value, json};

/// The keywords the validator knows. Annotations (`description`, `title`, `$schema`, `$id`)
/// change nothing.
const KNOWN: &[&str] = &[
    "$schema",
    "$id",
    "$defs",
    "$ref",
    "title",
    "description",
    "type",
    "const",
    "enum",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "minItems",
    "minimum",
    "maximum",
    "minLength",
    "pattern",
    "oneOf",
    "anyOf",
    "allOf",
    "not",
];

fn keywords_known(s: &Value, at: &str) {
    match s {
        Value::Object(m) => {
            for (k, v) in m {
                assert!(KNOWN.contains(&k.as_str()), "unknown keyword {k} at {at}");
                match k.as_str() {
                    "$defs" | "properties" => {
                        for (name, sub) in v.as_object().expect("an object") {
                            keywords_known(sub, &format!("{at}/{k}/{name}"));
                        }
                    }
                    "oneOf" | "anyOf" | "allOf" => {
                        for (i, sub) in v.as_array().expect("an array").iter().enumerate() {
                            keywords_known(sub, &format!("{at}/{k}/{i}"));
                        }
                    }
                    "items" | "not" | "additionalProperties" if v.is_object() => {
                        keywords_known(v, &format!("{at}/{k}"))
                    }
                    "pattern" => {
                        // Only `^<literal>.+`, which `matches` implements.
                        let p = v.as_str().unwrap();
                        assert!(
                            p.starts_with('^')
                                && p.ends_with(".+")
                                && !p[1..p.len() - 2]
                                    .contains(['.', '*', '+', '?', '[', '(', '\\', '$', '|']),
                            "pattern {p} at {at}"
                        );
                    }
                    _ => {}
                }
            }
        }
        Value::Bool(_) => {}
        _ => panic!("a schema at {at} is an object or a boolean"),
    }
}

fn matches(pattern: &str, s: &str) -> bool {
    let prefix = &pattern[1..pattern.len() - 2];
    s.starts_with(prefix) && s.len() > prefix.len()
}

fn is_type(t: &str, v: &Value) -> bool {
    match t {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => panic!("unknown type {t}"),
    }
}

/// Validates `v` against `s`; `root` resolves `$ref`s. `Err` says where and why.
fn check(root: &Value, s: &Value, v: &Value, at: &str) -> Result<(), String> {
    let fail = |why: String| Err(format!("{at}: {why}"));
    let s = match s {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return fail("nothing matches false".into()),
        Value::Object(m) => m,
        _ => unreachable!(),
    };
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let name = r.strip_prefix("#/$defs/").expect("a local $ref");
        check(root, &root["$defs"][name], v, at)?;
    }
    if let Some(t) = s.get("type") {
        let ok = match t {
            Value::String(t) => is_type(t, v),
            Value::Array(ts) => ts.iter().any(|t| is_type(t.as_str().unwrap(), v)),
            _ => unreachable!(),
        };
        if !ok {
            return fail(format!("{v} isn't {t}"));
        }
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        return fail(format!("{v} isn't {c}"));
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array)
        && !e.contains(v)
    {
        return fail(format!("{v} isn't one of {e:?}"));
    }
    if let Some(n) = v.as_f64() {
        if s.get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|m| n < m)
        {
            return fail(format!("{n} under the minimum"));
        }
        if s.get("maximum")
            .and_then(Value::as_f64)
            .is_some_and(|m| n > m)
        {
            return fail(format!("{n} over the maximum"));
        }
    }
    if let Some(st) = v.as_str() {
        if s.get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|m| (st.chars().count() as u64) < m)
        {
            return fail(format!("{st:?} too short"));
        }
        if let Some(p) = s.get("pattern").and_then(Value::as_str)
            && !matches(p, st)
        {
            return fail(format!("{st:?} doesn't match {p}"));
        }
    }
    if let Some(o) = v.as_object() {
        let props = s.get("properties").and_then(Value::as_object);
        for r in s
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if !o.contains_key(r.as_str().unwrap()) {
                return fail(format!("{r} is required"));
            }
        }
        for (k, val) in o {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => check(root, ps, val, &format!("{at}/{k}"))?,
                None => {
                    if let Some(ap) = s.get("additionalProperties") {
                        check(root, ap, val, &format!("{at}/{k}"))
                            .map_err(|_| format!("{at}: {k} isn't allowed"))?;
                    }
                }
            }
        }
    }
    if let Some(a) = v.as_array() {
        if s.get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| (a.len() as u64) < m)
        {
            return fail("too few items".into());
        }
        if let Some(items) = s.get("items") {
            for (i, x) in a.iter().enumerate() {
                check(root, items, x, &format!("{at}/{i}"))?;
            }
        }
    }
    if let Some(all) = s.get("allOf").and_then(Value::as_array) {
        for sub in all {
            check(root, sub, v, at)?;
        }
    }
    if let Some(any) = s.get("anyOf").and_then(Value::as_array)
        && !any.iter().any(|sub| check(root, sub, v, at).is_ok())
    {
        return fail(format!("{v} matches none of anyOf"));
    }
    if let Some(one) = s.get("oneOf").and_then(Value::as_array) {
        let n = one
            .iter()
            .filter(|sub| check(root, sub, v, at).is_ok())
            .count();
        if n != 1 {
            return fail(format!("{v} matches {n} of oneOf"));
        }
    }
    if let Some(not) = s.get("not")
        && check(root, not, v, at).is_ok()
    {
        return fail(format!("{v} matches not"));
    }
    Ok(())
}

fn valid(def: &str, v: &Value) -> Result<(), String> {
    let root = ops::schema();
    check(&root, &json!({"$ref": format!("#/$defs/{def}")}), v, def)
}

fn with_op(op: &str, req: &Value) -> Value {
    let mut m: Map<String, Value> = req.as_object().cloned().unwrap_or_default();
    m.insert("op".into(), json!(op));
    Value::Object(m)
}

fn example() -> Tour {
    parse_toml(include_str!("../examples/tour.toml")).unwrap()
}

fn library() -> Vec<Tour> {
    vec![example()]
}

#[test]
fn the_schema_is_well_formed_and_uses_only_what_the_validator_checks() {
    let s = ops::schema();
    assert_eq!(
        s["$schema"],
        json!("https://json-schema.org/draft/2020-12/schema")
    );
    keywords_known(&s, "#");
    for def in [
        "request",
        "reply",
        "list",
        "error",
        "tour",
        "step",
        "layer",
        "anchor",
        "find",
        "place",
        "pred",
        "narration",
        "branch",
        "nudge",
    ] {
        assert!(s["$defs"][def].is_object(), "$defs/{def}");
    }
    for r in [
        TourReason::Invalid,
        TourReason::NotFound,
        TourReason::NotRunning,
        TourReason::NoBack,
    ] {
        valid(
            "error",
            &ops::error(&TourError {
                reason: r,
                detail: "x".into(),
            }),
        )
        .unwrap();
    }
}

#[test]
fn the_example_walkthrough_matches_the_schema_as_authored_and_as_normalised() {
    let authored: Value = toml::from_str(include_str!("../examples/tour.toml")).unwrap();
    valid("tour", &authored).unwrap();
    valid("tour", &serde_json::to_value(example()).unwrap()).unwrap();
}

#[test]
fn a_walkthrough_keeps_the_arrow_head_on_the_row_it_names() {
    let mut authored = json!({"id": "row-note", "step": [{
        "id": "beta", "anchor": {"host": {"kind": "row", "key": "beta"}},
        "data": {"text": "This row."}, "place": {"arrow": true, "head": "on_anchor_rows"}
    }]});
    valid("tour", &authored).unwrap();
    let tour = parse_json(&authored.to_string()).unwrap();
    let layers = step_layers(&tour, 0, false);
    assert_eq!(layers[0].head, caretline_layers::HeadRule::OnAnchorRows);
    valid("tour", &serde_json::to_value(&tour).unwrap()).unwrap();
    authored["step"][0]["place"]["head"] = json!("other_rows");
    assert!(parse_json(&authored.to_string()).is_err());
    assert!(valid("tour", &authored).is_err());
}

fn accepted() -> Vec<(&'static str, Value)> {
    let tour = serde_json::to_value(example()).unwrap();
    vec![
        ("tour.start", json!({"tour": "example.basics"})),
        ("tour.start", json!({"tour": tour, "id": 7, "view": "main"})),
        ("tour.step", json!({"to": "next"})),
        ("tour.step", json!({"to": "back"})),
        ("tour.step", json!({"to": "edit"})),
        ("tour.restart", json!({})),
        ("tour.stop", json!({"id": "r1"})),
        ("tour.list", json!({})),
    ]
}

fn refused() -> Vec<(&'static str, Value)> {
    vec![
        ("tour.start", json!({})),
        ("tour.start", json!({"tour": 3})),
        ("tour.start", json!({"tour": {"id": "x", "colour": 1}})),
        ("tour.step", json!({})),
        ("tour.step", json!({"to": ""})),
        ("tour.step", json!({"to": "next", "speed": 2})),
        ("tour.stop", json!({"all": true})),
        ("tour.jump", json!({})),
    ]
}

#[test]
fn what_parse_accepts_the_schema_accepts_and_what_it_refuses_it_refuses() {
    for (op, req) in accepted() {
        ops::parse(op, &req, &library()).unwrap_or_else(|e| panic!("{op} {req}: {e}"));
        valid("request", &with_op(op, &req)).unwrap_or_else(|e| panic!("{op}: {e}"));
    }
    for (op, req) in refused() {
        assert!(ops::parse(op, &req, &library()).is_err(), "{op} {req}");
        assert!(
            valid("request", &with_op(op, &req)).is_err(),
            "the schema accepts {op} {req}"
        );
    }
    // By id, but not in the library: well formed, and not found.
    let e = ops::parse("tour.start", &json!({"tour": "nope"}), &library()).unwrap_err();
    assert_eq!(e.reason, TourReason::NotFound);
}

#[test]
fn parse_maps_requests_to_ops() {
    let lib = library();
    let p = |op: &str, req: Value| ops::parse(op, &req, &lib).unwrap();
    assert_eq!(
        p("tour.start", json!({"tour": "example.basics"})),
        Request::Apply(TourOp::Start(Box::new(example())))
    );
    assert_eq!(
        p("tour.step", json!({"to": "next"})),
        Request::Apply(TourOp::Next)
    );
    assert_eq!(
        p("tour.step", json!({"to": "back"})),
        Request::Apply(TourOp::Back)
    );
    assert_eq!(
        p("tour.step", json!({"to": "edit"})),
        Request::Apply(TourOp::To("edit".into()))
    );
    assert_eq!(p("tour.stop", json!({})), Request::Apply(TourOp::Stop));
    assert_eq!(
        p("tour.restart", json!({})),
        Request::Apply(TourOp::Restart)
    );
    assert_eq!(p("tour.list", json!({})), Request::List);
}

#[test]
fn replies_match_the_schema() {
    let lib = library();
    let mut s = TourState::default();
    valid("reply", &ops::reply(&s)).unwrap();
    assert_eq!(ops::reply(&s), json!({"tour": null, "step": null}));
    valid("list", &ops::list(&lib, &s)).unwrap();
    let Request::Apply(op) =
        ops::parse("tour.start", &json!({"tour": "example.basics"}), &lib).unwrap()
    else {
        panic!()
    };
    apply(&mut s, op, 0).unwrap();
    let r = ops::reply(&s);
    valid("reply", &r).unwrap();
    assert_eq!(r["step"], "welcome");
    assert_eq!(r["at"], 1);
    assert_eq!(r["of"], 4);
    assert_eq!(r["narration"]["title"], "Welcome");
    apply(&mut s, TourOp::Stop, 1).unwrap();
    let l = ops::list(&lib, &s);
    valid("list", &l).unwrap();
    assert_eq!(
        l["tours"][0]["seen"],
        json!({"version": 1, "end": "stopped"})
    );
    assert_eq!(
        l["current"],
        json!({"tour": "example.basics", "step": null})
    );
}

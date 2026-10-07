//! `ops::schema()`: the requests `ops::parse` accepts and the replies it gives match it, the
//! ones it refuses don't, and so do the protocol examples in the design (docs/design/layers.md
//! §7.4). The check is a small validator for the subset of JSON Schema the schema uses; a
//! test makes sure it uses no other keyword, so nothing is silently skipped.

use caretline_layers::ops::{self, Request};
use caretline_layers::*;
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

/// A request as a protocol sends it: its fields with its `op`.
fn with_op(op: &str, req: &Value) -> Value {
    let mut m: Map<String, Value> = req.as_object().cloned().unwrap_or_default();
    m.insert("op".into(), json!(op));
    Value::Object(m)
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
        "request", "reply", "resolved", "list", "error", "anchor", "layer", "content", "hint",
        "owner",
    ] {
        assert!(s["$defs"][def].is_object(), "$defs/{def}");
    }
    // Every Reason the crate gives is in the error's enum.
    for r in [
        Reason::RateLimited,
        Reason::DimNotAllowed,
        Reason::CaptureNotAllowed,
        Reason::TooLong,
        Reason::NotFound,
        Reason::NotAllowed,
        Reason::TooMany,
        Reason::Invalid,
    ] {
        let e = ops::error(&Refusal {
            reason: r,
            detail: "x".into(),
        });
        valid("error", &e).unwrap();
    }
}

/// Requests `parse` accepts (those of tests/ops.rs, and more).
fn accepted() -> Vec<(&'static str, Value)> {
    vec![
        (
            "hint.show",
            json!({"id": 4, "actor": "claude", "anchor": {"host": {"kind": "row", "key": "def"}},
                   "title": "Stale", "text": "This row hasn't synced.", "ttl_ms": 8000, "place": ["right", "below"]}),
        ),
        (
            "hint.show",
            json!({"actor": "a", "anchor": {"host": {"kind": "row", "key": "zzz"}}, "text": "Below."}),
        ),
        (
            "hint.show",
            json!({"actor": "c", "anchor": [{"host": {"kind": "diff", "key": "src/main.rs:42"}}, {"caret": true}], "text": "This line."}),
        ),
        (
            "hint.show",
            json!({"anchor": {"text": {"from": 4, "to": 9}, "in": "panel:2"}, "text": "x", "title": null, "ttl_ms": null, "arrow": false, "ring": false, "view": 0}),
        ),
        (
            "hint.show",
            json!({"anchor": [{"text": {"block": 7, "from": 0, "to": 3}, "in": "main"}, {"block": 7}, {"screen": "center"}], "text": "x"}),
        ),
        (
            "layer.push",
            json!({"layer": {"anchor": [{"host": {"kind": "row", "key": "ghi"}}], "content": {"kind": "hint", "data": {"text": "Host's."}}}}),
        ),
        (
            "layer.push",
            json!({"actor": "a", "layer": {"id": "x", "owner": "agent:b", "z": 25, "since_ms": 0, "ttl_ms": null,
                   "anchor": [{"caret": true, "in": "panel:2"}], "content": null, "arrow": false,
                   "ring": {"pulse": {"period_ms": 800, "cycles": 3}}, "spotlight": {"holes": ["anchor"]},
                   "capture": false, "hide_off_screen": true, "place": ["left"], "max_width": 30}}),
        ),
        (
            "layer.update",
            json!({"layer": {"id": "L-2", "anchor": [{"host": {"kind": "row", "key": "def"}}], "content": {"kind": "hint", "data": {"text": "Moved."}}}}),
        ),
        ("layer.list", json!({})),
        ("layer.list", json!({"id": "x", "actor": null})),
        ("hint.hide", json!({"actor": "a", "layer": "L-2"})),
        ("hint.hide", json!({"actor": "a", "all": true})),
        (
            "hint.hide",
            json!({"layer": "L-2", "all": false, "owner": null}),
        ),
        ("layer.pop", json!({"owner": "host"})),
        ("layer.pop", json!({"owner": "agent:claude"})),
        ("layer.pop", json!({"layer": "L-1"})),
        ("layer.pop", json!({"all": true, "layer": null})),
    ]
}

/// Requests `parse` refuses (those of tests/ops.rs, and more).
fn refused() -> Vec<(&'static str, Value)> {
    vec![
        ("hint.show", json!({"anchor": {"caret": true}})),
        (
            "hint.show",
            json!({"anchor": {"cells": {"x": 1, "y": 1, "w": 1, "h": 1}}, "text": "x"}),
        ),
        (
            "hint.show",
            json!({"anchor": {"caret": true}, "text": "x", "colour": "red"}),
        ),
        (
            "hint.show",
            json!({"anchor": {"caret": false}, "text": "x"}),
        ),
        (
            "hint.show",
            json!({"anchor": {"screen": "center", "in": "main"}, "text": "x"}),
        ),
        (
            "hint.show",
            json!({"anchor": {"caret": true, "in": ""}, "text": "x"}),
        ),
        (
            "hint.show",
            json!({"anchor": {"caret": true, "block": 2}, "text": "x"}),
        ),
        (
            "hint.show",
            json!({"anchor": {"caret": true}, "text": "x", "place": ["middle"]}),
        ),
        ("hint.hide", json!({})),
        ("hint.hide", json!({"owner": "host"})),
        ("hint.hide", json!({"layer": "L-1", "all": true})),
        ("layer.pop", json!({"layer": "L-1", "all": true})),
        ("layer.pop", json!({"layer": "L-1", "owner": "host"})),
        ("layer.pop", json!({"owner": "agent:"})),
        ("layer.pop", json!({"all": false})),
        (
            "layer.push",
            json!({"layer": {"content": {"kind": "hint"}}}),
        ),
        (
            "layer.push",
            json!({"layer": {"anchor": [{"caret": true}], "colour": "red"}}),
        ),
        (
            "layer.push",
            json!({"layer": {"anchor": [{"caret": true}], "z": 40000}}),
        ),
        ("layer.list", json!({"all": true})),
        ("layer.frobnicate", json!({})),
    ]
}

#[test]
fn what_parse_accepts_the_schema_accepts() {
    for (op, req) in accepted() {
        assert!(ops::parse(op, &req).is_ok(), "parse {op} {req}");
        let full = with_op(op, &req);
        valid("request", &full).unwrap_or_else(|e| panic!("{op} {req}: {e}"));
    }
}

#[test]
fn what_parse_refuses_the_schema_refuses() {
    for (op, req) in refused() {
        assert!(ops::parse(op, &req).is_err(), "parse {op} {req}");
        let full = with_op(op, &req);
        assert!(valid("request", &full).is_err(), "schema took {full}");
    }
}

#[test]
fn every_reply_shape_matches() {
    let mut anchors = AnchorMap::new();
    anchors.put(AnchorKey::host("row", "a"), Rect::new(2, 2, 10, 1));
    anchors.put_off(AnchorKey::host("row", "b"), Off::Below { x: None });
    let grid = Grid::new(80, 24);
    let renderers = Renderers::new().register(HINT, |_: &Value, a: Size| Size::new(20.min(a.w), 4));
    let mut layers = Layers::default();
    let mut replies = Vec::new();
    for (op, req) in [
        (
            "hint.show",
            json!({"actor": "a", "anchor": {"host": {"kind": "row", "key": "a"}}, "text": "On."}),
        ),
        (
            "hint.show",
            json!({"actor": "a", "anchor": {"host": {"kind": "row", "key": "b"}}, "text": "Off."}),
        ),
        (
            "hint.show",
            json!({"actor": "a", "anchor": {"host": {"kind": "row", "key": "c"}}, "text": "Gone."}),
        ),
        ("hint.hide", json!({"actor": "a", "layer": "L-1"})),
        ("layer.pop", json!({"owner": "guide"})),
        (
            "layer.update",
            json!({"layer": {"id": "nope", "anchor": [{"caret": true}], "ring": {}}}),
        ),
        ("layer.list", json!({})),
    ] {
        let reply = match ops::parse(op, &req).unwrap() {
            (Request::List, _) => ("list", ops::list(&layers)),
            (Request::Apply(o), actor) => {
                match apply(&mut layers, o, actor.as_deref(), 0, &Limits::default()) {
                    Ok(a) => (
                        "reply",
                        ops::reply(&a, Some(&plan(&layers, &anchors, &grid, &renderers))),
                    ),
                    Err(r) => ("error", ops::error(&r)),
                }
            }
        };
        replies.push(reply);
    }
    let kinds: Vec<&str> = replies.iter().map(|r| r.0).collect();
    assert_eq!(
        kinds,
        ["reply", "reply", "reply", "reply", "reply", "error", "list"]
    );
    assert_eq!(replies[1].1["resolved"], json!({"off": "below"}));
    assert_eq!(replies[2].1["reason"], json!("not_found"));
    for (def, v) in &replies {
        valid(def, v).unwrap_or_else(|e| panic!("{def} {v}: {e}"));
    }
    // Resolved in a named view.
    valid(
        "resolved",
        &json!({"rects": [{"x": 1, "y": 2, "w": 3, "h": 1}], "in": "panel:2"}),
    )
    .unwrap();
    assert!(valid("resolved", &json!({"rects": []})).is_err());
}

/// The JSON object at the start of a line (a comment may follow it).
fn first_object(line: &str) -> Option<&str> {
    let start = line.find('{')?;
    let (mut depth, mut in_str, mut esc) = (0, false, false);
    for (i, c) in line[start..].char_indices() {
        match c {
            _ if esc => esc = false,
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            '{' if !in_str => depth += 1,
            '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(&line[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn the_designs_protocol_examples_match() {
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/design/layers.md"),
    )
    .unwrap();
    let section = &doc[doc.find("### 7.4").unwrap()..doc.find("### 7.5").unwrap()];
    let block = section.split("```json").nth(1).unwrap();
    let block = &block[..block.find("```").unwrap()];
    let mut ops_by_id = std::collections::BTreeMap::new();
    let mut checked = 0;
    for line in block.lines() {
        let Some(obj) = first_object(line) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(obj) else {
            continue; // a sketch with … in it: not a layer op
        };
        let id = v["id"].clone().to_string();
        if let Some(op) = v["op"].as_str() {
            ops_by_id.insert(id, op.to_string());
            if !(op.starts_with("hint.") || op.starts_with("layer.")) {
                continue;
            }
            assert!(ops::parse(op, &v).is_ok(), "parse {line}");
            valid("request", &v).unwrap_or_else(|e| panic!("{line}: {e}"));
            checked += 1;
            continue;
        }
        let Some(op) = ops_by_id.get(&id) else {
            continue;
        };
        if !(op.starts_with("hint.") || op.starts_with("layer.")) {
            continue;
        }
        if let Some(e) = v.get("error") {
            valid("error", &json!({"error": e})).unwrap_or_else(|e| panic!("{line}: {e}"));
        } else {
            let mut r = v["result"].clone();
            r.as_object_mut().unwrap().remove("rev");
            let def = if op == "layer.list" { "list" } else { "reply" };
            valid(def, &r).unwrap_or_else(|e| panic!("{line}: {e}"));
        }
        checked += 1;
    }
    assert!(checked >= 14, "only {checked} examples checked");
}

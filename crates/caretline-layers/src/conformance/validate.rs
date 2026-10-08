//! A small JSON Schema validator for the subset `ops::schema()` uses (draft 2020-12 keywords:
//! `type`, `const`, `enum`, `properties`, `required`, `additionalProperties`, `items`,
//! `minItems`, `minimum`, `maximum`, `minLength`, a `^<literal>.+` `pattern`, `oneOf`,
//! `anyOf`, `allOf`, `not` and local `$ref`s). [`keywords_known`] fails a schema that uses any
//! other keyword, so nothing is silently skipped. No dependency but serde_json.
//!
//! The crate's own schema tests read this file too.

use serde_json::Value;

/// The keywords the validator knows. Annotations (`description`, `title`, `$schema`, `$id`)
/// change nothing.
pub const KNOWN: &[&str] = &[
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

/// Whether `s` (found at `at`) uses only keywords the validator checks.
pub fn keywords_known(s: &Value, at: &str) -> Result<(), String> {
    match s {
        Value::Object(m) => {
            for (k, v) in m {
                if !KNOWN.contains(&k.as_str()) {
                    return Err(format!("unknown keyword {k} at {at}"));
                }
                match k.as_str() {
                    "$defs" | "properties" => {
                        let Some(o) = v.as_object() else {
                            return Err(format!("{at}/{k} isn't an object"));
                        };
                        for (name, sub) in o {
                            keywords_known(sub, &format!("{at}/{k}/{name}"))?;
                        }
                    }
                    "oneOf" | "anyOf" | "allOf" => {
                        let Some(a) = v.as_array() else {
                            return Err(format!("{at}/{k} isn't an array"));
                        };
                        for (i, sub) in a.iter().enumerate() {
                            keywords_known(sub, &format!("{at}/{k}/{i}"))?;
                        }
                    }
                    "items" | "not" | "additionalProperties" if v.is_object() => {
                        keywords_known(v, &format!("{at}/{k}"))?
                    }
                    "pattern" => {
                        // Only `^<literal>.+`, which `matches` implements.
                        let p = v.as_str().unwrap_or_default();
                        let ok = p.starts_with('^')
                            && p.ends_with(".+")
                            && p.len() >= 3
                            && !p[1..p.len() - 2]
                                .contains(['.', '*', '+', '?', '[', '(', '\\', '$', '|']);
                        if !ok {
                            return Err(format!("pattern {p} at {at}"));
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        Value::Bool(_) => Ok(()),
        _ => Err(format!("a schema at {at} is an object or a boolean")),
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
        _ => false,
    }
}

/// Validates `v` against `s`; `root` resolves `$ref`s (`#/$defs/<name>`). `Err` says where
/// (`at`, then the path inside `v`) and why.
pub fn check(root: &Value, s: &Value, v: &Value, at: &str) -> Result<(), String> {
    let fail = |why: String| Err(format!("{at}: {why}"));
    let s = match s {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return fail("nothing matches false".into()),
        Value::Object(m) => m,
        _ => return fail("a schema is an object or a boolean".into()),
    };
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let Some(name) = r.strip_prefix("#/$defs/") else {
            return fail(format!("{r} isn't a local $ref"));
        };
        check(root, &root["$defs"][name], v, at)?;
    }
    if let Some(t) = s.get("type") {
        let ok = match t {
            Value::String(t) => is_type(t, v),
            Value::Array(ts) => ts
                .iter()
                .any(|t| is_type(t.as_str().unwrap_or_default(), v)),
            _ => false,
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
            if !o.contains_key(r.as_str().unwrap_or_default()) {
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

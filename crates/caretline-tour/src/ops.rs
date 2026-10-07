//! Protocol-neutral requests and replies for walkthroughs, mirroring
//! `caretline_layers::ops`: a host's own JSON protocol parses requests with [`parse`],
//! applies the op with [`apply`](crate::apply), and answers with [`reply`]. Pure: JSON in,
//! JSON out.
//!
//! | Op | Request fields | Becomes |
//! |---|---|---|
//! | `tour.start` | `tour`: a whole tour (an object), or the id of one in the host's library | [`TourOp::Start`] with the whole tour |
//! | `tour.step` | `to`: a step id, `"next"` or `"back"` | [`TourOp::To`], [`TourOp::Next`] or [`TourOp::Back`] |
//! | `tour.restart` | | [`TourOp::Restart`] |
//! | `tour.stop` | | [`TourOp::Stop`] |
//! | `tour.list` | | nothing applied; [`list`] answers |
//!
//! Every request may carry the host's `id` (a request id, ignored here) and `view` (its
//! routing, removed before parsing). Anything else is refused.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::Tour;
use crate::state::{TourError, TourOp, TourReason, TourState};

/// A parsed request: an op to apply, or a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Apply(TourOp),
    List,
}

fn invalid<T>(detail: impl Into<String>) -> Result<T, TourError> {
    Err(TourError {
        reason: TourReason::Invalid,
        detail: detail.into(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    #[serde(default)]
    id: Option<Value>,
    tour: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepTo {
    #[serde(default)]
    id: Option<Value>,
    to: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bare {
    #[serde(default)]
    id: Option<Value>,
}

fn fields(req: &Value) -> Value {
    let mut v = req.clone();
    if let Some(m) = v.as_object_mut() {
        m.remove("op");
        m.remove("view");
    }
    v
}

fn de<T: for<'de> Deserialize<'de>>(op: &str, req: &Value) -> Result<T, TourError> {
    serde_json::from_value(fields(req)).or_else(|e| invalid(format!("{op}: {e}")))
}

/// Parses a request. `library` is the host's walkthroughs, for `tour.start` by id.
pub fn parse(op: &str, req: &Value, library: &[Tour]) -> Result<Request, TourError> {
    match op {
        "tour.start" => {
            let s: Start = de(op, req)?;
            let _ = s.id;
            let tour = match s.tour {
                Value::String(id) => match library.iter().find(|t| t.id == id) {
                    Some(t) => t.clone(),
                    None => {
                        return Err(TourError {
                            reason: TourReason::NotFound,
                            detail: format!("no walkthrough {id:?}"),
                        });
                    }
                },
                v @ Value::Object(_) => {
                    serde_json::from_value(v).or_else(|e| invalid(format!("tour.start: {e}")))?
                }
                _ => return invalid("tour.start: `tour` is a tour or a walkthrough's id"),
            };
            Ok(Request::Apply(TourOp::Start(Box::new(tour))))
        }
        "tour.step" => {
            let s: StepTo = de(op, req)?;
            let _ = s.id;
            Ok(Request::Apply(match s.to.as_str() {
                "next" => TourOp::Next,
                "back" => TourOp::Back,
                "" => return invalid("tour.step: `to` is a step id, \"next\" or \"back\""),
                _ => TourOp::To(s.to),
            }))
        }
        "tour.restart" | "tour.stop" | "tour.list" => {
            let b: Bare = de(op, req)?;
            let _ = b.id;
            Ok(match op {
                "tour.restart" => Request::Apply(TourOp::Restart),
                "tour.stop" => Request::Apply(TourOp::Stop),
                _ => Request::List,
            })
        }
        _ => invalid(format!("unknown op {op:?}")),
    }
}

/// Where the walkthrough is after an op: `{"tour", "step", "at", "of", "narration"?}`, with
/// `step: null` when none is running (`tour` is then the last one, or null).
pub fn reply(s: &TourState) -> Value {
    let mut out = json!({
        "tour": s.tour.as_ref().map(|t| t.id.clone()),
        "step": s.step,
    });
    if let (Some(t), Some(i)) = (&s.tour, s.index()) {
        out["at"] = json!(i + 1);
        out["of"] = json!(t.steps.len());
        if let Some(n) = s.narration() {
            out["narration"] = json!(n);
        }
    }
    out
}

/// The result of `tour.list`: each walkthrough in the library (`id`, `version`, `title`,
/// `steps`, and `seen` when the state remembers it), and where the current one is.
pub fn list(library: &[Tour], s: &TourState) -> Value {
    let tours: Vec<Value> = library
        .iter()
        .map(|t| {
            let mut v = json!({
                "id": t.id,
                "version": t.version,
                "title": t.title,
                "steps": t.steps.len(),
            });
            if let Some(seen) = s.seen.get(&t.id) {
                v["seen"] = json!(seen);
            }
            v
        })
        .collect();
    json!({"tours": tours, "current": reply(s)})
}

/// A refusal as an error result: `{"error": {"reason": "not_found", "detail": "…"}}`.
pub fn error(e: &TourError) -> Value {
    json!({"error": {"reason": e.reason, "detail": e.detail}})
}

/// A JSON Schema (draft 2020-12) for the ops: every request [`parse`] accepts under
/// `$defs/request`, the replies ([`reply`], [`list`], [`error`]), and the walkthrough format
/// itself (`$defs/tour`, `step`, `layer`, `anchor`, `place`, `pred`, `narration`, `branch`,
/// `nudge`). Anchors are `caretline_layers`' own, with `find` beside them.
///
/// It describes the wire's shape. What it can't say is checked by the parser (a step's
/// `layers` and the single-layer fields aren't both given; a predicate has one key) and by
/// [`check`](crate::check).
pub fn schema() -> Value {
    let layers = caretline_layers::ops::schema();
    let u16 = json!({"type": "integer", "minimum": 0, "maximum": 65535});
    let u32 = json!({"type": "integer", "minimum": 0, "maximum": 4_294_967_295u64});
    let u64 = json!({"type": "integer", "minimum": 0});
    let anchors = json!({
        "oneOf": [
            {"$ref": "#/$defs/step_anchor"},
            {"type": "array", "items": {"$ref": "#/$defs/step_anchor"}}
        ]
    });
    let flag = |def: Value| json!({"anyOf": [{"type": "boolean"}, def]});
    let ring = layers["$defs"]["layer"]["properties"]["ring"]["anyOf"][0].clone();
    let spotlight = layers["$defs"]["layer"]["properties"]["spotlight"]["anyOf"][0].clone();
    let request = |op: &str, mut props: Value, required: &[&str]| {
        let p = props.as_object_mut().expect("properties");
        p.insert("op".into(), json!({"const": op}));
        p.insert(
            "id".into(),
            json!({"description": "The host's request id; any JSON, ignored here."}),
        );
        p.insert(
            "view".into(),
            json!({"description": "The host's routing; any JSON, ignored here."}),
        );
        let mut req = vec!["op"];
        req.extend(required);
        json!({"type": "object", "properties": props, "required": req, "additionalProperties": false})
    };
    let pred_keys = [
        "msg",
        "command",
        "event",
        "effect",
        "host_effect",
        "state",
        "ext",
        "caret_in",
        "selection",
        "changed",
        "folded",
        "after_ms",
        "any",
        "all",
        "not",
    ];
    let pred_one: Vec<Value> = pred_keys.iter().map(|k| json!({"required": [k]})).collect();
    let step_layer_props = json!({
        "anchor": anchors,
        "kind": {"type": "string"},
        "data": {"description": "The kind's data, opaque here."},
        "place": {"$ref": "#/$defs/place"},
        "capture": {"type": "boolean"}
    });
    let mut layer_props = step_layer_props.clone();
    layer_props["id"] = json!({"type": "string"});
    let mut step_props = step_layer_props;
    for (k, v) in [
        ("id", json!({"type": "string"})),
        (
            "layers",
            json!({"type": "array", "items": {"$ref": "#/$defs/layer"}}),
        ),
        ("narration", json!({"$ref": "#/$defs/narration"})),
        (
            "host",
            json!({"description": "The host's patch for the step; opaque."}),
        ),
        ("advance", json!({"$ref": "#/$defs/pred"})),
        ("skip_if", json!({"$ref": "#/$defs/pred"})),
        ("nudge", json!({"$ref": "#/$defs/nudge"})),
        (
            "next",
            json!({"type": "array", "items": {"$ref": "#/$defs/branch"}}),
        ),
    ] {
        step_props[k] = v;
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://caretline.app/schema/tour-ops.json",
        "title": "caretline-tour ops",
        "description": "Requests a host's protocol passes to caretline_tour::ops::parse, the replies ops::reply, ops::list and ops::error give, and the walkthrough format.",
        "anyOf": [
            {"$ref": "#/$defs/request"},
            {"$ref": "#/$defs/reply"},
            {"$ref": "#/$defs/list"},
            {"$ref": "#/$defs/error"}
        ],
        "$defs": {
            "view": layers["$defs"]["view"].clone(),
            "anchor": layers["$defs"]["anchor"].clone(),
            "side": layers["$defs"]["side"].clone(),
            "find": {
                "type": "object",
                "description": "A text the host searches for once, before the walkthrough starts (Tour::resolve_finds).",
                "properties": {"find": {"type": "string", "minLength": 1}, "in": {"$ref": "#/$defs/view"}},
                "required": ["find"],
                "additionalProperties": false
            },
            "step_anchor": {"oneOf": [{"$ref": "#/$defs/anchor"}, {"$ref": "#/$defs/find"}]},
            "place": {
                "anyOf": [
                    {"type": "array", "items": {"$ref": "#/$defs/side"}},
                    {
                        "type": "object",
                        "properties": {
                            "sides": {"type": "array", "items": {"$ref": "#/$defs/side"}},
                            "max_width": u16,
                            "max_w": u16,
                            "arrow": {"type": "boolean"},
                            "connector": {"type": "boolean"},
                            "ring": flag(ring),
                            "spotlight": flag(spotlight),
                            "hide_off_screen": {"type": "boolean"}
                        },
                        "additionalProperties": false
                    }
                ]
            },
            "narration": {
                "type": "object",
                "properties": {"title": {"type": "string"}, "text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": false
            },
            "pred": {
                "type": "object",
                "properties": {
                    "msg": {"type": "string"},
                    "command": {"type": "string"},
                    "event": {"type": "string"},
                    "effect": {"type": "string"},
                    "host_effect": {"type": "string"},
                    "count": u32,
                    "state": {"$ref": "#/$defs/state_test"},
                    "ext": {"$ref": "#/$defs/state_test"},
                    "caret_in": {"type": "string"},
                    "selection": {"const": "nonempty"},
                    "changed": {"type": "string"},
                    "folded": {"type": "string"},
                    "after_ms": u64,
                    "any": {"type": "array", "items": {"$ref": "#/$defs/pred"}},
                    "all": {"type": "array", "items": {"$ref": "#/$defs/pred"}},
                    "not": {"$ref": "#/$defs/pred"}
                },
                "anyOf": pred_one,
                "additionalProperties": false
            },
            "state_test": {
                "type": "object",
                "properties": {
                    "key": {"type": "string"},
                    "present": {"type": "boolean"},
                    "match": {"description": "A subset the host's value must contain."}
                },
                "required": ["key"],
                "additionalProperties": false
            },
            "branch": {
                "type": "object",
                "properties": {"if": {"$ref": "#/$defs/pred"}, "goto": {"type": "string"}},
                "required": ["goto"],
                "additionalProperties": false
            },
            "nudge": {
                "type": "object",
                "properties": {"after_ms": u64, "data": {"type": "object"}},
                "required": ["after_ms", "data"],
                "additionalProperties": false
            },
            "layer": {
                "type": "object",
                "properties": layer_props,
                "required": ["anchor"],
                "additionalProperties": false
            },
            "step": {
                "type": "object",
                "properties": step_props,
                "required": ["id"],
                "additionalProperties": false
            },
            "tour": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "version": u32,
                    "title": {"type": "string"},
                    "kind": {"type": "string"},
                    "reoffer": {"type": "boolean"},
                    "step": {"type": "array", "items": {"$ref": "#/$defs/step"}},
                    "steps": {"type": "array", "items": {"$ref": "#/$defs/step"}}
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "request": {
                "oneOf": [
                    {"$ref": "#/$defs/tour.start"},
                    {"$ref": "#/$defs/tour.step"},
                    {"$ref": "#/$defs/tour.restart"},
                    {"$ref": "#/$defs/tour.stop"},
                    {"$ref": "#/$defs/tour.list"}
                ]
            },
            "tour.start": request("tour.start", json!({
                "tour": {"anyOf": [{"$ref": "#/$defs/tour"}, {"type": "string", "minLength": 1}]}
            }), &["tour"]),
            "tour.step": request("tour.step", json!({
                "to": {"type": "string", "minLength": 1, "description": "A step id, \"next\" or \"back\"."}
            }), &["to"]),
            "tour.restart": request("tour.restart", json!({}), &[]),
            "tour.stop": request("tour.stop", json!({}), &[]),
            "tour.list": request("tour.list", json!({}), &[]),
            "reply": {
                "type": "object",
                "properties": {
                    "tour": {"type": ["string", "null"]},
                    "step": {"type": ["string", "null"]},
                    "at": u32,
                    "of": u32,
                    "narration": {"$ref": "#/$defs/narration"}
                },
                "required": ["tour", "step"],
                "additionalProperties": false
            },
            "list": {
                "type": "object",
                "properties": {
                    "tours": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string"},
                                "version": u32,
                                "title": {"type": "string"},
                                "steps": u32,
                                "seen": {
                                    "type": "object",
                                    "properties": {
                                        "version": u32,
                                        "end": {"enum": ["finished", "stopped"]}
                                    },
                                    "required": ["version", "end"],
                                    "additionalProperties": false
                                }
                            },
                            "required": ["id", "version", "title", "steps"],
                            "additionalProperties": false
                        }
                    },
                    "current": {"$ref": "#/$defs/reply"}
                },
                "required": ["tours", "current"],
                "additionalProperties": false
            },
            "error": {
                "type": "object",
                "properties": {
                    "error": {
                        "type": "object",
                        "properties": {
                            "reason": {"enum": ["invalid", "not_found", "not_running", "no_back"]},
                            "detail": {"type": "string"}
                        },
                        "required": ["reason", "detail"],
                        "additionalProperties": false
                    }
                },
                "required": ["error"],
                "additionalProperties": false
            }
        }
    })
}

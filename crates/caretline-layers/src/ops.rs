//! Protocol-neutral requests and replies (design §5.3): a host's own JSON protocol, or
//! caretline's, parses agents' requests with [`parse`], applies the op with [`apply`](crate::apply),
//! and answers with [`reply`]. Pure: JSON in, JSON out.
//!
//! | Op | Request fields | Becomes |
//! |---|---|---|
//! | `hint.show` | `anchor` (one or a list), `text`, `title?`, `ttl_ms?`, `place?`, `arrow?` (default true), `ring?` (default true), `avoid?` (one anchor or a list), `head?` (`any` or `on_anchor_rows`), `actor?` | a push of a [`HINT`] layer |
//! | `hint.hide` | `layer` or `all: true`, `actor?` | a pop |
//! | `layer.push`, `layer.update` | `layer` (a [`Layer`]), `actor?` | a push or update |
//! | `layer.pop` | `layer`, `owner` or `all: true`, `actor?` | a pop |
//! | `layer.list` | `actor?` | nothing applied; [`list`] answers |
//!
//! `actor` names the agent the request comes from; without it the request is the host's (or
//! the person's) own. A host decides which: a protocol client is normally an agent.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::geom::Side;
use crate::model::{
    Anchor, Applied, Content, HeadRule, Layer, LayerOp, Layers, Owner, Reason, Refusal, Selector,
};
use crate::place::Plan;
use crate::resolve::Off;

/// A parsed request: an op to apply, or a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // parsed once per request, never stored
pub enum Request {
    Apply(LayerOp),
    List,
}

fn invalid<T>(detail: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal {
        reason: Reason::Invalid,
        detail: detail.into(),
    })
}

/// One anchor or a list of fallbacks.
#[derive(Deserialize)]
#[serde(untagged)]
enum Anchors {
    One(Anchor),
    Many(Vec<Anchor>),
}

impl Anchors {
    fn list(self) -> Vec<Anchor> {
        match self {
            Anchors::One(a) => vec![a],
            Anchors::Many(v) => v,
        }
    }
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Show {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    actor: Option<String>,
    anchor: Anchors,
    text: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    ttl_ms: Option<u64>,
    #[serde(default)]
    place: Vec<Side>,
    #[serde(default = "yes")]
    arrow: bool,
    #[serde(default = "yes")]
    ring: bool,
    #[serde(default)]
    avoid: Option<Anchors>,
    #[serde(default)]
    head: HeadRule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WithLayer {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    actor: Option<String>,
    layer: Layer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pop {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    actor: Option<String>,
    #[serde(default)]
    layer: Option<String>,
    #[serde(default)]
    owner: Option<Owner>,
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    actor: Option<String>,
}

/// The request's fields, minus `op` and `view` (the host's routing).
fn fields(req: &Value) -> Value {
    let mut v = req.clone();
    if let Some(m) = v.as_object_mut() {
        m.remove("op");
        m.remove("view");
    }
    v
}

fn de<T: for<'de> Deserialize<'de>>(op: &str, req: &Value) -> Result<T, Refusal> {
    serde_json::from_value(fields(req)).or_else(|e| invalid(format!("{op}: {e}")))
}

/// Parses a request into what to apply and the actor it comes from.
pub fn parse(op: &str, req: &Value) -> Result<(Request, Option<String>), Refusal> {
    match op {
        "hint.show" => {
            let s: Show = de(op, req)?;
            let _ = s.id;
            let mut layer = Layer::new(Anchor::Caret);
            layer.anchor = s.anchor.list();
            layer.content = Some(Content::hint(s.title.as_deref(), &s.text));
            layer.arrow = s.arrow;
            layer.ring = s.ring.then(Default::default);
            layer.ttl_ms = s.ttl_ms;
            layer.place = s.place;
            layer.avoid = s.avoid.map(Anchors::list).unwrap_or_default();
            layer.head = s.head;
            Ok((Request::Apply(LayerOp::Push(layer)), s.actor))
        }
        "layer.push" | "layer.update" => {
            let w: WithLayer = de(op, req)?;
            let _ = w.id;
            let op = if op == "layer.push" {
                LayerOp::Push(w.layer)
            } else {
                LayerOp::Update(w.layer)
            };
            Ok((Request::Apply(op), w.actor))
        }
        "hint.hide" | "layer.pop" => {
            let p: Pop = de(op, req)?;
            let _ = p.id;
            let sel = match (p.layer, p.owner, p.all) {
                (Some(l), None, false) => Selector::Layer(l),
                (None, Some(o), false) if op == "layer.pop" => Selector::Owner(o),
                (None, None, true) => Selector::All(true),
                _ => return invalid(format!("{op}: give one of `layer`, `owner` or `all: true`")),
            };
            Ok((Request::Apply(LayerOp::Pop(sel)), p.actor))
        }
        "layer.list" => {
            let l: List = de(op, req)?;
            let _ = l.id;
            Ok((Request::List, l.actor))
        }
        _ => invalid(format!("unknown op {op:?}")),
    }
}

fn off_name(o: &Off) -> &'static str {
    match o {
        Off::Above { .. } => "above",
        Off::Below { .. } => "below",
        Off::Left { .. } => "left",
        Off::Right { .. } => "right",
    }
}

/// Where a layer landed in a plan: `{"rects": […]}`, `{"off": "below"}`, or `null` (with
/// `reason: "not_found"` beside it in [`reply`]); with `"in": "<view>"` when it resolved in a
/// named view.
pub fn resolved(plan: &Plan, layer: &str) -> Value {
    let Some(r) = plan
        .layers
        .iter()
        .find(|l| l.id == layer)
        .and_then(|l| l.anchor.as_ref())
    else {
        return Value::Null;
    };
    let mut v = match (&r.rects, &r.off) {
        (rects, _) if !rects.is_empty() => json!({"rects": rects}),
        (_, Some(o)) => json!({"off": off_name(o)}),
        _ => return Value::Null,
    };
    if let Some(view) = &r.view {
        v["in"] = json!(view);
    }
    v
}

/// The result of an applied op: the layer id, what it popped, and, given this frame's plan,
/// where it resolved.
pub fn reply(applied: &Applied, plan: Option<&Plan>) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(id) = &applied.layer {
        out.insert("layer".into(), json!(id));
        if let Some(p) = plan {
            let r = resolved(p, id);
            if r.is_null() {
                out.insert("reason".into(), json!("not_found"));
            }
            out.insert("resolved".into(), r);
        }
    }
    if !applied.popped.is_empty() || applied.layer.is_none() {
        out.insert("popped".into(), json!(applied.popped));
    }
    Value::Object(out)
}

/// The result of `layer.list`: every layer, for the actor's eyes (an agent sees all, as the
/// person does; it can change only its own).
pub fn list(layers: &Layers) -> Value {
    json!({"layers": layers.layers, "hidden": layers.hidden})
}

/// A refusal as an error result: `{"error": {"reason": "rate_limited", "detail": "…"}}`.
pub fn error(r: &Refusal) -> Value {
    json!({"error": {"reason": r.reason, "detail": r.detail}})
}

/// A JSON Schema (draft 2020-12) for the ops: every request [`parse`] accepts, with its `op`
/// (and the host's `id` and `view`, which it ignores), under `$defs/request`, and each reply
/// shape under `$defs/reply` ([`reply`]), `$defs/resolved` ([`resolved`]), `$defs/list`
/// ([`list`]) and `$defs/error` ([`error`]). The parts are `$defs` too: `anchor` (with its
/// `in` scope), `layer`, `content`, `hint`, `owner`, `side`, `rect`. The document as a whole
/// matches any request or reply.
///
/// It describes the wire's shape, as `parse` reads it; what [`apply`](crate::apply) then
/// refuses (no anchor, nothing to show, a host's limits) is semantic, and not in it.
pub fn schema() -> Value {
    let u16 = json!({"type": "integer", "minimum": 0, "maximum": 65535});
    let u32 = json!({"type": "integer", "minimum": 0, "maximum": 4_294_967_295u64});
    let u64 = json!({"type": "integer", "minimum": 0});
    let i16 = json!({"type": "integer", "minimum": -32768, "maximum": 32767});
    let scope = json!({"$ref": "#/$defs/view"});
    // A request: its `op`, the host's routing (`id`, `view`: anything), the actor, and its
    // own fields.
    let request = |op: &str, mut props: Value, required: &[&str]| {
        let p = props.as_object_mut().expect("properties");
        p.insert("op".into(), json!({"const": op}));
        p.insert(
            "id".into(),
            json!({"description": "The host's request id; any JSON, ignored here."}),
        );
        p.insert(
            "view".into(),
            json!({"description": "The host's routing (which screen); any JSON, ignored here."}),
        );
        p.insert(
            "actor".into(),
            json!({"type": ["string", "null"], "description": "The agent the request comes from; none: the host's or the person's own."}),
        );
        let mut req = vec!["op"];
        req.extend(required);
        json!({"type": "object", "properties": props, "required": req, "additionalProperties": false})
    };
    let given = |field: &str, s: Value| json!({"required": [field], "properties": {field: s}});
    let layer_given = given("layer", json!({"type": "string"}));
    let owner_given = given("owner", json!({"$ref": "#/$defs/owner"}));
    let all_true = given("all", json!({"const": true}));
    let only =
        |yes: &Value, no: [&Value; 2]| json!({"allOf": [yes, {"not": no[0]}, {"not": no[1]}]});
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://caretline.app/schema/layers-ops.json",
        "title": "caretline-layers ops",
        "description": "Requests a host's protocol passes to caretline_layers::ops::parse, and the replies ops::reply, ops::list and ops::error give.",
        "anyOf": [
            {"$ref": "#/$defs/request"},
            {"$ref": "#/$defs/reply"},
            {"$ref": "#/$defs/list"},
            {"$ref": "#/$defs/error"}
        ],
        "$defs": {
            "rect": {
                "type": "object",
                "properties": {"x": u16, "y": u16, "w": u16, "h": u16},
                "required": ["x", "y", "w", "h"],
                "additionalProperties": false
            },
            "side": {"enum": ["below", "above", "right", "left"]},
            "head": {
                "enum": ["any", "on_anchor_rows"],
                "description": "Where an arrow's head may end: any clear cell beside the anchor (the default), or only on the anchor's own rows."
            },
            "view": {"type": "string", "minLength": 1, "description": "A view's id, as the host names it (FrameResolver::id)."},
            "owner": {
                "anyOf": [
                    {"enum": ["person", "host", "guide"]},
                    {"type": "string", "pattern": "^agent:.+"}
                ]
            },
            "anchor": {
                "description": "What a layer points at: one target key, and for text, block and caret anchors an optional `in`, the view it resolves in.",
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "text": {
                                "type": "object",
                                "properties": {"block": u64, "from": u64, "to": u64},
                                "required": ["from", "to"],
                                "additionalProperties": false
                            },
                            "in": scope
                        },
                        "required": ["text"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {"block": u64, "in": scope},
                        "required": ["block"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {"caret": {"const": true}, "in": scope},
                        "required": ["caret"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {"screen": {"enum": ["center", "top", "bottom"]}},
                        "required": ["screen"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "host": {
                                "type": "object",
                                "properties": {"kind": {"type": "string"}, "key": {"type": "string"}},
                                "required": ["kind", "key"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["host"],
                        "additionalProperties": false
                    }
                ]
            },
            "anchors": {
                "description": "One anchor, or fallbacks in order.",
                "oneOf": [
                    {"$ref": "#/$defs/anchor"},
                    {"type": "array", "items": {"$ref": "#/$defs/anchor"}}
                ]
            },
            "content": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string"},
                    "data": {"description": "The kind's data, opaque here; a `hint`'s is #/$defs/hint."}
                },
                "required": ["kind"],
                "additionalProperties": false
            },
            "hint": {
                "type": "object",
                "properties": {"title": {"type": ["string", "null"]}, "text": {"type": "string"}},
                "additionalProperties": false
            },
            "layer": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "owner": {"$ref": "#/$defs/owner"},
                    "z": i16,
                    "since_ms": u64,
                    "ttl_ms": {"anyOf": [u64, {"type": "null"}]},
                    "anchor": {"type": "array", "items": {"$ref": "#/$defs/anchor"}},
                    "avoid": {"type": "array", "items": {"$ref": "#/$defs/anchor"}},
                    "content": {"anyOf": [{"$ref": "#/$defs/content"}, {"type": "null"}]},
                    "arrow": {"type": "boolean"},
                    "ring": {
                        "anyOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "pulse": {
                                        "anyOf": [
                                            {
                                                "type": "object",
                                                "properties": {"period_ms": u32, "cycles": u16},
                                                "required": ["period_ms", "cycles"],
                                                "additionalProperties": false
                                            },
                                            {"type": "null"}
                                        ]
                                    }
                                },
                                "additionalProperties": false
                            },
                            {"type": "null"}
                        ]
                    },
                    "spotlight": {
                        "anyOf": [
                            {
                                "type": "object",
                                "properties": {"holes": {"type": "array", "items": {"enum": ["anchor", "box"]}}},
                                "additionalProperties": false
                            },
                            {"type": "null"}
                        ]
                    },
                    "capture": {"type": "boolean"},
                    "hide_off_screen": {"type": "boolean"},
                    "place": {"type": "array", "items": {"$ref": "#/$defs/side"}},
                    "max_width": {"anyOf": [u16, {"type": "null"}]},
                    "head": {"$ref": "#/$defs/head"}
                },
                "required": ["anchor"],
                "additionalProperties": false
            },
            "request": {
                "oneOf": [
                    {"$ref": "#/$defs/hint.show"},
                    {"$ref": "#/$defs/hint.hide"},
                    {"$ref": "#/$defs/layer.push"},
                    {"$ref": "#/$defs/layer.update"},
                    {"$ref": "#/$defs/layer.pop"},
                    {"$ref": "#/$defs/layer.list"}
                ]
            },
            "hint.show": request("hint.show", json!({
                "anchor": {"$ref": "#/$defs/anchors"},
                "text": {"type": "string"},
                "title": {"type": ["string", "null"]},
                "ttl_ms": {"anyOf": [u64, {"type": "null"}]},
                "place": {"type": "array", "items": {"$ref": "#/$defs/side"}},
                "arrow": {"type": "boolean", "description": "Default true."},
                "ring": {"type": "boolean", "description": "Default true."},
                "avoid": {"$ref": "#/$defs/anchors"},
                "head": {"$ref": "#/$defs/head"}
            }), &["anchor", "text"]),
            "layer.push": request("layer.push", json!({"layer": {"$ref": "#/$defs/layer"}}), &["layer"]),
            "layer.update": request("layer.update", json!({"layer": {"$ref": "#/$defs/layer"}}), &["layer"]),
            "hint.hide": {
                "allOf": [
                    request("hint.hide", json!({
                        "layer": {"type": ["string", "null"]},
                        "owner": {"type": "null"},
                        "all": {"type": "boolean"}
                    }), &[]),
                    {"anyOf": [only(&layer_given, [&owner_given, &all_true]), only(&all_true, [&layer_given, &owner_given])]}
                ],
                "description": "One of `layer` or `all: true`."
            },
            "layer.pop": {
                "allOf": [
                    request("layer.pop", json!({
                        "layer": {"type": ["string", "null"]},
                        "owner": {"anyOf": [{"$ref": "#/$defs/owner"}, {"type": "null"}]},
                        "all": {"type": "boolean"}
                    }), &[]),
                    {"anyOf": [
                        only(&layer_given, [&owner_given, &all_true]),
                        only(&owner_given, [&layer_given, &all_true]),
                        only(&all_true, [&layer_given, &owner_given])
                    ]}
                ],
                "description": "One of `layer`, `owner` or `all: true`."
            },
            "layer.list": request("layer.list", json!({}), &[]),
            "resolved": {
                "description": "Where a layer's anchor landed: its cells, or which way it lies; `in`, the view it resolved in. Null: no anchor resolved.",
                "oneOf": [
                    {"type": "null"},
                    {
                        "type": "object",
                        "properties": {
                            "rects": {"type": "array", "items": {"$ref": "#/$defs/rect"}, "minItems": 1},
                            "in": scope
                        },
                        "required": ["rects"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "off": {"enum": ["above", "below", "left", "right"]},
                            "in": scope
                        },
                        "required": ["off"],
                        "additionalProperties": false
                    }
                ]
            },
            "reply": {
                "description": "An applied op: the layer pushed or updated and where it resolved (`reason: not_found` when nowhere), and the layers it popped.",
                "type": "object",
                "properties": {
                    "layer": {"type": "string"},
                    "resolved": {"$ref": "#/$defs/resolved"},
                    "reason": {"const": "not_found"},
                    "popped": {"type": "array", "items": {"type": "string"}}
                },
                "additionalProperties": false
            },
            "list": {
                "type": "object",
                "properties": {
                    "layers": {"type": "array", "items": {"$ref": "#/$defs/layer"}},
                    "hidden": {"type": "boolean"}
                },
                "required": ["layers", "hidden"],
                "additionalProperties": false
            },
            "error": {
                "type": "object",
                "properties": {
                    "error": {
                        "type": "object",
                        "properties": {
                            "reason": {"enum": [
                                "rate_limited", "dim_not_allowed", "capture_not_allowed",
                                "too_long", "not_found", "not_allowed", "too_many", "invalid"
                            ]},
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

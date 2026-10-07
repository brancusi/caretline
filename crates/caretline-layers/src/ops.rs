//! Protocol-neutral requests and replies (design §5.3): a host's own JSON protocol, or
//! caretline's, parses agents' requests with [`parse`], applies the op with [`apply`](crate::apply),
//! and answers with [`reply`]. Pure: JSON in, JSON out.
//!
//! | Op | Request fields | Becomes |
//! |---|---|---|
//! | `hint.show` | `anchor` (one or a list), `text`, `title?`, `ttl_ms?`, `place?`, `arrow?` (default true), `ring?` (default true), `actor?` | a push of a [`HINT`] layer |
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
    Anchor, Applied, Content, Layer, LayerOp, Layers, Owner, Reason, Refusal, Selector,
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

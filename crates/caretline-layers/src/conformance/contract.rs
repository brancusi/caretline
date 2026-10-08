//! A contract for a host's own protocol bridge: canonical `hint.*` and `layer.*` requests,
//! the reply each should get, and [`run`], which sends them through the host's bridge (a
//! closure, request in, reply out), validates every reply against
//! [`ops::schema`](crate::ops::schema), and checks what it means: where a layer's anchor
//! resolved against what the host's resolver says, `null` with `reason: not_found` for an
//! anchor found nowhere, what a pop popped, and a refusal's reason for malformed input. A host
//! that passes it can be driven by an agent the same way as any other.
//!
//! The bridge gets the request as the protocol carries it (`op` beside the fields) and
//! returns the reply as [`ops::reply`](crate::ops::reply), [`ops::list`](crate::ops::list) or
//! [`ops::error`](crate::ops::error) give it: a host whose protocol wraps replies (an `id`, a
//! `result`) unwraps them in the closure. It applies the op, plans the frame and answers with
//! that plan, as a live request would.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Kind, Violation, validate};
use crate::model::{Anchor, Reason};
use crate::resolve::{AnchorKey, Off, Resolve};

/// What the contract needs from the host's screen this frame.
pub struct Fixture<'a> {
    /// A host anchor on screen (a table row the host recorded in its `AnchorMap`).
    pub row: AnchorKey,
    /// A text anchor scoped to one of the host's views and on screen there
    /// (`Anchor::scoped`), for a host that shows caretline views.
    pub text: Option<Anchor>,
    /// The resolver the host plans with this frame: what the replies' `resolved` must match.
    pub anchors: &'a dyn Resolve,
    /// The actor the requests name; `None` (the default) sends them as the host's own, so a
    /// host's agent limits don't refuse them.
    pub actor: Option<String>,
}

impl<'a> Fixture<'a> {
    pub fn new(row: AnchorKey, anchors: &'a dyn Resolve) -> Fixture<'a> {
        Fixture {
            row,
            text: None,
            anchors,
            actor: None,
        }
    }

    pub fn with_text(mut self, text: Anchor) -> Fixture<'a> {
        self.text = Some(text);
        self
    }

    pub fn with_actor(mut self, actor: &str) -> Fixture<'a> {
        self.actor = Some(actor.into());
        self
    }

    fn row_anchor(&self) -> Anchor {
        Anchor::Host {
            kind: self.row.kind.clone(),
            key: self.row.key.clone(),
        }
    }

    fn missing(&self) -> Anchor {
        Anchor::Host {
            kind: self.row.kind.clone(),
            key: "conformance:missing".into(),
        }
    }
}

/// The reply a canonical request should get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expect {
    /// A `reply` with the new layer's id, resolved where the host's resolver puts `anchor`.
    Shown { anchor: Anchor },
    /// A `reply` with the new layer's id, `resolved: null` and `reason: not_found`.
    NotFound,
    /// A `reply` naming this layer.
    Layer { id: String },
    /// A `list` holding these layers.
    Listed { ids: Vec<String> },
    /// A `reply` that popped this layer.
    Popped { id: String },
    /// An `error` with this reason.
    Refused { reason: Reason },
}

/// One canonical request and what it should get.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub name: String,
    pub request: Value,
    pub expect: Expect,
}

/// The id `layer.push` uses.
pub const PUSHED: &str = "conformance:push";

/// The canonical requests, in the order [`run`] sends them, for the host's screen.
pub fn cases(fx: &Fixture<'_>) -> Vec<Case> {
    let row = fx.row_anchor();
    let missing = fx.missing();
    let mut out = Vec::new();
    let mut case = |name: &str, mut request: Value, expect: Expect| {
        if let (Some(a), Some(m)) = (&fx.actor, request.as_object_mut()) {
            m.insert("actor".into(), json!(a));
        }
        out.push(Case {
            name: name.into(),
            request,
            expect,
        });
    };
    let hint = |text: &str| json!({"kind": "hint", "data": {"text": text}});
    case(
        "hint.show at a host row",
        json!({"op": "hint.show", "anchor": row, "title": "Conformance", "text": "A hint at a row."}),
        Expect::Shown {
            anchor: row.clone(),
        },
    );
    if let Some(t) = &fx.text {
        case(
            "hint.show at a scoped text anchor",
            json!({"op": "hint.show", "anchor": t, "text": "A hint in a view."}),
            Expect::Shown { anchor: t.clone() },
        );
    }
    case(
        "hint.show with avoid",
        json!({"op": "hint.show", "anchor": row, "avoid": [fx.text.clone().unwrap_or(row.clone())],
               "text": "A hint that keeps off what it explains.", "arrow": false}),
        Expect::Shown {
            anchor: row.clone(),
        },
    );
    case(
        "hint.show at an anchor found nowhere",
        json!({"op": "hint.show", "anchor": missing, "text": "Nowhere."}),
        Expect::NotFound,
    );
    case(
        "hint.show falls back to the next anchor",
        json!({"op": "hint.show", "anchor": [missing, row], "text": "The second anchor."}),
        Expect::Shown {
            anchor: row.clone(),
        },
    );
    case(
        "layer.push",
        json!({"op": "layer.push", "layer": {"id": PUSHED, "anchor": [row], "content": hint("Pushed."), "ring": {}}}),
        Expect::Layer { id: PUSHED.into() },
    );
    case(
        "layer.update",
        json!({"op": "layer.update", "layer": {"id": PUSHED, "anchor": [row], "content": hint("Updated.")}}),
        Expect::Layer { id: PUSHED.into() },
    );
    case(
        "layer.list",
        json!({"op": "layer.list"}),
        Expect::Listed {
            ids: vec![PUSHED.into()],
        },
    );
    case(
        "layer.pop",
        json!({"op": "layer.pop", "layer": PUSHED}),
        Expect::Popped { id: PUSHED.into() },
    );
    case(
        "layer.pop of a layer gone",
        json!({"op": "layer.pop", "layer": PUSHED}),
        Expect::Refused {
            reason: Reason::NotFound,
        },
    );
    case(
        "layer.update of no layer",
        json!({"op": "layer.update", "layer": {"id": "conformance:none", "anchor": [row], "content": hint("x")}}),
        Expect::Refused {
            reason: Reason::NotFound,
        },
    );
    let invalid = Expect::Refused {
        reason: Reason::Invalid,
    };
    for (name, req) in [
        (
            "hint.show without text",
            json!({"op": "hint.show", "anchor": row}),
        ),
        (
            "hint.show at screen cells",
            json!({"op": "hint.show", "anchor": {"cells": {"x": 1, "y": 1, "w": 1, "h": 1}}, "text": "x"}),
        ),
        (
            "hint.show with an unknown field",
            json!({"op": "hint.show", "anchor": row, "text": "x", "colour": "red"}),
        ),
        (
            "hint.show with a scoped screen anchor",
            json!({"op": "hint.show", "anchor": {"screen": "center", "in": "main"}, "text": "x"}),
        ),
        (
            "hint.hide with neither layer nor all",
            json!({"op": "hint.hide"}),
        ),
        (
            "layer.pop with a layer and all",
            json!({"op": "layer.pop", "layer": PUSHED, "all": true}),
        ),
        (
            "layer.push with nothing to show",
            json!({"op": "layer.push", "layer": {"anchor": [row]}}),
        ),
        ("an unknown op", json!({"op": "layer.frobnicate"})),
    ] {
        case(name, req, invalid.clone());
    }
    out
}

fn off_name(o: &Off) -> &'static str {
    match o {
        Off::Above { .. } => "above",
        Off::Below { .. } => "below",
        Off::Left { .. } => "left",
        Off::Right { .. } => "right",
    }
}

/// Sends every case through `bridge` and checks the replies; then hides the layers the cases
/// showed (checking those replies too), so the host's layers end as they began. No violation
/// is a pass.
///
/// ```ignore
/// let fixture = Fixture::new(AnchorKey::host("row", "r-104"), &anchors);
/// let v = contract::run(&mut |req| my_host.handle(req), &fixture);
/// assert!(v.is_empty(), "{v:#?}");
/// ```
pub fn run(bridge: &mut dyn FnMut(&Value) -> Value, fx: &Fixture<'_>) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut shown = Vec::new();
    for c in cases(fx) {
        let reply = bridge(&c.request);
        let mut bad = |k: Kind, d: String| {
            out.push(Violation::new(
                None,
                k,
                format!("{}: {d}; got {reply}", c.name),
            ))
        };
        let def = match c.expect {
            Expect::Listed { .. } => "list",
            Expect::Refused { .. } => "error",
            _ => "reply",
        };
        if let Err(e) = validate(def, &reply) {
            bad(Kind::Schema, format!("not a {def}: {e}"));
            continue;
        }
        let layer = reply["layer"].as_str().map(String::from);
        match &c.expect {
            Expect::Shown { anchor } => {
                let Some(id) = layer else {
                    bad(Kind::Contract, "no layer id".into());
                    continue;
                };
                shown.push(id);
                let got = &reply["resolved"];
                match fx.anchors.resolve(anchor) {
                    None => bad(
                        Kind::Contract,
                        "the fixture's anchor doesn't resolve in the host's resolver".into(),
                    ),
                    Some(want) if !want.rects.is_empty() => {
                        let rects: Vec<crate::Rect> =
                            serde_json::from_value(got["rects"].clone()).unwrap_or_default();
                        // Each answered rect is one the resolver gave (clipped to the screen).
                        let ok = !rects.is_empty()
                            && rects.len() <= want.rects.len()
                            && rects.iter().all(|r| {
                                want.rects
                                    .iter()
                                    .any(|w| *w == *r || w.intersection(r) == *r)
                            });
                        if !ok {
                            bad(
                                Kind::Contract,
                                format!("resolved should be the resolver's rects {:?}", want.rects),
                            );
                        }
                        if got.get("in").and_then(Value::as_str) != want.view.as_deref() {
                            bad(
                                Kind::Contract,
                                format!("resolved \"in\" should be {:?}", want.view),
                            );
                        }
                    }
                    Some(want) => {
                        let off = want.off.as_ref().map(off_name);
                        if got.get("off").and_then(Value::as_str) != off {
                            bad(Kind::Contract, format!("resolved should be off {off:?}"));
                        }
                    }
                }
            }
            Expect::NotFound => {
                if let Some(id) = layer {
                    shown.push(id);
                } else {
                    bad(Kind::Contract, "no layer id".into());
                }
                if !reply["resolved"].is_null() || reply["reason"] != json!("not_found") {
                    bad(
                        Kind::Contract,
                        "resolved should be null with reason not_found".into(),
                    );
                }
            }
            Expect::Layer { id } => {
                if layer.as_deref() != Some(id) {
                    bad(Kind::Contract, format!("layer should be {id:?}"));
                }
            }
            Expect::Listed { ids } => {
                let listed: Vec<&str> = reply["layers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l["id"].as_str())
                    .collect();
                for id in ids.iter().chain(&shown) {
                    if !listed.contains(&id.as_str()) {
                        bad(Kind::Contract, format!("{id:?} isn't listed"));
                    }
                }
            }
            Expect::Popped { id } => {
                if !reply["popped"]
                    .as_array()
                    .is_some_and(|p| p.contains(&json!(id)))
                {
                    bad(Kind::Contract, format!("should have popped {id:?}"));
                }
            }
            Expect::Refused { reason } => {
                if reply["error"]["reason"] != json!(reason) {
                    bad(
                        Kind::Contract,
                        format!("should be refused as {}", json!(reason)),
                    );
                }
            }
        }
    }
    for id in shown {
        let mut req = json!({"op": "hint.hide", "layer": id});
        if let Some(a) = &fx.actor {
            req["actor"] = json!(a);
        }
        let reply = bridge(&req);
        if let Err(e) = validate("reply", &reply) {
            out.push(Violation::new(
                Some(&id),
                Kind::Schema,
                format!("hint.hide: not a reply: {e}; got {reply}"),
            ));
        } else if !reply["popped"]
            .as_array()
            .is_some_and(|p| p.contains(&json!(id)))
        {
            out.push(Violation::new(
                Some(&id),
                Kind::Contract,
                format!("hint.hide should have popped it; got {reply}"),
            ));
        }
    }
    out
}

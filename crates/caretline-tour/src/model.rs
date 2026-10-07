//! The authoring format: a [`Tour`] of [`Step`]s, each with layers, narration and the host's
//! own patch, read strictly from TOML or JSON.

use caretline_layers::{Anchor, Ring, Side, Spotlight};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::pred::Pred;

/// The `goto` that ends a walkthrough.
pub const END: &str = "end";

/// Step ids that mean something else in `tour.step {to}` and `goto`.
pub const RESERVED: &[&str] = &["next", "back", "end"];

/// The content kind every host renders, and the default for a tour that names none.
pub const DEFAULT_KIND: &str = caretline_layers::HINT;

fn is_false(b: &bool) -> bool {
    !*b
}

fn one() -> u32 {
    1
}

fn default_kind() -> String {
    DEFAULT_KIND.into()
}

/// A walkthrough: `{id, version, title, kind, step[]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tour {
    pub id: String,
    /// Bumped when the walkthrough changes enough to offer it again ([`Tour::reoffer`]).
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub title: String,
    /// The content kind of a layer that names none (default `hint`).
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Offer this version to someone who stopped or finished an earlier one
    /// ([`TourState::offer`](crate::TourState::offer)).
    #[serde(default, skip_serializing_if = "is_false")]
    pub reoffer: bool,
    /// The steps, in order. Wire name `step` (TOML's `[[step]]`); `steps` is read too.
    #[serde(rename = "step", alias = "steps", default)]
    pub steps: Vec<Step>,
}

impl Tour {
    /// The index of the step with this id.
    pub fn index(&self, id: &str) -> Option<usize> {
        self.steps.iter().position(|s| s.id == id)
    }

    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// Replaces every `find` anchor with what `find(text, view)` answers, once (the host
    /// searches its own document, before `tour.start`, so the started tour and every trace of
    /// it hold stable anchors). A find with no answer stays, and is left out when the step's
    /// layers are made. Returns the texts not found, as `(step id, layer id, text)`.
    pub fn resolve_finds(
        &mut self,
        mut find: impl FnMut(&str, Option<&str>) -> Option<Anchor>,
    ) -> Vec<(String, String, String)> {
        let mut missing = Vec::new();
        for s in &mut self.steps {
            for l in &mut s.layers {
                for a in &mut l.anchor {
                    if let StepAnchor::Find { text, view } = a {
                        match find(text, view.as_deref()) {
                            Some(found) => {
                                *a = StepAnchor::At(match view {
                                    Some(v) => Anchor::scoped(v, found),
                                    None => found,
                                })
                            }
                            None => missing.push((s.id.clone(), l.id.clone(), text.clone())),
                        }
                    }
                }
            }
        }
        missing
    }

    /// Whether any `find` anchor is still unresolved.
    pub fn has_finds(&self) -> bool {
        self.steps.iter().any(|s| {
            s.layers.iter().any(|l| {
                l.anchor
                    .iter()
                    .any(|a| matches!(a, StepAnchor::Find { .. }))
            })
        })
    }
}

/// One step: the host's scene (`host`), what it points at (`layers`), and what it says
/// (`narration`), with the predicates that move on from it.
///
/// On the wire a step has either `layers` or the single-layer shorthand (`anchor`, `kind`,
/// `data`, `place`, `capture` on the step itself), which reads as `layers[0]`; both is an
/// error. A layer's id defaults to `<step id>/<index>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StepWire", into = "StepWire")]
pub struct Step {
    pub id: String,
    pub layers: Vec<StepLayer>,
    /// The step's own words: not a layer, nothing placed. The host shows it where it likes.
    pub narration: Option<Narration>,
    /// The host's patch for this step (which panel is open, what is focused). Opaque: the
    /// tour hands it over on entering the step and never reads it.
    pub host: Option<Value>,
    /// Moves on when it holds ([`observe`](crate::observe)). A step without one moves only by
    /// ops: that is how pages are turned.
    pub advance: Option<Pred>,
    /// Passed over, going forward, when it holds on arrival.
    pub skip_if: Option<Pred>,
    /// Merged into each layer's data once the person has lingered `after_ms`.
    pub nudge: Option<Nudge>,
    /// Where going forward leads: the first branch whose `if` holds (or that has none). With
    /// none matching, the next step in order; after the last, the end.
    pub next: Vec<Branch>,
}

impl Step {
    /// A step with only an id.
    pub fn new(id: &str) -> Step {
        Step {
            id: id.into(),
            layers: Vec::new(),
            narration: None,
            host: None,
            advance: None,
            skip_if: None,
            nudge: None,
            next: Vec::new(),
        }
    }

    /// The anchors a predicate names: `"anchor"` is the first layer's, anything else a
    /// layer's id. Only resolved anchors (not `find`).
    pub fn anchors(&self, name: &str) -> Option<Vec<Anchor>> {
        let l = if name == "anchor" {
            self.layers.first()
        } else {
            self.layers.iter().find(|l| l.id == name)
        }?;
        Some(
            l.anchor
                .iter()
                .filter_map(StepAnchor::anchor)
                .cloned()
                .collect(),
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepWire {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layers: Option<Vec<StepLayer>>,
    #[serde(
        default,
        deserialize_with = "some_anchors",
        skip_serializing_if = "Option::is_none"
    )]
    anchor: Option<Vec<StepAnchor>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    place: Option<Place>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capture: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    narration: Option<Narration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    advance: Option<Pred>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skip_if: Option<Pred>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nudge: Option<Nudge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    next: Vec<Branch>,
}

impl TryFrom<StepWire> for Step {
    type Error = String;

    fn try_from(w: StepWire) -> Result<Step, String> {
        let short = w.anchor.is_some()
            || w.kind.is_some()
            || w.data.is_some()
            || w.place.is_some()
            || w.capture.is_some();
        let mut layers = match (w.layers, short) {
            (Some(_), true) => {
                return Err(format!(
                    "step {:?}: `layers` or the single-layer fields (anchor, kind, data, place, capture), not both",
                    w.id
                ));
            }
            (Some(l), false) => l,
            (None, true) => {
                let Some(anchor) = w.anchor else {
                    return Err(format!(
                        "step {:?}: the single-layer fields need an `anchor`",
                        w.id
                    ));
                };
                vec![StepLayer {
                    id: String::new(),
                    anchor,
                    kind: w.kind,
                    data: w.data.unwrap_or(Value::Null),
                    place: w.place.unwrap_or_default(),
                    capture: w.capture.unwrap_or(false),
                }]
            }
            (None, false) => Vec::new(),
        };
        for (i, l) in layers.iter_mut().enumerate() {
            if l.id.is_empty() {
                l.id = format!("{}/{i}", w.id);
            }
        }
        Ok(Step {
            id: w.id,
            layers,
            narration: w.narration,
            host: w.host.filter(|h| !h.is_null()),
            advance: w.advance,
            skip_if: w.skip_if,
            nudge: w.nudge,
            next: w.next,
        })
    }
}

impl From<Step> for StepWire {
    fn from(s: Step) -> StepWire {
        StepWire {
            id: s.id,
            layers: (!s.layers.is_empty()).then_some(s.layers),
            anchor: None,
            kind: None,
            data: None,
            place: None,
            capture: None,
            narration: s.narration,
            host: s.host,
            advance: s.advance,
            skip_if: s.skip_if,
            nudge: s.nudge,
            next: s.next,
        }
    }
}

/// One layer of a step. The tour owns it (owner `guide`); `kind` defaults to the tour's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepLayer {
    /// Defaults to `<step id>/<index>`.
    #[serde(default)]
    pub id: String,
    /// One anchor, or fallbacks in order.
    #[serde(deserialize_with = "anchors")]
    pub anchor: Vec<StepAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// What the layer's box shows, for the kind's renderer. Opaque.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
    #[serde(default, skip_serializing_if = "Place::is_default")]
    pub place: Place,
    /// A modal step (the host routes keys to the walkthrough).
    #[serde(default, skip_serializing_if = "is_false")]
    pub capture: bool,
}

/// A layer's anchor as authored: any [`Anchor`], or `{find = "text", in? = "view"}`, a text
/// the host searches for once ([`Tour::resolve_finds`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAnchor {
    At(Anchor),
    Find { text: String, view: Option<String> },
}

impl StepAnchor {
    /// The anchor, once resolved.
    pub fn anchor(&self) -> Option<&Anchor> {
        match self {
            StepAnchor::At(a) => Some(a),
            StepAnchor::Find { .. } => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindWire {
    find: String,
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    view: Option<String>,
}

impl Serialize for StepAnchor {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            StepAnchor::At(a) => a.serialize(s),
            StepAnchor::Find { text, view } => FindWire {
                find: text.clone(),
                view: view.clone(),
            }
            .serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for StepAnchor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<StepAnchor, D::Error> {
        use serde::de::Error;
        let v = Value::deserialize(d)?;
        if v.get("find").is_some() {
            let f: FindWire = serde_json::from_value(v).map_err(D::Error::custom)?;
            if f.find.is_empty() {
                return Err(D::Error::custom("`find` needs a text"));
            }
            if f.view.as_deref() == Some("") {
                return Err(D::Error::custom("a view id (`in`) can't be empty"));
            }
            return Ok(StepAnchor::Find {
                text: f.find,
                view: f.view,
            });
        }
        serde_json::from_value(v)
            .map(StepAnchor::At)
            .map_err(D::Error::custom)
    }
}

fn anchors<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<StepAnchor>, D::Error> {
    use serde::de::Error;
    match Value::deserialize(d)? {
        Value::Array(v) => v
            .into_iter()
            .map(|a| serde_json::from_value(a).map_err(D::Error::custom))
            .collect(),
        v => Ok(vec![serde_json::from_value(v).map_err(D::Error::custom)?]),
    }
}

fn some_anchors<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<StepAnchor>>, D::Error> {
    anchors(d).map(Some)
}

/// The step's own words, `{title?, text}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Narration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
}

/// A layer's placement request: the sides to try, a width cap, an arrow, a ring, a
/// spotlight, and whether to show only the edge chip off screen.
///
/// On the wire, an object (`{sides, max_width, arrow, ring, spotlight, hide_off_screen}`;
/// `max_w` and `connector` are read as `max_width` and `arrow`; `ring` and `spotlight` take
/// `true` or their object), or a list of sides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    pub sides: Vec<Side>,
    pub max_width: Option<u16>,
    pub arrow: bool,
    pub ring: Option<Ring>,
    pub spotlight: Option<Spotlight>,
    pub hide_off_screen: bool,
}

impl Place {
    pub fn is_default(&self) -> bool {
        *self == Place::default()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Flag<T> {
    On(bool),
    With(T),
}

impl<T> Flag<T> {
    fn get(self, on: impl FnOnce() -> T) -> Option<T> {
        match self {
            Flag::On(true) => Some(on()),
            Flag::On(false) => None,
            Flag::With(t) => Some(t),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceWire {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sides: Vec<Side>,
    #[serde(default, alias = "max_w", skip_serializing_if = "Option::is_none")]
    max_width: Option<u16>,
    #[serde(default, alias = "connector", skip_serializing_if = "is_false")]
    arrow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ring: Option<Flag<Ring>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spotlight: Option<Flag<Spotlight>>,
    #[serde(default, skip_serializing_if = "is_false")]
    hide_off_screen: bool,
}

impl Serialize for Place {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        PlaceWire {
            sides: self.sides.clone(),
            max_width: self.max_width,
            arrow: self.arrow,
            ring: self.ring.clone().map(Flag::With),
            spotlight: self.spotlight.clone().map(Flag::With),
            hide_off_screen: self.hide_off_screen,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Place {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Place, D::Error> {
        use serde::de::Error;
        let v = Value::deserialize(d)?;
        if v.is_array() {
            let sides = serde_json::from_value(v).map_err(D::Error::custom)?;
            return Ok(Place {
                sides,
                ..Place::default()
            });
        }
        if let Some(m) = v.as_object() {
            for (k, x) in m {
                if matches!(k.as_str(), "ring" | "spotlight") && !(x.is_boolean() || x.is_object())
                {
                    return Err(D::Error::custom(format!(
                        "`place.{k}` is true, false or an object"
                    )));
                }
            }
        }
        let w: PlaceWire = serde_json::from_value(v).map_err(D::Error::custom)?;
        Ok(Place {
            sides: w.sides,
            max_width: w.max_width,
            arrow: w.arrow,
            ring: w.ring.and_then(|f| f.get(Ring::default)),
            spotlight: w.spotlight.and_then(|f| f.get(Spotlight::default)),
            hide_off_screen: w.hide_off_screen,
        })
    }
}

/// A branch of `next`: go to `goto` (a step id, or `end`) if `if` holds, or always.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub when: Option<Pred>,
    pub goto: String,
}

/// `{after_ms, data}`: merged into each layer's data once the step has lasted `after_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nudge {
    pub after_ms: u64,
    pub data: Value,
}

/// Reads a tour from TOML. Strict: an unknown field anywhere outside `data` and `host` is an
/// error.
pub fn parse_toml(src: &str) -> Result<Tour, String> {
    toml::from_str(src).map_err(|e| e.to_string())
}

/// Reads a tour from JSON. Strict, as [`parse_toml`].
pub fn parse_json(src: &str) -> Result<Tour, String> {
    serde_json::from_str(src).map_err(|e| e.to_string())
}

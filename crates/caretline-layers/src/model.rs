//! The model: layers as data, the ops that change them, and the policy `apply` enforces.
//!
//! Every change goes through [`apply`] (an op, the actor, `now_ms`, the limits), and expiry
//! through [`expire`] with `now_ms`. Nothing here reads a clock: a trace of ops replays to the
//! same layers.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::geom::Side;

/// A block's id: a caretline `MarkId`'s number.
pub type BlockId = u64;

/// Who a layer belongs to. It decides the z band, the styling and who may remove it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Owner {
    /// A hint the person pinned.
    Person,
    /// The host application.
    #[default]
    Host,
    /// A walkthrough.
    Guide,
    /// An agent, by name. Wire form `agent:<name>`.
    Agent(String),
}

impl Owner {
    /// The z band: `host` 0–9, `guide` 10–19, `agent` 20–29, `person` 30–39.
    pub fn band(&self) -> (i16, i16) {
        match self {
            Owner::Host => (0, 9),
            Owner::Guide => (10, 19),
            Owner::Agent(_) => (20, 29),
            Owner::Person => (30, 39),
        }
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Owner::Agent(_))
    }

    fn wire(&self) -> String {
        match self {
            Owner::Person => "person".into(),
            Owner::Host => "host".into(),
            Owner::Guide => "guide".into(),
            Owner::Agent(n) => format!("agent:{n}"),
        }
    }
}

impl Serialize for Owner {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.wire())
    }
}

impl<'de> Deserialize<'de> for Owner {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Owner, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "person" => Owner::Person,
            "host" => Owner::Host,
            "guide" => Owner::Guide,
            _ => match s.strip_prefix("agent:") {
                Some(n) if !n.is_empty() => Owner::Agent(n.to_string()),
                _ => {
                    return Err(serde::de::Error::custom(format!(
                        "unknown owner {s:?}: person, host, guide or agent:<name>"
                    )));
                }
            },
        })
    }
}

/// Where a screen-placed layer goes (an anchor with nothing to point at).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenPos {
    Center,
    Top,
    Bottom,
}

/// What a layer points at: a stable key (chars, a mark id, a host key; never a screen
/// position), resolved to cells every frame by a [`Resolve`](crate::Resolve).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// Document chars `from..to`. Wire: `{"text": {"from": 412, "to": 421}}`.
    Text { from: usize, to: usize },
    /// Chars `from..to` within a block, counted from its start. Wire:
    /// `{"text": {"block": 7, "from": 4, "to": 9}}`.
    BlockText {
        block: BlockId,
        from: usize,
        to: usize,
    },
    /// A whole block. Wire: `{"block": 7}`.
    Block(BlockId),
    /// The primary caret. Wire: `{"caret": true}`.
    Caret,
    /// No target: placement only. Wire: `{"screen": "center"}`.
    Screen(ScreenPos),
    /// A kind the host resolves (an [`AnchorMap`](crate::AnchorMap) key). Wire:
    /// `{"host": {"kind": "row", "key": "a1b2"}}`.
    Host { kind: String, key: String },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum AnchorWire {
    Text(TextWire),
    Block(BlockId),
    Caret(bool),
    Screen(ScreenPos),
    Host(HostWire),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    block: Option<BlockId>,
    from: usize,
    to: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostWire {
    kind: String,
    key: String,
}

impl Serialize for Anchor {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let w = match self.clone() {
            Anchor::Text { from, to } => AnchorWire::Text(TextWire {
                block: None,
                from,
                to,
            }),
            Anchor::BlockText { block, from, to } => AnchorWire::Text(TextWire {
                block: Some(block),
                from,
                to,
            }),
            Anchor::Block(b) => AnchorWire::Block(b),
            Anchor::Caret => AnchorWire::Caret(true),
            Anchor::Screen(p) => AnchorWire::Screen(p),
            Anchor::Host { kind, key } => AnchorWire::Host(HostWire { kind, key }),
        };
        w.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Anchor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Anchor, D::Error> {
        Ok(match AnchorWire::deserialize(d)? {
            AnchorWire::Text(TextWire {
                block: None,
                from,
                to,
            }) => Anchor::Text { from, to },
            AnchorWire::Text(TextWire {
                block: Some(block),
                from,
                to,
            }) => Anchor::BlockText { block, from, to },
            AnchorWire::Block(b) => Anchor::Block(b),
            AnchorWire::Caret(true) => Anchor::Caret,
            AnchorWire::Caret(false) => {
                return Err(serde::de::Error::custom("\"caret\" must be true"));
            }
            AnchorWire::Screen(p) => Anchor::Screen(p),
            AnchorWire::Host(HostWire { kind, key }) => Anchor::Host { kind, key },
        })
    }
}

/// The parts of a layer a spotlight leaves lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Anchor,
    Box,
}

/// The conventional content kind every host should render: a short titled text.
pub const HINT: &str = "hint";

/// What a layer shows in its box: a kind the host registered a renderer for, and that
/// renderer's data. Opaque to this crate, as mark payloads are to the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub kind: String,
    #[serde(default)]
    pub data: Value,
}

/// The data of a [`HINT`]: `{"title": "…", "text": "…"}`. The crate checks its size for
/// agents and titles it with the agent's name; drawing it is the host's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub text: String,
}

impl Content {
    pub fn new(kind: &str, data: Value) -> Content {
        Content {
            kind: kind.into(),
            data,
        }
    }

    /// A [`HINT`].
    pub fn hint(title: Option<&str>, text: &str) -> Content {
        let h = Hint {
            title: title.map(String::from),
            text: text.into(),
        };
        Content {
            kind: HINT.into(),
            data: serde_json::to_value(h).unwrap_or_default(),
        }
    }

    /// Its data as a [`Hint`], if it is one.
    pub fn as_hint(&self) -> Option<Hint> {
        if self.kind != HINT {
            return None;
        }
        serde_json::from_value(self.data.clone()).ok()
    }
}

/// Dims everything outside the holes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spotlight {
    #[serde(default = "default_holes")]
    pub holes: Vec<Part>,
}

impl Default for Spotlight {
    fn default() -> Spotlight {
        Spotlight {
            holes: default_holes(),
        }
    }
}

fn default_holes() -> Vec<Part> {
    vec![Part::Anchor, Part::Box]
}

/// A ring's pulse, for a renderer with a frame clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pulse {
    pub period_ms: u32,
    pub cycles: u16,
}

/// Marks the anchor's cells.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ring {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse: Option<Pulse>,
}

/// One layer: what it points at, the box it shows there, and the geometry around it (an
/// arrow from the box to the anchor, a ring, a spotlight).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// Given by [`apply`] when pushed empty.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub owner: Owner,
    /// Draw order, clamped into the owner's band ([`Owner::band`]).
    #[serde(default)]
    pub z: i16,
    /// Set by [`apply`] to the push's `now_ms`.
    #[serde(default)]
    pub since_ms: u64,
    /// How long it lasts from `since_ms`; `None` until popped (never for agents).
    #[serde(default)]
    pub ttl_ms: Option<u64>,
    /// Fallbacks, in order: the first that resolves is used.
    pub anchor: Vec<Anchor>,
    /// The box's content; none for a layer that only rings or dims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    /// An arrow from the box to the anchor.
    #[serde(default, skip_serializing_if = "is_false")]
    pub arrow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring: Option<Ring>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spotlight: Option<Spotlight>,
    /// A modal step (walkthroughs only; agents can't).
    #[serde(default, skip_serializing_if = "is_false")]
    pub capture: bool,
    /// Off-screen, show only the edge chip, not the box.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hide_off_screen: bool,
    /// The sides to try for the box, in order (default below, above, right, left).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub place: Vec<Side>,
    /// The widest the box may be (default [`MAX_WIDTH`](crate::MAX_WIDTH), never over
    /// two-thirds of the area).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u16>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Layer {
    /// A layer at `anchor`, owned by the host, showing nothing yet.
    pub fn new(anchor: Anchor) -> Layer {
        Layer {
            id: String::new(),
            owner: Owner::Host,
            z: 0,
            since_ms: 0,
            ttl_ms: None,
            anchor: vec![anchor],
            content: None,
            arrow: false,
            ring: None,
            spotlight: None,
            capture: false,
            hide_off_screen: false,
            place: Vec::new(),
            max_width: None,
        }
    }

    pub fn with_content(mut self, c: Content) -> Layer {
        self.content = Some(c);
        self
    }

    pub fn with_arrow(mut self) -> Layer {
        self.arrow = true;
        self
    }

    pub fn with_ring(mut self) -> Layer {
        self.ring = Some(Ring::default());
        self
    }

    pub fn with_spotlight(mut self) -> Layer {
        self.spotlight = Some(Spotlight::default());
        self
    }

    /// The sides to try, in order.
    pub fn sides(&self) -> Vec<Side> {
        if self.place.is_empty() {
            Side::DEFAULT.to_vec()
        } else {
            self.place.clone()
        }
    }

    fn expired(&self, now_ms: u64) -> bool {
        self.ttl_ms
            .is_some_and(|t| now_ms >= self.since_ms.saturating_add(t))
    }
}

/// Every layer of a view, plus what the policy needs to remember. Serializable, so it can live
/// in a view's state and replay.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// The next number for a generated id.
    #[serde(default)]
    pub next: u64,
    /// All hidden (`Toggle`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Recent pushes per agent (`now_ms` values within the last second), for the rate limit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub recent: BTreeMap<String, Vec<u64>>,
}

impl Layers {
    pub fn get(&self, id: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// The layers in draw order: by z, then by when they were pushed.
    pub fn in_order(&self) -> Vec<&Layer> {
        let mut v: Vec<&Layer> = self.layers.iter().collect();
        v.sort_by_key(|l| l.z);
        v
    }
}

/// Which layers a `Pop` removes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    Layer(String),
    Owner(Owner),
    /// Every layer the actor may remove.
    All(bool),
}

/// A change to [`Layers`]. Traces record these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerOp {
    /// Adds a layer (its id given if empty).
    Push(Layer),
    /// Replaces the layer with the same id, keeping its `since_ms`.
    Update(Layer),
    Pop(Selector),
    /// Removes the newest layer, whoever owns it (the person's dismiss).
    PopNewest,
    /// Hides or shows every layer.
    Toggle,
    /// Removes every layer.
    Clear,
}

/// Whether agents may dim the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDim {
    #[default]
    Never,
    Always,
}

/// The policy for agent layers. The defaults are the design's; a host may tighten them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Agent layers per view (at most [`Limits::MAX_AGENT_LAYERS`]).
    pub agent_layers: u8,
    pub ttl_default_ms: u64,
    pub ttl_min_ms: u64,
    pub ttl_max_ms: u64,
    /// Pushes (and updates) per actor per second.
    pub per_second: u8,
    pub body_chars: usize,
    pub body_lines: usize,
    pub title_chars: usize,
    /// The largest host content payload an agent may send, as JSON bytes.
    pub content_bytes: usize,
    pub agent_dim: AgentDim,
}

impl Limits {
    /// The most agent layers a host may allow.
    pub const MAX_AGENT_LAYERS: u8 = 8;

    /// The cap on agent layers, clamped to 1..=8.
    pub fn agent_cap(&self) -> usize {
        self.agent_layers.clamp(1, Self::MAX_AGENT_LAYERS) as usize
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            agent_layers: 3,
            ttl_default_ms: 8_000,
            ttl_min_ms: 1_000,
            ttl_max_ms: 60_000,
            per_second: 2,
            body_chars: 280,
            body_lines: 6,
            title_chars: 40,
            content_bytes: 1_024,
            agent_dim: AgentDim::Never,
        }
    }
}

/// Why an op was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// More than the allowed pushes per second.
    RateLimited,
    /// An agent asked to dim or spotlight without the person's consent.
    DimNotAllowed,
    /// An agent asked for a modal layer.
    CaptureNotAllowed,
    /// Content, a hint's text or its title over the size limits.
    TooLong,
    /// No layer with that id.
    NotFound,
    /// The actor may not touch that layer, or run that op.
    NotAllowed,
    /// The actor's agent layers are at the cap and none of them is its own to replace.
    TooMany,
    /// A malformed layer: no anchor, nothing to show, a duplicate id, an arrow with no box.
    Invalid,
}

/// A refused op: the reason (stable, for clients) and a sentence for people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub reason: Reason,
    pub detail: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}",
            serde_json::to_value(self.reason)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            self.detail
        )
    }
}

impl std::error::Error for Refusal {}

fn refuse<T>(reason: Reason, detail: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal {
        reason,
        detail: detail.into(),
    })
}

/// What [`apply`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Applied {
    /// The id of the layer pushed or updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Ids removed (popped, or replaced at the cap).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub popped: Vec<String>,
}

/// Applies an op. `actor` is `None` for the person or the host (local input), `Some(name)` for
/// an agent: then the policy in [`Limits`] holds and the layer is the agent's whatever it
/// claims. Pure: `now_ms` is the only time.
pub fn apply(
    layers: &mut Layers,
    op: LayerOp,
    actor: Option<&str>,
    now_ms: u64,
    limits: &Limits,
) -> Result<Applied, Refusal> {
    // Forget pushes older than a second, so the record stays small.
    for v in layers.recent.values_mut() {
        v.retain(|&t| t.saturating_add(1_000) > now_ms && t <= now_ms);
    }
    layers.recent.retain(|_, v| !v.is_empty());
    match op {
        LayerOp::Push(layer) => push(layers, layer, actor, now_ms, limits, false),
        LayerOp::Update(layer) => push(layers, layer, actor, now_ms, limits, true),
        LayerOp::Pop(sel) => {
            let may = |l: &Layer| match actor {
                None => true,
                Some(a) => l.owner == Owner::Agent(a.to_string()),
            };
            let picked: Vec<String> = match &sel {
                Selector::Layer(id) => {
                    let Some(l) = layers.get(id) else {
                        return refuse(Reason::NotFound, format!("no layer {id:?}"));
                    };
                    if !may(l) {
                        return refuse(
                            Reason::NotAllowed,
                            format!("layer {id:?} isn't this actor's"),
                        );
                    }
                    vec![id.clone()]
                }
                Selector::Owner(o) => {
                    if actor.is_some_and(|a| *o != Owner::Agent(a.to_string())) {
                        return refuse(Reason::NotAllowed, "an agent removes only its own layers");
                    }
                    layers
                        .layers
                        .iter()
                        .filter(|l| l.owner == *o)
                        .map(|l| l.id.clone())
                        .collect()
                }
                Selector::All(_) => layers
                    .layers
                    .iter()
                    .filter(|l| may(l))
                    .map(|l| l.id.clone())
                    .collect(),
            };
            layers.layers.retain(|l| !picked.contains(&l.id));
            Ok(Applied {
                layer: None,
                popped: picked,
            })
        }
        LayerOp::PopNewest | LayerOp::Toggle | LayerOp::Clear if actor.is_some() => refuse(
            Reason::NotAllowed,
            "dismissing, hiding and clearing are the person's",
        ),
        LayerOp::PopNewest => {
            // Newest by when it was pushed; on a tie, the later in the list.
            let i = (0..layers.layers.len()).max_by_key(|&i| (layers.layers[i].since_ms, i));
            let popped = i.map(|i| layers.layers.remove(i).id).into_iter().collect();
            Ok(Applied {
                layer: None,
                popped,
            })
        }
        LayerOp::Toggle => {
            layers.hidden = !layers.hidden;
            Ok(Applied::default())
        }
        LayerOp::Clear => {
            let popped = layers.layers.drain(..).map(|l| l.id).collect();
            Ok(Applied {
                layer: None,
                popped,
            })
        }
    }
}

fn push(
    layers: &mut Layers,
    mut layer: Layer,
    actor: Option<&str>,
    now_ms: u64,
    limits: &Limits,
    update: bool,
) -> Result<Applied, Refusal> {
    if layer.anchor.is_empty() {
        return refuse(Reason::Invalid, "a layer needs at least one anchor");
    }
    if layer.content.is_none() && layer.ring.is_none() && layer.spotlight.is_none() {
        return refuse(
            Reason::Invalid,
            "a layer needs content, a ring or a spotlight",
        );
    }
    if layer.arrow && layer.content.is_none() {
        return refuse(Reason::Invalid, "an arrow needs a box to start from");
    }
    if layer.content.as_ref().is_some_and(|c| c.kind.is_empty()) {
        return refuse(Reason::Invalid, "content needs a kind");
    }
    if layer
        .content
        .as_ref()
        .is_some_and(|c| c.kind == HINT && c.as_hint().is_none())
    {
        return refuse(Reason::Invalid, "a hint's data is {\"title\"?, \"text\"}");
    }
    let existing = if update {
        match layers.layers.iter().position(|l| l.id == layer.id) {
            Some(i) => Some(i),
            None => return refuse(Reason::NotFound, format!("no layer {:?}", layer.id)),
        }
    } else {
        if !layer.id.is_empty() && layers.get(&layer.id).is_some() {
            return refuse(
                Reason::Invalid,
                format!("a layer {:?} already exists", layer.id),
            );
        }
        None
    };
    let mut popped = Vec::new();
    if let Some(a) = actor {
        let me = Owner::Agent(a.to_string());
        if a.is_empty() || a.chars().count() > 32 || a.chars().any(char::is_control) {
            return refuse(
                Reason::Invalid,
                "an actor's name is 1 to 32 printable chars",
            );
        }
        if let Some(i) = existing
            && layers.layers[i].owner != me
        {
            return refuse(
                Reason::NotAllowed,
                format!("layer {:?} isn't this actor's", layer.id),
            );
        }
        if layer.capture {
            return refuse(Reason::CaptureNotAllowed, "agents can't capture input");
        }
        if layer.spotlight.is_some() && limits.agent_dim != AgentDim::Always {
            return refuse(
                Reason::DimNotAllowed,
                "agents can't dim the screen unless the person allows it",
            );
        }
        if let Some(c) = &layer.content {
            if c.kind.chars().count() > limits.title_chars
                || serde_json::to_string(&c.data).map_or(usize::MAX, |j| j.len())
                    > limits.content_bytes
            {
                return refuse(
                    Reason::TooLong,
                    format!("content is at most {} bytes of JSON", limits.content_bytes),
                );
            }
            if let Some(h) = c.as_hint() {
                if h.text.chars().count() > limits.body_chars
                    || h.text.split('\n').count() > limits.body_lines
                {
                    return refuse(
                        Reason::TooLong,
                        format!(
                            "a hint's text is at most {} chars and {} lines",
                            limits.body_chars, limits.body_lines
                        ),
                    );
                }
                if h.title
                    .as_ref()
                    .is_some_and(|t| t.chars().count() > limits.title_chars)
                {
                    return refuse(
                        Reason::TooLong,
                        format!("a title is at most {} chars", limits.title_chars),
                    );
                }
            }
        }
        let recent = layers.recent.get(a).map_or(0, Vec::len);
        if recent >= limits.per_second as usize {
            return refuse(
                Reason::RateLimited,
                format!("at most {} pushes a second", limits.per_second),
            );
        }
        if existing.is_none() {
            let agents: Vec<usize> = (0..layers.layers.len())
                .filter(|&i| layers.layers[i].owner.is_agent())
                .collect();
            if agents.len() >= limits.agent_cap() {
                // Replace this actor's oldest; never another agent's.
                let oldest = agents
                    .iter()
                    .copied()
                    .filter(|&i| layers.layers[i].owner == me)
                    .min_by_key(|&i| (layers.layers[i].since_ms, i));
                match oldest {
                    Some(i) => popped.push(layers.layers.remove(i).id),
                    None => {
                        return refuse(
                            Reason::TooMany,
                            format!("at most {} agent layers", limits.agent_cap()),
                        );
                    }
                }
            }
        }
        layer.owner = me;
        let ttl = layer.ttl_ms.unwrap_or(limits.ttl_default_ms);
        layer.ttl_ms = Some(ttl.clamp(limits.ttl_min_ms, limits.ttl_max_ms));
        // Attribution: an agent's hint is titled with its name.
        if let Some(c) = &mut layer.content
            && let Some(mut h) = c.as_hint()
        {
            h.title = Some(match h.title.take().filter(|t| !t.trim().is_empty()) {
                Some(t) => format!("◆ {a} · {t}"),
                None => format!("◆ {a}"),
            });
            c.data = serde_json::to_value(h).unwrap_or_default();
        }
        layers.recent.entry(a.to_string()).or_default().push(now_ms);
    }
    let (lo, hi) = layer.owner.band();
    layer.z = layer.z.clamp(lo, hi);
    if let Some(i) = existing {
        layer.since_ms = layers.layers[i].since_ms;
        let id = layer.id.clone();
        layers.layers[i] = layer;
        return Ok(Applied {
            layer: Some(id),
            popped,
        });
    }
    if layer.id.is_empty() {
        loop {
            layers.next += 1;
            let id = format!("L-{}", layers.next);
            if layers.get(&id).is_none() {
                layer.id = id;
                break;
            }
        }
    }
    layer.since_ms = now_ms;
    let id = layer.id.clone();
    layers.layers.push(layer);
    Ok(Applied {
        layer: Some(id),
        popped,
    })
}

/// Drops layers whose time is up at `now_ms`. Returns whether any went.
pub fn expire(layers: &mut Layers, now_ms: u64) -> bool {
    let n = layers.layers.len();
    layers.layers.retain(|l| !l.expired(now_ms));
    layers.layers.len() != n
}

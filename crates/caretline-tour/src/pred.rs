//! Predicates (`advance`, `skip_if`, a branch's `if`) and the host that answers them.

use std::collections::{BTreeMap, BTreeSet};

use caretline_layers::Anchor;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// When a step moves on, is passed over, or which branch is taken. Pure: it sees what the
/// [`TourHost`] answers for one message, the counts since the step began, and the time.
///
/// Wire form, one key per predicate (`count` beside `msg`, `command` or `event`):
///
/// | Wire | Holds when |
/// |---|---|
/// | `{msg = "insert_text", count? = 5}` | A message of that kind was applied, `count` times (default 1) since the step began |
/// | `{command = "move.word_right", count?}` | That action or command ran, `count` times |
/// | `{event = "name", count?}` (also `effect`, `host_effect`) | That host event happened, `count` times |
/// | `{state = {key, present?, match?}}` (also `ext`) | The host's state at `key` is present (default), absent (`present = false`), or contains `match` |
/// | `{caret_in = "anchor"}` | The caret is inside the step's first layer's anchor (or the layer with that id) |
/// | `{selection = "nonempty"}` | Some selection is non-empty |
/// | `{changed = "anchor"}` | The message changed text at that anchor |
/// | `{folded = "anchor"}` | The anchor's block is folded |
/// | `{after_ms = N}` | The step has lasted N ms (`now_ms` from the host's ticks) |
/// | `{any = […]}`, `{all = […]}`, `{not = {…}}` | Composition |
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PredWire", into = "PredWire")]
pub enum Pred {
    Msg { kind: String, count: u32 },
    Command { id: String, count: u32 },
    Event { name: String, count: u32 },
    State { key: String, test: StateTest },
    CaretIn(String),
    Selection,
    Changed(String),
    Folded(String),
    AfterMs(u64),
    Any(Vec<Pred>),
    All(Vec<Pred>),
    Not(Box<Pred>),
}

/// What [`Pred::State`] asks of the host's value at its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateTest {
    /// Present (`true`) or absent (`false`).
    Present(bool),
    /// Present and containing this subset ([`subset`]).
    Match(Value),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    present: Option<bool>,
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    matches: Option<Value>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PredWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    msg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(
        default,
        alias = "effect",
        alias = "host_effect",
        skip_serializing_if = "Option::is_none"
    )]
    event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    count: Option<u32>,
    #[serde(default, alias = "ext", skip_serializing_if = "Option::is_none")]
    state: Option<StateWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    caret_in: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    changed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folded: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    any: Option<Vec<Pred>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    all: Option<Vec<Pred>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    not: Option<Box<Pred>>,
}

impl TryFrom<PredWire> for Pred {
    type Error = String;

    fn try_from(w: PredWire) -> Result<Pred, String> {
        let count = w.count;
        let mut found: Vec<Pred> = Vec::new();
        let n = |c: Option<u32>| c.unwrap_or(1);
        if let Some(kind) = w.msg {
            found.push(Pred::Msg {
                kind,
                count: n(count),
            });
        }
        if let Some(id) = w.command {
            found.push(Pred::Command {
                id,
                count: n(count),
            });
        }
        if let Some(name) = w.event {
            found.push(Pred::Event {
                name,
                count: n(count),
            });
        }
        let counted = !found.is_empty();
        if let Some(s) = w.state {
            let test = match (s.present, s.matches) {
                (Some(_), Some(_)) => {
                    return Err("`state` takes `present` or `match`, not both".into());
                }
                (Some(p), None) => StateTest::Present(p),
                (None, Some(m)) => StateTest::Match(m),
                (None, None) => StateTest::Present(true),
            };
            found.push(Pred::State { key: s.key, test });
        }
        if let Some(a) = w.caret_in {
            found.push(Pred::CaretIn(a));
        }
        if let Some(s) = w.selection {
            if s != "nonempty" {
                return Err(format!("`selection` is \"nonempty\", not {s:?}"));
            }
            found.push(Pred::Selection);
        }
        if let Some(a) = w.changed {
            found.push(Pred::Changed(a));
        }
        if let Some(a) = w.folded {
            found.push(Pred::Folded(a));
        }
        if let Some(ms) = w.after_ms {
            found.push(Pred::AfterMs(ms));
        }
        if let Some(v) = w.any {
            found.push(Pred::Any(v));
        }
        if let Some(v) = w.all {
            found.push(Pred::All(v));
        }
        if let Some(p) = w.not {
            found.push(Pred::Not(p));
        }
        if count.is_some() && !counted {
            return Err("`count` goes with `msg`, `command` or `event`".into());
        }
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err("a predicate is one of msg, command, event, state, caret_in, selection, changed, folded, after_ms, any, all or not".into()),
            _ => Err("a predicate has one key (and `count`); combine several with `all` or `any`".into()),
        }
    }
}

impl From<Pred> for PredWire {
    fn from(p: Pred) -> PredWire {
        let c = |c: u32| (c != 1).then_some(c);
        let mut w = PredWire::default();
        match p {
            Pred::Msg { kind, count } => {
                w.msg = Some(kind);
                w.count = c(count);
            }
            Pred::Command { id, count } => {
                w.command = Some(id);
                w.count = c(count);
            }
            Pred::Event { name, count } => {
                w.event = Some(name);
                w.count = c(count);
            }
            Pred::State { key, test } => {
                w.state = Some(match test {
                    StateTest::Present(p) => StateWire {
                        key,
                        present: (!p).then_some(false),
                        matches: None,
                    },
                    StateTest::Match(m) => StateWire {
                        key,
                        present: None,
                        matches: Some(m),
                    },
                })
            }
            Pred::CaretIn(a) => w.caret_in = Some(a),
            Pred::Selection => w.selection = Some("nonempty".into()),
            Pred::Changed(a) => w.changed = Some(a),
            Pred::Folded(a) => w.folded = Some(a),
            Pred::AfterMs(ms) => w.after_ms = Some(ms),
            Pred::Any(v) => w.any = Some(v),
            Pred::All(v) => w.all = Some(v),
            Pred::Not(p) => w.not = Some(p),
        }
        w
    }
}

impl Pred {
    /// The key its count is kept under in [`TourState::counts`](crate::TourState::counts),
    /// for the counted kinds.
    pub fn count_key(&self) -> Option<String> {
        match self {
            Pred::Msg { kind, .. } => Some(format!("msg:{kind}")),
            Pred::Command { id, .. } => Some(format!("command:{id}")),
            Pred::Event { name, .. } => Some(format!("event:{name}")),
            _ => None,
        }
    }

    /// Every predicate in it, itself first.
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a Pred>) {
        out.push(self);
        match self {
            Pred::Any(v) | Pred::All(v) => v.iter().for_each(|p| p.walk(out)),
            Pred::Not(p) => p.walk(out),
            _ => {}
        }
    }
}

/// What a host answers about the message it just applied (and its state now). Every method
/// has a default that answers no, so a host implements only the predicates its walkthroughs
/// use. Pure: the answers depend on the message and the host's state, never on a clock.
///
/// With the `caretline` feature, [`Editor`](crate::Editor) answers the editor's own questions
/// from caretline types and passes the rest to the host.
pub trait TourHost {
    /// Did this action or command run in this message?
    fn ran(&self, _command: &str) -> bool {
        false
    }
    /// Was a message of this kind applied?
    fn msg(&self, _kind: &str) -> bool {
        false
    }
    /// Did this host event happen?
    fn event(&self, _name: &str) -> bool {
        false
    }
    /// The host's state at `key`, for `state` predicates (present, or containing a subset).
    fn state(&self, _key: &str) -> Option<Value> {
        None
    }
    /// Is the caret inside these anchors (the first that the host can place)?
    fn caret_in(&self, _anchors: &[Anchor]) -> bool {
        false
    }
    /// Is some selection non-empty?
    fn selection(&self) -> bool {
        false
    }
    /// Did this message change text at these anchors?
    fn changed(&self, _anchors: &[Anchor]) -> bool {
        false
    }
    /// Is the block of these anchors folded?
    fn folded(&self, _anchors: &[Anchor]) -> bool {
        false
    }
}

/// A host that answers no to everything: ops alone, and time.
impl TourHost for () {}

impl<T: TourHost + ?Sized> TourHost for &T {
    fn ran(&self, c: &str) -> bool {
        (**self).ran(c)
    }
    fn msg(&self, k: &str) -> bool {
        (**self).msg(k)
    }
    fn event(&self, n: &str) -> bool {
        (**self).event(n)
    }
    fn state(&self, k: &str) -> Option<Value> {
        (**self).state(k)
    }
    fn caret_in(&self, a: &[Anchor]) -> bool {
        (**self).caret_in(a)
    }
    fn selection(&self) -> bool {
        (**self).selection()
    }
    fn changed(&self, a: &[Anchor]) -> bool {
        (**self).changed(a)
    }
    fn folded(&self, a: &[Anchor]) -> bool {
        (**self).folded(a)
    }
}

/// Whether `value` contains `pattern`: every key of a pattern object is in the value with a
/// matching value (recursively), and anything else is equal.
pub fn subset(value: &Value, pattern: &Value) -> bool {
    match (value, pattern) {
        (Value::Object(v), Value::Object(p)) => p
            .iter()
            .all(|(k, pv)| v.get(k).is_some_and(|vv| subset(vv, pv))),
        _ => value == pattern,
    }
}

/// Counts the counted predicates in `preds` the host says happened in this message. Each
/// counted key goes up once per message, however often it appears.
pub(crate) fn tally(preds: &[&Pred], host: &dyn TourHost, counts: &mut BTreeMap<String, u32>) {
    let mut all = Vec::new();
    for p in preds {
        p.walk(&mut all);
    }
    let mut seen_keys = BTreeSet::new();
    for p in all {
        let Some(key) = p.count_key() else { continue };
        if !seen_keys.insert(key.clone()) {
            continue;
        }
        let hit = match p {
            Pred::Msg { kind, .. } => host.msg(kind),
            Pred::Command { id, .. } => host.ran(id),
            Pred::Event { name, .. } => host.event(name),
            _ => false,
        };
        if hit {
            *counts.entry(key).or_default() += 1;
        }
    }
}

/// What a predicate is checked against.
pub(crate) struct Ctx<'a> {
    pub host: &'a dyn TourHost,
    pub counts: &'a BTreeMap<String, u32>,
    pub lasted_ms: u64,
    pub anchors: &'a dyn Fn(&str) -> Option<Vec<Anchor>>,
}

pub(crate) fn holds(p: &Pred, cx: &Ctx) -> bool {
    let counted = |key: Option<String>, n: u32| {
        key.and_then(|k| cx.counts.get(&k).copied()).unwrap_or(0) >= n
    };
    let anchors = |name: &str| (cx.anchors)(name).unwrap_or_default();
    match p {
        Pred::Msg { count, .. } | Pred::Command { count, .. } | Pred::Event { count, .. } => {
            counted(p.count_key(), *count)
        }
        Pred::State { key, test } => match (cx.host.state(key), test) {
            (v, StateTest::Present(want)) => v.is_some() == *want,
            (Some(v), StateTest::Match(m)) => subset(&v, m),
            (None, StateTest::Match(_)) => false,
        },
        Pred::CaretIn(a) => {
            let a = anchors(a);
            !a.is_empty() && cx.host.caret_in(&a)
        }
        Pred::Selection => cx.host.selection(),
        Pred::Changed(a) => {
            let a = anchors(a);
            !a.is_empty() && cx.host.changed(&a)
        }
        Pred::Folded(a) => {
            let a = anchors(a);
            !a.is_empty() && cx.host.folded(&a)
        }
        Pred::AfterMs(ms) => cx.lasted_ms >= *ms,
        Pred::Any(v) => v.iter().any(|p| holds(p, cx)),
        Pred::All(v) => v.iter().all(|p| holds(p, cx)),
        Pred::Not(p) => !holds(p, cx),
    }
}

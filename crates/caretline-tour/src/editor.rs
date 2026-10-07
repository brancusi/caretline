//! The editor's own predicates, answered from caretline types (feature `caretline`).

use caretline::{ChangeSet, Document, Effect, MarkId, Msg, View, command_msg};
use caretline_layers::Anchor;
use serde_json::Value;

use crate::pred::TourHost;

/// Answers a walkthrough's editor predicates for one message a caretline editor applied:
/// `msg` (the message's kind), `command` (the message equals that catalog command's), `event`
/// (an effect of that name, or a host effect `Effect::Host { name }`), `caret_in`,
/// `selection`, `changed` and `folded`. Anything it can't answer (a host command, a host
/// event, `state`) goes to the host given with [`Editor::with_host`].
///
/// ```ignore
/// let (effects, changes) = caretline::update_with_changes(&mut state, msg.clone());
/// let on = Editor::new(&state.doc, &state.view).message(&msg, &effects, changes.as_ref());
/// for fx in caretline_tour::observe(&mut tour, &on, now_ms) { /* … */ }
/// ```
pub struct Editor<'a> {
    pub doc: &'a Document,
    pub view: &'a View,
    pub msg: Option<&'a Msg>,
    pub effects: &'a [Effect],
    pub changes: Option<&'a ChangeSet>,
    pub host: &'a dyn TourHost,
}

impl<'a> Editor<'a> {
    /// The editor's state after the message, with no message yet (a tick).
    pub fn new(doc: &'a Document, view: &'a View) -> Editor<'a> {
        Editor {
            doc,
            view,
            msg: None,
            effects: &[],
            changes: None,
            host: &(),
        }
    }

    /// The message just applied, its effects and its changes.
    pub fn message(
        mut self,
        msg: &'a Msg,
        effects: &'a [Effect],
        changes: Option<&'a ChangeSet>,
    ) -> Editor<'a> {
        self.msg = Some(msg);
        self.effects = effects;
        self.changes = changes;
        self
    }

    /// The host that answers what the editor can't.
    pub fn with_host(mut self, host: &'a dyn TourHost) -> Editor<'a> {
        self.host = host;
        self
    }

    /// The char range an anchor covers in the document now: a text range, a block (from its
    /// mark to the next), or text within a block. `None` for the caret, a screen position or a
    /// host anchor.
    pub fn range(&self, a: &Anchor) -> Option<(usize, usize)> {
        match a.unscoped() {
            Anchor::Text { from, to } => Some((*from, *to)),
            Anchor::Block(b) => {
                let from = self.doc.marks.pos(MarkId(*b))?;
                let to = self
                    .doc
                    .marks
                    .iter()
                    .map(|m| m.pos)
                    .find(|&p| p > from)
                    .unwrap_or_else(|| self.doc.text.len_chars());
                Some((from, to))
            }
            Anchor::BlockText { block, from, to } => {
                let at = self.doc.marks.pos(MarkId(*block))?;
                Some((at + from, at + to))
            }
            _ => None,
        }
    }

    /// The block an anchor is in: its own, or the one whose mark is at or before its start.
    fn block(&self, a: &Anchor) -> Option<MarkId> {
        match a.unscoped() {
            Anchor::Block(b) | Anchor::BlockText { block: b, .. } => Some(MarkId(*b)),
            Anchor::Text { from, .. } => self.doc.marks.at_or_before(*from).map(|m| m.id),
            _ => None,
        }
    }
}

fn tag(v: &impl serde::Serialize, key: &str) -> Option<String> {
    serde_json::to_value(v)
        .ok()?
        .get(key)
        .and_then(Value::as_str)
        .map(String::from)
}

impl TourHost for Editor<'_> {
    fn ran(&self, command: &str) -> bool {
        self.msg
            .is_some_and(|m| command_msg(command, None).as_ref() == Some(m))
            || self.host.ran(command)
    }

    fn msg(&self, kind: &str) -> bool {
        self.msg
            .is_some_and(|m| tag(m, "msg").as_deref() == Some(kind))
            || self.host.msg(kind)
    }

    fn event(&self, name: &str) -> bool {
        self.effects.iter().any(|e| match e {
            Effect::Host { name: n, .. } => n == name,
            e => tag(e, "effect").as_deref() == Some(name),
        }) || self.host.event(name)
    }

    fn state(&self, key: &str) -> Option<Value> {
        self.host.state(key)
    }

    fn caret_in(&self, anchors: &[Anchor]) -> bool {
        let caret = self.view.caret();
        match anchors.iter().find_map(|a| self.range(a)) {
            Some((from, to)) => from <= caret && caret <= to,
            None => self.host.caret_in(anchors),
        }
    }

    fn selection(&self) -> bool {
        self.view.selection.ranges().iter().any(|r| !r.is_empty()) || self.host.selection()
    }

    fn changed(&self, anchors: &[Anchor]) -> bool {
        let Some(changes) = self.changes else {
            return self.host.changed(anchors);
        };
        let Some((from, to)) = anchors.iter().find_map(|a| self.range(a)) else {
            return self.host.changed(anchors);
        };
        changes
            .changes_iter()
            .any(|(cf, ct, _)| cf <= to && ct >= from)
    }

    fn folded(&self, anchors: &[Anchor]) -> bool {
        match anchors.iter().find_map(|a| self.block(a)) {
            Some(b) => self.view.folds.contains(&b),
            None => self.host.folded(anchors),
        }
    }
}

/// Resolves a `find` text in a caretline document: its first occurrence, as text within the
/// block it starts in when the document has marks (so it follows the block through edits),
/// else as a text range. For [`Tour::resolve_finds`](crate::Tour::resolve_finds):
///
/// ```ignore
/// tour.resolve_finds(|text, _view| find_in(&state.doc, text));
/// ```
pub fn find_in(doc: &Document, text: &str) -> Option<Anchor> {
    let s = doc.text.to_string();
    let b = s.find(text)?;
    let from = s[..b].chars().count();
    let to = from + text.chars().count();
    Some(match doc.marks.at_or_before(from) {
        Some(m) => Anchor::BlockText {
            block: m.id.0,
            from: from - m.pos,
            to: to - m.pos,
        },
        None => Anchor::Text { from, to },
    })
}

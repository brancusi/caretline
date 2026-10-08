//! What a host adds to the engine: named **commands**, **input rules**, a **decorator**, and
//! **ext reducers** for values of its own kept on each view ([`Host::ext`]).
//!
//! caretline edits text; what the text *means* is the host's. A host that wants a key to do
//! something only it understands (rewrite a line's prefix by its own rules) registers a
//! command: a pure function from the document, the view and some JSON arguments to an
//! [`Edit`]. [`Msg::Command`] runs it as one transaction and one undo step, so it is recorded
//! in traces and replays wherever the same commands are registered. An input rule may take an
//! editing message before the engine does (a shorthand typed at a line's start). A decorator
//! says what to draw in a block's hang and gutter (see [`Decoration`]).
//!
//! A host that keeps state of its own per view (what it shows over the text, a step it is
//! at) keeps it in [`View::ext`] under a key, so it is in the state, its traces and replays.
//! [`Host::ext`] registers the key's reducer: `apply` runs for [`Msg::Ext`] on the acting view,
//! and `observe` (optional) after every message, on each view that holds the key, with the
//! message, its effects and its text changes.
//!
//! Every extension is a pure function: no clock, no randomness, no I/O. The engine never
//! serializes code. A [`Host`] lives on the [`Document`] ([`Document::set_host`]), is shared by
//! its views, compares equal to any other host and is never part of the state's JSON: a state
//! read from JSON has no host until one is set.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helix::{Assoc, ChangeSet, Range, RopeSlice, Selection, Tendril, Transaction};
use crate::marks::{MarkAttrs, MarkId};
use crate::msg::{Effect, Msg};
use crate::outline::{BlockInfo, Outline};
use crate::state::{Document, State, View};
use crate::update::{self, Step};
use crate::view::Frame;

/// A host command: (document and view, arguments) to an edit, or why it can't run.
pub type CommandFn = dyn Fn(&Ctx, &Value) -> Result<Edit, String> + Send + Sync;
/// An input rule: an editing message to the edit that replaces it, or `None` to let it pass.
pub type InputRuleFn = dyn Fn(&Ctx, &Msg) -> Option<Edit> + Send + Sync;
/// A decorator: what to draw beside a block.
pub type DecoratorFn = dyn Fn(&Ctx, &BlockInfo) -> Decoration + Send + Sync;
/// A frame pass: draws over a rendered frame (through [`Frame::set`] and friends), at the end
/// of every [`crate::view::render`].
pub type FramePassFn = dyn Fn(&Ctx, &mut Frame) + Send + Sync;
/// An ext reducer's `apply`: (the acting view, the key's value there if any, the message's
/// `op`) to what changes, or why it can't (shown in the status).
pub type ExtApplyFn = dyn Fn(&Ctx, Option<&Value>, &Value) -> Result<ExtOut, String> + Send + Sync;
/// An ext reducer's `observe`: (a view holding the key, its value, what just happened) to what
/// changes, or `None` for nothing.
pub type ExtObserveFn = dyn Fn(&Ctx, &Value, &Observed) -> Option<ExtOut> + Send + Sync;

/// The extensions a host registers. Cheap to clone (shared).
#[derive(Clone, Default)]
pub struct Host {
    inner: Arc<Inner>,
}

#[derive(Clone, Default)]
struct Inner {
    commands: BTreeMap<String, Arc<CommandFn>>,
    input_rules: Vec<(String, Arc<InputRuleFn>)>,
    decorator: Option<Arc<DecoratorFn>>,
    /// Ext reducers, in registration order (observers run in it).
    exts: Vec<(String, ExtFns)>,
    /// Frame passes, in registration order (they draw in it).
    passes: Vec<(String, Arc<FramePassFn>)>,
}

/// An ext reducer's functions ([`Host::ext`]): `apply` for [`Msg::Ext`], and an optional
/// `observe` that runs after every message.
#[derive(Clone)]
#[non_exhaustive]
pub struct ExtFns {
    pub apply: Arc<ExtApplyFn>,
    pub observe: Option<Arc<ExtObserveFn>>,
}

impl ExtFns {
    /// A reducer that applies [`Msg::Ext`] operations and observes nothing.
    pub fn new(
        apply: impl Fn(&Ctx, Option<&Value>, &Value) -> Result<ExtOut, String> + Send + Sync + 'static,
    ) -> ExtFns {
        ExtFns {
            apply: Arc::new(apply),
            observe: None,
        }
    }

    /// The same, observing every message on each view that holds the key: mapping its own
    /// positions through the changes, expiring on the clock, following what the person does.
    #[must_use]
    pub fn with_observe(
        mut self,
        observe: impl Fn(&Ctx, &Value, &Observed) -> Option<ExtOut> + Send + Sync + 'static,
    ) -> ExtFns {
        self.observe = Some(Arc::new(observe));
        self
    }
}

/// What an ext reducer changes on its view. Everything is optional: [`ExtOut::new`] changes
/// nothing.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ExtOut {
    /// The key's new value: `None` keeps it, `Some(Value::Null)` removes the key.
    pub value: Option<Value>,
    /// Effects for the host's runtime, returned from `update` as [`Effect::Host`].
    pub effects: Vec<(String, Value)>,
    /// A one-line message for the view's status.
    pub status: Option<String>,
    /// The frame clock the view asks for ([`View::frame_clock`]; 0 turns it off).
    pub frame_clock: Option<u16>,
}

impl ExtOut {
    /// Changes nothing.
    pub fn new() -> ExtOut {
        ExtOut::default()
    }

    /// Sets the key's value.
    pub fn value(value: Value) -> ExtOut {
        ExtOut::new().with_value(value)
    }

    /// Removes the key from the view.
    pub fn remove() -> ExtOut {
        ExtOut::new().with_value(Value::Null)
    }

    /// The same, setting the key's value (`Value::Null` removes it).
    #[must_use]
    pub fn with_value(mut self, value: Value) -> ExtOut {
        self.value = Some(value);
        self
    }

    /// The same, with one more effect.
    #[must_use]
    pub fn with_effect(mut self, name: impl Into<String>, data: Value) -> ExtOut {
        self.effects.push((name.into(), data));
        self
    }

    /// The same, with a status message.
    #[must_use]
    pub fn with_status(mut self, text: impl Into<String>) -> ExtOut {
        self.status = Some(text.into());
        self
    }

    /// The same, asking for a frame clock of `fps` (0 turns it off).
    #[must_use]
    pub fn with_frame_clock(mut self, fps: u16) -> ExtOut {
        self.frame_clock = Some(fps);
        self
    }
}

/// What an observer sees after a message.
#[derive(Debug)]
#[non_exhaustive]
pub struct Observed<'a> {
    pub msg: &'a Msg,
    /// The message's effects (before any observer's).
    pub effects: &'a [Effect],
    /// The message's text changes, composed into one (what [`crate::update_with_changes`]
    /// returns): any view's edit, undo and redo, a change from elsewhere. `None` when the text
    /// didn't change.
    pub changes: Option<&'a ChangeSet>,
    /// Whether the message went through this view (a change from elsewhere goes through
    /// none).
    pub acting: bool,
}

impl Host {
    pub fn new() -> Host {
        Host::default()
    }

    /// Registers command `name` (a later registration of the same name replaces it).
    pub fn command(
        mut self,
        name: &str,
        f: impl Fn(&Ctx, &Value) -> Result<Edit, String> + Send + Sync + 'static,
    ) -> Host {
        Arc::make_mut(&mut self.inner)
            .commands
            .insert(name.to_string(), Arc::new(f));
        self
    }

    /// Adds an input rule. Rules run in the order they were added; the first to return an
    /// edit takes the message.
    pub fn input_rule(
        mut self,
        name: &str,
        f: impl Fn(&Ctx, &Msg) -> Option<Edit> + Send + Sync + 'static,
    ) -> Host {
        Arc::make_mut(&mut self.inner)
            .input_rules
            .push((name.to_string(), Arc::new(f)));
        self
    }

    /// Sets the decorator (replacing any before it).
    pub fn decorator(
        mut self,
        f: impl Fn(&Ctx, &BlockInfo) -> Decoration + Send + Sync + 'static,
    ) -> Host {
        Arc::make_mut(&mut self.inner).decorator = Some(Arc::new(f));
        self
    }

    /// Registers the reducer for the view values under `key` ([`View::ext`]; a later
    /// registration of the same key replaces it). [`Msg::Ext`] with that key runs `apply` on
    /// the acting view; `observe`, when set, runs after every message on each view holding the
    /// key, in registration order. Both must be pure: traces replay through them.
    pub fn ext(mut self, key: &str, fns: ExtFns) -> Host {
        let exts = &mut Arc::make_mut(&mut self.inner).exts;
        match exts.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = fns,
            None => exts.push((key.to_string(), fns)),
        }
        self
    }

    /// Adds a frame pass: `f` draws over every frame [`crate::view::render`] makes (not
    /// [`crate::view::render_plain`]), after the passes before it. A later pass of the same
    /// name replaces it in its place. It must be pure: snapshots, the protocol's `render` and
    /// replays draw through it. With none registered, rendering costs nothing more.
    pub fn frame_pass(
        mut self,
        name: &str,
        f: impl Fn(&Ctx, &mut Frame) + Send + Sync + 'static,
    ) -> Host {
        let passes = &mut Arc::make_mut(&mut self.inner).passes;
        match passes.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = Arc::new(f),
            None => passes.push((name.to_string(), Arc::new(f))),
        }
        self
    }

    /// The frame passes' names, in the order they run.
    pub fn frame_pass_names(&self) -> Vec<&str> {
        self.inner.passes.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// The keys with a registered ext reducer, in registration order.
    pub fn ext_keys(&self) -> Vec<&str> {
        self.inner.exts.iter().map(|(k, _)| k.as_str()).collect()
    }

    /// Whether any ext reducer observes messages.
    pub(crate) fn has_observers(&self) -> bool {
        self.inner.exts.iter().any(|(_, f)| f.observe.is_some())
    }

    fn ext_fns(&self, key: &str) -> Option<&ExtFns> {
        self.inner
            .exts
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, f)| f)
    }

    /// The registered command names, sorted.
    pub fn command_names(&self) -> Vec<&str> {
        self.inner.commands.keys().map(String::as_str).collect()
    }

    /// The input rules' names, in order.
    pub fn input_rule_names(&self) -> Vec<&str> {
        self.inner
            .input_rules
            .iter()
            .map(|(n, _)| n.as_str())
            .collect()
    }

    pub fn has_decorator(&self) -> bool {
        self.inner.decorator.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.commands.is_empty()
            && self.inner.input_rules.is_empty()
            && self.inner.decorator.is_none()
            && self.inner.exts.is_empty()
            && self.inner.passes.is_empty()
    }

    /// The decoration of `block`, from the decorator (none without one).
    pub fn decorate(&self, ctx: &Ctx, block: &BlockInfo) -> Option<Decoration> {
        self.inner.decorator.as_ref().map(|f| f(ctx, block))
    }
}

impl PartialEq for Host {
    /// Not part of the state's value: any two hosts are equal.
    fn eq(&self, _: &Host) -> bool {
        true
    }
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("commands", &self.command_names())
            .field("input_rules", &self.input_rule_names())
            .field("decorator", &self.has_decorator())
            .field("ext", &self.ext_keys())
            .field("frame_passes", &self.frame_pass_names())
            .finish()
    }
}

/// What an extension sees: the document and the view it acts through.
pub struct Ctx<'a> {
    pub doc: &'a Document,
    pub view: &'a View,
}

impl<'a> Ctx<'a> {
    pub fn new(doc: &'a Document, view: &'a View) -> Ctx<'a> {
        Ctx { doc, view }
    }

    pub fn text(&self) -> RopeSlice<'a> {
        self.doc.text.slice(..)
    }

    /// The blocks, in an outline document.
    pub fn blocks(&self) -> Option<Arc<Outline>> {
        self.doc.blocks()
    }

    pub fn selection(&self) -> &'a Selection {
        &self.view.selection
    }

    /// The primary caret.
    pub fn caret(&self) -> usize {
        self.view.caret()
    }

    /// The line ending Enter inserts.
    pub fn line_ending(&self) -> &'static str {
        self.doc.config.line_ending.as_str()
    }

    /// The current selection mapped through `changes` (sorted, in current positions):
    /// positions at an insertion move past it.
    pub fn mapped_selection(&self, changes: &[(usize, usize, String)]) -> Selection {
        let txn = Transaction::change(
            &self.doc.text,
            changes
                .iter()
                .map(|(a, b, t)| (*a, *b, (!t.is_empty()).then(|| Tendril::from(t.as_str())))),
        );
        let cs = txn.changes();
        self.view.selection.clone().transform(|r| Range {
            anchor: cs.map_pos(r.anchor, Assoc::After),
            head: cs.map_pos(r.head, Assoc::After),
            old_visual_position: None,
        })
    }
}

/// A change to the marks, made with an [`Edit`] (positions in the text after its changes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MarkOp {
    /// A new mark at the start of the line holding `pos` (a line that already has one keeps
    /// it, as it is).
    Mint {
        pos: usize,
        #[serde(default)]
        attrs: MarkAttrs,
    },
    Remove {
        id: MarkId,
    },
    /// A mark's blank row before its block.
    SetGap {
        id: MarkId,
        #[serde(default)]
        gap: Option<bool>,
    },
    /// A mark's payload.
    SetData {
        id: MarkId,
        #[serde(default)]
        data: Option<Value>,
    },
}

/// What a host command or input rule does: one transaction and one undo step.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edit {
    /// `[from, to)` replaced by text, in chars of the current text, sorted and apart.
    pub changes: Vec<(usize, usize, String)>,
    /// The selection after, in the new text. `None`: the selection mapped through the changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    /// Mark changes, applied after the text changes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<MarkOp>,
    /// A one-line message for the view's status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Effects for the host's runtime, returned from `update` as [`Effect::Host`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<(String, Value)>,
    /// Every block keeps its blank row (a change of shape never moves another block).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keep_gaps: bool,
}

impl Edit {
    /// An edit that only says something.
    pub fn status(text: impl Into<String>) -> Edit {
        Edit {
            status: Some(text.into()),
            ..Edit::default()
        }
    }

    /// Whether it changes neither text nor marks.
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty() && self.marks.is_empty()
    }
}

/// One drawn decoration: text in a slot beside a block, styled by a role the host names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deco {
    /// Drawn from the slot's left edge, clipped to its width.
    pub text: String,
    /// A style name the host defines (`"badge.warm"`). The renderer reports it per cell; colours
    /// stay with the host.
    pub role: String,
    /// Reported by hit-testing when the slot is clicked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// What to draw beside a block in an outline layout: in its hang (the columns before its
/// content) and its gutter (the columns before everything).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hang: Option<Deco>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gutter: Option<Deco>,
}

/// Runs `Msg::Command`.
pub(crate) fn run_command(state: &mut State, name: &str, args: &Value) -> Vec<Effect> {
    let host = state.doc.host.clone();
    let Some(f) = host.inner.commands.get(name).cloned() else {
        state.view.status = Some(format!("no command '{name}'"));
        return Vec::new();
    };
    let result = f(&Ctx::new(&state.doc, &state.view), args);
    match result {
        Ok(edit) => apply(state, edit),
        Err(why) => {
            state.view.status = Some(why);
            Vec::new()
        }
    }
}

/// Runs the frame passes over `frame`, but those named in `skip`.
pub(crate) fn run_frame_passes(doc: &Document, view: &View, frame: &mut Frame, skip: &[&str]) {
    let passes = &doc.host.inner.passes;
    if passes.is_empty() {
        return;
    }
    let ctx = Ctx::new(doc, view);
    for (name, f) in passes {
        if !skip.contains(&name.as_str()) {
            f(&ctx, frame);
        }
    }
}

/// Runs `Msg::Ext` on the acting view.
pub(crate) fn run_ext(state: &mut State, key: &str, op: &Value) -> Vec<Effect> {
    let host = state.doc.host.clone();
    let Some(fns) = host.ext_fns(key) else {
        state.view.status = Some(format!("no ext '{key}'"));
        return Vec::new();
    };
    let result = (fns.apply)(
        &Ctx::new(&state.doc, &state.view),
        state.view.ext.get(key),
        op,
    );
    match result {
        Ok(out) => apply_ext(&mut state.view, key, out),
        Err(why) => {
            state.view.status = Some(why);
            Vec::new()
        }
    }
}

/// Puts what a reducer returned into its view.
fn apply_ext(view: &mut View, key: &str, out: ExtOut) -> Vec<Effect> {
    match out.value {
        Some(Value::Null) => {
            view.ext.remove(key);
        }
        Some(v) => {
            view.ext.insert(key.to_string(), v);
        }
        None => {}
    }
    if let Some(text) = out.status {
        // One line, as `Msg::ShowStatus` keeps it.
        let line = text.lines().next().unwrap_or("").to_string();
        view.status = (!line.is_empty()).then_some(line);
    }
    if let Some(fps) = out.frame_clock {
        view.frame_clock = fps;
    }
    out.effects
        .into_iter()
        .map(|(name, data)| Effect::Host { name, data })
        .collect()
}

/// Runs the observers after `msg` (through `views[acting]`): each on every view that holds its
/// key, in registration order, then view order. Their effects follow the message's.
pub(crate) fn observe(
    doc: &Document,
    views: &mut [View],
    acting: usize,
    msg: &Msg,
    effects: &mut Vec<Effect>,
    changes: Option<&ChangeSet>,
) {
    let host = doc.host.clone();
    let mut more = Vec::new();
    for (i, view) in views.iter_mut().enumerate() {
        if view.ext.is_empty() {
            continue;
        }
        for (key, fns) in &host.inner.exts {
            let Some(f) = &fns.observe else { continue };
            let Some(value) = view.ext.get(key) else {
                continue;
            };
            let seen = Observed {
                msg,
                effects,
                changes,
                acting: i == acting && !msg.is_external(),
            };
            if let Some(out) = f(&Ctx::new(doc, view), value, &seen) {
                more.extend(apply_ext(view, key, out));
            }
        }
    }
    effects.extend(more);
}

/// The first input rule that takes `msg`, applied. `None`: no rule took it.
pub(crate) fn input_rules(state: &mut State, msg: &Msg) -> Option<Vec<Effect>> {
    if state.doc.host.inner.input_rules.is_empty() || !takes_input(msg) {
        return None;
    }
    let host = state.doc.host.clone();
    let edit = host
        .inner
        .input_rules
        .iter()
        .find_map(|(_, f)| f(&Ctx::new(&state.doc, &state.view), msg))?;
    Some(apply(state, edit))
}

/// The messages input rules see: those that edit through the keyboard or the clipboard.
fn takes_input(msg: &Msg) -> bool {
    msg.edits()
        && !matches!(
            msg,
            Msg::Undo
                | Msg::Redo
                | Msg::Command { .. }
                | Msg::Edit { .. }
                | Msg::External { .. }
                | Msg::InsertBlocks { .. }
        )
}

/// Applies an edit as one undo step. A malformed edit (ranges out of order or past the end)
/// changes nothing and says so.
pub(crate) fn apply(state: &mut State, edit: Edit) -> Vec<Effect> {
    let len = state.doc.text.len_chars();
    let mut at = 0;
    for &(from, to, _) in &edit.changes {
        if from < at || to < from || to > len {
            state.view.status = Some("a command's edit was out of range; nothing changed".into());
            return Vec::new();
        }
        at = to;
    }
    let pins = if edit.keep_gaps {
        crate::outline::rules::pins_all(state)
    } else {
        None
    };
    if !edit.is_noop() || edit.selection.is_some() {
        let selection = match edit.selection {
            Some(s) => s,
            None => Ctx::new(&state.doc, &state.view).mapped_selection(&edit.changes),
        };
        let txn = Transaction::change(
            &state.doc.text,
            edit.changes
                .iter()
                .map(|(a, b, t)| (*a, *b, (!t.is_empty()).then(|| Tendril::from(t.as_str())))),
        )
        .with_selection(selection);
        let ops = edit.marks;
        update::commit_with(state, txn, Step::default(), move |m, new| {
            for op in ops {
                match op {
                    MarkOp::Mint { pos, attrs } => {
                        m.mint_with(crate::marks::line_start_at(new, pos), attrs);
                    }
                    MarkOp::Remove { id } => {
                        m.remove(id);
                    }
                    MarkOp::SetGap { id, gap } => {
                        m.set_gap(id, gap);
                    }
                    MarkOp::SetData { id, data } => {
                        m.set_data(id, data);
                    }
                }
            }
        });
        state.view.fit(&state.doc);
    }
    if let Some(pins) = pins {
        crate::outline::rules::pin(state, pins);
    }
    if let Some(text) = edit.status {
        state.view.status = Some(text);
    }
    edit.effects
        .into_iter()
        .map(|(name, data)| Effect::Host { name, data })
        .collect()
}

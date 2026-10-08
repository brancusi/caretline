//! A lint for walkthroughs: what parses but can't work.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::model::{END, RESERVED, StepAnchor, Tour};
use crate::pred::Pred;

/// How bad a [`Problem`] is. [`apply`](crate::apply) refuses to start a tour with errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Error,
    Warning,
}

/// Something wrong with a walkthrough: a stable `code`, where, and a sentence for people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub level: Level,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    pub detail: String,
}

impl Problem {
    fn new(level: Level, code: &str, step: Option<&str>, detail: String) -> Problem {
        Problem {
            level,
            code: code.into(),
            step: step.map(String::from),
            layer: None,
            detail,
        }
    }

    pub(crate) fn layer(
        level: Level,
        code: &str,
        step: &str,
        layer: &str,
        detail: String,
    ) -> Problem {
        Problem {
            level,
            code: code.into(),
            step: Some(step.into()),
            layer: Some(layer.into()),
            detail,
        }
    }
}

/// Checks a walkthrough. Errors (a tour can't start with them):
///
/// - `no_steps`, `no_id`: no steps, or an empty tour or step id;
/// - `duplicate_step`, `duplicate_layer` (within a step);
/// - `reserved_id`: a step called `next`, `back` or `end`;
/// - `unknown_goto`: a branch to a step that isn't there;
/// - `no_anchor`: a layer with no anchor;
/// - `unknown_anchor`: `caret_in`, `changed` or `folded` naming no layer of the step;
/// - `hint_data`: a `hint` layer whose data isn't `{title?, text}`.
///
/// Warnings:
///
/// - `empty_step`: no layers, narration or host patch;
/// - `unscoped_find`: a `find` with no `in` (which view's text?);
/// - `nudge_data`: a nudge whose data isn't an object, or on a step without layers;
/// - `zero_count`: `count = 0`, which holds at once;
/// - `unreachable_branch`: a branch after one with no `if`.
pub fn check(tour: &Tour) -> Vec<Problem> {
    use Level::{Error, Warning};
    let mut out = Vec::new();
    if tour.id.is_empty() {
        out.push(Problem::new(
            Error,
            "no_id",
            None,
            "a tour needs an id".into(),
        ));
    }
    if tour.steps.is_empty() {
        out.push(Problem::new(
            Error,
            "no_steps",
            None,
            "a tour needs at least one step".into(),
        ));
    }
    let ids: BTreeSet<&str> = tour.steps.iter().map(|s| s.id.as_str()).collect();
    let mut seen = BTreeSet::new();
    for s in &tour.steps {
        let at = Some(s.id.as_str());
        if s.id.is_empty() {
            out.push(Problem::new(
                Error,
                "no_id",
                None,
                "a step needs an id".into(),
            ));
        }
        if !seen.insert(s.id.as_str()) {
            out.push(Problem::new(
                Error,
                "duplicate_step",
                at,
                format!("two steps are called {:?}", s.id),
            ));
        }
        if RESERVED.contains(&s.id.as_str()) {
            out.push(Problem::new(
                Error,
                "reserved_id",
                at,
                format!("{:?} means something else in `to` and `goto`", s.id),
            ));
        }
        if s.layers.is_empty() && s.narration.is_none() && s.host.is_none() {
            out.push(Problem::new(
                Warning,
                "empty_step",
                at,
                "no layers, narration or host patch: nothing shows".into(),
            ));
        }
        let mut layer_ids = BTreeSet::new();
        for l in &s.layers {
            let lp = |level, code: &str, detail: String| {
                Problem::layer(level, code, &s.id, &l.id, detail)
            };
            if !layer_ids.insert(l.id.as_str()) {
                out.push(lp(
                    Error,
                    "duplicate_layer",
                    format!("two layers are called {:?}", l.id),
                ));
            }
            if l.anchor.is_empty() {
                out.push(lp(Error, "no_anchor", "a layer needs an anchor".into()));
            }
            for a in &l.anchor {
                if let StepAnchor::Find { text, view: None } = a {
                    out.push(lp(
                        Warning,
                        "unscoped_find",
                        format!("`find = {text:?}` names no view (`in`)"),
                    ));
                }
            }
            let contentless = l.kind.is_none() && l.data.is_null();
            if !contentless
                && l.kind.as_deref().unwrap_or(&tour.kind) == caretline_layers::HINT
                && serde_json::from_value::<caretline_layers::Hint>(l.data.clone()).is_err()
            {
                out.push(lp(
                    Error,
                    "hint_data",
                    "a hint's data is {title?, text}".into(),
                ));
            }
        }
        if let Some(n) = &s.nudge
            && (!n.data.is_object() || s.layers.is_empty())
        {
            out.push(Problem::new(
                Warning,
                "nudge_data",
                at,
                "a nudge merges an object into each layer's data".into(),
            ));
        }
        let mut preds: Vec<&Pred> = Vec::new();
        for p in s.advance.iter().chain(s.skip_if.iter()) {
            p.walk(&mut preds);
        }
        for b in &s.next {
            if let Some(p) = &b.when {
                p.walk(&mut preds);
            }
        }
        for p in preds {
            match p {
                Pred::CaretIn(a) | Pred::Changed(a) | Pred::Folded(a) if s.anchors(a).is_none() => {
                    out.push(Problem::new(
                        Error,
                        "unknown_anchor",
                        at,
                        format!("{a:?} is neither \"anchor\" nor a layer of this step"),
                    ));
                }
                Pred::Msg { count: 0, .. }
                | Pred::Command { count: 0, .. }
                | Pred::Event { count: 0, .. } => {
                    out.push(Problem::new(
                        Warning,
                        "zero_count",
                        at,
                        "`count = 0` holds at once".into(),
                    ));
                }
                _ => {}
            }
        }
        let mut open = true;
        for b in &s.next {
            if b.goto != END && !ids.contains(b.goto.as_str()) {
                out.push(Problem::new(
                    Error,
                    "unknown_goto",
                    at,
                    format!("no step {:?}", b.goto),
                ));
            }
            if !open {
                out.push(Problem::new(
                    Warning,
                    "unreachable_branch",
                    at,
                    format!("the branch to {:?} follows one with no `if`", b.goto),
                ));
            }
            if b.when.is_none() {
                open = false;
            }
        }
    }
    out
}

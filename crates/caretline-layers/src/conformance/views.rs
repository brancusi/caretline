//! The kit's caretline half: resolvers that own their frames (so a scene built inside a
//! closure can hold them), and [`check_mapping`], whether an edit moved only the anchors in its
//! own document.

use caretline::view::Frame;
use caretline::{ChangeSet, Document};

use super::{Kind, Violation};
use crate::frame::{Edited, FrameResolver, map_anchors};
use crate::geom::Rect;
use crate::model::{Anchor, Layers};
use crate::resolve::{AnchorMap, Chain, Resolve, Resolved};

/// A caretline view a [`Scene`](super::Scene) owns: its frame, where it was drawn, its id,
/// clip and focus, as [`FrameResolver`] takes them.
#[derive(Debug, Clone)]
pub struct OwnedView {
    pub frame: Frame,
    pub doc: Option<Document>,
    pub x: u16,
    pub y: u16,
    pub id: Option<String>,
    pub clip: Option<Rect>,
    pub focused: bool,
}

impl OwnedView {
    pub fn new(frame: Frame) -> OwnedView {
        OwnedView {
            frame,
            doc: None,
            x: 0,
            y: 0,
            id: None,
            clip: None,
            focused: false,
        }
    }

    pub fn at(mut self, x: u16, y: u16) -> OwnedView {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_doc(mut self, doc: Document) -> OwnedView {
        self.doc = Some(doc);
        self
    }

    pub fn id(mut self, id: &str) -> OwnedView {
        self.id = Some(id.into());
        self
    }

    pub fn clip(mut self, clip: Rect) -> OwnedView {
        self.clip = Some(clip);
        self
    }

    pub fn focused(mut self) -> OwnedView {
        self.focused = true;
        self
    }

    /// Its [`FrameResolver`].
    pub fn resolver(&self) -> FrameResolver<'_> {
        let mut r = FrameResolver::new(&self.frame).at(self.x, self.y);
        if let Some(d) = &self.doc {
            r = r.with_doc(d);
        }
        if let Some(id) = &self.id {
            r = r.id(id);
        }
        if let Some(c) = self.clip {
            r = r.clip(c);
        }
        if self.focused {
            r = r.focused();
        }
        r
    }
}

/// The host's [`AnchorMap`] and its views, resolved as `Chain(vec![&anchors, &view, …])`.
#[derive(Debug, Clone, Default)]
pub struct OwnedViews {
    pub anchors: AnchorMap,
    pub views: Vec<OwnedView>,
}

impl OwnedViews {
    pub fn new(anchors: AnchorMap) -> OwnedViews {
        OwnedViews {
            anchors,
            views: Vec::new(),
        }
    }

    pub fn view(mut self, v: OwnedView) -> OwnedViews {
        self.views.push(v);
        self
    }
}

impl Resolve for OwnedViews {
    fn resolve(&self, anchor: &Anchor) -> Option<Resolved> {
        let rs: Vec<FrameResolver<'_>> = self.views.iter().map(OwnedView::resolver).collect();
        let mut chain: Vec<&dyn Resolve> = vec![&self.anchors];
        chain.extend(rs.iter().map(|r| r as &dyn Resolve));
        Chain(chain).resolve(anchor)
    }

    fn is_focused(&self) -> bool {
        self.views.iter().any(|v| v.focused)
    }
}

/// An edit a host made, and its views: which document each shows.
#[derive(Debug, Clone, Copy)]
pub struct Mapping<'a> {
    /// Every view the host shows, with the document it shows: `(view id, document id)`.
    pub views: &'a [(&'a str, &'a str)],
    /// The focused view's id: unscoped anchors belong to its document.
    pub focused: &'a str,
    /// The edited document's id.
    pub edited: &'a str,
    /// The edit's changes (`update_with_changes`, `update_doc_with_changes`).
    pub changes: &'a ChangeSet,
}

impl Mapping<'_> {
    /// The views that show the edited document: [`Edited::Views`]' `views`.
    pub fn edited_views(&self) -> Vec<&str> {
        self.views
            .iter()
            .filter(|(_, d)| *d == self.edited)
            .map(|(v, _)| *v)
            .collect()
    }

    /// Whether unscoped anchors map: the focused view shows the edited document.
    pub fn unscoped(&self) -> bool {
        self.views
            .iter()
            .any(|(v, d)| *v == self.focused && *d == self.edited)
    }

    /// Whether an anchor points into the edited document.
    fn moves(&self, a: &Anchor) -> bool {
        match a.view() {
            Some(v) => self
                .views
                .iter()
                .any(|(id, d)| *id == v && *d == self.edited),
            None => self.unscoped(),
        }
    }

    /// The layers as they should be after the edit: each anchor in the edited document mapped
    /// as [`Edited::All`] maps it (dropped when its text went), every other anchor as it was,
    /// a layer with no anchor left removed.
    pub fn expected(&self, before: &Layers) -> Layers {
        let mut out = before.clone();
        for l in &mut out.layers {
            let mut kept = Vec::new();
            for a in &l.anchor {
                if !self.moves(a) {
                    kept.push(a.clone());
                    continue;
                }
                let mut one = Layers::default();
                let mut solo = l.clone();
                solo.anchor = vec![a.clone()];
                one.layers.push(solo);
                map_anchors(&mut one, Edited::All, self.changes);
                if let Some(m) = one.layers.first() {
                    kept.extend(m.anchor.iter().cloned());
                }
            }
            l.anchor = kept;
        }
        out.layers.retain(|l| !l.anchor.is_empty());
        out
    }
}

/// Whether the host's layers after an edit (`after`, from `before` through its own
/// `observe` or `map_anchors`) moved the anchors in the edited document, and only those: an
/// anchor scoped to a view of another document stays put, an unscoped one follows the
/// focused view's document. A host with several documents passes [`Edited::Views`] built from
/// [`Mapping::edited_views`] and [`Mapping::unscoped`]; one that passes [`Edited::All`] fails
/// here as soon as an edit lands in a document an anchor doesn't point into.
pub fn check_mapping(before: &Layers, after: &Layers, m: &Mapping<'_>) -> Vec<Violation> {
    let mut out = Vec::new();
    let want = m.expected(before);
    for w in &want.layers {
        match after.get(&w.id) {
            None => out.push(Violation::new(
                Some(&w.id),
                Kind::Mapping,
                "dropped by the edit, though an anchor still points at something",
            )),
            Some(g) if g.anchor != w.anchor => {
                let was = before
                    .get(&w.id)
                    .map(|b| b.anchor.clone())
                    .unwrap_or_default();
                out.push(Violation::new(
                    Some(&w.id),
                    Kind::Mapping,
                    format!(
                        "anchors {} became {}, should be {}",
                        json(&was),
                        json(&g.anchor),
                        json(&w.anchor)
                    ),
                ));
            }
            _ => {}
        }
    }
    for g in &after.layers {
        if want.get(&g.id).is_none() {
            out.push(Violation::new(
                Some(&g.id),
                Kind::Mapping,
                "kept, though every anchor's text went in the edit",
            ));
        }
    }
    out
}

fn json(a: &[Anchor]) -> String {
    serde_json::to_string(a).unwrap_or_default()
}

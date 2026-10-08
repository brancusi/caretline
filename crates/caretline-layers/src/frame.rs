//! caretline anchors: resolving them from an editor's [`Frame`], and mapping text anchors
//! through a [`ChangeSet`] so they follow edits. The changes are the ones each editor message
//! made, as caretline hands them out: `caretline::update_with_changes`,
//! `caretline::update_doc_with_changes` or `Session::apply_with_changes`.

use caretline::view::{Frame, RowInfo};
use caretline::{Assoc, ChangeSet};
use caretline::{Document, MarkId};

use crate::geom::Rect;
use crate::model::{Anchor, Layers, expire};
use crate::resolve::{Off, Resolve, Resolved};

/// Resolves text, block and caret anchors from a rendered caretline [`Frame`] drawn at
/// (`x`, `y`) on the host's screen. Only visible rows are read; nothing scans the document.
/// With [`FrameResolver::with_doc`], a block or block-relative anchor that isn't on screen
/// still says which way it lies.
///
/// One document shown in several views gets one resolver per view, each with its own
/// [`id`](FrameResolver::id) (the host's names, such as `main` or `panel:2`), offset and
/// [`clip`](FrameResolver::clip), in a [`Chain`](crate::Chain) with the focused one marked
/// ([`focused`](FrameResolver::focused)). A scoped anchor ([`Anchor::In`]) resolves only in
/// the view it names; every answer carries the resolver's id (`Resolved::view`).
pub struct FrameResolver<'a> {
    frame: &'a Frame,
    x: u16,
    y: u16,
    doc: Option<&'a Document>,
    id: Option<String>,
    clip: Option<Rect>,
    focused: bool,
}

impl<'a> FrameResolver<'a> {
    pub fn new(frame: &'a Frame) -> FrameResolver<'a> {
        FrameResolver {
            frame,
            x: 0,
            y: 0,
            doc: None,
            id: None,
            clip: None,
            focused: false,
        }
    }

    /// Where the host drew the frame's top-left cell.
    pub fn at(mut self, x: u16, y: u16) -> FrameResolver<'a> {
        self.x = x;
        self.y = y;
        self
    }

    /// The document, for where blocks start.
    pub fn with_doc(mut self, doc: &'a Document) -> FrameResolver<'a> {
        self.doc = Some(doc);
        self
    }

    /// The view's stable id, as anchors name it in `in`. Without one, only unscoped anchors
    /// resolve here.
    pub fn id(mut self, id: &str) -> FrameResolver<'a> {
        self.id = Some(id.into());
        self
    }

    /// The screen cells the view shows (default: the whole frame at its offset). Cells
    /// outside it don't count as visible: an anchor clipped away lies off screen, the way
    /// its cells are.
    pub fn clip(mut self, clip: Rect) -> FrameResolver<'a> {
        self.clip = Some(clip);
        self
    }

    /// Marks this the view the host has focused ([`Resolve::is_focused`]).
    pub fn focused(mut self) -> FrameResolver<'a> {
        self.focused = true;
        self
    }

    /// The cells this view shows on the host's screen.
    fn shown(&self) -> Rect {
        let all = Rect::new(self.x, self.y, self.frame.width, self.frame.height);
        self.clip.map_or(all, |c| c.intersection(&all))
    }

    /// Keeps the cells inside the clip. Cells that all fall outside it say which way they lie;
    /// a direction from the frame is pulled inside the clip.
    fn bound(&self, r: Resolved) -> Resolved {
        let view = self.id.as_deref();
        if self.clip.is_none() {
            return r.in_view(view);
        }
        let clip = self.shown();
        if r.rects.is_empty() {
            let off = r.off.map(|o| match o {
                Off::Above { x } => Off::Above {
                    x: Some(x.unwrap_or(clip.x).clamp(clip.x, clip.right().max(1) - 1)),
                },
                Off::Below { x } => Off::Below {
                    x: Some(x.unwrap_or(clip.x).clamp(clip.x, clip.right().max(1) - 1)),
                },
                Off::Left { y } => Off::Left {
                    y: y.clamp(clip.y, clip.bottom().max(1) - 1),
                },
                Off::Right { y } => Off::Right {
                    y: y.clamp(clip.y, clip.bottom().max(1) - 1),
                },
            });
            return Resolved {
                rects: Vec::new(),
                off,
                view: None,
            }
            .in_view(view);
        }
        let shown: Vec<Rect> = r
            .rects
            .iter()
            .map(|c| c.intersection(&clip))
            .filter(|c| !c.is_empty())
            .collect();
        if !shown.is_empty() {
            return Resolved::at(shown).in_view(view);
        }
        let b = Rect::bounds(&r.rects);
        let col = b.x.clamp(clip.x, clip.right().max(1) - 1);
        let row = b.y.clamp(clip.y, clip.bottom().max(1) - 1);
        let off = if b.bottom() <= clip.y {
            Off::Above { x: Some(col) }
        } else if b.y >= clip.bottom() {
            Off::Below { x: Some(col) }
        } else if b.right() <= clip.x {
            Off::Left { y: row }
        } else {
            Off::Right { y: row }
        };
        Resolved::off(off).in_view(view)
    }

    fn text_rows(&self) -> impl Iterator<Item = (u16, &RowInfo)> + '_ {
        self.frame
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, RowInfo::Text { .. }))
            .map(|(y, r)| (y as u16, r))
    }

    fn rect(&self, x: u16, y: u16, w: u16) -> Rect {
        Rect::new(x.saturating_add(self.x), y.saturating_add(self.y), w, 1)
    }

    /// The cells of chars `from..to` (an empty range is the cell at `from`).
    fn text(&self, from: usize, to: usize) -> Option<Resolved> {
        let (from, to) = (from.min(to), from.max(to).max(from.min(to) + 1));
        let f = self.frame;
        let mut rects = Vec::new();
        let mut scrolled: Option<Off> = None;
        let (mut first, mut last) = (usize::MAX, 0usize);
        // The last row that starts at or before `from`: where a folded range shows.
        let mut before: Option<(u16, u16, usize)> = None;
        for (y, row) in self.text_rows() {
            let RowInfo::Text { chars, x, .. } = row else {
                continue;
            };
            first = first.min(chars.start);
            last = last.max(chars.end);
            if chars.start <= from {
                before = Some((y, *x, chars.end));
            }
            // A row shows the range if they share a char, or (an empty row) it sits at `from`.
            let meets =
                chars.start < to && from < chars.end || (chars.is_empty() && chars.start == from);
            if !meets {
                continue;
            }
            let (mut lo, mut hi) = (u16::MAX, 0u16);
            let (mut left, mut right) = (false, false);
            for cx in 0..f.width {
                match f.cell(cx, y).char_idx.map(|c| c as usize) {
                    Some(c) if c >= from && c < to => {
                        lo = lo.min(cx);
                        hi = hi.max(cx + 1);
                    }
                    Some(c) if c < from => left = true,
                    Some(_) => right = true,
                    None => {}
                }
            }
            if lo < hi {
                rects.push(self.rect(lo, y, hi - lo));
            } else if chars.is_empty() {
                rects.push(self.rect(*x, y, 1));
            } else if scrolled.is_none() {
                // The row holds the range but it's scrolled out of sight sideways.
                let yy = y.saturating_add(self.y);
                scrolled = Some(if left && !right {
                    Off::Right { y: yy }
                } else {
                    Off::Left { y: yy }
                });
            }
        }
        if !rects.is_empty() {
            return Some(Resolved::at(rects));
        }
        if let Some(off) = scrolled {
            return Some(Resolved::off(off));
        }
        if first == usize::MAX {
            return None;
        }
        if to <= first {
            return Some(Resolved::off(Off::Above { x: None }));
        }
        if from >= last.max(first + 1) {
            return Some(Resolved::off(Off::Below { x: None }));
        }
        // Between visible rows but on none: folded away. Point at the row it hides under.
        before.map(|(y, x, _)| Resolved::at(vec![self.rect(x, y, 1)]))
    }

    fn block_start(&self, block: u64) -> Option<usize> {
        if let Some(d) = self.doc {
            return d.marks.pos(MarkId(block));
        }
        self.text_rows().find_map(|(_, r)| match r {
            RowInfo::Text {
                block: Some(b),
                first: true,
                chars,
                ..
            } if b.0 == block => Some(chars.start),
            _ => None,
        })
    }

    fn block(&self, block: u64) -> Option<Resolved> {
        let f = self.frame;
        let mut rects = Vec::new();
        for (y, row) in self.text_rows() {
            let RowInfo::Text {
                block: Some(b), x, ..
            } = row
            else {
                continue;
            };
            if b.0 != block {
                continue;
            }
            let mut hi = *x + 1;
            for cx in *x..f.width {
                if f.cell(cx, y).char_idx.is_some() {
                    hi = hi.max(cx + 1);
                }
            }
            let x0 = (*x).min(f.width.saturating_sub(1));
            rects.push(self.rect(x0, y, hi.min(f.width).saturating_sub(x0).max(1)));
        }
        if !rects.is_empty() {
            return Some(Resolved::at(rects));
        }
        let pos = self.doc?.marks.pos(MarkId(block))?;
        match self.text(pos, pos)? {
            r if r.off.is_some() => Some(r),
            // Visible text but not its rows: it's folded under the row found.
            r => Some(r),
        }
    }
}

impl Resolve for FrameResolver<'_> {
    fn resolve(&self, anchor: &Anchor) -> Option<Resolved> {
        let anchor = match anchor {
            Anchor::In { view, anchor } if self.id.as_deref() == Some(view.as_str()) => anchor,
            Anchor::In { .. } => return None,
            a => a,
        };
        self.raw(anchor).map(|r| self.bound(r))
    }

    fn is_focused(&self) -> bool {
        self.focused
    }
}

impl FrameResolver<'_> {
    /// An unscoped anchor's cells in the whole frame, before the clip.
    fn raw(&self, anchor: &Anchor) -> Option<Resolved> {
        match anchor {
            Anchor::Text { from, to } => self.text(*from, *to),
            Anchor::BlockText { block, from, to } => {
                let start = self.block_start(*block)?;
                self.text(start + from, start + to)
            }
            Anchor::Block(b) => self.block(*b),
            Anchor::Caret => self
                .frame
                .cursor
                .map(|(x, y)| Resolved::at(vec![self.rect(x, y, 1)])),
            Anchor::Screen(_) | Anchor::Host { .. } | Anchor::In { .. } => None,
        }
    }
}

/// Which anchors an edit moves. A `ChangeSet` belongs to one document, so the host says which
/// of its views show the document that changed:
///
/// - [`Edited::All`]: the host shows one document (in one view or several). Every text anchor
///   maps, scoped or not.
/// - [`Edited::Views`]: one of several documents changed. An anchor scoped to a view
///   ([`Anchor::In`]) maps only when that view is in `views` (every view that shows the edited
///   document, not only the one the edit came through); an unscoped anchor maps only when
///   `unscoped` is set. Unscoped anchors resolve in the focused view first, so they belong to
///   the focused view's document: the host sets `unscoped` when the edited document is that
///   one. Anchors on another document are left as they are.
///
/// A host with several documents scopes its anchors (`Anchor::scoped`), so each names the
/// document it points into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edited<'a> {
    All,
    Views {
        /// The ids of the views that show the edited document (`FrameResolver::id`).
        views: &'a [&'a str],
        /// Whether unscoped anchors map: the edited document is the focused view's.
        unscoped: bool,
    },
}

impl Edited<'_> {
    /// Whether an anchor points into the edited document.
    fn covers(&self, a: &Anchor) -> bool {
        match (self, a.view()) {
            (Edited::All, _) => true,
            (Edited::Views { views, .. }, Some(v)) => views.contains(&v),
            (Edited::Views { unscoped, .. }, None) => *unscoped,
        }
    }
}

/// Maps the text anchors that point into the edited document ([`Edited`]) through its
/// changes: the start sticks after an insertion there, the end before one, and a range that
/// collapses (its text deleted) is dropped, so the next fallback anchor takes over. A layer
/// with no anchor left is removed. Block anchors need nothing: marks follow their blocks.
/// Anchors on another document are left alone. Returns whether anything changed.
pub fn map_anchors(layers: &mut Layers, edited: Edited<'_>, changes: &ChangeSet) -> bool {
    let mut changed = false;
    for l in &mut layers.layers {
        let before = l.anchor.clone();
        l.anchor
            .retain_mut(|a| !edited.covers(a) || map_anchor(a, changes));
        changed |= l.anchor != before;
    }
    let n = layers.layers.len();
    layers.layers.retain(|l| !l.anchor.is_empty());
    changed || layers.layers.len() != n
}

/// Maps one anchor through `changes`; `false` when it no longer points at anything.
fn map_anchor(a: &mut Anchor, changes: &ChangeSet) -> bool {
    match a {
        Anchor::In { anchor, .. } => map_anchor(anchor, changes),
        Anchor::Text { from, to } => {
            if *to > changes.len() || *from > *to {
                return false;
            }
            if from == to {
                let p = changes.map_pos(*from, Assoc::Before);
                *from = p;
                *to = p;
                return true;
            }
            let f = changes.map_pos(*from, Assoc::After);
            let t = changes.map_pos(*to, Assoc::Before);
            *from = f;
            *to = t;
            f < t
        }
        _ => true,
    }
}

/// After every message: maps the anchors in the edited document ([`Edited`]) through its
/// changes (if it edited) and drops expired layers. Returns whether the layers changed.
///
/// `changes` is what the editor returned for that message:
///
/// ```ignore
/// let (effects, changes) = caretline::update_with_changes(&mut editor, msg);
/// observe(&mut layers, Edited::All, changes.as_ref(), now_ms);
/// ```
pub fn observe(
    layers: &mut Layers,
    edited: Edited<'_>,
    changes: Option<&ChangeSet>,
    now_ms: u64,
) -> bool {
    let mapped = changes.is_some_and(|c| map_anchors(layers, edited, c));
    expire(layers, now_ms) | mapped
}

impl crate::place::Grid {
    /// Marks the cells of a caretline frame drawn at (`x`, `y`): text where it shows a
    /// grapheme, wide graphemes as such (the frame's second half is `""`).
    pub fn mark_frame(&mut self, frame: &Frame, x: u16, y: u16) {
        use crate::place::CellKind;
        for fy in 0..frame.height {
            for fx in 0..frame.width {
                let s = frame.cell(fx, fy).symbol.as_str();
                let k = if s.is_empty() {
                    CellKind::WideTail
                } else if crate::place::width(s) >= 2 {
                    CellKind::Wide
                } else if s.trim().is_empty() {
                    CellKind::Blank
                } else {
                    CellKind::Text
                };
                self.set_kind(x.saturating_add(fx), y.saturating_add(fy), k);
            }
        }
    }

    /// A grid the size of a caretline frame, its cells marked, the caret its cursor, and the
    /// area its text rows (the status row left out).
    pub fn from_frame(frame: &Frame) -> crate::place::Grid {
        let mut g = crate::place::Grid::new(frame.width, frame.height);
        g.mark_frame(frame, 0, 0);
        let text = frame
            .rows
            .iter()
            .take_while(|r| !matches!(r, RowInfo::Status))
            .count() as u16;
        g.with_area(Rect::new(0, 0, frame.width, text))
            .with_caret(frame.cursor)
    }
}

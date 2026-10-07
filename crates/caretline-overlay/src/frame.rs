//! caretline anchors: resolving them from an editor's [`Frame`], and mapping text anchors
//! through a [`ChangeSet`] so they follow edits.

use caretline::helix::{Assoc, ChangeSet, Rope, Tendril};
use caretline::view::{Frame, RowInfo};
use caretline::{Document, MarkId};

use crate::geom::Rect;
use crate::model::{Anchor, Layers, expire};
use crate::resolve::{Off, Resolve, Resolved};

/// Resolves text, block and caret anchors from a rendered caretline [`Frame`] drawn at
/// (`x`, `y`) on the host's screen. Only visible rows are read; nothing scans the document.
/// With [`FrameResolver::with_doc`], a block or block-relative anchor that isn't on screen
/// still says which way it lies.
pub struct FrameResolver<'a> {
    frame: &'a Frame,
    x: u16,
    y: u16,
    doc: Option<&'a Document>,
}

impl<'a> FrameResolver<'a> {
    pub fn new(frame: &'a Frame) -> FrameResolver<'a> {
        FrameResolver {
            frame,
            x: 0,
            y: 0,
            doc: None,
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
            Anchor::Cells(r) => Some(Resolved::at(vec![*r])),
            Anchor::Screen(_) | Anchor::Host { .. } => None,
        }
    }
}

/// Maps every text anchor through an edit's changes: the start sticks after an insertion
/// there, the end before one, and a range that collapses (its text deleted) is dropped, so
/// the next fallback anchor takes over. A layer with no anchor left is removed. Block anchors
/// need nothing: marks follow their blocks. Returns whether anything changed.
pub fn map_anchors(layers: &mut Layers, changes: &ChangeSet) -> bool {
    let len = changes.len();
    let mut changed = false;
    for l in &mut layers.layers {
        let before = l.anchor.clone();
        l.anchor.retain_mut(|a| match a {
            Anchor::Text { from, to } => {
                if *to > len || *from > *to {
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
        });
        changed |= l.anchor != before;
    }
    let n = layers.layers.len();
    layers.layers.retain(|l| !l.anchor.is_empty());
    changed || layers.layers.len() != n
}

/// After every message: maps anchors through its changes (if it edited) and drops expired
/// layers. Returns whether the layers changed.
pub fn observe(layers: &mut Layers, changes: Option<&ChangeSet>, now_ms: u64) -> bool {
    let mapped = changes.is_some_and(|c| map_anchors(layers, c));
    expire(layers, now_ms) | mapped
}

/// The changes that turn `old` into `new`, for a host that sees texts rather than messages
/// (caretline doesn't hand out a message's `ChangeSet` yet).
pub fn changes_between(old: &str, new: &str) -> ChangeSet {
    let rope = Rope::from(old);
    let diff = caretline::diff::changes(old, new);
    ChangeSet::from_changes(
        &rope,
        diff.into_iter()
            .map(|(f, t, s)| (f, t, (!s.is_empty()).then(|| Tendril::from(s.as_str())))),
    )
}

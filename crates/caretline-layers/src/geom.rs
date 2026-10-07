//! Cell geometry: rectangles of whole cells and directions.

use serde::{Deserialize, Serialize};

/// A rectangle of whole cells: columns `x..x + w`, rows `y..y + h`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, w, h }
    }

    /// The column after the last.
    pub const fn right(&self) -> u16 {
        self.x.saturating_add(self.w)
    }

    /// The row after the last.
    pub const fn bottom(&self) -> u16 {
        self.y.saturating_add(self.h)
    }

    pub const fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub const fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        !self.is_empty()
            && !o.is_empty()
            && self.x < o.right()
            && o.x < self.right()
            && self.y < o.bottom()
            && o.y < self.bottom()
    }

    /// The cells both cover (empty when they don't meet).
    pub fn intersection(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= x || b <= y {
            return Rect::default();
        }
        Rect::new(x, y, r - x, b - y)
    }

    /// The smallest rectangle covering both (an empty one counts for nothing).
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }

    /// Grown by `dx` columns and `dy` rows on every side, clipped to `within`.
    pub fn grow(&self, dx: u16, dy: u16, within: &Rect) -> Rect {
        let x = self.x.saturating_sub(dx);
        let y = self.y.saturating_sub(dy);
        Rect::new(
            x,
            y,
            self.right().saturating_add(dx) - x,
            self.bottom().saturating_add(dy) - y,
        )
        .intersection(within)
    }

    /// Moved by (`dx`, `dy`), saturating at 0 and `u16::MAX`.
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        let m = |v: u16, d: i32| (v as i32 + d).clamp(0, u16::MAX as i32) as u16;
        Rect::new(m(self.x, dx), m(self.y, dy), self.w, self.h)
    }

    /// The bounding rectangle of several (empty for none).
    pub fn bounds(rects: &[Rect]) -> Rect {
        rects.iter().fold(Rect::default(), |a, r| a.union(r))
    }
}

/// A side of a rectangle, and the four ways a route can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Below,
    Above,
    Right,
    Left,
}

impl Side {
    /// The default order a callout tries: below, above, right, left.
    pub const DEFAULT: [Side; 4] = [Side::Below, Side::Above, Side::Right, Side::Left];
}

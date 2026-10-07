//! Arrow routing: A* on the cell grid inside a corridor between the callout and the anchor.
//!
//! Costs (design §3.4, with text made dearer): a blank cell 1, a dimmed cell 2, a blank cell
//! between words 6, a text cell 16, so a route goes round words whenever a blank way exists
//! within the corridor; a bend 3, and a third bend 20 more. Callouts, holes, protected cells, the anchor and both halves of a
//! wide grapheme can't be crossed.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::geom::{Rect, Side};

pub(crate) const BLANK: u32 = 1;
pub(crate) const DIMMED: u32 = 2;
pub(crate) const TEXT: u32 = 16;
/// A blank cell with text beside it on its row: a gap between words.
pub(crate) const GAP: u32 = 6;
const BEND: u32 = 3;
const THIRD_BEND: u32 = 20;
/// How far from the anchor's middle the arrow may leave the box.
const START_SPAN: u16 = 10;
/// How far the corridor reaches past the box edge and the anchor.
const CORRIDOR: u16 = 4;
/// A cell the router can't enter.
const NONE: u32 = u32::MAX;

/// A direction on the grid: which way an arrow enters or leaves a cell.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// What the router needs to know about a cell.
pub(crate) trait Field {
    /// The cost of entering the cell, or `None` if it can't be crossed.
    fn cost(&self, x: u16, y: u16) -> Option<u32>;
}

/// What routing found: a path, no way through, or (with a limit) only routes dearer than it.
#[derive(Debug, Clone)]
pub(crate) enum Routed {
    Found(Path),
    NoWay,
    Over,
}

impl Routed {
    pub(crate) fn path(self) -> Option<Path> {
        match self {
            Routed::Found(p) => Some(p),
            _ => None,
        }
    }
}

/// A routed arrow: its cells from the callout to the head, with the direction each is
/// entered and left.
#[derive(Debug, Clone)]
pub(crate) struct Path {
    pub cells: Vec<(u16, u16, Dir, Dir)>,
    pub junction: (u16, u16),
    pub cost: u32,
}

/// A monotone queue for small integer keys (Dial's buckets): `pop` returns the least key, and
/// among equal keys the first pushed, as a heap keyed by (key, push order) would, in O(1).
/// Keys pushed are never below the last key popped (a consistent heuristic's `f`, a
/// Dijkstra distance). Keys within [`RING`] of the current one go in a ring of buckets
/// (first-in-first-out lists in one arena); the rare farther ones wait in a heap and join the
/// ring as it reaches them.
struct Buckets<T> {
    nodes: Vec<(T, u32)>,
    head: [u32; RING as usize],
    tail: [u32; RING as usize],
    /// The key of the current bucket.
    cur: u32,
    len: usize,
    started: bool,
    far: BinaryHeap<Reverse<(u32, u32, T)>>,
    seq: u32,
}

const RING: u32 = 64;
const END: u32 = u32::MAX;

impl<T: Copy + Ord> Buckets<T> {
    fn with_capacity(n: usize) -> Buckets<T> {
        Buckets {
            nodes: Vec::with_capacity(n),
            head: [END; RING as usize],
            tail: [END; RING as usize],
            cur: 0,
            len: 0,
            started: false,
            far: BinaryHeap::new(),
            seq: 0,
        }
    }

    fn append(&mut self, key: u32, v: T) {
        let b = (key % RING) as usize;
        let n = self.nodes.len() as u32;
        self.nodes.push((v, END));
        match self.tail[b] {
            END => self.head[b] = n,
            t => self.nodes[t as usize].1 = n,
        }
        self.tail[b] = n;
    }

    fn push(&mut self, key: u32, v: T) {
        if !self.started {
            self.started = true;
            self.cur = key;
        }
        debug_assert!(key >= self.cur, "keys never go back");
        let key = key.max(self.cur);
        self.len += 1;
        if key - self.cur < RING {
            self.append(key, v);
        } else {
            self.seq += 1;
            self.far.push(Reverse((key, self.seq, v)));
        }
    }

    fn pop(&mut self) -> Option<(u32, T)> {
        if self.len == 0 {
            return None;
        }
        loop {
            let b = (self.cur % RING) as usize;
            let h = self.head[b];
            if h != END {
                let (v, next) = self.nodes[h as usize];
                self.head[b] = next;
                if next == END {
                    self.tail[b] = END;
                }
                self.len -= 1;
                return Some((self.cur, v));
            }
            self.cur += 1;
            // The key the ring now reaches: what waited for it joins, in push order.
            let edge = self.cur + RING - 1;
            while let Some(Reverse((k, _, v))) = self.far.peek().copied() {
                if k != edge {
                    break;
                }
                self.far.pop();
                self.append(edge, v);
            }
        }
    }
}

/// Whether an arrow ends at a cell: just outside `anchor`'s edge facing `side`.
fn is_goal(side: Side, anchor: Rect, x: u16, y: u16) -> bool {
    match side {
        Side::Above => y + 1 == anchor.y && x >= anchor.x && x < anchor.right(),
        Side::Below => y == anchor.bottom() && x >= anchor.x && x < anchor.right(),
        Side::Right => x == anchor.right() && y >= anchor.y && y < anchor.bottom(),
        Side::Left => x + 1 == anchor.x && y >= anchor.y && y < anchor.bottom(),
    }
}

/// The least an arrow can cost from each cell of `region` to `anchor`'s side: one search back
/// from the goal, shared by every candidate box on that side. Boxes and bends are left out, so
/// it never overestimates; the router uses it as its heuristic and placement as a bound.
pub(crate) struct ToGoal {
    region: Rect,
    dist: Vec<u32>,
}

impl ToGoal {
    pub(crate) fn new(field: &impl Field, side: Side, anchor: Rect, region: Rect) -> ToGoal {
        let (w, h) = (region.w as usize, region.h as usize);
        let mut cost = vec![NONE; w * h];
        let mut dist = vec![NONE; w * h];
        let mut heap: Buckets<u32> = Buckets::with_capacity(w * h * 2);
        for cy in 0..h {
            for cx in 0..w {
                let (x, y) = (region.x + cx as u16, region.y + cy as u16);
                if let Some(c) = field.cost(x, y) {
                    cost[cy * w + cx] = c;
                    if is_goal(side, anchor, x, y) {
                        dist[cy * w + cx] = 0;
                        heap.push(0, (cy * w + cx) as u32);
                    }
                }
            }
        }
        // From a cell, the way on costs what entering the next cell costs.
        while let Some((d, i)) = heap.pop() {
            let i = i as usize;
            if d > dist[i] {
                continue;
            }
            let nd = d + cost[i];
            let (cx, cy) = (i % w, i / w);
            let mut relax = |j: usize| {
                if cost[j] != NONE && nd < dist[j] {
                    dist[j] = nd;
                    heap.push(nd, j as u32);
                }
            };
            if cx > 0 {
                relax(i - 1);
            }
            if cx + 1 < w {
                relax(i + 1);
            }
            if cy > 0 {
                relax(i - w);
            }
            if cy + 1 < h {
                relax(i + w);
            }
        }
        ToGoal { region, dist }
    }

    /// The least cost on from a cell (`None`: no way), or nothing known outside the region.
    fn get(&self, x: u16, y: u16) -> Option<Option<u32>> {
        if !self.region.contains(x, y) {
            return None;
        }
        let d = self.dist
            [(y - self.region.y) as usize * self.region.w as usize + (x - self.region.x) as usize];
        Some((d != NONE).then_some(d))
    }

    /// The least a route from `boxr` on `side` can cost (its first cell, its offset and the
    /// way on), `None` if there's no way at all.
    pub(crate) fn bound(
        &self,
        field: &impl Field,
        boxr: Rect,
        side: Side,
        anchor: Rect,
    ) -> Option<u32> {
        starts(boxr, side, anchor)
            .into_iter()
            .filter_map(|((x, y), _, extra)| {
                let on = match self.get(x, y) {
                    Some(d) => d?,
                    None => 0,
                };
                Some(field.cost(x, y)? + extra + on)
            })
            .min()
    }
}

type Cell = (u16, u16);

/// The cells a route from `boxr` on `side` to `anchor` may use: the starts and the anchor,
/// with room round them.
pub(crate) fn reach(boxr: Rect, side: Side, anchor: Rect, area: Rect) -> Rect {
    corridor(&starts(boxr, side, anchor), anchor, area)
}

fn corridor(starts: &[(Cell, Cell, u32)], anchor: Rect, area: Rect) -> Rect {
    let edge = starts.iter().fold(Rect::default(), |r, s| {
        r.union(&Rect::new(s.0.0, s.0.1, 1, 1))
    });
    edge.union(&anchor).grow(CORRIDOR, CORRIDOR, &area)
}

/// Where an arrow may leave a box: the first cell outside it, its junction on the border,
/// and the extra cost of leaving there.
fn starts(boxr: Rect, side: Side, anchor: Rect) -> Vec<(Cell, Cell, u32)> {
    let mut starts: Vec<(Cell, Cell, u32)> = Vec::new();
    if boxr.w < 3 || boxr.h < 3 || anchor.is_empty() {
        return starts;
    }
    let jx = anchor.x + anchor.w.saturating_sub(1) / 2;
    let jy = anchor.y;
    match side {
        Side::Above | Side::Below => {
            let cx = jx.clamp(boxr.x + 1, boxr.right() - 2);
            for x in (boxr.x + 1).max(cx.saturating_sub(START_SPAN))
                ..(boxr.right() - 1).min(cx + START_SPAN + 1)
            {
                let (sy, jy) = if side == Side::Above {
                    (Some(boxr.bottom()), boxr.bottom() - 1)
                } else {
                    (boxr.y.checked_sub(1), boxr.y)
                };
                if let Some(sy) = sy {
                    starts.push(((x, sy), (x, jy), x.abs_diff(jx) as u32 / 2));
                }
            }
        }
        Side::Right | Side::Left => {
            let cy = jy.clamp(boxr.y + 1, boxr.bottom() - 2);
            for y in (boxr.y + 1).max(cy.saturating_sub(START_SPAN))
                ..(boxr.bottom() - 1).min(cy + START_SPAN + 1)
            {
                let (sx, jx) = if side == Side::Left {
                    (Some(boxr.right()), boxr.right() - 1)
                } else {
                    (boxr.x.checked_sub(1), boxr.x)
                };
                if let Some(sx) = sx {
                    starts.push(((sx, y), (jx, y), y.abs_diff(jy) as u32 * 2));
                }
            }
        }
    }
    starts
}

/// Routes from the edge of `boxr` (on `side` of the anchor) to the cell just outside
/// `anchor`'s facing edge. The arrow may leave the box anywhere along the facing edge (not a
/// corner); leaving away from the anchor's middle costs a little, so it stays close when the
/// way is clear.
///
/// `limit` stops the search once every route left would cost more than it ([`Routed::Over`]):
/// a caller scoring boxes needs no route that can't win. `u32::MAX` for none.
pub(crate) fn route(
    field: &impl Field,
    boxr: Rect,
    side: Side,
    anchor: Rect,
    area: Rect,
    limit: u32,
    to_goal: Option<&ToGoal>,
) -> Routed {
    if boxr.w < 3 || boxr.h < 3 || anchor.is_empty() {
        return Routed::NoWay;
    }
    let starts = starts(boxr, side, anchor);
    let dir = match side {
        Side::Above => Dir::Down,
        Side::Below => Dir::Up,
        Side::Right => Dir::Left,
        Side::Left => Dir::Right,
    };
    let arrive = dir;
    let goal = |x: u16, y: u16| is_goal(side, anchor, x, y);
    let corridor = corridor(&starts, anchor, area);
    // The heuristic: cells to the nearest goal cell (each costs at least 1).
    let cells_to = |x: u16, y: u16| -> u32 {
        let (gx, gy) = match side {
            Side::Above => (
                x.clamp(anchor.x, anchor.right() - 1),
                anchor.y.saturating_sub(1),
            ),
            Side::Below => (x.clamp(anchor.x, anchor.right() - 1), anchor.bottom()),
            Side::Right => (anchor.right(), y.clamp(anchor.y, anchor.bottom() - 1)),
            Side::Left => (
                anchor.x.saturating_sub(1),
                y.clamp(anchor.y, anchor.bottom() - 1),
            ),
        };
        (x.abs_diff(gx) + y.abs_diff(gy)) as u32
    };
    // Or, better, the least cost on ([`ToGoal`]); `None`: the goal can't be reached from there.
    let h = |x: u16, y: u16| -> Option<u32> {
        match to_goal.and_then(|t| t.get(x, y)) {
            Some(d) => d.map(|d| d.max(cells_to(x, y))),
            None => Some(cells_to(x, y)),
        }
    };
    let cw = corridor.w as usize;
    let ch = corridor.h as usize;
    let n = cw * ch * 4;
    // Each corridor cell's cost, asked of the field once.
    let mut cost_at = vec![NONE; cw * ch];
    for cy in 0..ch {
        for cx in 0..cw {
            if let Some(c) = field.cost(corridor.x + cx as u16, corridor.y + cy as u16) {
                cost_at[cy * cw + cx] = c;
            }
        }
    }
    // State: cell and the direction it was entered, as `cell * 4 + dir`. (Bends are costed as
    // they happen; the penalty for more than two is added to the finished route, for scoring.)
    let idx = |x: u16, y: u16, d: Dir| {
        ((y - corridor.y) as usize * cw + (x - corridor.x) as usize) * 4 + d.index()
    };
    let mut best = vec![u32::MAX; n];
    let mut prev = vec![u32::MAX; n];
    // Entries (state, g) by `f`: cheapest estimate first, first pushed among equals.
    let mut heap: Buckets<(u32, u32)> = Buckets::with_capacity(n);
    // The starts, least `f` first (in their order among equals), so keys never go back.
    let mut first: Vec<(u32, u32, u32)> = Vec::with_capacity(starts.len());
    for &((x, y), _, extra) in &starts {
        if !corridor.contains(x, y) {
            continue;
        }
        let c = cost_at[(y - corridor.y) as usize * cw + (x - corridor.x) as usize];
        if c == NONE {
            continue;
        }
        let g = c + extra;
        let i = idx(x, y, dir);
        let Some(hh) = h(x, y) else { continue };
        if g < best[i] {
            best[i] = g;
            first.push((g + hh, i as u32, g));
        }
    }
    first.sort_by_key(|e| e.0);
    for (f, i, g) in first {
        heap.push(f, (i, g));
    }
    let mut found = None;
    while let Some((f, (i, g))) = heap.pop() {
        // The heuristic never overestimates, so every route left costs at least `f`.
        if f > limit {
            return Routed::Over;
        }
        let i = i as usize;
        if g > best[i] {
            continue;
        }
        let d = Dir::ALL[i % 4];
        let cell = i / 4;
        let (cx, cy) = (cell % cw, cell / cw);
        let (x, y) = (cx as u16 + corridor.x, cy as u16 + corridor.y);
        if d == arrive && goal(x, y) {
            found = Some((i, g));
            break;
        }
        for nd in Dir::ALL {
            if nd == d.opposite() {
                continue;
            }
            let (nx, ny) = match nd {
                Dir::Up if cy > 0 => (cx, cy - 1),
                Dir::Down if cy + 1 < ch => (cx, cy + 1),
                Dir::Left if cx > 0 => (cx - 1, cy),
                Dir::Right if cx + 1 < cw => (cx + 1, cy),
                _ => continue,
            };
            let c = cost_at[ny * cw + nx];
            if c == NONE {
                continue;
            }
            let ng = g + c + if nd == d { 0 } else { BEND };
            let j = (ny * cw + nx) * 4 + nd.index();
            if ng < best[j] {
                let (px, py) = (nx as u16 + corridor.x, ny as u16 + corridor.y);
                let Some(hh) = h(px, py) else { continue };
                best[j] = ng;
                prev[j] = i as u32;
                heap.push(ng + hh, (j as u32, ng));
            }
        }
    }
    let Some((end, cost)) = found else {
        return Routed::NoWay;
    };
    let decode = |i: usize| {
        let d = Dir::ALL[i % 4];
        let c = i / 4;
        (
            (c % cw) as u16 + corridor.x,
            (c / cw) as u16 + corridor.y,
            d,
        )
    };
    let mut chain = vec![end];
    let mut i = end;
    while prev[i] != u32::MAX {
        i = prev[i] as usize;
        chain.push(i);
    }
    chain.reverse();
    let states: Vec<(u16, u16, Dir)> = chain.into_iter().map(decode).collect();
    let first = (states[0].0, states[0].1);
    let Some(junction) = starts.iter().find(|s| s.0 == first).map(|s| s.1) else {
        return Routed::NoWay;
    };
    let mut cells = Vec::with_capacity(states.len());
    for (k, &(x, y, din)) in states.iter().enumerate() {
        let dout = states.get(k + 1).map_or(arrive, |s| s.2);
        cells.push((x, y, din, dout));
    }
    let bends = cells.iter().filter(|c| c.2 != c.3).count();
    let cost = cost + if bends > 2 { THIRD_BEND } else { 0 };
    Routed::Found(Path {
        cells,
        junction,
        cost,
    })
}

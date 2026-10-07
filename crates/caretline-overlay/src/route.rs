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
/// How far the corridor reaches past the box edge and the anchor.
const CORRIDOR: u16 = 4;

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

    fn step(self, x: u16, y: u16) -> Option<(u16, u16)> {
        Some(match self {
            Dir::Up => (x, y.checked_sub(1)?),
            Dir::Down => (x, y.checked_add(1)?),
            Dir::Left => (x.checked_sub(1)?, y),
            Dir::Right => (x.checked_add(1)?, y),
        })
    }

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

/// A routed arrow: its cells from the callout to the head, with the direction each is
/// entered and left.
#[derive(Debug, Clone)]
pub(crate) struct Path {
    pub cells: Vec<(u16, u16, Dir, Dir)>,
    pub junction: (u16, u16),
    pub cost: u32,
}

/// Routes from the edge of `boxr` (on `side` of the anchor) to the cell just outside
/// `anchor`'s facing edge. The arrow may leave the box anywhere along the facing edge (not a
/// corner); leaving away from the anchor's middle costs a little, so it stays close when the
/// way is clear.
pub(crate) fn route(
    field: &dyn Field,
    boxr: Rect,
    side: Side,
    anchor: Rect,
    area: Rect,
) -> Option<Path> {
    if boxr.w < 3 || boxr.h < 3 || anchor.is_empty() {
        return None;
    }
    let jx = anchor.x + anchor.w.saturating_sub(1) / 2;
    let jy = anchor.y;
    // Each start: the first cell outside the box, its junction on the border, the extra cost.
    let mut starts: Vec<((u16, u16), (u16, u16), u32)> = Vec::new();
    let dir = match side {
        Side::Above => Dir::Down,
        Side::Below => Dir::Up,
        Side::Right => Dir::Left,
        Side::Left => Dir::Right,
    };
    match side {
        Side::Above | Side::Below => {
            for x in boxr.x + 1..boxr.right() - 1 {
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
            for y in boxr.y + 1..boxr.bottom() - 1 {
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
    let arrive = dir;
    let goal = |x: u16, y: u16| match side {
        Side::Above => y + 1 == anchor.y && x >= anchor.x && x < anchor.right(),
        Side::Below => y == anchor.bottom() && x >= anchor.x && x < anchor.right(),
        Side::Right => x == anchor.right() && y >= anchor.y && y < anchor.bottom(),
        Side::Left => x + 1 == anchor.x && y >= anchor.y && y < anchor.bottom(),
    };
    let edge = starts.iter().fold(Rect::default(), |r, s| {
        r.union(&Rect::new(s.0.0, s.0.1, 1, 1))
    });
    let corridor = edge.union(&anchor).grow(CORRIDOR, CORRIDOR, &area);
    // The heuristic: cells to the nearest goal cell (each costs at least 1).
    let h = |x: u16, y: u16| -> u32 {
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
    let cw = corridor.w as usize;
    let n = cw * corridor.h as usize * 16;
    // State: cell, the direction it was entered, bends so far (0..=3).
    let idx = |x: u16, y: u16, d: Dir, b: u8| {
        (((y - corridor.y) as usize * cw + (x - corridor.x) as usize) * 4 + d.index()) * 4
            + b as usize
    };
    let mut best = vec![u32::MAX; n];
    let mut prev = vec![u32::MAX; n];
    let mut heap = BinaryHeap::new();
    let mut seq = 0u32;
    for &((x, y), _, extra) in &starts {
        if !corridor.contains(x, y) {
            continue;
        }
        let Some(c) = field.cost(x, y) else { continue };
        let g = c + extra;
        let i = idx(x, y, dir, 0);
        if g < best[i] {
            best[i] = g;
            seq += 1;
            heap.push(Reverse((g + h(x, y), seq, x, y, dir, 0u8, g)));
        }
    }
    let mut found = None;
    while let Some(Reverse((_, _, x, y, d, b, g))) = heap.pop() {
        let i = idx(x, y, d, b);
        if g > best[i] {
            continue;
        }
        if goal(x, y) && d == arrive {
            found = Some((i, g));
            break;
        }
        for nd in Dir::ALL {
            if nd == d.opposite() {
                continue;
            }
            let Some((nx, ny)) = nd.step(x, y) else {
                continue;
            };
            if !corridor.contains(nx, ny) {
                continue;
            }
            let Some(c) = field.cost(nx, ny) else {
                continue;
            };
            let (nb, extra) = if nd == d {
                (b, 0)
            } else if b >= 2 {
                (3, BEND + if b == 2 { THIRD_BEND } else { 0 })
            } else {
                (b + 1, BEND)
            };
            let ng = g + c + extra;
            let j = idx(nx, ny, nd, nb);
            if ng < best[j] {
                best[j] = ng;
                prev[j] = i as u32;
                seq += 1;
                heap.push(Reverse((ng + h(nx, ny), seq, nx, ny, nd, nb, ng)));
            }
        }
    }
    let (end, cost) = found?;
    let decode = |i: usize| {
        let d = Dir::ALL[(i / 4) % 4];
        let c = i / 16;
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
    let junction = starts.iter().find(|s| s.0 == first).map(|s| s.1)?;
    let mut cells = Vec::with_capacity(states.len());
    for (k, &(x, y, din)) in states.iter().enumerate() {
        let dout = states.get(k + 1).map_or(arrive, |s| s.2);
        cells.push((x, y, din, dout));
    }
    Some(Path {
        cells,
        junction,
        cost,
    })
}

/// The glyph for a route cell entered going `din` and left going `dout`; `head` for the last.
pub(crate) fn glyph(din: Dir, dout: Dir, head: bool, ascii: bool) -> &'static str {
    if head {
        return match (din, ascii) {
            (Dir::Up, false) => "▲",
            (Dir::Down, false) => "▼",
            (Dir::Left, false) => "◀",
            (Dir::Right, false) => "▶",
            (Dir::Up, true) => "^",
            (Dir::Down, true) => "v",
            (Dir::Left, true) => "<",
            (Dir::Right, true) => ">",
        };
    }
    if din == dout {
        return match (din, ascii) {
            (Dir::Up | Dir::Down, false) => "│",
            (Dir::Left | Dir::Right, false) => "─",
            (Dir::Up | Dir::Down, true) => "|",
            (Dir::Left | Dir::Right, true) => "-",
        };
    }
    if ascii {
        return "+";
    }
    // A bend joins the side it came from and the side it leaves by.
    let from = din.opposite();
    let has = |d: Dir| from == d || dout == d;
    match (
        has(Dir::Up),
        has(Dir::Down),
        has(Dir::Left),
        has(Dir::Right),
    ) {
        (true, _, _, true) => "╰",
        (true, _, true, _) => "╯",
        (_, true, _, true) => "╭",
        _ => "╮",
    }
}

/// The junction glyph where a route leaves a box placed on `side` of its anchor.
pub(crate) fn junction(side: Side, ascii: bool) -> &'static str {
    if ascii {
        return "+";
    }
    match side {
        Side::Above => "┬",
        Side::Below => "┴",
        Side::Right => "┤",
        Side::Left => "├",
    }
}

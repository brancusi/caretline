//! Anchors to cells: the [`Resolve`] trait, the host's [`AnchorMap`], and [`Chain`] to use
//! several at once.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::geom::Rect;
use crate::model::Anchor;

/// Which way an anchor that isn't on screen lies, and roughly where along that edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Off {
    /// Above the first row; `x` its column if known.
    Above {
        x: Option<u16>,
    },
    Below {
        x: Option<u16>,
    },
    /// Left of the first column (a scrolled line); `y` its row.
    Left {
        y: u16,
    },
    Right {
        y: u16,
    },
}

/// Where an anchor is this frame: the cells it covers (several when text wraps), or which way
/// it lies when it isn't visible, and the view it resolved in (wire `in`), when a
/// [`FrameResolver`](crate::FrameResolver) with an id answered.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rects: Vec<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<Off>,
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
}

impl Resolved {
    pub fn at(rects: Vec<Rect>) -> Resolved {
        Resolved {
            rects,
            off: None,
            view: None,
        }
    }

    pub fn off(off: Off) -> Resolved {
        Resolved {
            rects: Vec::new(),
            off: Some(off),
            view: None,
        }
    }

    /// The same, resolved in `view`.
    pub fn in_view(mut self, view: Option<&str>) -> Resolved {
        self.view = view.map(String::from);
        self
    }

    /// Whether any of its cells show.
    pub fn visible(&self) -> bool {
        !self.rects.is_empty()
    }

    /// The bounding rectangle of its cells.
    pub fn bounds(&self) -> Rect {
        Rect::bounds(&self.rects)
    }
}

/// Turns anchors into cells. `None` means "not mine, or not found": with [`Chain`], the
/// others are asked.
pub trait Resolve {
    fn resolve(&self, anchor: &Anchor) -> Option<Resolved>;

    /// Whether this resolves the view the host has focused: [`Chain`] asks it first, and takes
    /// its off-screen answer over another view's. Default: no.
    fn is_focused(&self) -> bool {
        false
    }
}

impl<R: Resolve + ?Sized> Resolve for &R {
    fn resolve(&self, anchor: &Anchor) -> Option<Resolved> {
        (**self).resolve(anchor)
    }

    fn is_focused(&self) -> bool {
        (**self).is_focused()
    }
}

/// A host anchor's key: a kind the host names and a key within it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnchorKey {
    pub kind: String,
    pub key: String,
}

impl AnchorKey {
    pub fn host(kind: &str, key: &str) -> AnchorKey {
        AnchorKey {
            kind: kind.into(),
            key: key.into(),
        }
    }
}

/// What a host records while it draws: where each of its anchor keys landed. Resolves
/// [`Anchor::Host`]. Rebuilt every frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnchorMap {
    map: BTreeMap<AnchorKey, Resolved>,
}

impl AnchorMap {
    pub fn new() -> AnchorMap {
        AnchorMap::default()
    }

    /// Records cells for a key (call again to add a wrapped row).
    pub fn put(&mut self, key: AnchorKey, rect: Rect) {
        let r = self.map.entry(key).or_default();
        r.off = None;
        r.rects.push(rect);
    }

    /// Records that a key's target is off screen, that way.
    pub fn put_off(&mut self, key: AnchorKey, off: Off) {
        self.map.insert(key, Resolved::off(off));
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn get(&self, key: &AnchorKey) -> Option<&Resolved> {
        self.map.get(key)
    }
}

impl Resolve for AnchorMap {
    fn resolve(&self, anchor: &Anchor) -> Option<Resolved> {
        match anchor {
            Anchor::Host { kind, key } => self
                .map
                .get(&AnchorKey {
                    kind: kind.clone(),
                    key: key.clone(),
                })
                .cloned(),
            _ => None,
        }
    }
}

/// Several resolvers, such as a host's [`AnchorMap`] and one
/// [`FrameResolver`](crate::FrameResolver) per view of a document:
/// `Chain(vec![&anchors, &main, &panel])`. Of the answers, the first that holds:
///
/// 1. the focused resolver's ([`Resolve::is_focused`]), if its cells show;
/// 2. the first whose cells show, in order;
/// 3. the focused resolver's off-screen direction;
/// 4. the first off-screen direction, in order.
///
/// A scoped anchor ([`Anchor::In`](crate::Anchor::In)) is answered only by the view it
/// names, so this order is for unscoped ones.
pub struct Chain<'a>(pub Vec<&'a dyn Resolve>);

impl Resolve for Chain<'_> {
    fn resolve(&self, anchor: &Anchor) -> Option<Resolved> {
        let (mut shown, mut focused_off, mut off) = (None, None, None);
        for r in &self.0 {
            let Some(a) = r.resolve(anchor) else { continue };
            let focused = r.is_focused();
            if a.visible() {
                if focused {
                    return Some(a);
                }
                shown.get_or_insert(a);
            } else if focused {
                focused_off.get_or_insert(a);
            } else {
                off.get_or_insert(a);
            }
        }
        shown.or(focused_off).or(off)
    }

    fn is_focused(&self) -> bool {
        self.0.iter().any(|r| r.is_focused())
    }
}

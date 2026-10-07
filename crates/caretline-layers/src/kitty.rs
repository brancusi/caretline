//! Pixel plumbing for the kitty graphics protocol (feature `kitty`).
//!
//! The host rasterises: for each layer part it wants in pixels (a panel under the words, an
//! arrow, a ring, a veil) it hands over a [`Picture`]: straight-alpha RGBA, a shape key, the
//! cells the image covers, and whether it goes below or above the text. [`KittyState::frame`]
//! turns the [`Plan`] and those pictures into the APC bytes that make the terminal show them,
//! to be written after the text frame inside the same synchronized update (DEC mode 2026).
//! The crate does no I/O: it never reads the terminal, never writes a file itself, and never
//! looks at a clock.
//!
//! - **Ids from content.** An image id is a hash of the picture's shape key and the cell size;
//!   a placement id a hash of (layer id, part). Never counters, so the same frames give the
//!   same bytes, on replay and on every machine. A collision is resolved by rehashing with a
//!   fixed salt.
//! - **Moves** are a re-place (`a=p`) with the same image and placement ids: no pixels sent.
//! - **Changes** transmit the new image, place it, then delete the old one (`a=d,d=I`), all in
//!   one update, so nothing flickers.
//! - **Gone** parts are deleted by id (`a=d`), never `d=A`: the screen may hold other
//!   programs' images.
//! - **Crops:** a picture that reaches past the screen (or its `clip`) is placed with a
//!   source rect (`x,y,w,h`), so a tall image scrolls by re-cropping.
//! - **Transport:** `t=d`, zlib level 6 (`o=z`), base64 in 4096-byte chunks by default;
//!   `t=t` (a temporary file) through a [`TempFiles`] writer the host supplies.
//!
//! [`KittyState`] is a renderer cache: what the terminal holds, derived only from the frames
//! drawn, never part of the layers' state, and safe to drop ([`KittyState::reset`]) at the
//! cost of one full re-send.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::geom::Rect;
use crate::place::Plan;

/// A cell's size in device pixels (what `CSI 16 t` reports). It arrives as a value, like a
/// message: the crate never asks the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellPx {
    pub w: u16,
    pub h: u16,
}

impl CellPx {
    pub const fn new(w: u16, h: u16) -> CellPx {
        CellPx { w, h }
    }
}

/// Pixels: `w`×`h`, row by row, 4 bytes each (red, green, blue, alpha; straight alpha).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<[u8]>,
}

impl Image {
    /// `None` when the data isn't `w * h * 4` bytes or the image is empty.
    pub fn new(w: u32, h: u32, rgba: impl Into<Arc<[u8]>>) -> Option<Image> {
        let rgba = rgba.into();
        (w > 0 && h > 0 && rgba.len() == w as usize * h as usize * 4).then_some(Image {
            w,
            h,
            rgba,
        })
    }
}

/// Whether a picture goes under the text (above cell backgrounds) or over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Z {
    Below,
    Above,
}

/// A box of cells that may reach past the screen: `x` and `y` can be negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cells {
    pub x: i32,
    pub y: i32,
    pub w: u16,
    pub h: u16,
}

impl Cells {
    pub const fn new(x: i32, y: i32, w: u16, h: u16) -> Cells {
        Cells { x, y, w, h }
    }
}

impl From<Rect> for Cells {
    fn from(r: Rect) -> Cells {
        Cells::new(r.x as i32, r.y as i32, r.w, r.h)
    }
}

/// One image for one part of a layer, as the host rasterised it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// The layer it belongs to: shown only while the plan places that layer.
    pub layer: String,
    /// Which part of the layer (`"panel"`, `"arrow"`, …): one picture per (layer, part).
    pub part: String,
    /// The shape key: a hash of everything the pixels came from. The same key means the same
    /// pixels, so the terminal keeps them and only the placement moves.
    pub key: u64,
    pub z: Z,
    /// The cells the whole image covers; the terminal scales it to fit.
    pub at: Cells,
    /// The cells it may show in (default: the plan's screen). The rest is cropped off.
    pub clip: Option<Rect>,
    /// The pixels. May be `None` when [`KittyState::holds`] the key: the host need not
    /// rasterise again.
    pub image: Option<Image>,
}

/// How pixels reach the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// `t=d`: inline, zlib-compressed and base64 encoded, in chunks. Works over SSH.
    #[default]
    Direct,
    /// `t=t`: a temporary file the terminal reads and deletes, written by the host's
    /// [`TempFiles`]. Local sessions only. Falls back to `Direct` for a file not written.
    File,
}

/// Transmission options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub transport: Transport,
    /// Base64 bytes per `t=d` chunk (the protocol's limit is 4096).
    pub chunk: usize,
    /// zlib level, 0–10.
    pub level: u8,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            transport: Transport::Direct,
            chunk: 4096,
            level: 6,
        }
    }
}

/// The host's temporary-file writer for `t=t`, so I/O stays outside the crate. Ghostty reads
/// only files in its temp directory whose name contains `tty-graphics-protocol`; `name` does.
pub trait TempFiles {
    /// Writes `data` to a file called `name` in the temp directory and returns its full path,
    /// or `None` if it couldn't.
    fn write(&mut self, name: &str, data: &[u8]) -> Option<String>;
}

/// What one frame sends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// The APC bytes (with the cursor saved and restored round them), empty when nothing
    /// changed.
    pub bytes: Vec<u8>,
    /// Image ids transmitted.
    pub sent: Vec<u32>,
    /// Placements made or moved.
    pub placed: usize,
    /// Images or placements deleted.
    pub deleted: usize,
    /// Pictures that came without pixels the terminal doesn't hold: (layer, part). Not shown.
    pub missing: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Place {
    col: u16,
    row: u16,
    c: u16,
    r: u16,
    z: i32,
    crop: Option<(u32, u32, u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held {
    key: u64,
    w: u32,
    h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    image: u32,
    id: u32,
    at: Place,
}

/// The kitty renderer cache: which images the terminal holds and where each part is placed.
/// Derived only from the frames drawn; drop it ([`KittyState::reset`]) and the next frame
/// sends everything again.
#[derive(Debug, Clone, Default)]
pub struct KittyState {
    opts: Options,
    /// Image id → what the terminal holds there.
    images: BTreeMap<u32, Held>,
    /// (layer, part) → its placement.
    placed: BTreeMap<(String, String), Placed>,
}

const SALT: u64 = 0x9e37_79b9_7f4a_7c15;
/// Below text, above non-default cell backgrounds: from here up.
const Z_BELOW: i32 = -(1 << 20);
const Z_ABOVE: i32 = 1;

fn fnv(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in *p {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // A separator, so ("ab", "c") and ("a", "bc") differ.
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A non-zero 32-bit id from a hash.
fn id32(h: u64) -> u32 {
    let v = (h ^ (h >> 32)) as u32;
    if v == 0 { 1 } else { v }
}

impl KittyState {
    pub fn new(opts: Options) -> KittyState {
        KittyState {
            opts,
            ..KittyState::default()
        }
    }

    pub fn options(&self) -> Options {
        self.opts
    }

    /// Changes how later images are sent. Images already held stay.
    pub fn set_options(&mut self, opts: Options) {
        self.opts = opts;
    }

    /// Whether the terminal holds the pixels for this shape key at this cell size: a picture
    /// with it needs no image.
    pub fn holds(&self, key: u64, cell: CellPx) -> bool {
        let id = self.image_id(key, cell);
        self.images.get(&id).is_some_and(|h| h.key == key)
    }

    /// The image id a shape key gets at a cell size: a hash, rehashed with a fixed salt while
    /// it collides with a different key the terminal holds.
    pub fn image_id(&self, key: u64, cell: CellPx) -> u32 {
        let mut h = fnv(&[
            &key.to_le_bytes(),
            &cell.w.to_le_bytes(),
            &cell.h.to_le_bytes(),
        ]);
        loop {
            let id = id32(h);
            match self.images.get(&id) {
                Some(held) if held.key != key => h = fnv(&[&h.to_le_bytes(), &SALT.to_le_bytes()]),
                _ => return id,
            }
        }
    }

    /// Whether the terminal holds nothing of ours.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.placed.is_empty()
    }

    /// Forgets what the terminal holds, without telling it. The next frame sends everything.
    pub fn reset(&mut self) {
        self.images.clear();
        self.placed.clear();
    }

    /// The bytes that delete every image this state placed (on exit, or when pixels are
    /// turned off), and forgets them.
    pub fn clear(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for id in self.images.keys() {
            delete_image(&mut out, *id);
        }
        self.reset();
        out
    }

    /// One frame: transmits what the terminal lacks, places what's new, moves what moved,
    /// deletes what's gone. Pictures show in the plan's draw order (then the order given);
    /// a picture for a layer the plan doesn't place is left out. `files` is needed only for
    /// [`Transport::File`].
    pub fn frame(
        &mut self,
        plan: &Plan,
        pictures: &[Picture],
        cell: CellPx,
        mut files: Option<&mut dyn TempFiles>,
    ) -> Output {
        let order: BTreeMap<&str, usize> = plan
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id.as_str(), i))
            .collect();
        let screen = Rect::new(0, 0, plan.width, plan.height);
        // The pictures shown, in draw order; a repeated (layer, part) keeps the last.
        let mut shown: Vec<(usize, usize, &Picture)> = Vec::new();
        let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for (i, p) in pictures.iter().enumerate() {
            let Some(&rank) = order.get(p.layer.as_str()) else {
                continue;
            };
            match seen.get(&(p.layer.as_str(), p.part.as_str())) {
                Some(&k) => shown[k] = (rank, i, p),
                None => {
                    seen.insert((p.layer.as_str(), p.part.as_str()), shown.len());
                    shown.push((rank, i, p));
                }
            }
        }
        shown.sort_by_key(|(rank, i, _)| (*rank, *i));

        let mut out = Output::default();
        let mut body = Vec::new();
        let mut next: BTreeMap<(String, String), Placed> = BTreeMap::new();
        let mut ids: BTreeSet<u32> = BTreeSet::new();
        let (mut below, mut above) = (0i32, 0i32);
        for (_, _, p) in shown {
            let clip = p.clip.map_or(screen, |c| c.intersection(&screen));
            if placement(p.at, &clip, (1, 1)).is_none() {
                continue;
            }
            let image = self.image_id(p.key, cell);
            if !self.images.get(&image).is_some_and(|h| h.key == p.key) {
                let Some(img) = &p.image else {
                    out.missing.push((p.layer.clone(), p.part.clone()));
                    continue;
                };
                self.transmit(&mut body, image, p.key, img, files.as_deref_mut());
                self.images.insert(
                    image,
                    Held {
                        key: p.key,
                        w: img.w,
                        h: img.h,
                    },
                );
                out.sent.push(image);
            }
            let held = self.images[&image];
            let Some(at) = placement(p.at, &clip, (held.w, held.h)) else {
                continue;
            };
            let z = match p.z {
                Z::Below => {
                    below += 1;
                    Z_BELOW + below
                }
                Z::Above => {
                    above += 1;
                    Z_ABOVE + above
                }
            };
            let at = Place { z, ..at };
            let key = (p.layer.clone(), p.part.clone());
            let mut h = fnv(&[p.layer.as_bytes(), p.part.as_bytes()]);
            let mut id = id32(h);
            while !ids.insert(id) {
                h = fnv(&[&h.to_le_bytes(), &SALT.to_le_bytes()]);
                id = id32(h);
            }
            let now = Placed { image, id, at };
            if self.placed.get(&key) != Some(&now) {
                place(&mut body, &now);
                out.placed += 1;
            }
            next.insert(key, now);
        }

        // Deletes go last, after every new placement: a swapped part never shows a gap.
        let used: BTreeSet<u32> = next.values().map(|p| p.image).collect();
        let pairs: BTreeSet<(u32, u32)> = next.values().map(|p| (p.image, p.id)).collect();
        let unused: Vec<u32> = self
            .images
            .keys()
            .copied()
            .filter(|i| !used.contains(i))
            .collect();
        for id in &unused {
            delete_image(&mut body, *id);
            self.images.remove(id);
            out.deleted += 1;
        }
        for old in self.placed.values() {
            if used.contains(&old.image) && !pairs.contains(&(old.image, old.id)) {
                let _ = write!(
                    Text(&mut body),
                    "\x1b_Ga=d,d=i,i={},p={},q=2\x1b\\",
                    old.image,
                    old.id
                );
                out.deleted += 1;
            }
        }
        self.placed = next;
        if !body.is_empty() {
            // Save and restore the cursor: placing moves it.
            out.bytes.extend_from_slice(b"\x1b7");
            out.bytes.extend_from_slice(&body);
            out.bytes.extend_from_slice(b"\x1b8");
        }
        out
    }

    fn transmit(
        &self,
        out: &mut Vec<u8>,
        id: u32,
        key: u64,
        img: &Image,
        files: Option<&mut (dyn TempFiles + '_)>,
    ) {
        let (w, h) = (img.w, img.h);
        if self.opts.transport == Transport::File
            && let Some(files) = files
        {
            let name = format!("caretline-tty-graphics-protocol-{id:08x}-{key:016x}.rgba");
            if let Some(path) = files.write(&name, &img.rgba) {
                let _ = write!(
                    Text(out),
                    "\x1b_Ga=t,f=32,s={w},v={h},i={id},t=t,q=2;{}\x1b\\",
                    base64(path.as_bytes())
                );
                return;
            }
        }
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&img.rgba, self.opts.level.min(10));
        let b = base64(&z);
        let size = self.opts.chunk.clamp(4, 4096) / 4 * 4;
        let chunks: Vec<&[u8]> = b.as_bytes().chunks(size).collect();
        for (i, c) in chunks.iter().enumerate() {
            let m = (i + 1 < chunks.len()) as u8;
            if i == 0 {
                let _ = write!(
                    Text(out),
                    "\x1b_Ga=t,f=32,s={w},v={h},i={id},o=z,q=2,m={m};"
                );
            } else {
                let _ = write!(Text(out), "\x1b_Gm={m};");
            }
            out.extend_from_slice(c);
            out.extend_from_slice(b"\x1b\\");
        }
    }
}

/// Where a picture shows: the cells of `a` inside `clip`, scaled, with the source rect (in
/// the image's `px`) of the part that shows when some of it is cropped off. `None` when none
/// of it shows.
fn placement(a: Cells, clip: &Rect, px: (u32, u32)) -> Option<Place> {
    if a.w == 0 || a.h == 0 {
        return None;
    }
    let (ax0, ay0) = (a.x as i64, a.y as i64);
    let (ax1, ay1) = (ax0 + a.w as i64, ay0 + a.h as i64);
    let x0 = ax0.max(clip.x as i64);
    let y0 = ay0.max(clip.y as i64);
    let x1 = ax1.min(clip.right() as i64);
    let y1 = ay1.min(clip.bottom() as i64);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let crop = if (x0, y0, x1, y1) == (ax0, ay0, ax1, ay1) {
        None
    } else {
        let (iw, ih) = (px.0 as i64, px.1 as i64);
        let sx = |c: i64| (c - ax0) * iw / a.w as i64;
        let sy = |c: i64| (c - ay0) * ih / a.h as i64;
        let (px0, py0, px1, py1) = (sx(x0), sy(y0), sx(x1), sy(y1));
        Some((
            px0 as u32,
            py0 as u32,
            (px1 - px0).max(1) as u32,
            (py1 - py0).max(1) as u32,
        ))
    };
    Some(Place {
        col: x0 as u16,
        row: y0 as u16,
        c: (x1 - x0) as u16,
        r: (y1 - y0) as u16,
        z: 0,
        crop,
    })
}

fn place(out: &mut Vec<u8>, p: &Placed) {
    let a = &p.at;
    let mut t = Text(out);
    let _ = write!(
        t,
        "\x1b[{};{}H\x1b_Ga=p,i={},p={},c={},r={},z={},C=1,q=2",
        a.row as u32 + 1,
        a.col as u32 + 1,
        p.image,
        p.id,
        a.c,
        a.r,
        a.z
    );
    if let Some((x, y, w, h)) = a.crop {
        let _ = write!(t, ",x={x},y={y},w={w},h={h}");
    }
    let _ = write!(t, "\x1b\\");
}

fn delete_image(out: &mut Vec<u8>, id: u32) {
    let _ = write!(Text(out), "\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
}

/// `fmt::Write` into a byte buffer.
struct Text<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Text<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// Standard base64, padded.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// A stable 64-bit hash (FNV-1a) of some byte strings, for shape keys: the same inputs give
/// the same key on every machine and every Rust version (unlike `std`'s hashers).
pub fn shape_key(parts: &[&[u8]]) -> u64 {
    fnv(parts)
}

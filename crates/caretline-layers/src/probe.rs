//! The pixel probe (feature `kitty`): the bytes that ask a terminal what it can do, and a
//! pure parser for its replies, so a runtime's own input parser can turn them into messages
//! instead of keys. The crate does no terminal I/O: the runtime writes [`request`] and feeds
//! what it reads to [`scan`].
//!
//! The request asks, in order:
//! 1. a kitty graphics query (`a=q`) on a 1×1 image: `OK` if the terminal shows images;
//! 2. XTVERSION (`CSI > q`): the terminal's name and version;
//! 3. `CSI 16 t`: the cell size in device pixels;
//! 4. DA1 (`CSI c`), which every terminal answers: the fence. A reply missing by then never
//!    comes.

use crate::kitty::CellPx;

/// The image id the graphics query uses.
pub const QUERY_ID: u32 = 31;

/// The probe: a graphics query, XTVERSION, the cell size, and DA1 as the fence.
pub fn request() -> Vec<u8> {
    format!("\x1b_Gi={QUERY_ID},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[>0q\x1b[16t\x1b[c").into_bytes()
}

/// Asks only for the cell size (after a font-size change), with no fence.
pub fn cell_size_request() -> &'static [u8] {
    b"\x1b[16t"
}

/// A terminal's reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A kitty graphics answer: `ESC _ G i=<id>;OK ESC \` or an error message.
    Graphics {
        id: Option<u32>,
        ok: bool,
        message: String,
    },
    /// XTVERSION: `ESC P > | <name and version> ESC \`.
    Version(String),
    /// `CSI 6 ; h ; w t`: the cell size in pixels.
    CellSize(CellPx),
    /// `CSI 4 ; h ; w t`: the text area's size in pixels.
    WindowSize { w: u32, h: u32 },
    /// DA1: `CSI ? <attrs> c`.
    Da1(Vec<u32>),
}

/// What the start of some input is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scan {
    /// A reply, and how many bytes it took.
    Reply(Reply, usize),
    /// It could still become a reply: wait for more bytes (or, after a pause, treat them as
    /// keys).
    Partial,
    /// Not a reply: a key, a mouse report, a paste or text.
    No,
}

/// Reads a reply at the start of `input`.
pub fn scan(input: &[u8]) -> Scan {
    match input {
        [] | [0x1b] => Scan::Partial,
        [0x1b, b'_', rest @ ..] => match rest {
            [] => Scan::Partial,
            [b'G', ..] => string(input, 2).map_or(Scan::Partial, |(body, n)| {
                Scan::Reply(graphics(&body[1..]), n)
            }),
            _ => Scan::No,
        },
        [0x1b, b'P', rest @ ..] => match rest {
            [] | [b'>'] => Scan::Partial,
            [b'>', b'|', ..] => string(input, 2).map_or(Scan::Partial, |(body, n)| {
                Scan::Reply(
                    Reply::Version(String::from_utf8_lossy(&body[2..]).into_owned()),
                    n,
                )
            }),
            _ => Scan::No,
        },
        [0x1b, b'[', rest @ ..] => csi(rest),
        _ => Scan::No,
    }
}

/// A string ended by ST (`ESC \`) after `start`: its body and the bytes taken.
fn string(input: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let end = input[start..].windows(2).position(|w| w == b"\x1b\\")?;
    Some((&input[start..start + end], start + end + 2))
}

fn graphics(body: &[u8]) -> Reply {
    let body = String::from_utf8_lossy(body);
    let (keys, message) = body.split_once(';').unwrap_or((&body, ""));
    let id = keys
        .split(',')
        .find_map(|kv| kv.strip_prefix("i="))
        .and_then(|v| v.parse().ok());
    Reply::Graphics {
        id,
        ok: message == "OK",
        message: message.to_string(),
    }
}

fn csi(rest: &[u8]) -> Scan {
    // Parameters and intermediates, then a final byte.
    let Some(end) = rest.iter().position(|b| (0x40..=0x7e).contains(b)) else {
        return if rest.iter().all(|b| (0x20..=0x3f).contains(b)) {
            Scan::Partial
        } else {
            Scan::No
        };
    };
    let (params, fin) = (&rest[..end], rest[end]);
    if params.iter().any(|b| !(0x20..=0x3f).contains(b)) {
        return Scan::No;
    }
    let n = 2 + end + 1;
    let text = String::from_utf8_lossy(params);
    match fin {
        b'c' if text.starts_with('?') => {
            let attrs = text[1..]
                .split(';')
                .filter_map(|s| s.parse().ok())
                .collect();
            Scan::Reply(Reply::Da1(attrs), n)
        }
        b't' => {
            let nums: Vec<u32> = text.split(';').map(|s| s.parse().unwrap_or(0)).collect();
            match nums.as_slice() {
                [6, h, w] => Scan::Reply(
                    Reply::CellSize(CellPx::new(
                        (*w).min(u16::MAX as u32) as u16,
                        (*h).min(u16::MAX as u32) as u16,
                    )),
                    n,
                ),
                [4, h, w] => Scan::Reply(Reply::WindowSize { w: *w, h: *h }, n),
                _ => Scan::No,
            }
        }
        _ => Scan::No,
    }
}

/// The probe's answers so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    /// The graphics query's answer: `Some(true)` for OK.
    pub graphics: Option<bool>,
    /// What XTVERSION said.
    pub version: Option<String>,
    pub cell: Option<CellPx>,
    /// DA1 came back: every other answer is in.
    pub fenced: bool,
}

impl Probe {
    /// Takes in a reply. Returns whether it belonged to the probe.
    pub fn add(&mut self, reply: &Reply) -> bool {
        match reply {
            Reply::Graphics { id, ok, .. } if *id == Some(QUERY_ID) || id.is_none() => {
                self.graphics = Some(*ok);
            }
            Reply::Version(v) => self.version = Some(v.clone()),
            Reply::CellSize(c) if c.w > 0 && c.h > 0 => self.cell = Some(*c),
            Reply::Da1(_) => self.fenced = true,
            _ => return false,
        }
        true
    }

    /// The terminal's name, lowercased (`ghostty`, `kitty`, …), from XTVERSION.
    pub fn terminal(&self) -> Option<String> {
        let v = self.version.as_ref()?;
        let name = v.split([' ', '(']).next()?.to_ascii_lowercase();
        (!name.is_empty()).then_some(name)
    }

    /// Whether pixels can be on: the graphics query said OK and the cell size came back.
    /// Which terminals to trust is the host's call ([`Probe::terminal`]).
    pub fn graphics_ok(&self) -> bool {
        self.graphics == Some(true) && self.cell.is_some()
    }
}

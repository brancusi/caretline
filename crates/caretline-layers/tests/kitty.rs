#![cfg(all(feature = "kitty", feature = "caretline"))]
//! The kitty plumbing: byte goldens for a first frame, a scroll (re-place only), a changed
//! part (transmit, place, then delete the old image), a crop at the area's edge and removal;
//! replay gives the same bytes; the probe parser reads every reply.

mod common;

use caretline_layers::kitty::*;
use caretline_layers::probe::{self, Probe, Reply, Scan};
use caretline_layers::*;
use common::golden;
use serde_json::{Value, json};

const CELL: CellPx = CellPx::new(4, 8);

fn renderers() -> Renderers {
    Renderers::new().register("card", |_: &Value, avail: Size| Size::new(10.min(avail.w), 4))
}

/// A one-layer plan: a card under a host anchor at row `y`.
fn plan_at(y: u16, with_layer: bool) -> Plan {
    let mut m = AnchorMap::new();
    m.put(AnchorKey::host("row", "a"), Rect::new(6, y, 5, 1));
    let mut layers = Layers::default();
    if with_layer {
        let mut l = Layer::new(Anchor::Host { kind: "row".into(), key: "a".into() })
            .with_content(Content::new("card", json!({"text": "hi"})))
            .with_arrow();
        l.id = "tip".into();
        apply(&mut layers, LayerOp::Push(l), None, 0, &Limits::default()).unwrap();
    }
    plan(&layers, &m, &Grid::new(80, 24), &renderers())
}

/// A solid image of `w`×`h` cells at the test cell size, 1 px per cell (scaled up by the
/// terminal), so goldens stay short.
fn solid(w: u16, h: u16, rgba: [u8; 4]) -> Image {
    let n = w as usize * h as usize;
    Image::new(w as u32, h as u32, rgba.repeat(n)).unwrap()
}

fn pic(part: &str, key: u64, z: Z, at: Cells, image: Image) -> Picture {
    Picture { layer: "tip".into(), part: part.into(), key, z, at, clip: None, image: Some(image) }
}

/// The host's pictures for the plan: a panel under the box and a ring over the anchor.
fn pictures(p: &Plan, panel_color: [u8; 4]) -> Vec<Picture> {
    let Some(l) = p.layers.first() else { return Vec::new() };
    let r = l.rect.unwrap();
    let a = l.anchor.as_ref().unwrap().rects[0];
    let panel_key = shape_key(&[b"panel", &panel_color, &r.w.to_le_bytes(), &r.h.to_le_bytes()]);
    vec![
        pic("panel", panel_key, Z::Below, Cells::new(r.x as i32 - 1, r.y as i32, r.w + 2, r.h), solid(r.w + 2, r.h, panel_color)),
        pic("ring", shape_key(&[b"ring", &a.w.to_le_bytes()]), Z::Above, Cells::new(a.x as i32 - 1, a.y as i32, a.w + 2, 1), solid(a.w + 2, 1, [255, 255, 255, 128])),
    ]
}

/// The bytes as text: escapes visible, one command per line.
fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b).replace("\x1b\\", "\x1b\\\n").replace('\x1b', "⎋")
}

const BLUE: [u8; 4] = [31, 36, 48, 250];
const GREEN: [u8; 4] = [20, 80, 40, 250];

/// first → scroll by one row → recolour the panel → remove the layer.
fn sequence(k: &mut KittyState) -> Vec<Output> {
    let mut outs = Vec::new();
    let p0 = plan_at(3, true);
    outs.push(k.frame(&p0, &pictures(&p0, BLUE), CELL, None));
    // Scrolled: the anchor is a row up; the host passes no pixels for what the terminal holds.
    let p1 = plan_at(2, true);
    let mut pics = pictures(&p1, BLUE);
    for pi in &mut pics {
        assert!(k.holds(pi.key, CELL));
        pi.image = None;
    }
    outs.push(k.frame(&p1, &pics, CELL, None));
    outs.push(k.frame(&p1, &pictures(&p1, GREEN), CELL, None));
    let gone = plan_at(2, false);
    outs.push(k.frame(&gone, &[], CELL, None));
    outs
}

#[test]
fn byte_goldens_first_scroll_change_remove() {
    let mut k = KittyState::new(Options::default());
    let outs = sequence(&mut k);
    let names = ["first", "scroll", "change", "remove"];
    for (o, n) in outs.iter().zip(names) {
        golden(&format!("kitty.{n}.txt"), &show(&o.bytes));
    }
    // A first frame transmits both images and places them.
    assert_eq!((outs[0].sent.len(), outs[0].placed, outs[0].deleted), (2, 2, 0));
    // A scroll re-places only: no pixels, nothing deleted.
    assert!(outs[1].sent.is_empty() && outs[1].deleted == 0 && outs[1].placed == 2);
    assert!(!show(&outs[1].bytes).contains("a=t"));
    // A changed panel: sent, placed, then the old image deleted, in that order.
    let s = show(&outs[2].bytes);
    assert_eq!((outs[2].sent.len(), outs[2].placed, outs[2].deleted), (1, 1, 1));
    let (t, p, d) = (s.find("a=t").unwrap(), s.find("a=p").unwrap(), s.find("a=d,d=I").unwrap());
    assert!(t < p && p < d, "{s}");
    // Removed: both images deleted by id, never d=A.
    assert_eq!(outs[3].deleted, 2);
    assert!(!show(&outs[3].bytes).contains("d=A"));
    assert!(k.is_empty());
    // An unchanged frame sends nothing at all.
    let p = plan_at(3, true);
    k.frame(&p, &pictures(&p, BLUE), CELL, None);
    assert!(k.frame(&p, &pictures(&p, BLUE), CELL, None).bytes.is_empty());
}

#[test]
fn replay_gives_identical_bytes_and_a_reset_resends_once() {
    let a: Vec<Vec<u8>> = sequence(&mut KittyState::default()).into_iter().map(|o| o.bytes).collect();
    let b: Vec<Vec<u8>> = sequence(&mut KittyState::default()).into_iter().map(|o| o.bytes).collect();
    assert_eq!(a, b);
    // Dropping the cache midway costs one full re-send, then the same bytes as before.
    let mut k = KittyState::default();
    let p0 = plan_at(3, true);
    let first = k.frame(&p0, &pictures(&p0, BLUE), CELL, None);
    k.reset();
    let again = k.frame(&p0, &pictures(&p0, BLUE), CELL, None);
    assert_eq!(first, again);
    let p1 = plan_at(2, true);
    assert_eq!(k.frame(&p1, &pictures(&p1, BLUE), CELL, None).bytes, a[1]);
}

#[test]
fn a_picture_past_the_edge_is_cropped() {
    let p = plan_at(3, true);
    let mut k = KittyState::default();
    // 10×6 cells at 2×2 px per cell, starting two cells left of and three rows above the
    // screen, clipped to rows 0..4.
    let img = Image::new(20, 12, vec![0u8; 20 * 12 * 4]).unwrap();
    let veil = Picture {
        layer: "tip".into(),
        part: "veil".into(),
        key: 7,
        z: Z::Above,
        at: Cells::new(-2, -3, 10, 6),
        clip: Some(Rect::new(0, 0, 40, 4)),
        image: Some(img),
    };
    let s = show(&k.frame(&p, std::slice::from_ref(&veil), CELL, None).bytes);
    assert!(s.contains("⎋[1;1H⎋_Ga=p,"), "{s}");
    assert!(s.contains(",c=8,r=3,") && s.contains(",x=4,y=6,w=16,h=6⎋"), "{s}");
    // Moved down a row with the same pixels: a re-crop, no transmit.
    let moved = Picture { at: Cells::new(-2, -2, 10, 6), image: None, ..veil.clone() };
    let s = show(&k.frame(&p, &[moved], CELL, None).bytes);
    assert!(!s.contains("a=t") && s.contains(",c=8,r=4,") && s.contains(",x=4,y=4,w=16,h=8⎋"), "{s}");
    // Entirely outside: not shown, and its image freed.
    let out = Picture { at: Cells::new(0, 20, 4, 1), image: None, ..veil };
    let o = k.frame(&p, &[out], CELL, None);
    assert!(o.placed == 0 && o.deleted == 1 && k.is_empty(), "{}", show(&o.bytes));
}

#[test]
fn pictures_without_pixels_the_terminal_lacks_are_reported() {
    let p = plan_at(3, true);
    let mut k = KittyState::default();
    let mut pics = pictures(&p, BLUE);
    pics[0].image = None;
    let o = k.frame(&p, &pics, CELL, None);
    assert_eq!(o.missing, vec![("tip".to_string(), "panel".to_string())]);
    assert_eq!(o.placed, 1);
    // A picture for a layer the plan doesn't place is left out.
    let mut other = pictures(&p, BLUE);
    for x in &mut other {
        x.layer = "elsewhere".into();
    }
    let o = k.frame(&p, &other, CELL, None);
    assert_eq!(o.placed, 0);
}

#[test]
fn ids_come_from_content_and_collisions_rehash() {
    let k = KittyState::default();
    assert_eq!(k.image_id(42, CELL), KittyState::default().image_id(42, CELL));
    assert_ne!(k.image_id(42, CELL), k.image_id(43, CELL));
    // Another cell size, another id: images rasterised for one size never stand in for
    // another.
    assert_ne!(k.image_id(42, CELL), k.image_id(42, CellPx::new(8, 16)));
}

struct Files(Vec<(String, usize)>);

impl TempFiles for Files {
    fn write(&mut self, name: &str, data: &[u8]) -> Option<String> {
        self.0.push((name.to_string(), data.len()));
        Some(format!("/tmp/x/{name}"))
    }
}

#[test]
fn file_transport_uses_the_hosts_writer() {
    let p = plan_at(3, true);
    let mut k = KittyState::new(Options { transport: Transport::File, ..Options::default() });
    let mut files = Files(Vec::new());
    let o = k.frame(&p, &pictures(&p, BLUE), CELL, Some(&mut files));
    assert_eq!(files.0.len(), 2);
    assert!(files.0.iter().all(|(n, _)| n.contains("tty-graphics-protocol")));
    let s = show(&o.bytes);
    assert_eq!(s.matches(",t=t,").count(), 2, "{s}");
    assert!(!s.contains("o=z"));
    // No writer: inline after all.
    let mut k = KittyState::new(Options { transport: Transport::File, ..Options::default() });
    assert!(show(&k.frame(&p, &pictures(&p, BLUE), CELL, None).bytes).contains("o=z"));
}

#[test]
fn large_images_go_in_4096_byte_chunks() {
    let p = plan_at(3, true);
    let mut k = KittyState::default();
    // Noise doesn't compress: several chunks.
    let mut x = 1u32;
    let data: Vec<u8> = (0..64 * 64 * 4)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    let pic = Picture {
        layer: "tip".into(),
        part: "noise".into(),
        key: 1,
        z: Z::Above,
        at: Cells::new(0, 0, 4, 4),
        clip: None,
        image: Image::new(64, 64, data),
    };
    let b = k.frame(&p, &[pic], CELL, None).bytes;
    let s = String::from_utf8(b).unwrap();
    let chunks: Vec<&str> = s.split("\x1b_G").skip(1).filter(|c| !c.starts_with("a=p")).collect();
    assert!(chunks.len() > 2);
    for (i, c) in chunks.iter().enumerate() {
        let payload = c.split_once(';').unwrap().1.trim_end_matches("\x1b\\");
        let last = i + 1 == chunks.len();
        assert!(payload.len() <= 4096);
        assert_eq!(c.contains("m=1;"), !last, "{i}");
        if !last {
            assert_eq!(payload.len(), 4096);
        }
    }
}

#[test]
fn clear_deletes_every_image_by_id() {
    let p = plan_at(3, true);
    let mut k = KittyState::default();
    k.frame(&p, &pictures(&p, BLUE), CELL, None);
    let s = show(&k.clear());
    assert_eq!(s.matches("a=d,d=I,").count(), 2, "{s}");
    assert!(k.is_empty());
}

#[test]
fn the_probe_parser_reads_every_reply() {
    let req = String::from_utf8(probe::request()).unwrap();
    assert!(req.starts_with("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"));
    assert!(req.ends_with("\x1b[>0q\x1b[16t\x1b[c"));

    // What Ghostty 1.3.1 answers, in one read, with a key typed in the middle.
    let input = b"\x1b_Gi=31;OK\x1b\\\x1bP>|ghostty 1.3.1\x1b\\x\x1b[6;34;16t\x1b[?62;22;52c";
    let mut at = 0;
    let mut replies = Vec::new();
    let mut other = Vec::new();
    while at < input.len() {
        match probe::scan(&input[at..]) {
            Scan::Reply(r, n) => {
                replies.push(r);
                at += n;
            }
            Scan::No => {
                other.push(input[at]);
                at += 1;
            }
            Scan::Partial => panic!("complete input read as partial at {at}"),
        }
    }
    assert_eq!(other, b"x");
    assert_eq!(
        replies,
        vec![
            Reply::Graphics { id: Some(31), ok: true, message: "OK".into() },
            Reply::Version("ghostty 1.3.1".into()),
            Reply::CellSize(CellPx::new(16, 34)),
            Reply::Da1(vec![62, 22, 52]),
        ]
    );
    let mut p = Probe::default();
    for r in &replies {
        assert!(p.add(r));
    }
    assert!(p.fenced && p.graphics_ok());
    assert_eq!(p.terminal().as_deref(), Some("ghostty"));
    assert_eq!(p.cell, Some(CellPx::new(16, 34)));

    // Partial input waits; keys are not replies.
    for partial in [&b"\x1b"[..], b"\x1b_", b"\x1b_Gi=31;O", b"\x1bP>", b"\x1bP>|ghos", b"\x1b[6;34", b"\x1b[?62;2"] {
        assert_eq!(probe::scan(partial), Scan::Partial, "{partial:?}");
    }
    for key in [&b"a"[..], b"\x1b[A", b"\x1b[1;5C", b"\x1b[6~", b"\x1b[<0;3;4M", b"\x1bx", b"\x1b[200~"] {
        assert_eq!(probe::scan(key), Scan::No, "{key:?}");
    }
    // An error answer, the window size, a terminal with no graphics.
    assert_eq!(
        probe::scan(b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\"),
        Scan::Reply(Reply::Graphics { id: Some(31), ok: false, message: "ENOTSUPPORTED:no".into() }, 26)
    );
    assert_eq!(probe::scan(b"\x1b[4;1258;2528t"), Scan::Reply(Reply::WindowSize { w: 2528, h: 1258 }, 14));
    let mut p = Probe::default();
    p.add(&Reply::Da1(vec![1, 2]));
    assert!(p.fenced && !p.graphics_ok());
}

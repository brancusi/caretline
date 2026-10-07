//! A ratatui screen with a hint over it: a host that draws its own rows, records where its
//! anchors landed, then lays out and composes the overlay on the same buffer.
//!
//!   cargo run -p caretline-overlay --example overlay_ratatui --features ratatui
//!   cargo run -p caretline-overlay --example overlay_ratatui --features ratatui -- --print
//!
//! Keys: any key toggles the spotlight, q quits.

use std::collections::BTreeMap;

use caretline_overlay::{
    AnchorKey, AnchorMap, Callout, Glyphs, Grid, Item, Layer, LayerOp, Layers, Limits, Opts, Rect,
    Ring, Spotlight, Theme, Themed, apply, compose, layout,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::style::{Color, Style};

const ROWS: &[(&str, &str)] = &[
    ("inbox", "Inbox"),
    ("drafts", "Drafts"),
    ("archive", "Archive"),
    ("settings", "Settings"),
    ("help", "Help and shortcuts"),
];

/// The host's own drawing: a title, a list of rows, a status line. Records each row's cells.
fn draw_host(buf: &mut Buffer, anchors: &mut AnchorMap) -> Rect {
    let area = buf.area;
    let plain = Style::default()
        .fg(Color::Rgb(0xd8, 0xdc, 0xe6))
        .bg(Color::Rgb(0x12, 0x14, 0x1a));
    buf.set_style(area, plain);
    buf.set_string(2, 1, "A small app", plain);
    for (i, (key, label)) in ROWS.iter().enumerate() {
        let y = 3 + i as u16 * 2;
        if y + 1 >= area.height {
            break;
        }
        buf.set_string(4, y, label, plain);
        anchors.put(
            AnchorKey::host("row", key),
            Rect::new(4, y, label.chars().count() as u16, 1),
        );
    }
    let status = area.height.saturating_sub(1);
    buf.set_string(
        0,
        status,
        format!(
            " {:width$}",
            "press q to quit",
            width = area.width as usize - 1
        ),
        plain.bg(Color::Rgb(0x33, 0x40, 0x5c)),
    );
    Rect::new(0, 0, area.width, status)
}

fn layers(spotlight: bool) -> Layers {
    let mut items = vec![
        Item::Callout(Callout {
            title: Some("Shortcuts live here".into()),
            body: "Open this row to see every key, or press {{key:help}} from anywhere.".into(),
            ..Callout::default()
        }),
        Item::Arrow(Default::default()),
        Item::Ring(Ring::default()),
    ];
    if spotlight {
        items.push(Item::Spotlight(Spotlight::default()));
    }
    let mut l = Layers::default();
    let layer = Layer::new(
        caretline_overlay::Anchor::Host {
            kind: "row".into(),
            key: "help".into(),
        },
        items,
    );
    apply(&mut l, LayerOp::Push(layer), None, 0, &Limits::default()).expect("a valid layer");
    l
}

fn frame(buf: &mut Buffer, spotlight: bool) {
    let mut anchors = AnchorMap::new();
    let area = draw_host(buf, &mut anchors);
    let keys: BTreeMap<String, String> = [("help".to_string(), "F1".to_string())].into();
    let grid = Grid::scan(&Themed::new(buf, Theme::dark())).with_area(area);
    let scene = layout(
        &layers(spotlight),
        &anchors,
        &grid,
        &Opts {
            glyphs: Glyphs::Rounded,
            keys: &keys,
        },
    );
    compose(&scene, &mut Themed::new(buf, Theme::dark()));
}

fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--print") {
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 72, 16));
        frame(&mut buf, true);
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            println!("{}", line.trim_end());
        }
        return Ok(());
    }
    let mut terminal = ratatui::init();
    let mut spotlight = true;
    loop {
        terminal.draw(|f| frame(f.buffer_mut(), spotlight))?;
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if k.code == KeyCode::Char('q') {
                break;
            }
            spotlight = !spotlight;
        }
    }
    ratatui::restore();
    Ok(())
}

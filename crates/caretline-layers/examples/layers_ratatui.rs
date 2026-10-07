//! A ratatui host that draws its own `hint` from the placement caretline-layers returns.
//!
//! The host draws a table whose rows have stable ids, records each row's cells in an
//! `AnchorMap` while drawing, registers a renderer that measures `hint` boxes, takes an
//! agent's `hint.show` request as JSON (as it would arrive over the host's own protocol),
//! parses it with `ops::parse`, applies it, plans, and draws the box, arrow, ring and dimming
//! itself.
//!
//!   cargo run -p caretline-layers --example layers_ratatui
//!   cargo run -p caretline-layers --example layers_ratatui -- --print
//!
//! Keys: s toggles a spotlight, q quits.

use caretline_layers::ops::{self, Request};
use caretline_layers::*;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::style::{Color, Modifier, Style};
use serde_json::{Value, json};

const ROWS: &[(&str, &str, &str)] = &[
    ("r-101", "Quarterly report", "ready"),
    ("r-102", "Release checklist", "draft"),
    ("r-103", "Onboarding guide", "ready"),
    ("r-104", "Style sheet", "stale"),
    ("r-105", "Archive index", "ready"),
];

const FG: Color = Color::Rgb(0xd8, 0xdc, 0xe6);
const BG: Color = Color::Rgb(0x12, 0x14, 0x1a);
const PANEL: Color = Color::Rgb(0x1f, 0x24, 0x30);
const ACCENT: Color = Color::Rgb(0x5f, 0xb3, 0xff);

/// Words wrapped to `w` columns.
fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in s.split_whitespace() {
        let cur = lines.last_mut().unwrap();
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > w {
            lines.push(word.to_string());
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
    }
    lines
}

/// The host's `hint` renderer: a bordered box with one cell of padding, the title, the text.
struct HintBox;

impl HintBox {
    fn lines(data: &Value, inner: usize) -> Vec<String> {
        let h: Hint = serde_json::from_value(data.clone()).unwrap_or_default();
        let mut lines = wrap(&h.text, inner);
        if let Some(t) = h.title {
            lines.insert(0, t);
        }
        lines
    }
}

impl Renderer for HintBox {
    fn measure(&self, data: &Value, avail: Size) -> Size {
        let inner = avail.w.saturating_sub(4).max(8) as usize;
        let lines = HintBox::lines(data, inner);
        let w = lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0)
            .min(inner);
        Size::new(w as u16 + 4, lines.len() as u16 + 2)
    }
}

/// The host's own drawing: a title, the table, a status line. Records each row's cells under
/// its stable id, and tells the grid where text is.
fn draw_table(buf: &mut Buffer, anchors: &mut AnchorMap, grid: &mut Grid) {
    let area = buf.area;
    let plain = Style::default().fg(FG).bg(BG);
    buf.set_style(area, plain);
    buf.set_string(2, 1, "Documents", plain.add_modifier(Modifier::BOLD));
    grid.mark_text(2, 1, "Documents");
    for (i, (id, name, state)) in ROWS.iter().enumerate() {
        let y = 3 + i as u16;
        if y + 1 >= area.height {
            break;
        }
        let row = format!("{name:<24}{state}");
        buf.set_string(4, y, &row, plain);
        grid.mark_text(4, y, &row);
        anchors.put(
            AnchorKey::host("row", id),
            Rect::new(4, y, row.chars().count() as u16, 1),
        );
    }
    let status = area.height.saturating_sub(1);
    let line = format!(
        " {:width$}",
        "s spotlight · q quit",
        width = area.width as usize - 1
    );
    buf.set_string(0, status, line, plain.bg(Color::Rgb(0x33, 0x40, 0x5c)));
}

/// What an agent sends over the host's protocol.
fn request(spotlight: bool) -> Value {
    let mut req = json!({
        "op": "hint.show", "actor": "helper",
        "anchor": {"host": {"kind": "row", "key": "r-104"}},
        "title": "Out of date", "text": "This document changed upstream since it was last opened.",
        "ttl_ms": 60000
    });
    if spotlight {
        // Not something an agent may ask for: the host pushes this one itself.
        req["actor"] = Value::Null;
    }
    req
}

fn layers(spotlight: bool) -> Layers {
    let mut layers = Layers::default();
    let req = request(spotlight);
    let (parsed, actor) = ops::parse("hint.show", &req).expect("a valid request");
    let Request::Apply(mut op) = parsed else {
        unreachable!()
    };
    if let (true, LayerOp::Push(l)) = (spotlight, &mut op) {
        l.spotlight = Some(Spotlight::default());
    }
    let applied = apply(&mut layers, op, actor.as_deref(), 0, &Limits::default()).expect("applied");
    let _reply = ops::reply(&applied, None);
    layers
}

fn set(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style) {
    if x < buf.area.width && y < buf.area.height {
        buf[(x, y)].set_symbol(s).set_style(style);
    }
}

/// The host draws the plan: dimming, ring, arrow, then the box and its text.
fn draw_plan(buf: &mut Buffer, p: &Plan, layers: &Layers) {
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            if p.dimmed(x, y) {
                buf[(x, y)].set_style(Style::default().fg(Color::Rgb(0x55, 0x58, 0x60)));
            }
        }
    }
    let border = Style::default().fg(ACCENT).bg(PANEL);
    for l in &p.layers {
        for r in &l.ring {
            for x in r.x..r.right() {
                buf[(x, r.y)].set_style(Style::default().bg(Color::Rgb(0x26, 0x3a, 0x55)));
            }
        }
        if let Some(rt) = &l.route {
            let n = rt.steps.len();
            for (k, s) in rt.steps.iter().enumerate() {
                let g = if k + 1 == n {
                    match s.leave {
                        Dir::Up => "▲",
                        Dir::Down => "▼",
                        Dir::Left => "◀",
                        Dir::Right => "▶",
                    }
                } else if s.enter == s.leave {
                    if matches!(s.enter, Dir::Up | Dir::Down) {
                        "│"
                    } else {
                        "─"
                    }
                } else {
                    match (s.enter, s.leave) {
                        (Dir::Right, Dir::Down) | (Dir::Up, Dir::Left) => "╮",
                        (Dir::Left, Dir::Down) | (Dir::Up, Dir::Right) => "╭",
                        (Dir::Right, Dir::Up) | (Dir::Down, Dir::Left) => "╯",
                        _ => "╰",
                    }
                };
                set(buf, s.x, s.y, g, Style::default().fg(ACCENT).bg(BG));
            }
        }
        let (Some(r), Some(c)) = (l.rect, layers.get(&l.id).and_then(|x| x.content.as_ref()))
        else {
            continue;
        };
        let fill = Style::default().fg(FG).bg(PANEL);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                set(buf, x, y, " ", fill);
            }
        }
        let lines = HintBox::lines(&c.data, r.w.saturating_sub(4) as usize);
        if l.mode == Some(Mode::Strip) {
            buf.set_stringn(r.x + 1, r.y, lines.join(" "), r.w as usize - 1, fill);
            continue;
        }
        let (x1, y1) = (r.right() - 1, r.bottom() - 1);
        for x in r.x + 1..x1 {
            set(buf, x, r.y, "─", border);
            set(buf, x, y1, "─", border);
        }
        for y in r.y + 1..y1 {
            set(buf, r.x, y, "│", border);
            set(buf, x1, y, "│", border);
        }
        set(buf, r.x, r.y, "╭", border);
        set(buf, x1, r.y, "╮", border);
        set(buf, r.x, y1, "╰", border);
        set(buf, x1, y1, "╯", border);
        // Attribution is this host's to draw: an agent's name in the top border, clear of
        // where an arrow attaches there.
        if let Some(actor) = l.owner.actor() {
            let label = format!(" ◆ {actor} ");
            let n = label.chars().count() as u16;
            let mut at = 2;
            if let Some(a) = l.route.as_ref().map(|rt| rt.attach)
                && a.edge == Edge::Top
                && (at..at + n).contains(&a.offset)
            {
                at = a.offset + 2;
            }
            if at + n < r.w {
                buf.set_string(r.x + at, r.y, label, border);
            }
        }
        if let (Some(rt), Some(side)) = (&l.route, l.side) {
            let j = match side {
                Side::Above => "┬",
                Side::Below => "┴",
                Side::Right => "┤",
                Side::Left => "├",
            };
            set(buf, rt.junction.0, rt.junction.1, j, border);
        }
        for (k, line) in lines.iter().enumerate().take(r.h as usize - 2) {
            let style = if k == 0 {
                border.add_modifier(Modifier::BOLD)
            } else {
                fill
            };
            buf.set_stringn(r.x + 2, r.y + 1 + k as u16, line, r.w as usize - 4, style);
        }
    }
}

fn frame(buf: &mut Buffer, spotlight: bool) {
    let area = buf.area;
    let mut anchors = AnchorMap::new();
    let mut grid =
        Grid::new(area.width, area.height).with_area(Rect::new(0, 0, area.width, area.height - 1));
    draw_table(buf, &mut anchors, &mut grid);
    let renderers = Renderers::new().register(HINT, HintBox);
    let layers = layers(spotlight);
    let p = plan(&layers, &anchors, &grid, &renderers);
    draw_plan(buf, &p, &layers);
}

fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--print") {
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 72, 16));
        frame(&mut buf, false);
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            println!("{}", line.trim_end());
        }
        return Ok(());
    }
    let mut terminal = ratatui::init();
    let mut spotlight = false;
    loop {
        terminal.draw(|f| frame(f.buffer_mut(), spotlight))?;
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('s') => spotlight = !spotlight,
                _ => {}
            }
        }
    }
    ratatui::restore();
    Ok(())
}

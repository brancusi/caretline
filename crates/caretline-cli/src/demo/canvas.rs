//! Shared layer painting for the built-in demos. Presentation data stays in the demo;
//! this is only a disposable renderer cache, including pending image cleanup.

use std::time::Instant;

use caretline::Frame;
use caretline_layers::kitty::{CellPx, KittyState, Options, Picture, TempFiles, Transport};
use caretline_layers::{FrameResolver, Grid, HINT, Layers, Plan, Rect, Renderers, plan};

use crate::layers::{self, HintRenderer, Rasters, Surface};
use crate::runtime::{Decor, Gfx};

#[derive(Default, Clone, Copy)]
pub(super) struct Cost {
    pub bytes: usize,
    pub sent: usize,
    pub micros: u128,
}

pub(super) struct Canvas {
    pub pixels: bool,
    pub kitty: KittyState,
    pub rasters: Rasters,
    pub cost: Cost,
    pending: Vec<u8>,
}

/// Local temporary-file transport, in the directory Ghostty permits.
struct TmpFiles;

impl TempFiles for TmpFiles {
    fn write(&mut self, name: &str, data: &[u8]) -> Option<String> {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, data).ok()?;
        Some(path.to_string_lossy().into_owned())
    }
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            pixels: true,
            kitty: KittyState::new(Options::default()),
            rasters: Rasters::default(),
            cost: Cost::default(),
            pending: Vec::new(),
        }
    }

    pub fn mode(&self, gfx: &Gfx) -> &'static str {
        if self.pixels && gfx.cell_px.is_some() {
            "pixels"
        } else {
            "cells"
        }
    }

    pub fn toggle_transport(&mut self) {
        let transport = match self.kitty.options().transport {
            Transport::Direct => Transport::File,
            Transport::File => Transport::Direct,
        };
        // Do not merely forget the images: another key can turn pixels off before
        // the next paint. Queue their deletion so rapid t/p or t/q leaves no ghosts.
        self.pending.extend(self.kitty.clear());
        self.kitty.set_options(Options {
            transport,
            ..self.kitty.options()
        });
    }

    pub fn hide(&mut self) -> Vec<u8> {
        let mut bytes = std::mem::take(&mut self.pending);
        bytes.extend(self.kitty.clear());
        bytes
    }

    pub fn paint(
        &mut self,
        layers: &Layers,
        frame: &mut Frame,
        gfx: &Gfx,
        area: Option<Rect>,
    ) -> Decor {
        let mut grid = Grid::from_frame(frame);
        if let Some(area) = area {
            grid = grid.with_area(area);
        }
        let renderers = Renderers::new().register(HINT, HintRenderer);
        let p = plan(layers, &FrameResolver::new(frame), &grid, &renderers);
        self.paint_plan(layers, frame, gfx, &p, grid.area)
    }

    pub fn paint_plan(
        &mut self,
        layers: &Layers,
        frame: &mut Frame,
        gfx: &Gfx,
        p: &Plan,
        area: Rect,
    ) -> Decor {
        let px = gfx.cell_px.filter(|_| self.pixels);
        let surface = if px.is_some() {
            Surface::TextOnly
        } else {
            Surface::Cells
        };
        let dim = layers::draw(frame, p, layers, surface);
        let mut bytes = std::mem::take(&mut self.pending);
        match px {
            Some(cell) => {
                let t = Instant::now();
                let made = self.rasters.made;
                let pics = layers::pictures(p, cell, area, &self.kitty, &mut self.rasters);
                let out = self.emit(p, &pics, cell);
                if !out.is_empty() {
                    self.cost = Cost {
                        bytes: out.len(),
                        sent: (self.rasters.made - made) as usize,
                        micros: t.elapsed().as_micros(),
                    };
                }
                bytes.extend(out);
            }
            None => bytes.extend(self.kitty.clear()),
        }
        Decor { dim, bytes }
    }

    /// Custom host pictures use the same transport, ids, placement and cleanup lifecycle.
    pub fn emit(&mut self, p: &Plan, pics: &[Picture], cell: CellPx) -> Vec<u8> {
        let file = self.kitty.options().transport == Transport::File;
        let mut files = TmpFiles;
        let out = self
            .kitty
            .frame(p, pics, cell, if file { Some(&mut files) } else { None });
        let mut bytes = std::mem::take(&mut self.pending);
        bytes.extend(out.bytes);
        bytes
    }
}

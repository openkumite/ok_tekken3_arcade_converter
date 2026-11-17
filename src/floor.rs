//! The fighting floor (not part of the stage model; drawn by the game's floor routine).
//!
//! Entries 0..99 of a stage's floor table (`roms`) form a 10 x 10 map of 64 x 64-texel tiles; each cell is 2500
//! world units square, col = ((x + 1250) / 2500) mod 10, row = 9 - ((z + 1250) / 2500) mod 10, and the pattern
//! repeats for ever. Here: one period of it, 25 m square, centred.

use crate::glb::Group;
use crate::tim::{Vram, page_origin, rgba15};
use crate::{SCALE, atlas};
use anyhow::{Context, Result};

/// World units per floor tile.
const CELL: i32 = 2500;
/// The floor map is 10 x 10 cells.
const N: i32 = 10;
pub const CELLS: usize = (N * N) as usize;
/// The side of the exported floor, in metres.
pub const SIZE_M: f64 = (CELL * N) as f64 * SCALE;
const TILE: usize = 64;

/// A floor entry's (tpage, v << 8 | u, clut).
type Key = (u32, u32, u32);

/// The stages lit at runtime patch their floor entries at load time. Stage 7 is the only one with several floor
/// tiles; its two tiles of page 0x810 were resolved by matching seams against the stage's 8-bit floor TIMs
/// (estimated, not read from the game's code): (stage, entry, the tile's VRAM x and y).
const OVERRIDES: [(usize, Key, (usize, usize)); 2] =
    [(7, (0x810, 0x4000, 0x196C), (0, 768)), (7, (0x810, 0x0000, 0x196F), (0, 576))];

pub struct Floor {
    pub group: Group,
    /// the atlas of its tiles
    pub png: Vec<u8>,
    /// how many different tiles it has
    pub tiles: usize,
}

fn key((w0, w1): (u32, u32)) -> Key {
    ((w0 >> 16) & 0x3BFF, w0 & 0xFFFF, w1 >> 16)
}

/// The floor of stage `stage` from its table's 100 entries, its tiles read from the stage's video memory.
pub fn build_floor(vram: &Vram, stage: usize, entries: &[(u32, u32)]) -> Result<Floor> {
    let (mut keys, mut tiles) = (Vec::new(), Vec::new());
    for &e in entries {
        let k @ (tpage, uv, clut) = key(e);
        if keys.contains(&k) {
            continue;
        }
        let (px, py, div) = page_origin(tpage);
        let mut bpp = (tpage >> 7) & 3;
        let (mut vx, mut vy) = ((px + (uv & 0xFF) as i32 / div) as usize, (py + (uv >> 8) as i32) as usize);
        let (mut cx, mut cy) = ((clut & 0x3F) as usize * 16, ((clut >> 6) & 0x3FF) as usize);
        if let Some(o) = OVERRIDES.iter().find(|o| o.0 == stage && o.1 == k) {
            (vx, vy) = o.2;
        }
        // the lit stages patch these entries at runtime: fall back to the TIM's own depth and CLUT
        if let Some(r) = vram.rects.iter().find(|r| r.contains(vx as i32, vy as i32))
            && let Some(c) = r.clut
            && (r.bpp != bpp || !vram.loaded(cx, cy))
        {
            (bpp, (cx, cy)) = (r.bpp, c);
        }
        let mut rgba = Vec::with_capacity(TILE * TILE * 4);
        for r in 0..TILE {
            for u in 0..TILE {
                rgba.extend(rgba15(vram.raw_texel(vx, vy + r, u, bpp, cx, cy)?, false));
            }
        }
        keys.push(k);
        tiles.push(atlas::Patch { w: TILE, h: TILE, rgba });
    }
    let a = atlas::build(8 * (TILE + 2), &tiles)?;
    let mut g = Group::default();
    for j in -N / 2..N - N / 2 {
        for i in -N / 2..N - N / 2 {
            let (col, row) = (i.rem_euclid(N), N - 1 - j.rem_euclid(N));
            let e = entries.get((row * N + col) as usize).context("the floor table has fewer than 100 entries")?;
            let tile = keys.iter().position(|k| *k == key(*e)).and_then(|t| a.place.get(t));
            let &(ox, oy) = tile.context("a floor tile")?;
            let (x0, z0) = (i * CELL - CELL / 2, j * CELL - CELL / 2);
            let (x1, z1) = (x0 + CELL, z0 + CELL);
            // texture u runs along +x, v along -z (PlayStation axes); glTF z = -z, negated as an integer as the
            // stage's vertices are
            let corner = |x: i32, z: i32, u: usize, v: usize| {
                (
                    [f64::from(x) * SCALE, 0.0, f64::from(-z) * SCALE],
                    [(ox + u) as f64 / a.w as f64, (oy + v) as f64 / a.h as f64],
                )
            };
            let (ca, cb, cc, cd) =
                (corner(x0, z1, 0, 0), corner(x1, z1, TILE, 0), corner(x0, z0, 0, TILE), corner(x1, z0, TILE, TILE));
            for (p, uv) in [ca, cc, cb, cb, cc, cd] {
                g.pos.push(p);
                g.uv.push(uv);
                g.nrm.push([0.0, 1.0, 0.0]);
                g.col.push([1.0; 3]);
            }
        }
    }
    Ok(Floor { group: g, png: atlas::png(a.w, a.h, &a.rgba)?, tiles: tiles.len() })
}

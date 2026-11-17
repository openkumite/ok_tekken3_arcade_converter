//! A stage's sky: a 2D panorama of 64 x 64 tiles the game draws behind everything (routine 0x801A7688; the
//! addresses here are TET2/VER.E1's).
//!
//! ```text
//! 0x801749A0 + 20 * stage   {s16 cols, s16 rows, u32 map, u32 clutMap, u32 typeMap, s16 ybase, u16 fovDeg}
//! 0x80174AA4 +  2 * stage   u16 pitchmul            (right after the 13 records)
//! 0x80174AC0 + 12 * stage   3 x {r, g, b, flag}: the colour of 't' tiles; of the rectangle above the panorama;
//!                           of the rectangle below it and of 'd' tiles. The flag: the rectangle is drawn.
//! map, clutMap              u16[rows * cols]: a tile's code and its CLUT word; the columns repeat
//! typeMap                   u8[rows * period]: '1' a textured tile, 't' and 'd' a tile of one colour, '0' nothing;
//!                           period = (0x2D000 / fov) / 64 columns make a full turn
//! ```
//!
//! A tile's code: bits 0-1 u / 64, bits 2-3 v / 64, bits 8-11 the texture page's x / 64, bit 12 its y + 256, bit 13
//! its y + 512; always 4 bits a texel, texel colour 0 transparent; the texels are the stage's own TIMs'.
//!
//! Each frame, on a 512 x 480 screen: 512 px across are `fov` degrees, the panorama scrolled by the camera's yaw;
//! its top row is at `y = (-pitch * pitchmul) / 256 + ybase` (pitch signed, 4096 a turn). Stages 0, 3, 5 and 11
//! keep y <= 0, stage 11 also y >= -192. With y > 0 and the top colour's flag, the rows above are that colour;
//! below the panorama, with the bottom colour's flag, the rows down to row 360 are that colour (the floor covers
//! the rest). Stage 3 draws no tiles: a gradient from (0, 20, 102) at y to black at y + 192, black below. Stage 6
//! is black. Stage 5 draws its row 3 upside down.

use crate::atlas::{self, Patch};
use crate::roms::EXE_BASE;
use crate::tim::{Vram, rgba15};
use crate::{bytes, u16_at, u32_at};
use anyhow::{Context, Result};
use serde_json::{Value, json};

const STAGES: usize = 13;
const TILE: usize = 64;
/// 512 px a `fov`: the pixels of a full turn, times `fov`.
const TURN: usize = 512 * 360;
/// What the routine does by the stage's number.
const GRADIENT_STAGE: usize = 3;
const GRADIENT: ([u8; 3], usize) = ([0, 20, 102], 192);
const BLACK_STAGE: usize = 6;
const MIRRORED_ROW: (usize, usize) = (5, 3);
const MAX_Y_0: [usize; 4] = [0, 3, 5, 11];
const MIN_Y: (usize, i32) = (11, -192);

/// One stage's sky tables.
#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    pub cols: usize,
    pub rows: usize,
    /// the columns of a full turn
    pub period: usize,
    pub map: Vec<u16>,
    pub clut: Vec<u16>,
    pub types: Vec<u8>,
    pub ybase: i16,
    pub fov: u16,
    pub pitchmul: u16,
    /// 't' tiles, the top rectangle, the bottom rectangle and 'd' tiles: r, g, b, flag
    pub colours: [[u8; 4]; 3],
}

/// The record at `at` if it can be a sky's: (cols, rows, the three pointers as offsets, ybase, fov). The game's
/// have at most 16 columns, 11 rows and 36 to 69 degrees; the caps bound the picture (11520 x 1024).
fn record(exe: &[u8], at: u64) -> Option<(usize, usize, [u64; 3], i16, u16)> {
    let h = |o: u64| u16_at(exe, at + o, "").ok();
    let ptr = |o: u64| {
        let p = u32_at(exe, at + o, "").ok()?.checked_sub(EXE_BASE)?;
        ((p as usize) < exe.len()).then_some(u64::from(p))
    };
    let (cols, rows, fov) = (h(0)?, h(2)?, h(18)?);
    ((1..=64).contains(&cols) && (1..=16).contains(&rows) && (16..=360).contains(&fov)).then_some((
        cols.into(),
        rows.into(),
        [ptr(4)?, ptr(8)?, ptr(12)?],
        h(16)? as i16,
        fov,
    ))
}

/// The 13 stages' skies of the unpacked program: the tables are found by their look (13 records that can be
/// skies, one after another; one place in every program version).
pub fn parse_tables(exe: &[u8]) -> Result<Vec<Sky>> {
    let rec = |p: u64, s: usize| record(exe, p + 20 * s as u64);
    let p = (0..exe.len() as u64)
        .step_by(4)
        .find(|&p| (0..STAGES).all(|s| rec(p, s).is_some()))
        .context("no table of the stages' skies in the main program")?;
    let (pitch, colours) = (p + 20 * STAGES as u64, p + (20 * STAGES as u64 + 2 * STAGES as u64).next_multiple_of(4));
    let mut skies = Vec::new();
    for s in 0..STAGES {
        let (cols, rows, [map, clut, types], ybase, fov) = rec(p, s).context("a sky's record")?;
        let period = TURN / usize::from(fov) / TILE;
        let words = |at: u64, what: &str| -> Result<Vec<u16>> {
            let b = bytes(exe, at, rows * cols * 2, what)?;
            Ok(b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
        };
        let c = bytes(exe, colours + 12 * s as u64, 12, "a sky's colours")?.as_chunks::<4>().0;
        skies.push(Sky {
            cols,
            rows,
            period,
            map: words(map, "a sky's tile map")?,
            clut: words(clut, "a sky's CLUT map")?,
            types: bytes(exe, types, rows * period, "a sky's type map")?.to_vec(),
            ybase,
            fov,
            pitchmul: u16_at(exe, pitch + 2 * s as u64, "a sky's pitch factor")?,
            colours: [c[0], c[1], c[2]],
        });
    }
    Ok(skies)
}

/// A tile code's texture page (as a GPU texture page word: 4 bits a texel) and its corner (u, v) in it.
pub fn tile(code: u16) -> (u32, usize, usize) {
    let c = u32::from(code);
    (
        (c >> 8) & 0xF | ((c >> 12) & 1) << 4 | ((c >> 13) & 1) << 11,
        (c as usize & 3) * TILE,
        (c as usize >> 2 & 3) * TILE,
    )
}

/// The picture of stage `stage`'s sky, rows top down: the full turn (period * 64 x rows * 64), transparent where
/// nothing is drawn; stage 3's gradient (it repeats across); none for the black stage.
pub fn render(stage: usize, sky: &Sky, vram: &Vram) -> Result<Option<Patch>> {
    if stage == BLACK_STAGE {
        return Ok(None);
    }
    if stage == GRADIENT_STAGE {
        let (c, h) = GRADIENT;
        let row = |r: usize| {
            let [red, green, blue] = c.map(|v| (usize::from(v) * (h - r) / h) as u8);
            [red, green, blue, 255].repeat(TILE)
        };
        let rgba = (0..h).flat_map(row).collect();
        return Ok(Some(Patch { w: TILE, h, rgba }));
    }
    let (w, h) = (sky.period * TILE, sky.rows * TILE);
    let mut rgba = vec![0; w * h * 4];
    for r in 0..sky.rows {
        for c in 0..sky.period {
            let ty = *sky.types.get(r * sky.period + c).context("a sky's type map")?;
            let m = r * sky.cols + c % sky.cols;
            let (code, clut) =
                (*sky.map.get(m).context("a sky's tile map")?, *sky.clut.get(m).context("a sky's CLUT map")?);
            let (tpage, u0, v0) = tile(code);
            let flat = match ty {
                b't' => Some(sky.colours[0]),
                b'd' => Some(sky.colours[2]),
                _ => None,
            };
            if flat.is_none() && ty != b'1' {
                continue;
            }
            for y in 0..TILE {
                let at = ((r * TILE + y) * w + c * TILE) * 4;
                let line = rgba.get_mut(at..at + TILE * 4).context("a sky tile outside its picture")?;
                let v = if (stage, r) == MIRRORED_ROW { TILE - 1 - y } else { y };
                for (x, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    *px = match flat {
                        Some(f) => [f[0], f[1], f[2], 255],
                        None => rgba15(vram.texel(tpage, clut.into(), u0 + x, v0 + v)?, false),
                    };
                }
            }
        }
    }
    Ok(Some(Patch { w, h, rgba }))
}

/// How the game places stage `stage`'s sky (the module docs; rows of a 480-row screen), for the scene's extras
/// and the manifest; none for the black stage.
pub fn placement(stage: usize, sky: &Sky) -> Option<Value> {
    let rect = |c: [u8; 4]| (c[3] != 0).then_some([c[0], c[1], c[2]]);
    (stage != BLACK_STAGE).then(|| {
        json!({
            "fov": sky.fov,
            "ybase": sky.ybase,
            "pitchmul": sky.pitchmul,
            "top": rect(sky.colours[1]),
            "bottom": rect(sky.colours[2]),
            "max_y": MAX_Y_0.contains(&stage).then_some(0),
            "min_y": (stage == MIN_Y.0).then_some(MIN_Y.1),
        })
    })
}

/// The sky of stage `stage` for its glb: the picture as a PNG and its placement.
pub fn build(stage: usize, sky: &Sky, vram: &Vram) -> Result<Option<(Vec<u8>, Value)>> {
    let (Some(p), Some(v)) = (render(stage, sky, vram)?, placement(stage, sky)) else { return Ok(None) };
    Ok(Some((atlas::png(p.w, p.h, &p.rgba)?, v)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn tile_codes() {
        assert_eq!(tile(0), (0, 0, 0));
        // u = 3 * 64, v = 2 * 64, page x = 5 * 64
        assert_eq!(tile(0x050B), (5, 192, 128));
        // bit 12: y + 256 (the page word's bit 4); bit 13: y + 512 (its bit 11); the other bits are not read
        assert_eq!(tile(0x1F00), (0x1F, 0, 0));
        assert_eq!(tile(0x2000), (0x800, 0, 0));
        assert_eq!(tile(0xC0F0), (0, 0, 0));
    }

    /// A program of 13 skies of 2 x 1 tiles, fov 360 (a period of 8 columns), after `pad` bytes: the records, the
    /// pitch factors, the colours, then one tile map, CLUT map and type map all the stages share.
    fn program(pad: usize) -> Vec<u8> {
        let maps = (pad + 20 * 13 + 28 + 12 * 13) as u32;
        let mut exe = vec![0; pad];
        for s in 0..13u16 {
            for v in [2u16, 1] {
                exe.extend(v.to_le_bytes());
            }
            for o in [0, 4, 8] {
                exe.extend((EXE_BASE + maps + o).to_le_bytes());
            }
            exe.extend((-(s as i16)).to_le_bytes());
            exe.extend(360u16.to_le_bytes());
        }
        exe.extend((0..14u16).flat_map(|s| (100 + s).to_le_bytes()));
        for s in 0..13u8 {
            exe.extend([1, 2, 3, 0, 4, 5, 6, 1, 7, 8, s, 0]);
        }
        // the tile map (codes 1: u = 64, and 0), the CLUT map (words 480 << 6), the type map
        for v in [1u16, 0, 480 << 6, 480 << 6] {
            exe.extend(v.to_le_bytes());
        }
        exe.extend(b"1t0d1000");
        exe
    }

    #[test]
    fn tables() {
        let exe = program(40);
        let skies = parse_tables(&exe).unwrap();
        assert_eq!(skies.len(), 13);
        let s = &skies[12];
        assert_eq!((s.cols, s.rows, s.period, s.ybase, s.fov, s.pitchmul), (2, 1, 8, -12, 360, 112));
        assert_eq!((&s.map[..], &s.clut[..], &s.types[..]), (&[1, 0][..], &[480 << 6; 2][..], &b"1t0d1000"[..]));
        assert_eq!(s.colours, [[1, 2, 3, 0], [4, 5, 6, 1], [7, 8, 12, 0]]);
        assert_eq!(
            placement(11, s).unwrap(),
            json!({"fov": 360, "ybase": -12, "pitchmul": 112, "top": [4, 5, 6], "bottom": null, "max_y": 0, "min_y": -192})
        );
        assert_eq!(placement(1, s).unwrap()["max_y"], Value::Null);
        assert_eq!(placement(6, s), None);
        // cut anywhere: an error, never a panic
        for n in 0..exe.len() {
            assert!(parse_tables(&exe[..n]).is_err(), "cut at {n}");
        }
        assert!(parse_tables(&vec![0; 4096]).is_err());
    }

    #[test]
    fn pictures() {
        let sky = &parse_tables(&program(0)).unwrap()[5];
        // a 4-bit TIM: the CLUT at (0, 480), colour i = i * 0x421; 128 x 64 texels at (0, 0), texel (u, v) = v & 15
        let mut tim = Vec::new();
        for v in [0x10u32, 8, 12 + 32] {
            tim.extend(v.to_le_bytes());
        }
        for v in [0u16, 480, 16, 1].into_iter().chain((0..16).map(|i| i * 0x421)) {
            tim.extend(v.to_le_bytes());
        }
        tim.extend((12u32 + 32 * 64 * 2).to_le_bytes());
        for v in [0u16, 0, 32, 64].into_iter().chain((0..64).flat_map(|v| [(v & 15) * 0x1111; 32])) {
            tim.extend(v.to_le_bytes());
        }
        let mut vram = Vram::default();
        vram.load_tims(&tim).unwrap();
        let grey = |i: u16| rgba15(i * 0x421, false);
        let p = render(0, sky, &vram).unwrap().unwrap();
        assert_eq!((p.w, p.h, p.rgba.len()), (512, 64, 512 * 64 * 4));
        let at = |p: &Patch, x: usize, y: usize| <[u8; 4]>::try_from(&p.rgba[(y * p.w + x) * 4..][..4]).unwrap();
        // column 0 textured (row 0 is colour 0: transparent), 1 the 't' colour, 2 nothing, 3 the 'd' colour
        assert_eq!([at(&p, 0, 0), at(&p, 63, 5), at(&p, 5, 63)], [[0; 4], grey(5), grey(15)]);
        assert_eq!([at(&p, 64, 0), at(&p, 128, 9), at(&p, 192, 63)], [[1, 2, 3, 255], [0; 4], [7, 8, 5, 255]]);
        // column 4 is textured with the map's column 0 again; the rest nothing
        assert_eq!([at(&p, 256, 5), at(&p, 320, 5)], [grey(5), [0; 4]]);
        // a row drawn upside down (stage 5's row 3 in the game: here a sky of 4 rows of the same tiles)
        let tall = Sky {
            rows: 4,
            map: sky.map.repeat(4),
            clut: sky.clut.repeat(4),
            types: sky.types.repeat(4),
            ..sky.clone()
        };
        let p = render(5, &tall, &vram).unwrap().unwrap();
        assert_eq!([at(&p, 0, 2 * 64 + 5), at(&p, 0, 3 * 64 + 5)], [grey(5), grey(10)]);
        // the gradient and the black stage
        let p = render(3, sky, &vram).unwrap().unwrap();
        assert_eq!((p.w, p.h), (64, 192));
        assert_eq!(
            [at(&p, 0, 0), at(&p, 63, 96), at(&p, 9, 191)],
            [[0, 20, 102, 255], [0, 10, 51, 255], [0, 0, 0, 255]]
        );
        assert!(render(6, sky, &vram).unwrap().is_none() && build(6, sky, &vram).unwrap().is_none());
        // maps shorter than the sky says
        assert!(render(0, &Sky { types: vec![b'1'; 3], ..sky.clone() }, &vram).is_err());
    }
}

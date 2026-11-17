//! What the game's glb of a stage has besides the still model: the parts the game never draws left out, the
//! torches' flame, the carousel and the helicopter. All from routines of TET2/VER.E1:
//!
//! - **Cells never drawn** (0x801D73E4 returns at once for them): three of the first stage's and three of the
//!   sixth's.
//! - **The flame** (the seventh and the thirteenth stage; `stage::FLAME_FRAMES`): the faces whose texture the game
//!   copies the flame's pictures over.
//! - **The carousel** (the fourth stage; placed by 0x801E59FC, turned by 0x801E5AD0): the half platform with its
//!   horses turns by 5/4096 of a turn each frame (after 136 frames, a sixth of a turn, the game sets it back: the
//!   model repeats every sixth, so here it just turns on); two half roof rings and four banners stand still.
//!   The game draws the far cells (from 0xD2 on) behind it and the banners before everything.
//! - **The helicopter** (the twelfth stage; 0x801E3DE0, 0x801E40C0): its body behind the model, its rotors turning
//!   by 0x29F/4096 of a turn each frame. shortcut: it hovers where a round starts it. The game also sways it
//!   (yaw, height and roll, a few units) and after two seconds flies it one of three ways (0x801E4A0C, 0x801E4E04,
//!   0x801E54B4), which are not worked out; upgrade: its flights as keyframes.
//!
//! Angles are the PlayStation's (4096 a turn; its Y axis is down): about Y they turn the other way in the glb.
//! The order of the helicopter's two rotations (it is tilted, then turned) is a guess.

use crate::model::{Face, parse_model, parse_stage};
use crate::roms::{Files, Stage, lz_decompress};
use crate::stage::{Built, build};
use crate::tim::Vram;
use crate::{SCALE, u32_at};
use anyhow::{Context, Result};
use std::f64::consts::TAU;

/// A turn about the line through `pivot` (metres, the model's space) along `axis`, `rate` radians a frame.
pub struct Spin {
    pub pivot: [f64; 3],
    pub axis: [f64; 3],
    pub rate: f64,
}

/// Meshes drawn together, in the order the game draws them: far first.
pub struct Layer {
    pub built: Built,
    pub spin: Option<Spin>,
}

const DROPPED: [(usize, [usize; 3]); 2] = [(0, [0xC6, 0xC7, 0xD8]), (5, [0x53, 0x63, 0x6B])];
/// Where in VRAM the flame's pictures are copied to.
const FLAME_AT: [(usize, [i32; 2]); 2] = [(6, [16, 896]), (12, [16, 960])];
const CAROUSEL_STAGE: usize = 3;
const CAROUSEL_BACK_CELLS: usize = 0xD2;
const HELICOPTER_STAGE: usize = 11;

type M3 = [[f64; 3]; 3];
const ONE: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn angle(units: i32) -> f64 {
    f64::from(units) * TAU / 4096.0
}
fn about_y(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}
fn about_x(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}
fn turn(m: &M3, p: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[0] * p[0] + r[1] * p[1] + r[2] * p[2])
}
fn times(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
/// A point of the stage model's space (the PlayStation's axes) in the glb's metres.
fn point(x: i32, y: i32, z: i32) -> [f64; 3] {
    [f64::from(x) * SCALE, f64::from(-y) * SCALE, f64::from(-z) * SCALE]
}

/// Model `model` of an object's files (the first: its textures), moved by `local`, turned by `rot`, moved by `at`.
fn object(files: &[Vec<u8>], model: usize, local: [f64; 3], rot: &M3, at: [f64; 3]) -> Result<Built> {
    let (tims, model) =
        (files.first().context("an object's textures")?, files.get(model).context("an object's model")?);
    let mut vram = Vram::default();
    // (packed as the program is, if it is not a TIM: as a stage's texture files)
    match u32_at(tims, 0, "an object's textures")? {
        0x10 => vram.load_tims(tims)?,
        _ => vram.load_tims(&lz_decompress(tims, 0)?)?,
    }
    let (verts, faces) = parse_model(model, true)?;
    let mut built = build(vram, &verts, &faces, None)?;
    for (_, g) in &mut built.groups {
        for p in &mut g.pos {
            *p = add(turn(rot, add(*p, local)), at);
        }
    }
    Ok(built)
}

/// Stage `stage`'s layers for the game's glb, far first.
pub fn layers(stage: usize, st: &Stage, files: &Files) -> Result<Vec<Layer>> {
    let mut vram = Vram::default();
    for t in &st.tims {
        vram.load_tims(t)?;
    }
    let flame = FLAME_AT.iter().find(|f| f.0 == stage).map(|f| f.1).filter(|_| files.flames.is_some());
    if let (Some(_), Some(f)) = (flame, &files.flames) {
        vram.load_tims(f)?;
    }
    let (verts, mut faces) = parse_stage(&st.model)?;
    if let Some((_, cells)) = DROPPED.iter().find(|d| d.0 == stage) {
        faces.retain(|f| !cells.contains(&f.cell));
    }
    let still = |built| Layer { built, spin: None };
    let model = |faces: &[Face]| Ok::<_, anyhow::Error>(still(build(vram.clone(), &verts, faces, flame)?));
    match (stage, &files.carousel, &files.helicopter) {
        (CAROUSEL_STAGE, Some(c), _) => {
            let at = point(0, -300, 3100);
            let (back, front): (Vec<Face>, Vec<Face>) = faces.into_iter().partition(|f| f.cell >= CAROUSEL_BACK_CELLS);
            let half = about_y(-angle(0x800));
            Ok(vec![
                model(&back)?,
                Layer {
                    built: object(c, 1, [0.0; 3], &about_y(-angle(0x955)), at)?,
                    spin: Some(Spin { pivot: at, axis: [0.0, 1.0, 0.0], rate: angle(5) }),
                },
                still(object(c, 2, [0.0; 3], &ONE, at)?),
                still(object(c, 3, [0.0; 3], &half, at)?),
                model(&front)?,
                still(object(c, 4, [0.0; 3], &half, [0.0; 3])?),
            ])
        }
        (HELICOPTER_STAGE, _, Some(h)) => {
            let (rot, at) = (times(&about_y(-angle(0x71C)), &about_x(angle(-0x9F))), point(0, -400, 4200));
            let rotor = |model: usize, local: [f64; 3], axis: [f64; 3]| {
                let spin = Spin { pivot: add(turn(&rot, local), at), axis: turn(&rot, axis), rate: angle(0x29F) };
                Ok::<_, anyhow::Error>(Layer { built: object(h, model, local, &rot, at)?, spin: Some(spin) })
            };
            Ok(vec![
                still(object(h, 1, [0.0; 3], &rot, at)?),
                rotor(2, point(0, -300, 0), [0.0, 1.0, 0.0])?,
                rotor(3, point(0, 280, -1230), [1.0, 0.0, 0.0])?,
                model(&faces)?,
            ])
        }
        _ => Ok(vec![model(&faces)?]),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn turns() {
        // a quarter turn about Y takes +Z to +X; about X, +Y to +Z; the PlayStation's point has Y and Z the other way
        let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(near(turn(&about_y(angle(1024)), [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]));
        assert!(near(turn(&about_x(angle(1024)), [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]));
        assert!(near(turn(&times(&about_y(angle(1024)), &about_x(angle(1024))), [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]));
        assert_eq!(point(1000, -300, 3100), [1.0, 0.3, -3.1]);
    }
}

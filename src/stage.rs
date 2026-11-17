//! A stage's model as meshes: its faces grouped by material, the textures they use cut out of the video memory.
//! A face's texels go into the atlas (one region per texture page, CLUT and blending: the box of all the texels its
//! faces use); a face of a tiled texture (command bit 0x80) whose coordinates run past its TIM keeps the TIM as a
//! picture of its own, repeated.

use crate::atlas::{self, decode_patch};
use crate::glb::{Group, Material};
use crate::model::{Face, parse_stage};
use crate::tim::{Vram, page_origin};
use crate::{SCALE, roms};
use anyhow::{Context, Result, bail};

pub struct Built {
    pub vertices: usize,
    pub faces: usize,
    /// one per material, in the glb's order
    pub groups: Vec<(Material, Group)>,
    /// (name, PNG): "atlas" first if any face uses it, then the tiles
    pub images: Vec<(String, Vec<u8>)>,
    pub vram: Vram,
    /// vertex colours stored (drawn unlit), or normals for the game's lighting
    pub prelit: bool,
    /// the flame's pictures: the first image of them, how many follow one another, and the frames each is shown
    pub flipbook: Option<(usize, usize, u32)>,
}

/// The torches' flame (0x801E2114 of TET2/VER.E1): every 4th frame the next of 16 pictures of 64 x 64 texels is
/// copied over the stage's own flame texture. They are loaded at VRAM (512 + 16 (i / 8), 512 + 64 (i % 8)), 4 bits
/// a texel, and drawn through the CLUT of the faces that show them.
pub const FLAME_FRAMES: usize = 16;
pub const FLAME_STEP: u32 = 4;
const FLAME_SIZE: i32 = 64;

/// A tiled texture: (tpage, clut, semi-transparent) and its TIM's place in the page (u, v, w, h in texels).
type Tile = (u32, u32, bool, [i32; 4]);

#[derive(Clone, Copy)]
enum Tex {
    None,
    Region(usize),
    Tile(usize),
    Flame,
}

/// The game's stages decode at most 2 million texels; the cap bounds what a broken model can ask.
const MAX_TEXELS: usize = 1 << 24;

/// The TIM under texel (u, v) of a texture page, in the page's texels.
fn find_tile(vram: &Vram, tpage: u32, u: i32, v: i32) -> Option<[i32; 4]> {
    let (px, py, div) = page_origin(tpage);
    let r = vram.rects.iter().find(|r| r.contains(px + u / div, py + v))?;
    Some([(r.x - px) * div, r.y - py, r.w * div, r.h])
}

/// How Python writes a bool (the groups are in the order of the Python tool's keys as text).
fn py(b: bool) -> &'static str {
    if b { "True" } else { "False" }
}

pub fn build_stage(stage: &roms::Stage) -> Result<Built> {
    let mut vram = Vram::default();
    for t in &stage.tims {
        vram.load_tims(t)?;
    }
    let (verts, faces) = parse_stage(&stage.model)?;
    build(vram, &verts, &faces, None)
}

/// The meshes of a model's `faces`, their textures in `vram`. `flame`: the VRAM place of the texture the flame's
/// pictures are copied over (they are in `vram` too): the faces showing it get the pictures, one image each.
pub fn build(vram: Vram, verts: &[[i16; 3]], faces: &[Face], flame: Option<[i32; 2]>) -> Result<Built> {
    // the flame's faces: their texture's corner in its page (texels), and their (tpage, clut, semi)
    let mut flame_tex = None;

    // the texture regions: (tpage, clut, semi) and [umin, vmin, umax, vmax]
    let mut regions: Vec<((u32, u32, bool), [i32; 4])> = Vec::new();
    let mut tiles: Vec<Tile> = Vec::new();
    let mut texs = Vec::new();
    for f in faces {
        let Some((tpage, clut)) = f.tex else {
            texs.push(Tex::None);
            continue;
        };
        let semi = f.cmd & 2 != 0;
        let b = f.uv.iter().fold([255, 255, 0, 0], |b, &(u, v)| [b[0].min(u), b[1].min(v), b[2].max(u), b[3].max(v)]);
        let tile = if f.cmd & 0x80 != 0 { find_tile(&vram, tpage, b[0], b[1]) } else { None };
        let (px, py, div) = page_origin(tpage);
        let at = flame.map(|[x, y]| [(x - px) * div, y - py]).filter(|&[u, v]| {
            b[0] >= u && b[1] >= v && b[2] < u + FLAME_SIZE && b[3] < v + FLAME_SIZE && (tpage >> 7) & 3 == 0
        });
        if let Some(at) = at {
            flame_tex.get_or_insert((at, tpage, clut, semi));
            texs.push(Tex::Flame);
            continue;
        }
        texs.push(match tile {
            Some(t) if b[2] >= t[0] + t[2] || b[3] >= t[1] + t[3] => {
                let key = (tpage, clut, semi, t);
                Tex::Tile(tiles.iter().position(|k| *k == key).unwrap_or_else(|| {
                    tiles.push(key);
                    tiles.len() - 1
                }))
            }
            _ => {
                let key = (tpage, clut, semi);
                let i = regions.iter().position(|(k, _)| *k == key).unwrap_or_else(|| {
                    regions.push((key, b));
                    regions.len() - 1
                });
                if let Some((_, r)) = regions.get_mut(i) {
                    *r = [r[0].min(b[0]), r[1].min(b[1]), r[2].max(b[2]), r[3].max(b[3])];
                }
                Tex::Region(i)
            }
        });
    }
    let size = |w: i32, h: i32| (w as usize, h as usize);
    let texels: usize = regions
        .iter()
        .map(|(_, r)| size(r[2] - r[0] + 1, r[3] - r[1] + 1))
        .chain(tiles.iter().map(|t| size(t.3[2], t.3[3])))
        .map(|(w, h)| w * h)
        .sum();
    if texels > MAX_TEXELS {
        bail!("the stage's textures are too large ({texels} texels)");
    }

    let mut images = Vec::new();
    let mut atlas = None;
    if !regions.is_empty() {
        let patches = regions
            .iter()
            .map(|&((tp, cl, semi), r)| {
                decode_patch(&vram, tp, cl, (r[0], r[1]), size(r[2] - r[0] + 1, r[3] - r[1] + 1), semi)
            })
            .collect::<Result<Vec<_>>>()?;
        let a = atlas::build(1024, &patches)?;
        images.push(("atlas".to_string(), atlas::png(a.w, a.h, &a.rgba)?));
        atlas = Some(a);
    }
    let mut tile_img = Vec::new();
    for &(tp, cl, semi, t) in &tiles {
        let p = decode_patch(&vram, tp, cl, (t[0], t[1]), size(t[2], t[3]), semi)?;
        tile_img.push(images.len());
        images.push((format!("tile{}", images.len()), atlas::png(p.w, p.h, &p.rgba)?));
    }

    let mut flipbook = None;
    if let Some((_, _, clut, semi)) = flame_tex {
        flipbook = Some((images.len(), FLAME_FRAMES, FLAME_STEP));
        for i in 0..FLAME_FRAMES as u32 {
            // (the page at x 512, y 512 or 768; 4 bits a texel)
            let tpage = 8 | (i % 8 / 4) << 4 | 1 << 11;
            let from = ((i / 8) as i32 * FLAME_SIZE, (i % 4) as i32 * FLAME_SIZE);
            let p = decode_patch(&vram, tpage, clut, from, size(FLAME_SIZE, FLAME_SIZE), semi)?;
            images.push((format!("flame_{i:02}"), atlas::png(p.w, p.h, &p.rgba)?));
        }
    }

    // the triangles, grouped by material; each group with the Python tool's key as text, their order in the glb
    let mut groups: Vec<(String, Material, Group)> = Vec::new();
    for (f, &tex) in faces.iter().zip(&texs) {
        let (n, semi, raw) = (f.idx.len(), f.cmd & 2 != 0, f.cmd & 1 != 0);
        let mut pos = Vec::new();
        for &i in &f.idx {
            let v = verts.get(i).context("a face's vertex is not in the model")?;
            // PlayStation (+Y down) to glTF (+Y up); negated as integers: 0 stays +0.0
            pos.push([
                f64::from(v[0]) * SCALE,
                f64::from(-i32::from(v[1])) * SCALE,
                f64::from(-i32::from(v[2])) * SCALE,
            ]);
        }
        // the face normal (Newell's method over the polygon's outline: 0-1-3-2 or 0-1-2)
        let ring: &[usize] = if n == 4 { &[0, 1, 3, 2] } else { &[0, 1, 2] };
        let mut nrm = [0.0f64; 3];
        for (k, &a) in ring.iter().enumerate() {
            let (p0, p1) = (pos[a], pos[ring[(k + 1) % ring.len()]]);
            nrm[0] += (p0[1] - p1[1]) * (p0[2] + p1[2]);
            nrm[1] += (p0[2] - p1[2]) * (p0[0] + p1[0]);
            nrm[2] += (p0[0] - p1[0]) * (p0[1] + p1[1]);
        }
        let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
        if len == 0.0 {
            continue; // a polygon of no area: the PlayStation draws nothing
        }
        let nrm = nrm.map(|c| c / len);
        let cols: Vec<[f64; 3]> = match &f.col {
            Some(c) if !(raw && f.tex.is_some()) => {
                let full = if f.tex.is_some() { 128.0 } else { 255.0 };
                c.iter().map(|c| c.map(|ch| (f64::from(ch) / full).min(1.0))).collect()
            }
            _ => vec![[1.0; 3]; n],
        };
        let uv_in = |ox: i32, oy: i32, w: f64, h: f64| -> Vec<[f64; 2]> {
            f.uv.iter().map(|&(u, v)| [(f64::from(ox + u) + 0.5) / w, (f64::from(oy + v) + 0.5) / h]).collect()
        };
        let (uvs, key, material) = match tex {
            Tex::Tile(i) => {
                let &(tp, cl, tsemi, t) = tiles.get(i).context("a face's tile")?;
                let key = format!("('tile', ({tp}, {cl}, {}, {}, {}, {}, {}))", py(tsemi), t[0], t[1], t[2], t[3]);
                let name = format!("tile_{tp:04x}_{cl:04x}");
                let image = tile_img.get(i).copied();
                (uv_in(-t[0], -t[1], t[2].into(), t[3].into()), key, Material { name, tex: image, semi: tsemi })
            }
            Tex::Region(i) => {
                let (a, (_, r)) =
                    (atlas.as_ref().context("a face's atlas")?, regions.get(i).context("a face's region")?);
                let &(ox, oy) = a.place.get(i).context("a face's region")?;
                let name = if semi { "atlas_semi" } else { "atlas" }.to_string();
                (
                    uv_in(ox as i32 - r[0], oy as i32 - r[1], a.w as f64, a.h as f64),
                    format!("('atlas', {})", py(semi)),
                    Material { name, tex: Some(0), semi },
                )
            }
            Tex::Flame => {
                let ((at, ..), (first, ..)) = (flame_tex.context("a flame")?, flipbook.context("a flame")?);
                let size = f64::from(FLAME_SIZE);
                let material = Material { name: "flame".to_string(), tex: Some(first), semi };
                (uv_in(-at[0], -at[1], size, size), "('flame',)".to_string(), material)
            }
            Tex::None => {
                let name = if semi { "flat_semi" } else { "flat" }.to_string();
                (vec![[0.0; 2]; n], format!("('flat', {})", py(semi)), Material { name, tex: None, semi })
            }
        };
        let g = match groups.iter().position(|g| g.0 == key) {
            Some(g) => g,
            None => {
                groups.push((key, material, Group::default()));
                groups.len() - 1
            }
        };
        let g = &mut groups.get_mut(g).context("a face's group")?.2;
        let order: &[[usize; 3]] = if n == 4 { &[[0, 1, 2], [1, 3, 2]] } else { &[[0, 1, 2]] };
        for tri in order {
            let [a, b, c] = tri.map(|k| pos[k]);
            let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let cross = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            if cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2] == 0.0 {
                continue; // the degenerate half of a quad
            }
            for &k in tri {
                g.pos.push(pos[k]);
                g.nrm.push(nrm);
                g.col.push(cols[k]);
                g.uv.push(uvs[k]);
            }
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Built {
        vertices: verts.len(),
        faces: faces.len(),
        // (a group all of whose triangles were degenerate has nothing to draw)
        groups: groups.into_iter().filter(|g| !g.2.pos.is_empty()).map(|g| (g.1, g.2)).collect(),
        images,
        vram,
        prelit: faces.iter().any(|f| f.col.is_some()),
        flipbook,
    })
}

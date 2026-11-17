//! A stage's model file.
//!
//! ```text
//! u32 unk (0x41), u32 unk, u32 cellCount (0x100)
//! cellCount x 28-byte cell records (a 16 x 16 grid), offsets relative to file + 0xC:
//!   u32 vtxOff, u32 vtxCount, u32 nrmOff (0, filled at runtime), u32 nrmCount, u32 primOff, u32 primCount, u32 unk
//! vertices: int16 x, y, z, pad   (PlayStation axes: +Y is down)
//! primitives: GPU-packet-like records
//!   byte0 = GPU packet word count, byte1 = record word count - 1,
//!   byte2 = 1 if pre-lit (vertex colours stored) / 0 if lit (normal indices),
//!   byte3 = GPU command (0x20 poly | 0x10 gouraud | 0x08 quad | 0x04 textured | 0x02 semi-transparent |
//!           0x01 raw texture) | 0x80 = tiled texture
//!   [uv0 + clut] [uv1 + tpage] [uv2] [uv3]   (textured only)
//!   pre-lit: colour words (1 flat / n gouraud), then n u16 vertex indices
//!   lit:     flat -> u16 normal, n u16 vertices;  gouraud -> n x (u16 normal, u16 vertex)
//! ```

use crate::{bytes, u16_at, u32_at};
use anyhow::{Context, Result, bail};

/// The game's models have 256 cells, at most 3627 vertices and 1541 faces; the caps bound what a broken file can ask.
const MAX_CELLS: u32 = 4096;
const MAX_ITEMS: u64 = 1 << 16;

/// The offsets in a cell record are relative to this.
const T: u64 = 0xC;

pub struct Face {
    pub cmd: u8,
    /// 3 or 4 indices into the model's vertices (a quad's as a strip: 0-1-3-2 is its outline)
    pub idx: Vec<usize>,
    /// (tpage, clut) of a textured face
    pub tex: Option<(u32, u32)>,
    /// a textured face's texel per vertex
    pub uv: Vec<(i32, i32)>,
    /// a pre-lit face's colour per vertex
    pub col: Option<Vec<[u8; 3]>>,
}

/// The model's vertices and faces.
pub fn parse_stage(b: &[u8]) -> Result<(Vec<[i16; 3]>, Vec<Face>)> {
    let cells = u32_at(b, 8, "the model's cell count")?;
    if cells > MAX_CELLS {
        bail!("the model has {cells} cells");
    }
    let (mut verts, mut faces) = (Vec::new(), Vec::new());
    for i in 0..u64::from(cells) {
        let rec = |k: u64| u32_at(b, T + 28 * i + 4 * k, "a model cell");
        let (vo, nv, po, np) = (rec(0)?, rec(1)?, rec(4)?, rec(5)?);
        let base = verts.len();
        if base as u64 + u64::from(nv) > MAX_ITEMS || faces.len() as u64 + u64::from(np) > MAX_ITEMS {
            bail!("the model has too many vertices or faces");
        }
        for k in 0..u64::from(nv) {
            let c =
                |j: u64| Ok::<_, anyhow::Error>(u16_at(b, T + u64::from(vo) + 8 * k + 2 * j, "a model vertex")? as i16);
            verts.push([c(0)?, c(1)?, c(2)?]);
        }
        let mut p = T + u64::from(po);
        for _ in 0..np {
            let &[_, nw, prelit, cmd] = bytes(b, p, 4, "a model primitive")? else { bail!("a model primitive") };
            let nw = usize::from(nw) + 1;
            let words: Vec<u32> = bytes(b, p, nw * 4, "a model primitive")?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect();
            let word = |q: usize| words.get(q).copied().context("a model primitive is shorter than its command needs");
            let n = if cmd & 8 != 0 { 4 } else { 3 };
            let gouraud = cmd & 0x10 != 0;
            let (mut q, mut uv, mut tex) = (1, Vec::new(), None);
            if cmd & 4 != 0 {
                for j in 0..n {
                    uv.push(((word(q + j)? & 0xFF) as i32, ((word(q + j)? >> 8) & 0xFF) as i32));
                }
                tex = Some((word(q + 1)? >> 16, word(q)? >> 16));
                q += n;
            }
            let mut col = None;
            if prelit != 0 {
                let nc = if gouraud { n } else { 1 };
                let mut c = Vec::new();
                for j in 0..nc {
                    let w = word(q + j)?.to_le_bytes();
                    c.push([w[0], w[1], w[2]]);
                }
                q += nc;
                if !gouraud {
                    c = vec![c[0]; n];
                }
                col = Some(c);
            }
            // the rest as 16-bit halves: where the vertex indices are among them
            let half =
                |h: usize| Ok::<_, anyhow::Error>(base + ((word(q + h / 2)? >> (16 * (h % 2))) & 0xFFFF) as usize);
            let idx = (0..n)
                .map(|k| {
                    half(if prelit != 0 {
                        k
                    } else if gouraud {
                        2 * k + 1
                    } else {
                        k + 1
                    })
                })
                .collect::<Result<_>>()?;
            faces.push(Face { cmd, idx, tex, uv, col });
            p += nw as u64 * 4;
        }
    }
    Ok((verts, faces))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A model of one cell: 4 vertices and the primitives `prims` (each a record of words).
    fn model(prims: &[&[u32]]) -> Vec<u8> {
        let mut words = vec![0x41, 0, 1, /* the cell */ 28, 4, 0, 0, 28 + 32, prims.len() as u32, 0];
        for v in [[0i16, 0, 0], [100, 0, 0], [0, -100, 0], [100, -100, 50]] {
            words.extend([(v[0] as u16 as u32) | (v[1] as u16 as u32) << 16, v[2] as u16 as u32]);
        }
        words.extend(prims.iter().flat_map(|p| p.iter().copied()));
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn primitives() {
        // (byte3 the command, byte2 pre-lit, byte1 the record's words - 1)
        let prelit_tex_quad =
            [0x2D01_0700, 0x1234_0201, 0x5678_0403, 0x0605, 0x0807, 0x0030_2010, 0x0001_0000, 0x0003_0002];
        let lit_flat_tri = [0x2000_0200, 0x0000_0009, 0x0002_0001];
        let lit_gouraud_tri = [0x3000_0300, 0x0000_0009, 0x0001_0009, 0x0002_0009];
        let (verts, faces) = parse_stage(&model(&[&prelit_tex_quad, &lit_flat_tri, &lit_gouraud_tri])).unwrap();
        assert_eq!(verts, [[0, 0, 0], [100, 0, 0], [0, -100, 0], [100, -100, 50]]);
        let f = &faces[0];
        assert_eq!((f.cmd, &f.idx[..], f.tex), (0x2D, &[0, 1, 2, 3][..], Some((0x5678, 0x1234))));
        assert_eq!(f.uv, [(1, 2), (3, 4), (5, 6), (7, 8)]);
        assert_eq!(f.col, Some(vec![[0x10, 0x20, 0x30]; 4]));
        for f in &faces[1..] {
            assert_eq!((&f.idx[..], f.tex, &f.col), (&[0, 1, 2][..], None, &None));
        }
    }

    #[test]
    fn broken_models() {
        let whole = model(&[&[0x2000_0200, 0x0000_0009, 0x0002_0001]]);
        for n in 0..whole.len() {
            assert!(parse_stage(&whole[..n]).is_err(), "cut at {n}");
        }
        // a record too short for its command (a quad in two words), counts beyond the caps
        assert!(parse_stage(&model(&[&[0x2800_0100, 0x0001_0000]])).is_err());
        let mut many = whole.clone();
        many[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_stage(&many).is_err());
        many[8..12].copy_from_slice(&1u32.to_le_bytes());
        many[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_stage(&many).is_err());
    }
}

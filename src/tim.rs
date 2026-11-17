//! The stages' textures: PlayStation TIM files, and the video memory they are loaded into.
//!
//! A TIM: u32 0x10, u32 flags (bits 0-1 the depth: 0 = 4 bits, 1 = 8 bits, else 15-bit colour; bit 3 = it has a
//! CLUT), then a CLUT block if any and the image block, each `u32 length (of the whole block), u16 x, y, w, h` (the
//! rectangle in VRAM, in 16-bit words) and w * h words. A stage's texture file is a run of TIMs.

use crate::{bytes, u16_at, u32_at};
use anyhow::{Context, Result, bail};

/// One block of a TIM: its rectangle in VRAM and its words.
#[derive(Debug, PartialEq)]
pub struct Block<'a> {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub data: &'a [u8],
}

#[derive(Debug, PartialEq)]
pub struct Tim<'a> {
    pub bpp: u32,
    pub clut: Option<Block<'a>>,
    pub img: Block<'a>,
}

/// The block at `q` and the offset after it.
fn block(b: &[u8], q: u64) -> Result<(Block<'_>, u64)> {
    let len = u64::from(u32_at(b, q, "a TIM block")?);
    let h = |i: u64| u16_at(b, q + 4 + 2 * i, "a TIM block");
    // (a block that claims more than the file has is cut at the file's end; too few words fail at the upload)
    let end = (q + len).min(b.len() as u64);
    let data = bytes(b, q + 12, end.saturating_sub(q + 12) as usize, "a TIM block")?;
    Ok((Block { x: h(0)?, y: h(1)?, w: h(2)?, h: h(3)?, data }, q + len))
}

/// The TIMs of a texture file, up to the first thing that is not one.
pub fn parse_tims(b: &[u8]) -> Result<Vec<Tim<'_>>> {
    let (mut out, mut p) = (Vec::new(), 0u64);
    while p + 8 <= b.len() as u64 && u32_at(b, p, "a TIM")? == 0x10 {
        let flags = u32_at(b, p + 4, "a TIM")?;
        let mut q = p + 8;
        let mut clut = None;
        if flags & 8 != 0 {
            let (c, next) = block(b, q)?;
            (clut, q) = (Some(c), next);
        }
        let (img, next) = block(b, q)?;
        out.push(Tim { bpp: flags & 3, clut, img });
        p = next;
    }
    Ok(out)
}

/// The image rectangle of an uploaded TIM, its depth and its CLUT's place.
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub bpp: u32,
    pub clut: Option<(usize, usize)>,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.x <= x && x < self.x + self.w && self.y <= y && y < self.y + self.h
    }
}

pub const W: usize = 1024;
pub const H: usize = 1024;

/// The video memory a stage's TIMs are loaded into: 1024 x 1024 16-bit words.
pub struct Vram {
    px: Vec<u16>,
    /// true where a TIM wrote
    loaded: Vec<bool>,
    pub rects: Vec<Rect>,
}

impl Default for Vram {
    fn default() -> Self {
        Vram { px: vec![0; W * H], loaded: vec![false; W * H], rects: Vec::new() }
    }
}

impl Vram {
    fn upload(&mut self, b: &Block) -> Result<()> {
        let (x, w) = (usize::from(b.x), usize::from(b.w));
        let words = bytes(b.data, 0, w * usize::from(b.h) * 2, "a TIM's pixels")?;
        for (r, row) in words.chunks_exact((w * 2).max(1)).enumerate() {
            // (a row past the right edge runs on into the next one)
            let at = (usize::from(b.y) + r) % H * W + x;
            let (px, loaded) = (self.px.get_mut(at..at + w), self.loaded.get_mut(at..at + w));
            let (Some(px), Some(loaded)) = (px, loaded) else { bail!("a TIM's rectangle is outside the video memory") };
            for (d, s) in px.iter_mut().zip(row.as_chunks::<2>().0) {
                *d = u16::from_le_bytes(*s);
            }
            loaded.fill(true);
        }
        Ok(())
    }

    /// Loads a texture file's TIMs.
    pub fn load_tims(&mut self, file: &[u8]) -> Result<()> {
        for t in parse_tims(file)? {
            if let Some(c) = &t.clut {
                self.upload(c)?;
            }
            self.upload(&t.img)?;
            let (x, y, w, h) = (t.img.x.into(), t.img.y.into(), t.img.w.into(), t.img.h.into());
            let clut = t.clut.map(|c| (usize::from(c.x), usize::from(c.y)));
            self.rects.push(Rect { x, y, w, h, bpp: t.bpp, clut });
        }
        Ok(())
    }

    /// Whether a TIM wrote the word at (x, y).
    pub fn loaded(&self, x: usize, y: usize) -> bool {
        self.loaded.get(y * W + x).copied().unwrap_or(false)
    }

    /// Texel `u` of the row starting at VRAM word (vx, vy), of depth `bpp`, through the CLUT at word (cx, cy).
    pub fn raw_texel(&self, vx: usize, vy: usize, u: usize, bpp: u32, cx: usize, cy: usize) -> Result<u16> {
        let word = |x: usize| self.px.get(vy % H * W + x % W).copied().context("a texel outside the video memory");
        let clut =
            |i: u16| self.px.get(cy * W + cx + usize::from(i)).copied().context("a CLUT outside the video memory");
        match bpp {
            0 => clut((word(vx + u / 4)? >> ((u & 3) * 4)) & 0xF),
            1 => clut((word(vx + u / 2)? >> ((u & 1) * 8)) & 0xFF),
            _ => word(vx + u),
        }
    }

    /// The colour of texel (u, v) of a texture page with a CLUT, by the PlayStation's rules.
    pub fn texel(&self, tpage: u32, clut: u32, u: usize, v: usize) -> Result<u16> {
        let (px, py, _) = page_origin(tpage);
        let (cx, cy) = ((clut & 0x3F) as usize * 16, ((clut >> 6) & 0x3FF) as usize);
        self.raw_texel(px as usize, py as usize + v, u, (tpage >> 7) & 3, cx, cy)
    }
}

/// A texture page's corner in VRAM, and how many texels of its depth one word holds.
pub fn page_origin(tpage: u32) -> (i32, i32, i32) {
    let div = [4, 2, 1, 1][((tpage >> 7) & 3) as usize];
    ((tpage & 0xF) as i32 * 64, ((tpage >> 4) & 1) as i32 * 256 + ((tpage >> 11) & 1) as i32 * 512, div)
}

/// A 15-bit colour as RGBA: colour 0 is transparent; with `semi`, the STP bit makes a blended texel.
pub fn rgba15(c: u16, semi: bool) -> [u8; 4] {
    if c == 0 {
        return [0, 0, 0, 0];
    }
    let c5 = |v: u16| ((v & 31) << 3 | (v & 31) >> 2) as u8;
    [c5(c), c5(c >> 5), c5(c >> 10), if semi && c & 0x8000 != 0 { 128 } else { 255 }]
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(rgba15(0, false), [0, 0, 0, 0]);
        assert_eq!(rgba15(0x7FFF, true), [255, 255, 255, 255]);
        assert_eq!(rgba15(0x8000, false), [0, 0, 0, 255]);
        assert_eq!(rgba15(0x8000, true), [0, 0, 0, 128]);
        // r = 1, g = 16, b = 31
        assert_eq!(rgba15(1 | 16 << 5 | 31 << 10, false), [8, 132, 255, 255]);
    }

    /// A 4-bit TIM: a 16-colour CLUT at (0, 480), an 8 x 2 texel image (2 x 2 words) at (320, 0).
    fn tim4() -> Vec<u8> {
        let mut t = Vec::new();
        for v in [0x10u32, 8, 12 + 32] {
            t.extend(v.to_le_bytes());
        }
        for v in [0u16, 480, 16, 1].into_iter().chain((0..16).map(|i| i * 0x421)) {
            t.extend(v.to_le_bytes());
        }
        t.extend((12u32 + 8).to_le_bytes());
        for v in [320u16, 0, 2, 2, 0x3210, 0x7654, 0xBA98, 0xFEDC] {
            t.extend(v.to_le_bytes());
        }
        t
    }

    #[test]
    fn tim() {
        let mut file = tim4();
        file.extend(tim4());
        file.extend([0xEE; 9]);
        let tims = parse_tims(&file).unwrap();
        assert_eq!(tims.len(), 2);
        assert_eq!(tims[0], tims[1]);
        let t = &tims[0];
        assert_eq!(t.bpp, 0);
        let c = t.clut.as_ref().unwrap();
        assert_eq!((c.x, c.y, c.w, c.h, c.data.len()), (0, 480, 16, 1, 32));
        assert_eq!((t.img.x, t.img.y, t.img.w, t.img.h), (320, 0, 2, 2));
        assert_eq!(t.img.data, [0x10, 0x32, 0x54, 0x76, 0x98, 0xBA, 0xDC, 0xFE]);

        let mut vram = Vram::default();
        vram.load_tims(&file).unwrap();
        assert!(vram.loaded(321, 1) && !vram.loaded(322, 1));
        assert!(vram.rects[0].contains(321, 1) && !vram.rects[0].contains(322, 1));
        assert_eq!(vram.rects[0].clut, Some((0, 480)));
        // texture page 5 (x = 320), 4-bit; CLUT word 480 << 6: texel u of row 0 is colour u, of row 1 colour 8 + u
        for u in 0..8 {
            assert_eq!(vram.texel(5, 480 << 6, u, 0).unwrap(), u as u16 * 0x421);
            assert_eq!(vram.texel(5, 480 << 6, u, 1).unwrap(), (8 + u as u16) * 0x421);
        }
        assert_eq!(vram.raw_texel(320, 1, 3, 0, 0, 480).unwrap(), 11 * 0x421);
    }

    #[test]
    fn broken_tims() {
        let whole = tim4();
        // cut anywhere: an error or fewer TIMs, never a panic
        for n in 0..whole.len() {
            let _ = Vram::default().load_tims(&whole[..n]);
        }
        assert!(Vram::default().load_tims(&whole[..whole.len() - 1]).is_err());
        // a rectangle past the last word
        let mut low = whole.clone();
        // (the image block is after the 8-byte header and the 44-byte CLUT block; its x and y after its length)
        low[56..58].copy_from_slice(&1023u16.to_le_bytes());
        low[58..60].copy_from_slice(&1023u16.to_le_bytes());
        assert!(Vram::default().load_tims(&low).is_err());
    }
}

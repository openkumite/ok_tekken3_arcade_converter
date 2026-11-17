//! Texture patches cut out of the video memory, and the atlas they are packed into.

use crate::tim::{Vram, rgba15};
use anyhow::{Context, Result, bail};

/// An RGBA picture, rows top down.
pub struct Patch {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// `w` x `h` texels of a texture page from (u0, v0), wrapping in the page as the PlayStation does.
pub fn decode_patch(
    vram: &Vram,
    tpage: u32,
    clut: u32,
    (u0, v0): (i32, i32),
    (w, h): (usize, usize),
    semi: bool,
) -> Result<Patch> {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for v in (v0..).take(h) {
        for u in (u0..).take(w) {
            rgba.extend(rgba15(vram.texel(tpage, clut, (u & 0xFF) as usize, (v & 0xFF) as usize)?, semi));
        }
    }
    Ok(Patch { w, h, rgba })
}

/// The atlas: its picture, and where each patch is in it (in the order given).
pub struct Atlas {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub place: Vec<(usize, usize)>,
}

/// No stage of the game needs more than 2048 rows.
const MAX_HEIGHT: usize = 8192;

/// A shelf packer, tallest patches first; each patch gets a border of one texel, its own edge repeated. The height
/// is a power of two.
pub fn build(width: usize, items: &[Patch]) -> Result<Atlas> {
    const PAD: usize = 1;
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(items[i].h));
    let (mut x, mut y, mut shelf) = (0, 0, 0);
    let mut place = vec![(0, 0); items.len()];
    for i in order {
        let (w, h) = (items[i].w, items[i].h);
        if w == 0 || h == 0 || w + 2 * PAD > width {
            bail!("a texture patch of {w} x {h} texels does not fit the atlas");
        }
        if x + w + 2 * PAD > width {
            (x, y, shelf) = (0, y + shelf, 0);
        }
        place[i] = (x + PAD, y + PAD);
        x += w + 2 * PAD;
        shelf = shelf.max(h + 2 * PAD);
        if y + shelf > MAX_HEIGHT {
            bail!("the stage's textures do not fit an atlas of {width} x {MAX_HEIGHT}");
        }
    }
    let h = (y + shelf).next_power_of_two();
    let mut rgba = vec![0; width * h * 4];
    for (item, &(px, py)) in items.iter().zip(&place) {
        for r in 0..item.h + 2 * PAD {
            let src = item.rgba.chunks_exact(item.w * 4).nth(r.saturating_sub(PAD).min(item.h - 1));
            let src = src.context("a texture patch smaller than its size")?;
            let (first, last) = (&src[..4], &src[src.len() - 4..]);
            let at = ((py - PAD + r) * width + px - PAD) * 4;
            let line = rgba.get_mut(at..at + (item.w + 2 * PAD) * 4).context("a texture patch outside the atlas")?;
            for (d, s) in line.iter_mut().zip(first.iter().chain(src).chain(last)) {
                *d = *s;
            }
        }
    }
    Ok(Atlas { w: width, h, rgba, place })
}

/// An RGBA picture as a PNG.
pub fn png(w: usize, h: usize, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w.try_into()?, h.try_into()?);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header()?;
    wr.write_image_data(rgba)?;
    wr.finish()?;
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn packs_with_borders() {
        let patch = |w: usize, h: usize, v: u8| Patch { w, h, rgba: vec![v; w * h * 4] };
        // 8 wide: the tall one first, the second beside it, the third on the next shelf
        let a = build(8, &[patch(2, 1, 1), patch(2, 3, 2), patch(3, 1, 3)]).unwrap();
        assert_eq!(a.place, [(5, 1), (1, 1), (1, 6)]);
        assert_eq!((a.w, a.h), (8, 8));
        let at = |x: usize, y: usize| a.rgba[(y * 8 + x) * 4];
        assert_eq!(
            [at(0, 0), at(3, 4), at(4, 0), at(7, 2), at(4, 3), at(0, 5), at(4, 7), at(5, 7)],
            [2, 2, 1, 1, 0, 3, 3, 0]
        );
        assert!(build(8, &[patch(7, 1, 1)]).is_err());
        assert!(build(8, &[patch(2, MAX_HEIGHT, 1)]).is_err());
        assert_eq!(build(8, &[]).unwrap().h, 1);
    }
}

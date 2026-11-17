//! The ROM set and what is read straight from it.
//!
//! Data ROMs `tet1rom{0,1,2}{l,u}` are byte-interleaved pairs forming one linear "banked" region. The game's file
//! table lists each stage as one model file followed by a run of TIM files ([`STAGES_RAW`]).
//!
//! The main program (`tet*.2e`/`.2j`, byte-interleaved) is LZ-compressed in ROM; a boot table entry
//! `{src, dst = 0x80010000}` points at it. A flag byte is read LSB-first with a leading-1 sentinel: bit 1 = a literal
//! byte, bit 0 = a big-endian u16 back-reference (len = (v >> 11) & 31 or 32, dist = v & 0x7ff or 0x800). In the
//! executable a table of 13 `{u32 index, u32 ptr}` pairs points at each stage's floor table: u32 count (128), then
//! `{u32 tpage << 16 | v << 8 | u, u32 clut << 16}` entries, of which entries 0..99 are the floor's 10 x 10 map.

use crate::{floor, u32_at};
use anyhow::{Context, Result, bail};
use ok_converter_common::io::Vfs;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{Read, Seek};
use zip::ZipArchive;

/// A set of ROM files by name (`tet1rom0l.6`).
pub trait RomSet {
    fn has(&self, name: &str) -> bool;
    fn read(&self, name: &str) -> Result<Vec<u8>>;
}

/// The ROM set unpacked in a folder.
pub struct FolderRoms<'a>(pub &'a dyn Vfs);

impl RomSet for FolderRoms<'_> {
    fn has(&self, name: &str) -> bool {
        self.0.exists(name)
    }
    fn read(&self, name: &str) -> Result<Vec<u8>> {
        self.0.read(name)
    }
}

/// No ROM of the set is larger than 4 MiB.
const MAX_ROM: u64 = 16 << 20;

/// The ROM set as MAME's .zip (bytes: `ZipRoms::new(Cursor::new(bytes))`); a ROM is found by its base name, whatever
/// folder the archive keeps it in.
pub struct ZipRoms<R> {
    zip: RefCell<ZipArchive<R>>,
    names: BTreeMap<String, usize>,
}

impl<R: Read + Seek> ZipRoms<R> {
    pub fn new(reader: R) -> Result<Self> {
        let zip = ZipArchive::new(reader).context("the ROM set is not a zip archive")?;
        let names = (0..zip.len())
            .filter_map(|i| {
                let name = zip.name_for_index(i)?.ok()?;
                Some((name.rsplit('/').next()?.to_string(), i))
            })
            .collect();
        Ok(ZipRoms { zip: RefCell::new(zip), names })
    }
}

impl<R: Read + Seek> RomSet for ZipRoms<R> {
    fn has(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }
    fn read(&self, name: &str) -> Result<Vec<u8>> {
        let i = *self.names.get(name).with_context(|| format!("the ROM set has no {name}"))?;
        let mut zip = self.zip.try_borrow_mut()?;
        let mut out = Vec::new();
        zip.by_index(i)
            .and_then(|f| Ok(f.take(MAX_ROM + 1).read_to_end(&mut out)?))
            .with_context(|| format!("cannot read {name} from the ROM set"))?;
        if out.len() as u64 > MAX_ROM {
            bail!("{name} is too large for a ROM");
        }
        Ok(out)
    }
}

pub const DATA_ROMS: [(&str, &str); 3] =
    [("tet1rom0l.6", "tet1rom0u.9"), ("tet1rom1l.7", "tet1rom1u.10"), ("tet1rom2l.8", "tet1rom2u.11")];

/// Program ROM pairs `NAME.2e`/`NAME.2j` (any one works; the parent "World TET2/VER.E1" is tried first).
pub const PROGRAM_ROMS: [&str; 7] =
    ["tet2vere1", "tet3verd", "tet1vere1", "tet2verb", "tet2vera", "tet3vera", "tet1vera"];

/// (offset, size) in the banked data region, from the game's own file table. Each stage is one model file followed
/// by its TIM texture files.
const STAGES_RAW: [(usize, usize); 66] = [
    (0x11d6678, 0x0c0d0),
    (0x11e2748, 0x14864),
    (0x11f6fac, 0x14ae4),
    (0x120ba90, 0x14ba4),
    (0x1220634, 0x14f84),
    (0x12355b8, 0x14d84),
    (0x124a33c, 0x10804),
    (0x125ab7c, 0x0f204),
    (0x1269d80, 0x14744),
    (0x127e4c4, 0x14e24),
    (0x12932e8, 0x14be4),
    (0x12a7ecc, 0x14a04),
    (0x12bc8d0, 0x14a04),
    (0x12d12d4, 0x04204),
    (0x12d550c, 0x0b104),
    (0x12e0610, 0x14c44),
    (0x12f5254, 0x14724),
    (0x1309978, 0x14928),
    (0x131e2a0, 0x14a04),
    (0x1332ca4, 0x05ac4),
    (0x133878c, 0x0ca4c),
    (0x13451d8, 0x148b4),
    (0x1359a8c, 0x14c2c),
    (0x136e6b8, 0x13144),
    (0x1381828, 0x0c860),
    (0x138e088, 0x14e2c),
    (0x13a2eb4, 0x14a04),
    (0x13b78b8, 0x13984),
    (0x13cb23c, 0x03184),
    (0x13ce3f4, 0x0b79c),
    (0x13d9b90, 0x14a44),
    (0x13ee5d4, 0x14b68),
    (0x140313c, 0x14c44),
    (0x1417d80, 0x14a04),
    (0x142c784, 0x01084),
    (0x142d82c, 0x0db6c),
    (0x143b398, 0x14ac4),
    (0x144fe5c, 0x14c04),
    (0x1464a60, 0x00844),
    (0x14652c0, 0x0928c),
    (0x146e54c, 0x14524),
    (0x1482a70, 0x0ed84),
    (0x1491838, 0x0fe3c),
    (0x14a1674, 0x14ee4),
    (0x14b6558, 0x14e24),
    (0x14cb37c, 0x14e24),
    (0x14e01a0, 0x14f04),
    (0x14f50a4, 0x14a04),
    (0x1509aa8, 0x14a04),
    (0x151e4ac, 0x03184),
    (0x152164c, 0x14e04),
    (0x1536450, 0x14ea4),
    (0x154b2f4, 0x0bdc4),
    (0x15570ec, 0x0c674),
    (0x1563760, 0x14c44),
    (0x15783a4, 0x14a04),
    (0x158cda8, 0x14a04),
    (0x15a17ac, 0x14a04),
    (0x15b61b0, 0x11884),
    (0x15c7a50, 0x0616c),
    (0x15cdbbc, 0x14948),
    (0x15e2504, 0x14a04),
    (0x15f6f2c, 0x0b064),
    (0x1601f90, 0x14884),
    (0x1616814, 0x14d04),
    (0x162b518, 0x0ad44),
];

/// The two halves of a ROM pair as one: `a`'s bytes at the even offsets, `b`'s at the odd.
pub fn interleave(a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
    if a.len() != b.len() {
        bail!("the halves of a ROM pair differ in size ({} and {} bytes)", a.len(), b.len());
    }
    Ok(a.iter().zip(b).flat_map(|(&a, &b)| [a, b]).collect())
}

/// The banked data region: the three data ROM pairs, one after another.
pub fn load_banked(roms: &dyn RomSet) -> Result<Vec<u8>> {
    let mut bank = Vec::new();
    for (lo, hi) in DATA_ROMS {
        bank.extend(interleave(&roms.read(lo)?, &roms.read(hi)?).with_context(|| format!("{lo} and {hi}"))?);
    }
    Ok(bank)
}

/// One stage's files in the banked region.
pub struct Stage<'a> {
    pub model: &'a [u8],
    pub tims: Vec<&'a [u8]>,
}

/// The stage files grouped: a file that is not a TIM starts a stage, the TIMs after it are its textures.
pub fn group_stages(bank: &[u8]) -> Result<Vec<Stage<'_>>> {
    let mut stages: Vec<Stage> = Vec::new();
    for (off, size) in STAGES_RAW {
        let file = crate::bytes(bank, off as u64, size, "a stage file")?;
        if u32_at(file, 0, "a stage file")? != 0x10 {
            stages.push(Stage { model: file, tims: Vec::new() });
        } else {
            stages.last_mut().context("a texture file before the first stage model")?.tims.push(file);
        }
    }
    Ok(stages)
}

/// The unpacked program is 2.7 MiB; the cap keeps a broken stream from growing without end.
const MAX_EXE: usize = 8 << 20;

/// The LZ stream of the program ROM at `pos`, unpacked (the module docs have the format).
pub fn lz_decompress(src: &[u8], mut pos: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut next = || -> Result<u8> {
        let b = *src.get(pos).context("the compressed program ends early")?;
        pos += 1;
        Ok(b)
    };
    loop {
        let mut f = next()?;
        if f == 0 {
            return Ok(out);
        }
        while f >= 2 {
            if f & 1 != 0 {
                out.push(next()?);
            } else {
                let v = usize::from(u16::from_be_bytes([next()?, next()?]));
                let d = if v & 0x7FF == 0 { 0x800 } else { v & 0x7FF };
                let n = if v >> 11 == 0 { 32 } else { v >> 11 };
                let s = out.len().checked_sub(d).context("the compressed program refers back before its start")?;
                // (byte by byte: the copy may overlap what it writes)
                for k in s..s + n {
                    let b = *out.get(k).context("the compressed program refers past its end")?;
                    out.push(b);
                }
            }
            f >>= 1;
        }
        if out.len() > MAX_EXE {
            bail!("the compressed program unpacks to more than {MAX_EXE} bytes");
        }
    }
}

/// Where the program ROM is mapped, and where its executable is unpacked to.
const ROM_BASE: u32 = 0x1FC0_0000;
const EXE_BASE: u32 = 0x8001_0000;

/// The 100 `(w0, w1)` entries of a stage's floor map.
pub type FloorTable = Vec<(u32, u32)>;

/// The floor tables of the 13 stages in one program ROM (both halves interleaved).
fn floor_tables_in(prog: &[u8]) -> Result<Vec<FloorTable>> {
    let src = (0..0x40000u64)
        .step_by(4)
        .find_map(|p| {
            let (s, d) = (u32_at(prog, p, "").ok()?, u32_at(prog, p + 4, "").ok()?);
            let s = usize::try_from(s.checked_sub(ROM_BASE)?).ok()?;
            (d == EXE_BASE && s < prog.len()).then_some(s)
        })
        .context("no boot table entry for the main program")?;
    let exe = lz_decompress(prog, src)?;
    let words: Vec<u32> = exe.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    let in_exe = |p: u32| p.checked_sub(EXE_BASE).is_some_and(|o| (o as usize) < exe.len());
    let table = words
        .windows(26)
        .find(|t| t.as_chunks::<2>().0.iter().enumerate().all(|(k, &[i, p])| i as usize == k && in_exe(p)))
        .context("no table of the stages' floors in the main program")?;
    let mut tables = Vec::new();
    for ptr in table.iter().skip(1).step_by(2) {
        let o = u64::from(ptr - EXE_BASE) + 4;
        let entry = |j: u64| -> Result<(u32, u32)> {
            Ok((u32_at(&exe, o + 8 * j, "a floor table")?, u32_at(&exe, o + 8 * j + 4, "a floor table")?))
        };
        tables.push((0..floor::CELLS as u64).map(entry).collect::<Result<_>>()?);
    }
    Ok(tables)
}

/// The floor tables of the first program ROM pair of the set that has them, and the pair's name.
pub fn load_floor_tables(roms: &dyn RomSet) -> Result<(&'static str, Vec<FloorTable>)> {
    let mut last = None;
    for ver in PROGRAM_ROMS {
        let (e, j) = (format!("{ver}.2e"), format!("{ver}.2j"));
        if !roms.has(&e) || !roms.has(&j) {
            continue;
        }
        let tables = (|| floor_tables_in(&interleave(&roms.read(&e)?, &roms.read(&j)?)?))();
        match tables {
            Ok(t) => return Ok((ver, t)),
            Err(err) => last = Some(err.context(format!("{e} and {j}"))),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("the ROM set has no program ROM pair (tet*.2e and tet*.2j)")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ok_converter_common::io::StdFs;
    use std::io::{Cursor, Write};

    #[test]
    fn interleaves() {
        assert_eq!(interleave(&[1, 3, 5], &[2, 4, 6]).unwrap(), [1, 2, 3, 4, 5, 6]);
        assert!(interleave(&[1, 2], &[1]).is_err());
    }

    #[test]
    fn lz() {
        // flags 0b1_0111: three literals, one back-reference (len 2, dist 3); then the end
        assert_eq!(lz_decompress(&[0x17, b'a', b'b', b'c', 0x10, 0x03, 0], 0).unwrap(), b"abcab");
        // len bits 0 = 32, an overlapping copy at dist 1
        assert_eq!(lz_decompress(&[0x05, 7, 0x00, 0x01, 0], 0).unwrap(), [7; 33]);
        // dist bits 0 = 0x800: 0x800 literals, then a copy of the first two
        let mut src = Vec::new();
        for chunk in (0..0x800u32).map(|i| i as u8).collect::<Vec<_>>().chunks(7) {
            src.push(0xFF);
            src.extend(chunk);
        }
        // (0x800 = 292 * 7 + 4: the last flag byte has four literals; its bits, a 1 sentinel over 1111)
        let last = src.len() - 5;
        src[last] = 0x1F;
        src.extend([0x02, 0x10, 0x00, 0]);
        let out = lz_decompress(&src, 0).unwrap();
        assert_eq!(out.len(), 0x802);
        assert_eq!(out[0x800..], [0, 1]);
        // from an offset
        assert_eq!(lz_decompress(&[9, 9, 0x03, 5, 0], 2).unwrap(), [5]);
    }

    #[test]
    fn lz_errors() {
        assert!(lz_decompress(&[0x17, b'a', b'b'], 0).is_err(), "truncated");
        assert!(lz_decompress(&[], 0).is_err());
        assert!(lz_decompress(&[0x02, 0x10, 0x03, 0], 0).is_err(), "a reference before the start");
        // a stream that never ends: literal, then copies of 32 for ever
        let mut bomb = vec![0x03, 1];
        bomb.resize(2 + 3 * (MAX_EXE / 32 + 8), 0);
        for c in bomb[2..].as_chunks_mut::<3>().0 {
            *c = [0x02, 0x00, 0x01];
        }
        assert!(lz_decompress(&bomb, 0).is_err(), "too large");
    }

    #[test]
    fn zip_and_folder_read_the_same() {
        let files: [(&str, &[u8]); 2] = [("tet1rom0l.6", b"low half"), ("tet2vere1.2e", &[0, 1, 2, 3])];
        let dir = std::env::temp_dir().join(format!("t3_arcade_roms_{}", std::process::id()));
        let fs = StdFs(dir.clone());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in files {
            fs.write(name, data).unwrap();
            // (MAME's merged sets keep the ROMs in folders)
            zip.start_file(format!("tekken3/{name}"), zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        let (zipped, folder) = (ZipRoms::new(zip.finish().unwrap()).unwrap(), FolderRoms(&fs));
        for (name, data) in files {
            assert!(zipped.has(name) && folder.has(name));
            assert_eq!(zipped.read(name).unwrap(), data);
            assert_eq!(folder.read(name).unwrap(), data);
        }
        assert!(!zipped.has("nothing.2e") && !folder.has("nothing.2e"));
        assert!(zipped.read("nothing.2e").is_err() && folder.read("nothing.2e").is_err());
        assert!(ZipRoms::new(Cursor::new(b"not a zip".to_vec())).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

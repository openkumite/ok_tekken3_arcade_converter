//! The ROM set and what is read straight from it.
//!
//! Data ROMs `tet1rom{0,1,2}{l,u}` are byte-interleaved pairs forming one linear "banked" region. The program's
//! file table gives each stage an archive, one model file followed by texture files (runs of TIMs): twelve in the
//! banked region, the eighth stage's in the program ROM with its texture files LZ-packed (`stages_in`).
//!
//! The main program (`tet*.2e`/`.2j`, byte-interleaved) is LZ-compressed in ROM; a boot table entry
//! `{src, dst = 0x80010000}` points at it. A flag byte is read LSB-first with a leading-1 sentinel: bit 1 = a literal
//! byte, bit 0 = a big-endian u16 back-reference (len = (v >> 11) & 31 or 32, dist = v & 0x7ff or 0x800). In the
//! executable a table of 13 `{u32 index, u32 ptr}` pairs points at each stage's floor table: u32 count (128), then
//! `{u32 tpage << 16 | v << 8 | u, u32 clut << 16}` entries, of which entries 0..99 are the floor's 10 x 10 map.

use crate::sky::{self, Sky};
use crate::{bytes, floor, u32_at};
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

/// Each stage's group of files: the program's table at 0x8017C320 (the addresses here are TET2/VER.E1's), the same
/// words in every program version.
const STAGE_GROUPS: [u32; 13] = [0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x24, 0x1F, 0x20, 0x21, 0x22, 0x23];
/// A file address from here on is an offset in the program ROM; below it, in the banked data region.
const PROGRAM_FILES: u32 = 0x200_0000;
/// The stages' archives have 3 to 9 files.
const MAX_FILES: u32 = 64;

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

/// One stage's files: its model, and its texture files (runs of TIMs), unpacked.
pub struct Stage {
    pub model: Vec<u8>,
    pub tims: Vec<Vec<u8>>,
}

/// The files of the archive at the address `addr`: `u32 n, n x {u32 offset (from the archive), u32 size}`.
fn archive<'a>(bank: &'a [u8], prog: &'a [u8], addr: u32) -> Result<Vec<&'a [u8]>> {
    let (rom, at) = match addr.checked_sub(PROGRAM_FILES) {
        Some(o) => (prog, u64::from(o)),
        None => (bank, u64::from(addr)),
    };
    let n = u32_at(rom, at, "an archive's file count")?;
    if !(2..=MAX_FILES).contains(&n) {
        bail!("an archive of {n} files at {addr:#x}: not a stage's");
    }
    let file = |i: u64| {
        let (o, size) =
            (u32_at(rom, at + 4 + 8 * i, "an archive's files")?, u32_at(rom, at + 8 + 8 * i, "an archive's files")?);
        bytes(rom, at + u64::from(o), size as usize, "a stage file")
    };
    (0..u64::from(n)).map(file).collect()
}

/// A stage of its archive's files: the model, then the texture files; one that is not a TIM is LZ-packed, as the
/// program is (the stage kept in the program ROM).
fn stage(files: &[&[u8]]) -> Result<Stage> {
    let (model, tims) = files.split_first().context("a stage's archive is empty")?;
    if u32_at(model, 0, "a stage's model")? == TIM {
        bail!("a stage's first file is a texture, not its model");
    }
    let tim = |t: &&[u8]| {
        let t = if u32_at(t, 0, "a stage's texture file")? == TIM { t.to_vec() } else { lz_decompress(t, 0)? };
        if u32_at(&t, 0, "a stage's unpacked texture file")? != TIM {
            bail!("a stage's texture file is not a TIM, packed or not");
        }
        Ok(t)
    };
    Ok(Stage { model: model.to_vec(), tims: tims.iter().map(tim).collect::<Result<_>>()? })
}

/// The 13 stages' files. The program's group table (0x801FF738 in TET2/VER.E1: 0x2C entries `{u32 pointer to a
/// list of archive addresses, u32 count}`) is found by what it holds: the first place in the program where every
/// stage's group is one archive of a model and its textures.
fn stages_in(exe: &[u8], words: &[u32], bank: &[u8], prog: &[u8]) -> Result<Vec<Stage>> {
    if !words.windows(STAGE_GROUPS.len()).any(|w| w == STAGE_GROUPS) {
        bail!("no table of the stages' file groups in the main program");
    }
    let stage_at = |p: usize, g: u32| {
        let (&ptr, &count) = (words.get(p + 2 * g as usize)?, words.get(p + 2 * g as usize + 1)?);
        let addr = u32_at(exe, u64::from(ptr.checked_sub(EXE_BASE).filter(|_| count == 1)?), "").ok()?;
        stage(&archive(bank, prog, addr).ok()?).ok()
    };
    (0..words.len())
        .find_map(|p| STAGE_GROUPS.iter().map(|&g| stage_at(p, g)).collect())
        .context("no table of the stages' files in the main program")
}

/// The unpacked program is 2.7 MiB; the cap keeps a broken stream from growing without end.
const MAX_EXE: usize = 8 << 20;

/// The LZ stream of `src` at `pos`, unpacked (the module docs have the format).
pub fn lz_decompress(src: &[u8], mut pos: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut next = || -> Result<u8> {
        let b = *src.get(pos).context("the compressed data ends early")?;
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
                let s = out.len().checked_sub(d).context("the compressed data refers back before its start")?;
                // (byte by byte: the copy may overlap what it writes)
                for k in s..s + n {
                    let b = *out.get(k).context("the compressed data refers past its end")?;
                    out.push(b);
                }
            }
            f >>= 1;
        }
        if out.len() > MAX_EXE {
            bail!("the compressed data unpacks to more than {MAX_EXE} bytes");
        }
    }
}

/// Where the program ROM is mapped, and where its executable is unpacked to.
const ROM_BASE: u32 = 0x1FC0_0000;
pub(crate) const EXE_BASE: u32 = 0x8001_0000;
/// The first word of a TIM.
const TIM: u32 = 0x10;

/// The 100 `(w0, w1)` entries of a stage's floor map.
pub type FloorTable = Vec<(u32, u32)>;

/// The floor tables of the 13 stages.
fn floor_tables_in(exe: &[u8], words: &[u32]) -> Result<Vec<FloorTable>> {
    let in_exe = |p: u32| p.checked_sub(EXE_BASE).is_some_and(|o| (o as usize) < exe.len());
    let table = words
        .windows(26)
        .find(|t| t.as_chunks::<2>().0.iter().enumerate().all(|(k, &[i, p])| i as usize == k && in_exe(p)))
        .context("no table of the stages' floors in the main program")?;
    let mut tables = Vec::new();
    for ptr in table.iter().skip(1).step_by(2) {
        let o = u64::from(ptr - EXE_BASE) + 4;
        let entry = |j: u64| -> Result<(u32, u32)> {
            Ok((u32_at(exe, o + 8 * j, "a floor table")?, u32_at(exe, o + 8 * j + 4, "a floor table")?))
        };
        tables.push((0..floor::CELLS as u64).map(entry).collect::<Result<_>>()?);
    }
    Ok(tables)
}

/// What is read through one program ROM pair: each of the 13 stages' files, floor table and sky.
pub struct Program {
    /// the pair's name
    pub name: &'static str,
    pub stages: Vec<Stage>,
    pub floors: Vec<FloorTable>,
    pub skies: Vec<Sky>,
}

/// The stages of one program ROM (both halves interleaved) and the banked data region.
fn program_in(name: &'static str, bank: &[u8], prog: &[u8]) -> Result<Program> {
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
    Ok(Program {
        name,
        stages: stages_in(&exe, &words, bank, prog)?,
        floors: floor_tables_in(&exe, &words)?,
        skies: sky::parse_tables(&exe)?,
    })
}

/// The stages read through the first program ROM pair of the set that has its tables. Every pair of
/// [`PROGRAM_ROMS`] does: the tables are found by their contents, not at one version's addresses.
pub fn load_program(roms: &dyn RomSet) -> Result<Program> {
    let bank = load_banked(roms)?;
    let mut last = None;
    for ver in PROGRAM_ROMS {
        let (e, j) = (format!("{ver}.2e"), format!("{ver}.2j"));
        if !roms.has(&e) || !roms.has(&j) {
            continue;
        }
        match (|| program_in(ver, &bank, &interleave(&roms.read(&e)?, &roms.read(&j)?)?))() {
            Ok(p) => return Ok(p),
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

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn stage_files() {
        // an archive of a model and a TIM file at 8 of the banked region; in the program ROM at 4, the same with
        // the TIM file packed (flags 0b1_1111: four literals)
        let bank = le(&[0, 0, 2, 20, 8, 28, 4, 0x41, 0x100, TIM]);
        let mut prog = le(&[0, 2, 20, 4, 24, 6, 0x41]);
        prog.extend([0x1F, 0x10, 0, 0, 0, 0]);
        let files = archive(&bank, &prog, 8).unwrap();
        assert_eq!(files, [&le(&[0x41, 0x100])[..], &le(&[TIM])[..]]);
        for addr in [8, PROGRAM_FILES + 4] {
            let st = stage(&archive(&bank, &prog, addr).unwrap()).unwrap();
            assert_eq!((st.model[0], &st.tims), (0x41, &vec![le(&[TIM])]), "{addr:#x}");
        }
        // an address outside, a count beyond the cap, a file past the end; a TIM first, a texture file that is none
        assert!(archive(&bank, &prog, 0x100).is_err() && archive(&bank, &prog, PROGRAM_FILES + 0x100).is_err());
        assert!(archive(&le(&[MAX_FILES + 1]), &prog, 0).is_err());
        assert!(archive(&bank[..36], &prog, 8).is_err());
        assert!(stage(&[&le(&[TIM]), &le(&[TIM])]).is_err());
        assert!(stage(&[&le(&[0x41]), &[0x1F, 0x11, 0, 0, 0, 0]]).is_err());
        assert!(stage(&[&le(&[0x41]), &[0x1F, 0x10]]).is_err() && stage(&[]).is_err());

        // the program: the stage groups' table, 3 words of something else, then the group table (every group one
        // archive: the list at word 16 for all but group 0x24, whose list at word 17 names the program ROM's)
        let mut words = STAGE_GROUPS.to_vec();
        words.extend([7, 7, 7, 8, PROGRAM_FILES + 4, 7]);
        for g in 0..0x2C {
            words.extend([EXE_BASE + if g == 0x24 { 17 * 4 } else { 16 * 4 }, 1]);
        }
        let exe = le(&words);
        let stages = stages_in(&exe, &words, &bank, &prog).unwrap();
        assert_eq!(stages.len(), 13);
        assert!(stages.iter().all(|s| s.model == le(&[0x41, 0x100]) && s.tims == [le(&[TIM])]));
        // no table of the stages' groups; groups of two archives
        assert!(stages_in(&exe, &words[13..], &bank, &prog).is_err());
        words[19..].as_chunks_mut::<2>().0.iter_mut().for_each(|g| g[1] = 2);
        assert!(stages_in(&exe, &words, &bank, &prog).is_err());
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

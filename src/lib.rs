//! The stages of the Tekken 3 arcade game (Namco System 12, 1997; MAME's `tekken3` ROM set) as glbs: read from the
//! ROM set (`roms`: a .zip or a folder; the data region, the program's file, floor and sky tables), each stage's
//! textures (`tim`), its model (`model`) built into meshes (`stage`, `atlas`), its fighting floor (`floor`), its sky
//! (`sky`), written as a glb (`glb`) into a pack (`pack`). Files go through `ok_converter_common::io::Vfs`; `main` is the command line.
//!
//! Everything in the ROMs is little-endian and untrusted: a read outside the data is an error, never a panic.

pub mod atlas;
pub mod floor;
pub mod glb;
pub mod model;
pub mod objects;
pub mod pack;
pub mod roms;
pub mod sky;
pub mod stage;
pub mod tim;

use anyhow::{Context, Result};

/// World units to metres.
pub const SCALE: f64 = 0.001;

/// `n` bytes at `o`; `what` names them in the error.
pub(crate) fn bytes<'a>(b: &'a [u8], o: u64, n: usize, what: &str) -> Result<&'a [u8]> {
    usize::try_from(o)
        .ok()
        .and_then(|o| b.get(o..o.checked_add(n)?))
        .with_context(|| format!("{what}: {n} bytes at {o:#x} are outside the data ({:#x} bytes)", b.len()))
}

pub(crate) fn u16_at(b: &[u8], o: u64, what: &str) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(b, o, 2, what)?.try_into()?))
}

pub(crate) fn u32_at(b: &[u8], o: u64, what: &str) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(b, o, 4, what)?.try_into()?))
}

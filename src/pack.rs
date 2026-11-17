//! The arcade pack: the stages of your Tekken 3 arcade ROM set converted ([`convert`]).
//!
//! A pack (any `Vfs`: a folder, or the browser's pack file):
//!
//! ```text
//! manifest.json        {"pack": "t3arcade", "format": FORMAT, "source": {"program": the program ROM pair read},
//!                       "stages": [{"name", "file", "game_file", "title", "replaces", "lighting": "prelit" | "lit",
//!                                   "backdrop_scale", "faces", "triangles", "vertices",
//!                                   "floor": {"tiles", "size_m"}}, ...]}
//! stages/stage_NN.glb  each stage as a glb (`glb`: metres, Y up), as the Python tool this converter comes from
//!                      wrote it: to look at in any glTF viewer
//! assets/arcade/stage_NN.glb
//!                      the same stage laid out as OpenKumite's stages (`glb::write_game`); `assets/` is the folder
//!                      of a pack's files the game reads as they are
//! ```

use crate::floor::{self, build_floor};
use crate::glb::{self, BACKDROP_SCALE};
use crate::roms::{RomSet, group_stages, load_banked, load_floor_tables};
use crate::stage::build_stage;
use anyhow::{Context, Result, bail};
use ok_converter_common::io::{Ctx, Vfs};
use serde_json::{Value, json};

/// The pack's layout and contents; another one is converted again.
pub const FORMAT: i64 = 1;
/// What a pack's manifest names it.
pub const KIND: &str = "t3arcade";
/// The folder of the pack's files the game reads as they are (as the other packs').
pub const ASSETS: &str = "assets";

/// Descriptive titles (identified by look from the exported geometry, not official names).
const TITLES: [&str; 13] = [
    "City street with elevated railway",
    "Chinese garden with dragon wall",
    "Forest on a cliff",
    "Amusement park at night",
    "Temple courtyard",
    "Downtown street",
    "Lantern hall",
    "Circular arena",
    "Wooden dojo",
    "Industrial helipad at night",
    "Jungle ruins",
    "Ring on steel towers",
    "Torch-lit hall",
];

/// The PlayStation game's stage each arcade stage stands in for: the same number (the two games keep their stages in
/// the same order; twelve are the same place by look; the eighth, a ruined stone arena here, is a plaza before a
/// skyline there).
const REPLACES: [Option<u32>; 13] = [
    Some(0),
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(7),
    Some(8),
    Some(9),
    Some(10),
    Some(11),
    Some(12),
];

/// Converts every stage of the ROM set, giving each one's manifest entry, glb and the game's glb to `each` in turn; returns the
/// name of the program ROM pair the floors were read from.
pub fn stages(roms: &dyn RomSet, each: &mut dyn FnMut(Value, Vec<u8>, Vec<u8>) -> Result<()>) -> Result<&'static str> {
    let bank = load_banked(roms)?;
    let stages = group_stages(&bank)?;
    if stages.len() != TITLES.len() {
        bail!("{} stage models in the data ROMs, not {}: not the tekken3 ROM set", stages.len(), TITLES.len());
    }
    let (program, floors) = load_floor_tables(roms)?;
    for (i, ((stage, entries), (title, replaces))) in
        stages.iter().zip(&floors).zip(TITLES.iter().zip(REPLACES)).enumerate()
    {
        let name = format!("stage_{i:02}");
        let (entry, glb, game) = (|| {
            let st = build_stage(stage)?;
            let floor = build_floor(&st.vram, i, entries)?;
            let entry = json!({
                "name": name,
                "file": format!("stages/{name}.glb"),
                "game_file": format!("{ASSETS}/arcade/{name}.glb"),
                "title": title,
                "replaces": replaces,
                "lighting": if st.prelit { "prelit" } else { "lit" },
                "backdrop_scale": BACKDROP_SCALE,
                "faces": st.faces,
                "triangles": st.groups.iter().map(|(_, g)| g.pos.len() / 3).sum::<usize>(),
                "vertices": st.vertices,
                "floor": {"tiles": floor.tiles, "size_m": floor::SIZE_M},
            });
            Ok::<_, anyhow::Error>((entry, glb::write(&name, &st, &floor)?, glb::write_game(title, &st, &floor)?))
        })()
        .with_context(|| format!("stage {i}"))?;
        each(entry, glb, game)?;
    }
    Ok(program)
}

/// Converts the stages of the ROM set `roms` into a pack in `ctx.out`, emptied first; writes the manifest last and
/// returns it.
pub fn convert(ctx: &Ctx, roms: &dyn RomSet) -> Result<Value> {
    ctx.out.remove_all("")?;
    let mut entries = Vec::new();
    let program = stages(roms, &mut |entry, glb, game| {
        ctx.out.write(entry["file"].as_str().context("a stage's file")?, &glb)?;
        ctx.out.write(entry["game_file"].as_str().context("a stage's file")?, &game)?;
        ctx.log(&format!(
            "[t3arcade] {}: {} faces, {} triangles, {}, {} floor tiles",
            entry["name"].as_str().unwrap_or_default(),
            entry["faces"],
            entry["triangles"],
            entry["lighting"].as_str().unwrap_or_default(),
            entry["floor"]["tiles"]
        ));
        entries.push(entry);
        Ok(())
    })?;
    let manifest = json!({"pack": KIND, "format": FORMAT, "source": {"program": program}, "stages": entries});
    ctx.out.write("manifest.json", serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    ctx.out.checkpoint(&[])?;
    Ok(manifest)
}

/// The manifest of the pack in `pack`, checked: an arcade pack of this format.
pub fn manifest(pack: &dyn Vfs) -> Result<Value> {
    let m: Value = serde_json::from_slice(&pack.read("manifest.json")?).context("the pack's manifest.json")?;
    if m["pack"] != KIND {
        bail!("not a Tekken 3 arcade pack");
    }
    if m["format"] != FORMAT {
        bail!("a Tekken 3 arcade pack of another version: convert the arcade ROM set again");
    }
    Ok(m)
}

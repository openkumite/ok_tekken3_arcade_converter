//! The arcade pack: the stages of your Tekken 3 arcade ROM set converted ([`convert`]).
//!
//! A pack (any `Vfs`: a folder, or the browser's pack file):
//!
//! ```text
//! manifest.json        {"pack": "t3arcade", "format": FORMAT, "source": {"program": the program ROM pair read},
//!                       "stages": [{"name", "file", "game_file", "title", "replaces", "lighting": "prelit" | "lit",
//!                                   "backdrop_scale" (of `file`'s backdrop node; the game's glb has the game's),
//!                                   "faces", "triangles", "vertices", "floor": {"tiles", "size_m"},
//!                                   "sky": null | {"fov", "ybase", "pitchmul", "top", "bottom", "max_y", "min_y"}
//!                                   (`sky::placement`)}, ...]}
//! stages/stage_NN.glb  each stage as a glb (`glb`: metres, Y up), as the Python tool this converter comes from
//!                      wrote it: to look at in any glTF viewer
//! stages/hidden.glb, assets/arcade/hidden.glb
//!                      an arena in the ROM set that no stage of the game uses (`"replaces": null`), when the
//!                      set has it
//! assets/arcade/stage_NN.glb
//!                      the same stage laid out as OpenKumite's stages (`glb::write_game`); `assets/` is the folder
//!                      of a pack's files the game reads as they are
//! ```

use crate::floor::{self, build_floor};
use crate::glb::BACKDROP_SCALE;
use crate::roms::{RomSet, load_program};
use crate::stage::build_stage;
use crate::{glb, sky};
use anyhow::{Context, Result, bail};
use ok_converter_common::io::{Ctx, Vfs};
use serde_json::{Value, json};

/// The pack's layout and contents; another one is converted again.
pub const FORMAT: i64 = 3;
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
    "Schoolyard before a city skyline",
    "Wooden dojo",
    "Industrial helipad at night",
    "Jungle ruins",
    "Ring on steel towers",
    "Torch-lit hall",
];

/// The PlayStation game's stage each arcade stage stands in for: the same number (the two games keep their stages in
/// the same order, and all 13 are the same place by look).
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

/// The hidden arena's name in the pack (`assets/arcade/hidden.glb`), its title, and the stage it is laid out as.
pub const HIDDEN: &str = "hidden";
const HIDDEN_TITLE: &str = "Circular stone arena (in the ROM set, used by no stage)";
const HIDDEN_AS: usize = 7;

/// Converts every stage of the ROM set, giving each one's manifest entry, glb and the game's glb to `each` in turn; returns the
/// name of the program ROM pair they were read through.
pub fn stages(roms: &dyn RomSet, each: &mut dyn FnMut(Value, Vec<u8>, Vec<u8>) -> Result<()>) -> Result<&'static str> {
    let program = load_program(roms)?;
    let n = [program.stages.len(), program.floors.len(), program.skies.len()];
    if n != [TITLES.len(); 3] {
        bail!("{n:?} stages' files, floors and skies in the program, not {} of each", TITLES.len());
    }
    let tables = program.stages.iter().zip(&program.floors).zip(&program.skies).zip(TITLES.iter().zip(REPLACES));
    let named = tables.enumerate().map(|(i, (((stage, entries), sky), (title, replaces)))| {
        (format!("stage_{i:02}"), i, stage, entries, Some(sky), *title, replaces)
    });
    // the hidden arena: laid out as the stage in whose place its files are; no floor table or sky of its own in the
    // program. shortcut: that stage's floor table, its fourth tile (v 0x40 of the lower page: not a floor's texture
    // here) taken a page up, where the arena's fourth floor tile is: by look, not from the program; no sky: its
    // walls close the view
    let floor: Option<Vec<_>> = program.floors.get(HIDDEN_AS).map(|t| {
        t.iter().map(|&(w0, w1)| if w0 & 0xFF00 == 0x4000 { (w0 & !0x0010_0000, w1) } else { (w0, w1) }).collect()
    });
    let hidden = program.hidden.as_ref().zip(floor.as_ref());
    let hidden =
        hidden.map(|(stage, entries)| (HIDDEN.to_string(), HIDDEN_AS, stage, entries, None, HIDDEN_TITLE, None));
    for (name, i, stage, entries, sky, title, replaces) in named.chain(hidden) {
        let (entry, glb, game) = (|| {
            let st = build_stage(stage)?;
            let floor = build_floor(&st.vram, entries)?;
            let sky = sky.map(|sky| sky::build(i, sky, &st.vram)).transpose()?.flatten();
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
                "sky": sky.as_ref().map(|s| &s.1),
            });
            let game = glb::write_game(i, title, &st, &floor, sky.as_ref())?;
            Ok::<_, anyhow::Error>((entry, glb::write(&name, &st, &floor)?, game))
        })()
        .with_context(|| name.clone())?;
        each(entry, glb, game)?;
    }
    Ok(program.name)
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

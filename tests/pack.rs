//! The pack end to end. Without the ROM set: zero-filled ROMs give an error. With it (`T3_ARCADE_ROMS`: the .zip or
//! the folder): a zip and a folder give the same pack, twice the same bytes, 13 stages and the hidden arena (the game's glbs laid out as
//! its stages), the same through every program ROM pair of the set; with the Python tool's export too
//! (`T3_ARCADE_PY_EXPORT`: the folder t3_stage_export.py wrote), every glb but the eighth stage's decodes to the
//! same content; with the reference skies (`T3_ARCADE_SKY_REF`: the folder of `stage_NN_sky.png` the research
//! script sky_render.py wrote), the skies are the same pictures.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ok_converter_common::io::{Ctx, StdFs, Vfs};
use ok_tekken3_arcade_converter::pack;
use ok_tekken3_arcade_converter::roms::{DATA_ROMS, FolderRoms, PROGRAM_ROMS, RomSet, ZipRoms};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

/// A ROM set in memory.
struct MemRoms(BTreeMap<String, Vec<u8>>);

impl RomSet for MemRoms {
    fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }
    fn read(&self, name: &str) -> anyhow::Result<Vec<u8>> {
        self.0.get(name).cloned().ok_or_else(|| anyhow::anyhow!("no {name}"))
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("t3_arcade_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The pack of `roms`, converted into a folder of its own: every file's bytes by path.
fn convert(roms: &dyn RomSet, name: &str) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let out = StdFs(scratch(name));
    let r = pack::convert(&Ctx { out: &out, log: &|_| {}, jobs: 1 }, roms).and_then(|m| {
        assert_eq!(pack::manifest(&out)?, m);
        let mut files = BTreeMap::from([("manifest.json".to_string(), out.read("manifest.json")?)]);
        for dir in ["stages", "assets/arcade"] {
            for e in out.list(dir)? {
                let path = format!("{dir}/{}", e.name);
                files.insert(path.clone(), out.read(&path)?);
            }
        }
        Ok(files)
    });
    let _ = std::fs::remove_dir_all(&out.0);
    r
}

fn open(path: &str) -> Box<dyn RomSet> {
    if Path::new(path).is_dir() {
        // (leaked: the set borrows its folder, and a test ends with its process)
        Box::new(FolderRoms(Box::leak(Box::new(StdFs(path.into())))))
    } else {
        Box::new(ZipRoms::new(std::fs::File::open(path).unwrap()).unwrap())
    }
}

#[test]
fn zero_filled_roms_are_an_error() {
    let mut roms = MemRoms(BTreeMap::new());
    for (lo, hi) in DATA_ROMS {
        roms.0.insert(lo.into(), vec![0; 4 << 20]);
        roms.0.insert(hi.into(), vec![0; 4 << 20]);
    }
    for ext in ["2e", "2j"] {
        roms.0.insert(format!("{}.{ext}", PROGRAM_ROMS[0]), vec![0; 2 << 20]);
    }
    assert!(convert(&roms, "zeros").is_err());
    // and with nothing, or halves that do not pair
    assert!(convert(&MemRoms(BTreeMap::new()), "none").is_err());
    roms.0.insert(DATA_ROMS[0].0.into(), vec![0; 5]);
    assert!(convert(&roms, "odd").is_err());
}

#[test]
fn zip_and_folder_give_the_same_pack() {
    let Ok(path) = std::env::var("T3_ARCADE_ROMS") else {
        eprintln!("T3_ARCADE_ROMS is not set: the ROM tests are skipped");
        return;
    };
    let given = open(&path);
    let first = convert(given.as_ref(), "given").unwrap();
    assert_eq!(first.len(), 29, "13 stages and the hidden arena, each twice, and the manifest");
    // the game's glb of each: its sky and its floor's copies, every primitive drawn by index with a texture
    for (path, glb) in first.iter().filter(|f| f.0.starts_with("assets/")) {
        let n = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let j: Value = serde_json::from_slice(&glb[20..20 + n]).unwrap();
        let (nodes, scene) = (j["nodes"].as_array().unwrap(), &j["scenes"][0]);
        assert_eq!(
            (nodes[0]["name"].as_str(), nodes[1]["name"].as_str(), nodes.len()),
            (Some("sky"), Some("floor"), 2 + 81),
            "{path}"
        );
        let extras = &scene["extras"];
        assert_eq!(
            (&extras["drum"], &extras["drum_cull"], &extras["floor_period"]),
            (&"model".into(), &true.into(), &serde_json::json!([25.0, 25.0])),
            "{path}"
        );
        // the sky: the last image, of no texture; the black stage has none
        let (images, sky) = (j["images"].as_array().unwrap(), &extras["sky"]);
        assert_eq!(sky.is_null(), path.ends_with("stage_06.glb") || path.ends_with("hidden.glb"), "{path}");
        assert_eq!(images.len(), j["textures"].as_array().unwrap().len() + usize::from(!sky.is_null()), "{path}");
        if !sky.is_null() {
            assert_eq!(
                (sky["image"].as_u64(), images.last().unwrap()["name"].as_str()),
                (Some(images.len() as u64 - 1), Some("sky"))
            );
            assert!(sky["fov"].is_u64() && sky["ybase"].is_i64() && sky["pitchmul"].is_u64(), "{path}");
        }
        for p in j["meshes"].as_array().unwrap().iter().flat_map(|m| m["primitives"].as_array().unwrap()) {
            let count = |a: &Value| j["accessors"][a.as_u64().unwrap() as usize]["count"].clone();
            let n = count(&p["indices"]);
            assert!(["POSITION", "TEXCOORD_0", "COLOR_0"].iter().all(|k| count(&p["attributes"][k]) == n), "{path}");
            assert!(
                j["materials"][p["material"].as_u64().unwrap() as usize]["pbrMetallicRoughness"]["baseColorTexture"]
                    .is_object()
            );
        }
    }
    // what moves: the carousel, the helicopter's two rotors, the flames of two stages (16 pictures each)
    let moving = |stage: &str, key: &str| {
        let glb = &first[&format!("assets/arcade/{stage}.glb")];
        let n = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let j: Value = serde_json::from_slice(&glb[20..20 + n]).unwrap();
        let flames =
            j["images"].as_array().unwrap().iter().filter(|i| i["name"].as_str().unwrap().starts_with("flame_"));
        let prims = j["meshes"][0]["primitives"].as_array().unwrap().iter();
        (prims.filter(|p| !p["extras"][key].is_null()).count(), flames.count())
    };
    assert_eq!([moving("stage_03", "spin"), moving("stage_11", "spin")], [(1, 0), (2, 0)]);
    assert_eq!([moving("stage_06", "frames"), moving("stage_12", "frames")], [(1, 16), (1, 16)]);
    assert_eq!([moving("stage_00", "spin"), moving("stage_00", "frames")], [(0, 0), (0, 0)]);
    let manifest: Value = serde_json::from_slice(&first["manifest.json"]).unwrap();
    let stages = manifest["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 14);
    assert_eq!((&stages[13]["name"], &stages[13]["replaces"]), (&"hidden".into(), &Value::Null));
    assert_eq!(stages.iter().filter(|s| s["sky"].is_object()).count(), 12);
    assert_eq!(stages[11]["sky"]["min_y"], -192);
    assert_eq!(convert(given.as_ref(), "again").unwrap(), first, "converting twice");

    // the ROMs the conversion reads, as a folder and as a zip (in a folder of the archive, as MAME's merged sets are)
    let program = manifest["source"]["program"].as_str().unwrap();
    let names = DATA_ROMS.iter().flat_map(|&(lo, hi)| [lo.to_string(), hi.to_string()]);
    let names = names.chain(["2e", "2j"].map(|ext| format!("{program}.{ext}")));
    let folder = StdFs(scratch("roms"));
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in names {
        let data = given.read(&name).unwrap();
        folder.write(&name, &data).unwrap();
        zip.start_file(format!("tekken3/{name}"), zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&data).unwrap();
    }
    assert!(convert(&FolderRoms(&folder), "folder").unwrap() == first, "the pack of a folder differs");
    assert!(
        convert(&ZipRoms::new(zip.finish().unwrap()).unwrap(), "zip").unwrap() == first,
        "the pack of a zip differs"
    );
    std::fs::remove_dir_all(&folder.0).unwrap();

    // through each program ROM pair of the set (their tables are at other addresses): the same stages
    let mut roms =
        MemRoms(DATA_ROMS.iter().flat_map(|&(lo, hi)| [lo, hi]).map(|n| (n.into(), given.read(n).unwrap())).collect());
    for ver in PROGRAM_ROMS.iter().filter(|v| given.has(&format!("{v}.2e")) && given.has(&format!("{v}.2j"))) {
        let pair = ["2e", "2j"].map(|ext| format!("{ver}.{ext}"));
        pair.iter().for_each(|n| drop(roms.0.insert(n.clone(), given.read(n).unwrap())));
        let pack = convert(&roms, "pair").unwrap();
        assert!(pack.iter().all(|(path, file)| path == "manifest.json" || *file == first[path]), "{ver}");
        pair.iter().for_each(|n| drop(roms.0.remove(n)));
    }
}

/// A glb's JSON and binary chunk.
fn read_glb(b: &[u8]) -> (Value, Vec<u8>) {
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    assert_eq!((u32_at(0), u32_at(4), u32_at(8)), (0x4654_6C67, 2, b.len()), "the glb header");
    let json_len = u32_at(12);
    assert_eq!(u32_at(16), 0x4E4F_534A);
    let bin = 20 + json_len;
    assert_eq!((u32_at(bin + 4), bin + 8 + u32_at(bin)), (0x004E_4942, b.len()));
    (serde_json::from_slice(&b[20..bin]).unwrap(), b[bin + 8..].to_vec())
}

fn view<'a>(j: &Value, bin: &'a [u8], view: &Value) -> &'a [u8] {
    let v = &j["bufferViews"][view.as_u64().unwrap() as usize];
    let (o, n) = (v["byteOffset"].as_u64().unwrap() as usize, v["byteLength"].as_u64().unwrap() as usize);
    &bin[o..o + n]
}

/// The glb's content with nothing of its layout: every accessor a primitive names replaced by its type, count,
/// bounds and values (floats as their bits), every image by its name and decoded pixels' size; the buffer, its
/// views and the generator's name left out. The images' pixels are returned beside it.
fn decode(glb: &[u8]) -> (Value, Vec<(png::OutputInfo, Vec<u8>)>) {
    let (mut j, bin) = read_glb(glb);
    let layout = j.clone();
    let accessor = |i: &Value| {
        let mut a = layout["accessors"][i.as_u64().unwrap() as usize].clone();
        let data = view(&layout, &bin, &a["bufferView"]);
        let bits: Vec<Value> = match a["componentType"].as_u64().unwrap() {
            5126 | 5125 => data.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c).into()).collect(),
            5123 => data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c).into()).collect(),
            t => panic!("an accessor of component type {t}"),
        };
        let per = match a["type"].as_str().unwrap() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            t => panic!("an accessor of type {t}"),
        };
        assert_eq!(bits.len() as u64, a["count"].as_u64().unwrap() * per, "an accessor's view is not its count");
        a["bufferView"] = Value::Null;
        a["bits"] = bits.into();
        a
    };
    for mesh in j["meshes"].as_array_mut().unwrap() {
        for p in mesh["primitives"].as_array_mut().unwrap() {
            for (_, a) in p["attributes"].as_object_mut().unwrap() {
                *a = accessor(a);
            }
            if let Some(i) = p.get_mut("indices") {
                *i = accessor(i);
            }
        }
    }
    let mut pixels = Vec::new();
    for image in j["images"].as_array_mut().unwrap() {
        let mut r = png::Decoder::new(Cursor::new(view(&layout, &bin, &image["bufferView"]))).read_info().unwrap();
        let mut px = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut px).unwrap();
        px.truncate(info.buffer_size());
        image["bufferView"] = Value::Null;
        image["size"] = format!("{} x {} {:?} {:?}", info.width, info.height, info.color_type, info.bit_depth).into();
        pixels.push((info, px));
    }
    for layout_only in ["accessors", "bufferViews", "buffers", "asset"] {
        j.as_object_mut().unwrap().remove(layout_only);
    }
    (j, pixels)
}

/// Where two JSON values first differ, and how.
fn diff(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: Vec<_> = x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))).collect();
            keys.into_iter().find_map(|k| {
                diff(x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), &format!("{path}.{k}"))
            })
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            x.iter().zip(y).enumerate().find_map(|(i, (x, y))| diff(x, y, &format!("{path}[{i}]")))
        }
        (Value::Array(x), Value::Array(y)) => Some(format!("{path}: {} items here, {} in Python's", x.len(), y.len())),
        _ if a == b => None,
        _ => {
            let bits = |v: &Value| match v.as_u64() {
                Some(u) if path.contains(".bits[") => format!("{u:#x} ({})", f32::from_bits(u as u32)),
                _ => v.to_string(),
            };
            Some(format!("{path}: {} here, {} in Python's", bits(a), bits(b)))
        }
    }
}

#[test]
fn the_glbs_are_the_python_tools() {
    let (Ok(roms), Ok(export)) = (std::env::var("T3_ARCADE_ROMS"), std::env::var("T3_ARCADE_PY_EXPORT")) else {
        eprintln!("T3_ARCADE_ROMS and T3_ARCADE_PY_EXPORT are not both set: the parity test is skipped");
        return;
    };
    let ours = convert(open(&roms).as_ref(), "parity").unwrap();
    let manifest: Value = serde_json::from_slice(&ours["manifest.json"]).unwrap();
    let theirs: Value =
        serde_json::from_slice(&std::fs::read(Path::new(&export).join("stages/manifest.json")).unwrap()).unwrap();
    // the hidden arena is what the Python tool has as its eighth stage: the same counts
    let (hidden, stages) = manifest["stages"].as_array().unwrap().split_last().unwrap();
    for key in ["lighting", "faces", "triangles", "vertices", "floor"] {
        assert_eq!(hidden[key], theirs[7][key], "the hidden arena: the manifest's {key}");
    }
    assert_eq!(stages.len(), theirs.as_array().unwrap().len(), "the number of stages");
    let mut same = 0;
    for (i, (stage, py)) in stages.iter().zip(theirs.as_array().unwrap()).enumerate() {
        // the Python tool takes the eighth stage from an archive the game never loads; here it is the game's own
        if i == 7 {
            continue;
        }
        let file = stage["file"].as_str().unwrap();
        for key in ["name", "file", "title", "lighting", "backdrop_scale", "faces", "triangles", "vertices", "floor"] {
            assert_eq!(stage[key], py[key], "{file}: the manifest's {key}");
        }
        let (a, a_px) = decode(&ours[file]);
        let (b, b_px) = decode(&std::fs::read(Path::new(&export).join(file)).unwrap());
        if let Some(d) = diff(&a, &b, "") {
            panic!("{file}: {d}");
        }
        for (i, ((ia, pa), (ib, pb))) in a_px.iter().zip(&b_px).enumerate() {
            if let Some(at) = pa.iter().zip(pb).position(|(x, y)| x != y) {
                let (px, w) = (at / 4, ia.width as usize);
                let (x, y) = (&pa[px * 4..px * 4 + 4], &pb[px * 4..px * 4 + 4]);
                panic!("{file}: image {i} at ({}, {}): {x:?} here, {y:?} in Python's", px % w, px / w);
            }
            assert_eq!((pa.len(), ia.line_size), (pb.len(), ib.line_size), "{file}: image {i}");
        }
        same += 1;
    }
    eprintln!("{same} of {} stages decode to the same content as the Python tool's", stages.len());
    assert_eq!(same, 12);
}

#[test]
fn the_skies_are_the_reference_scripts() {
    let (Ok(roms), Ok(reference)) = (std::env::var("T3_ARCADE_ROMS"), std::env::var("T3_ARCADE_SKY_REF")) else {
        eprintln!("T3_ARCADE_ROMS and T3_ARCADE_SKY_REF are not both set: the skies' parity test is skipped");
        return;
    };
    let ours = convert(open(&roms).as_ref(), "skies").unwrap();
    let mut same = 0;
    // (the script draws no gradient: stage 3's tiles, which the game does not draw; and nothing for the black stage 6)
    for i in (0..13).filter(|i| ![3, 6].contains(i)) {
        let (j, pixels) = decode(&ours[&format!("assets/arcade/stage_{i:02}.glb")]);
        let (info, px) = &pixels[j["scenes"][0]["extras"]["sky"]["image"].as_u64().unwrap() as usize];
        let file = std::fs::File::open(Path::new(&reference).join(format!("stage_{i:02}_sky.png"))).unwrap();
        let mut r = png::Decoder::new(std::io::BufReader::new(file)).read_info().unwrap();
        let mut theirs = vec![0; r.output_buffer_size().unwrap()];
        let their_info = r.next_frame(&mut theirs).unwrap();
        assert_eq!(
            (info.width, info.height, info.color_type, info.bit_depth),
            (their_info.width, their_info.height, their_info.color_type, their_info.bit_depth),
            "stage {i}"
        );
        if let Some(at) = px.iter().zip(&theirs).position(|(x, y)| x != y) {
            let (p, w) = (at / 4, info.width as usize);
            panic!(
                "stage {i} at ({}, {}): {:?} here, {:?} in the script's",
                p % w,
                p / w,
                &px[p * 4..p * 4 + 4],
                &theirs[p * 4..p * 4 + 4]
            );
        }
        same += 1;
    }
    assert_eq!(same, 11);
}

//! A stage as a glTF 2.0 binary: a root node named after the stage with two children, "backdrop" (the stage model,
//! one primitive per material) and "floor".
//!
//! The backdrop node has the scale the Python tool measured from frames of the game, 9.6 ([`BACKDROP_SCALE`]; the
//! game's own is 10, [`GAME_SCALE`]); the floor is at true scale. Vertices are not indexed; a pre-lit stage's
//! materials are unlit (`KHR_materials_unlit`), its light in the vertex colours. [`write_game`] writes the stage
//! as the game places it.

use crate::floor::Floor;
use crate::objects::Layer;
use crate::stage::Built;
use anyhow::{Result, bail};
use serde_json::{Value, json};

/// Stage model units to world units in the plain glb: the Python tool's measure, kept so that the glb stays the
/// Python tool's. The game's value is 10 ([`GAME_SCALE`]).
pub const BACKDROP_SCALE: f64 = 9.6;

/// How the game places the stage model (routine 0x801D9118 of TET2/VER.E1): one model unit is 10 world units; its
/// Y axis is scaled by 95/100, on stage 3 by 7/10 more; its origin is 50 world units along the model's +Y, which
/// is down (table 0x8017C454: 50 for every stage).
pub const GAME_SCALE: f64 = 10.0;
const GAME_Y_SCALE: f64 = 0.95;
const GAME_Y_SCALE_STAGE_3: f64 = 0.665;
const GAME_Y: f64 = -50.0 * crate::SCALE;

#[derive(Debug)]
pub struct Material {
    pub name: String,
    /// the image it draws, if textured
    pub tex: Option<usize>,
    /// semi-transparent: blended
    pub semi: bool,
}

/// The triangles of one material, three vertices each. Computed in f64 and written as f32, as the Python tool does.
#[derive(Default)]
pub struct Group {
    pub pos: Vec<[f64; 3]>,
    pub nrm: Vec<[f64; 3]>,
    pub col: Vec<[f64; 3]>,
    pub uv: Vec<[f64; 2]>,
}

#[derive(Default)]
struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    materials: Vec<Value>,
}

impl Writer {
    fn view(&mut self, data: &[u8], target: Option<u32>) -> usize {
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        let mut v = json!({"buffer": 0, "byteOffset": self.bin.len(), "byteLength": data.len()});
        if let Some(t) = target {
            v["target"] = t.into();
        }
        self.bin.extend(data);
        self.views.push(v);
        self.views.len() - 1
    }

    fn accessor<const N: usize>(&mut self, vals: &[[f64; N]], bounds: bool) -> usize {
        let data: Vec<u8> = vals.iter().flatten().flat_map(|&c| (c as f32).to_le_bytes()).collect();
        let view = self.view(&data, Some(34962));
        let ty = if N == 3 { "VEC3" } else { "VEC2" };
        let mut acc = json!({"bufferView": view, "componentType": 5126, "count": vals.len(), "type": ty});
        if bounds {
            let fold =
                |f: fn(f64, f64) -> f64| vals.iter().copied().reduce(|a, b| std::array::from_fn(|i| f(a[i], b[i])));
            acc["min"] = json!(fold(f64::min).map(Vec::from));
            acc["max"] = json!(fold(f64::max).map(Vec::from));
        }
        self.accessors.push(acc);
        self.accessors.len() - 1
    }

    fn material(&mut self, m: &Material, unlit: bool) -> usize {
        let mut pbr = json!({"metallicFactor": 0.0, "roughnessFactor": 1.0});
        if let Some(t) = m.tex {
            pbr["baseColorTexture"] = json!({"index": t});
        }
        let mut j = json!({"name": m.name, "doubleSided": true, "pbrMetallicRoughness": pbr});
        if m.semi {
            j["alphaMode"] = "BLEND".into();
        } else if m.tex.is_some() {
            j["alphaMode"] = "MASK".into();
            j["alphaCutoff"] = 0.5.into();
        }
        if unlit {
            j["extensions"] = json!({"KHR_materials_unlit": {}});
        }
        self.materials.push(j);
        self.materials.len() - 1
    }

    fn primitive(&mut self, m: &Material, g: &Group, unlit: bool) -> Value {
        let material = self.material(m, unlit);
        let mut attrs = json!({
            "POSITION": self.accessor(&g.pos, true),
            "NORMAL": self.accessor(&g.nrm, false),
            "COLOR_0": self.accessor(&g.col, false),
        });
        if m.tex.is_some() {
            attrs["TEXCOORD_0"] = self.accessor(&g.uv, false).into();
        }
        json!({"attributes": attrs, "material": material, "mode": 4})
    }
}

/// The glb of the stage `name`.
pub fn write(name: &str, st: &Built, floor: &Floor) -> Result<Vec<u8>> {
    if st.groups.is_empty() {
        bail!("the stage model has nothing to draw");
    }
    let mut w = Writer::default();
    let (mut images, mut textures) = (Vec::new(), Vec::new());
    let pngs = st.images.iter().map(|(n, p)| (n.as_str(), p)).chain([("floor", &floor.png)]);
    for (i, (iname, png)) in pngs.enumerate() {
        images.push(json!({"bufferView": w.view(png, None), "mimeType": "image/png", "name": iname}));
        // sampler 0: nearest, clamped; 1: nearest, repeated (the tiles)
        textures.push(json!({"source": i, "sampler": if iname.starts_with("tile") { 1 } else { 0 }}));
    }
    let unlit = st.prelit;
    let backdrop: Vec<Value> = st.groups.iter().map(|(m, g)| w.primitive(m, g, unlit)).collect();
    let floor_mat = Material { name: "floor".into(), tex: Some(st.images.len()), semi: false };
    let floor = w.primitive(&floor_mat, &floor.group, unlit);
    let mut gltf = json!({
        "asset": {"version": "2.0", "generator": env!("CARGO_PKG_NAME")},
        "scene": 0,
        "scenes": [{"name": name, "nodes": [0]}],
        "nodes": [
            {"name": name, "children": [1, 2]},
            {"name": "backdrop", "mesh": 0, "scale": [BACKDROP_SCALE, BACKDROP_SCALE, BACKDROP_SCALE]},
            {"name": "floor", "mesh": 1},
        ],
        "meshes": [
            {"name": format!("{name}_backdrop"), "primitives": backdrop},
            {"name": format!("{name}_floor"), "primitives": [floor]},
        ],
        "materials": w.materials,
        "textures": textures,
        "images": images,
        "samplers": [
            {"magFilter": 9728, "minFilter": 9728, "wrapS": 33071, "wrapT": 33071},
            {"magFilter": 9728, "minFilter": 9728, "wrapS": 10497, "wrapT": 10497},
        ],
        "accessors": w.accessors,
        "bufferViews": w.views,
        "buffers": [{"byteLength": w.bin.len()}],
    });
    if unlit {
        gltf["extensionsUsed"] = json!(["KHR_materials_unlit"]);
    }
    container(&gltf, w.bin)
}

/// The floor map's repeat (m), and how many copies of it the game's stage holds either way of the middle one.
const FLOOR_PERIOD: f64 = crate::floor::SIZE_M;
const FLOOR_COPIES: i32 = 4;

/// Stage `stage` as OpenKumite's stage glbs are laid out (ok_tekken3_converter's `export_stages`), `title` its
/// scene's name, `sky` its sky's picture (a PNG) and placement (`sky::build`):
///
/// - node `sky`: the stage model as the game places it ([`GAME_SCALE`], the Y scale and the height in its
///   vertices), its `layers` in the order the game draws them (far first), one primitive per image of each; an
///   untextured face draws a white image's texel. Semi-transparent faces are left out: the game's stages have
///   none, and it would draw them solid. A primitive that moves says how in its extras: `spin` (`pivot`, metres;
///   `axis`; `rate`, radians a frame of 1/60 s: it turns about the line through the pivot along the axis; `period`,
///   where it has one: after so many frames it is back where it began), or
///   `frames` and `step` (a flame: its image and the `frames - 1` after it follow one another, each shown for
///   `step` frames).
/// - node `floor`: the floor map's copies `floor_I_J`, one period apart.
/// - the scene's extras: `floor_period` (m); `drum: "model"`: the sky node is the stage's model, not a drum of
///   panels. The game keeps it centred on the fighters' midpoint (x, z) and draws it after the floor without
///   depth, far faces first. `drum_cull: true`: back faces are not drawn. The game culls by the winding on the
///   screen (NCLIP: it draws a face whose vertices run clockwise there); here the triangles are wound as glTF has
///   it, counter-clockwise seen from the front. `sky` (absent on the black stage): the panorama behind everything,
///   `image` the index of the image named `sky` (no mesh uses it; the last image), the rest as `sky::placement`.
///
/// Every primitive has positions, texture coordinates, colours (the light of a pre-lit stage; white on a lit one,
/// which the game draws with its textures as they are) and indices.
pub fn write_game(
    stage: usize,
    title: &str,
    layers: &[Layer],
    floor: &Floor,
    sky: Option<&(Vec<u8>, Value)>,
) -> Result<Vec<u8>> {
    let mut w = Writer::default();
    let (mut images, mut textures) = (Vec::new(), Vec::new());
    let white = crate::atlas::png(1, 1, &[255; 4])?;
    let pngs = layers.iter().flat_map(|l| &l.built.images).map(|(n, p)| (n.as_str(), p));
    let pngs = pngs.chain([("floor", &floor.png), ("white", &white)]);
    for (i, (iname, png)) in pngs.enumerate() {
        images.push(json!({"bufferView": w.view(png, None), "mimeType": "image/png", "name": iname}));
        textures.push(json!({"source": i, "sampler": if iname.starts_with("tile") { 1 } else { 0 }}));
        let pbr = json!({"baseColorTexture": {"index": i}, "metallicFactor": 0.0, "roughnessFactor": 1.0});
        w.materials.push(json!({"name": iname, "doubleSided": true, "alphaMode": "MASK", "alphaCutoff": 0.5, "pbrMetallicRoughness": pbr}));
    }
    let floor_image = layers.iter().map(|l| l.built.images.len()).sum::<usize>();
    let white_image = floor_image + 1;
    // `flip`: the model's triangles are wound clockwise seen from the front (the PlayStation's way); their second
    // and third corners change places
    let primitive = |w: &mut Writer, image: usize, g: &Group, pos: Vec<[f64; 3]>, flip: bool| -> Result<Value> {
        let uv = if image == white_image { vec![[0.5; 2]; pos.len()] } else { g.uv.clone() };
        if uv.len() != pos.len() || g.col.len() != pos.len() {
            bail!("a face without its texture coordinates or colours");
        }
        let corner = |i: u32| match i % 3 {
            1 if flip => i + 1,
            2 if flip => i - 1,
            _ => i,
        };
        let indices: Vec<u8> = (0..u32::try_from(pos.len())?).map(corner).flat_map(u32::to_le_bytes).collect();
        let view = w.view(&indices, Some(34963));
        w.accessors.push(json!({"bufferView": view, "componentType": 5125, "count": pos.len(), "type": "SCALAR"}));
        let indices = w.accessors.len() - 1;
        let attrs = json!({"POSITION": w.accessor(&pos, true), "TEXCOORD_0": w.accessor(&uv, false), "COLOR_0": w.accessor(&g.col, false)});
        Ok(json!({"attributes": attrs, "indices": indices, "material": image, "mode": 4}))
    };
    let y_scale = GAME_SCALE * if stage == 3 { GAME_Y_SCALE_STAGE_3 } else { GAME_Y_SCALE };
    let place = |p: &[f64; 3]| [p[0] * GAME_SCALE, p[1] * y_scale + GAME_Y, p[2] * GAME_SCALE];
    let (mut model, mut first_image) = (Vec::new(), 0);
    for l in layers {
        for (m, g) in l.built.groups.iter().filter(|(m, _)| !m.semi) {
            let image = m.tex.map_or(white_image, |t| first_image + t);
            let mut p = primitive(&mut w, image, g, g.pos.iter().map(place).collect(), true)?;
            // shortcut: the turn's axis is kept as it is under the model's Y scale (right for an upright axis, a
            // little off for the tilted helicopter's rotors); upgrade: the Y scale as a node's, the turn inside it
            if let Some(s) = &l.spin {
                p["extras"] = json!({"spin": {"pivot": place(&s.pivot), "axis": s.axis, "rate": s.rate}});
                if let Some(period) = s.period {
                    p["extras"]["spin"]["period"] = period.into();
                }
            } else if let Some((_, frames, step)) = l.built.flipbook.filter(|f| Some(f.0) == m.tex) {
                p["extras"] = json!({"frames": frames, "step": step});
            }
            model.push(p);
        }
        first_image += l.built.images.len();
    }
    if model.is_empty() {
        bail!("the stage model has nothing to draw");
    }
    let floor = primitive(&mut w, floor_image, &floor.group, floor.group.pos.clone(), false)?;
    let mut extras = json!({"floor_period": [FLOOR_PERIOD, FLOOR_PERIOD], "drum": "model", "drum_cull": true, "background_rgb": [0, 0, 0]});
    if let Some((png, placement)) = sky {
        let mut placement = placement.clone();
        placement["image"] = images.len().into();
        images.push(json!({"bufferView": w.view(png, None), "mimeType": "image/png", "name": "sky"}));
        extras["sky"] = placement;
    }
    let mut nodes = vec![json!({"name": "sky", "mesh": 0}), json!({"name": "floor", "children": []})];
    let mut copies = Vec::new();
    for j in -FLOOR_COPIES..=FLOOR_COPIES {
        for i in -FLOOR_COPIES..=FLOOR_COPIES {
            copies.push(nodes.len());
            let at = [f64::from(i) * FLOOR_PERIOD, 0.0, f64::from(j) * FLOOR_PERIOD];
            nodes.push(json!({"name": format!("floor_{i}_{j}"), "mesh": 1, "translation": at}));
        }
    }
    nodes[1]["children"] = json!(copies);
    let gltf = json!({
        "asset": {"version": "2.0", "generator": env!("CARGO_PKG_NAME")},
        "scene": 0,
        "scenes": [{"name": title, "nodes": [0, 1], "extras": extras}],
        "nodes": nodes,
        "meshes": [{"name": "sky", "primitives": model}, {"name": "floor", "primitives": [floor]}],
        "materials": w.materials,
        "textures": textures,
        "images": images,
        "samplers": [
            {"magFilter": 9728, "minFilter": 9728, "wrapS": 33071, "wrapT": 33071},
            {"magFilter": 9728, "minFilter": 9728, "wrapS": 10497, "wrapT": 10497},
        ],
        "accessors": w.accessors,
        "bufferViews": w.views,
        "buffers": [{"byteLength": w.bin.len()}],
    });
    container(&gltf, w.bin)
}

/// The glb container of a glTF document and its binary chunk.
fn container(gltf: &Value, mut bin: Vec<u8>) -> Result<Vec<u8>> {
    let mut js = serde_json::to_vec(gltf)?;
    js.resize(js.len().next_multiple_of(4), b' ');
    bin.resize(bin.len().next_multiple_of(4), 0);
    let mut out = Vec::new();
    for v in [0x4654_6C67, 2, u32::try_from(12 + 8 + js.len() + 8 + bin.len())?, u32::try_from(js.len())?, 0x4E4F_534A]
    {
        out.extend(v.to_le_bytes());
    }
    out.extend(js);
    out.extend(u32::try_from(bin.len())?.to_le_bytes());
    out.extend(0x004E_4942u32.to_le_bytes());
    out.extend(bin);
    Ok(out)
}

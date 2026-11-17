# ok_tekken3_arcade_converter

Converts the stages of your own Tekken 3 arcade ROM set (Namco System 12; MAME's `tekken3`) into a pack OpenKumite
can play in place of the PlayStation game's arenas (OpenKumite: an open-source 3D fighting game engine, a separate
project). The arcade game's stages are 3D models; the PlayStation game drew them as flat panoramas.

> **Unofficial.** ok_tekken3_arcade_converter is not affiliated with, endorsed or sponsored by Bandai Namco
> Entertainment. Tekken is a trademark of Bandai Namco Entertainment Inc. This repository contains no game data:
> you need your own legally obtained ROM set, which the converter reads on your computer. Do not ask for, or share,
> ROMs or converted files.

**Players do not run it.** OpenKumite links this crate and converts in the game. This repository is the library and
the developer tool.

## Input

The ROM set as MAME's `tekken3.zip`, or the same files unpacked in a folder. It reads the data ROMs
`tet1rom0l.6`, `tet1rom0u.9`, `tet1rom1l.7`, `tet1rom1u.10`, `tet1rom2l.8`, `tet1rom2u.11` and one program ROM pair
(`.2e` and `.2j`), the first there is of `tet2vere1`, `tet3verd`, `tet1vere1`, `tet2verb`, `tet2vera`, `tet3vera`,
`tet1vera` (each gives the same pack: the program's tables are found by their contents, not at one version's
addresses). The other files of the set are not read.

## The library

- `roms::ZipRoms::new(reader)` / `roms::FolderRoms(&vfs)`: the ROM set (`roms::RomSet`).
- `pack::convert(ctx, roms)`: the ROM set into a pack (any `Vfs`); returns its manifest.
- `pack::manifest(pack)`: a pack's manifest, checked.

A pack:

```text
manifest.json                {"pack": "t3arcade", "format": 3, "source": {"program": ...}, "stages": [...]}
stages/stage_NN.glb          each stage as a plain glb (a "backdrop" node and a "floor" node): to look at
assets/arcade/stage_NN.glb   the same stage laid out as OpenKumite's stages: what the game reads
```

A stage's manifest entry: `name`, `file`, `game_file`, `title` (a description by look, not an official name),
`replaces` (the PlayStation game's stage id it stands in for), `lighting` (`prelit`: its light is in the vertex
colours; `lit`: the arcade game lights it as it draws), `backdrop_scale` (of the plain glb), `faces`, `triangles`,
`vertices`, `floor`, `sky` (how the game places the sky's picture; `null` on the stage whose sky is black).

The 13 stages are in the PlayStation game's order, so stage N replaces its stage N: the same place by look.

The ROM set also keeps an arena no stage of the game uses, a circular stone arena, where the eighth stage's files
would be in the stages' order. It is converted as `hidden` (`stages/hidden.glb`, `assets/arcade/hidden.glb`,
`"replaces": null`), laid out as the eighth stage with that stage's floor table (one tile of it moved to the arena's own fourth floor
tile, by look) and no sky: the program has neither for it.

The game's glb has the stage's model as the arcade game places it (one model unit is 10 world units, its height
scaled by 0.95, 5 cm under the floor), its triangles counter-clockwise seen from the front (`drum_cull` in the
scene's extras: the game does not draw back faces), and the sky as an image named `sky` that no mesh uses: the
panorama of a full turn, described by `sky` in the scene's extras (`src/sky.rs`).

## The developer tool, `t3_arcade`

```sh
t3_arcade pack tekken3.zip PACK      # the ROM set (the .zip or the folder) -> the pack (~16 MB); PACK is emptied first
t3_arcade list tekken3.zip           # each stage: its faces, lighting, floor tiles
```

## What it reads, and what it does not

The converter began as a port of a Python tool (`t3_stage_export.py`) that worked the formats out; the stages'
files, the sky and the model's placement were then read from the arcade program's code. Each module's
documentation says what it reads:

- `roms`: the data ROMs interleaved into one region; the program, unpacked (LZ), its file table (each stage's
  archive: twelve in the data ROMs, the eighth stage's in the program ROM, its textures LZ-packed) and its floor
  tables.
- `tim`: the textures (TIM) in the PlayStation's video memory layout.
- `model`: a stage's model, a 16 x 16 grid of cells of faces.
- `stage`, `atlas`: the faces grouped by texture, the textures cut into one atlas (tiled textures apart).
- `floor`: the fighting floor, a 10 x 10 map of 2.5 m tiles that repeats.
- `sky`: the sky behind a stage's model, a panorama of 64 x 64 tiles, and how the game scrolls it.
- `glb`: the two glbs.

Not converted: the lighting of the `lit` stages (the game draws their textures as they are: their faces are white
in the game's glb), semi-transparent faces in the game's glb, and everything that is not a stage (fighters,
animations, sound).

The model's scale in the game's glb (10) is read from the program's code. The plain glb keeps the 9.6 the Python
tool's author measured from frames of the game, so that it stays that tool's glb. Its eighth stage is not: the
Python tool takes that one from an archive the game never loads.

## Checks

`sh check.sh` runs formatting, clippy, the tests and `cargo deny`; the git hooks and CI run it. The tests that need
the ROM set are skipped without it:

```sh
T3_ARCADE_ROMS=/path/to/tekken3.zip sh check.sh                       # also: the pack from a zip and a folder
T3_ARCADE_ROMS=... T3_ARCADE_PY_EXPORT=/path/to/export sh check.sh    # also: the plain glbs against the Python tool's
T3_ARCADE_ROMS=... T3_ARCADE_SKY_REF=/path/to/skies sh check.sh       # also: the skies against reference pictures
```

## Releases

A tag `vX.Y.Z` (Cargo.toml's version; only organisation admins push `v*` tags) publishes a release: `t3_arcade` for
Linux (amd64, arm64), Windows and macOS (universal), with SHA256SUMS (`.github/workflows/release.yml`). `main` takes
pull requests only, its CI passing; Dependabot opens one grouped pull request a week.

## License

MIT ([LICENSE](LICENSE)). The repository contains no data from Tekken 3.

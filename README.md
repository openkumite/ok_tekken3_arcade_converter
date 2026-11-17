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
`tet1vera`. The other files of the set are not read.

## The library

- `roms::ZipRoms::new(reader)` / `roms::FolderRoms(&vfs)`: the ROM set (`roms::RomSet`).
- `pack::convert(ctx, roms)`: the ROM set into a pack (any `Vfs`); returns its manifest.
- `pack::manifest(pack)`: a pack's manifest, checked.

A pack:

```text
manifest.json                {"pack": "t3arcade", "format": 1, "source": {"program": ...}, "stages": [...]}
stages/stage_NN.glb          each stage as a plain glb (a "backdrop" node and a "floor" node): to look at
assets/arcade/stage_NN.glb   the same stage laid out as OpenKumite's stages: what the game reads
```

A stage's manifest entry: `name`, `file`, `game_file`, `title` (a description by look, not an official name),
`replaces` (the PlayStation game's stage id it stands in for), `lighting` (`prelit`: its light is in the vertex
colours; `lit`: the arcade game lights it as it draws), `backdrop_scale`, `faces`, `triangles`, `vertices`, `floor`.

The 13 stages are in the PlayStation game's order, so stage N replaces its stage N. Twelve are the same place by
look. The eighth is a ruined stone arena here and a plaza before a skyline there.

## The developer tool, `t3_arcade`

```sh
t3_arcade pack tekken3.zip PACK      # the ROM set (the .zip or the folder) -> the pack (~14 MB); PACK is emptied first
t3_arcade list tekken3.zip           # each stage: its faces, lighting, floor tiles
```

## What it reads, and what it does not

The converter is a port of a Python tool (`t3_stage_export.py`) that worked the formats out; each module's
documentation says what it reads:

- `roms`: the data ROMs interleaved into one region, the stages' files in it; the program, unpacked (LZ), and its
  floor tables.
- `tim`: the textures (TIM) in the PlayStation's video memory layout.
- `model`: a stage's model, a 16 x 16 grid of cells of faces.
- `stage`, `atlas`: the faces grouped by texture, the textures cut into one atlas (tiled textures apart).
- `floor`: the fighting floor, a 10 x 10 map of 2.5 m tiles that repeats.
- `glb`: the two glbs.

Not converted: the sky behind a stage's model, the lighting of the `lit` stages (they are written with their
faces' own colours), semi-transparent faces in the game's glb, and everything that is not a stage (fighters,
animations, sound).

The backdrop's scale (9.6) was measured from frames of the arcade game, not read from its code.

## Checks

`sh check.sh` runs formatting, clippy, the tests and `cargo deny`; the git hooks and CI run it. The tests that need
the ROM set are skipped without it:

```sh
T3_ARCADE_ROMS=/path/to/tekken3.zip sh check.sh                       # also: the pack from a zip and a folder
T3_ARCADE_ROMS=... T3_ARCADE_PY_EXPORT=/path/to/export sh check.sh    # also: every glb against the Python tool's
```

## Releases

A tag `vX.Y.Z` (Cargo.toml's version; only organisation admins push `v*` tags) publishes a release: `t3_arcade` for
Linux (amd64, arm64), Windows and macOS (universal), with SHA256SUMS (`.github/workflows/release.yml`). `main` takes
pull requests only, its CI passing; Dependabot opens one grouped pull request a week.

## License

MIT ([LICENSE](LICENSE)). The repository contains no data from Tekken 3.

//! `t3_arcade pack ROMS PACK`: the arcade pack (pack.rs) of the ROM set ROMS, MAME's tekken3.zip or the folder it
//! unpacks to, into the folder PACK (emptied first). `t3_arcade list ROMS`: its stages.

use anyhow::{Context, Result, bail};
use ok_converter_common::io::{Ctx, StdFs, print_line};
use ok_tekken3_arcade_converter::pack;
use ok_tekken3_arcade_converter::roms::{FolderRoms, RomSet, ZipRoms};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("pack") => pack_main(&args[1..]),
        Some("list") => list_main(&args[1..]),
        _ => Err(anyhow::anyhow!("usage: t3_arcade pack ROMS PACK\n       t3_arcade list ROMS")),
    };
    if let Err(e) = r {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// Runs `f` on the ROM set at `path`: a folder, or a .zip.
fn with_roms<T>(path: &str, f: impl FnOnce(&dyn RomSet) -> Result<T>) -> Result<T> {
    if Path::new(path).is_dir() {
        return f(&FolderRoms(&StdFs(path.into())));
    }
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {path}"))?;
    f(&ZipRoms::new(std::io::BufReader::new(file)).with_context(|| path.to_string())?)
}

/// `pack ROMS PACK`
fn pack_main(args: &[String]) -> Result<()> {
    let [roms, out] = args else { bail!("usage: t3_arcade pack ROMS PACK") };
    with_roms(roms, |roms| pack::convert(&Ctx { out: &StdFs(out.into()), log: &print_line, jobs: 1 }, roms))?;
    Ok(())
}

/// `list ROMS`: each stage's index, title, faces, lighting and floor tiles.
fn list_main(args: &[String]) -> Result<()> {
    let [roms] = args else { bail!("usage: t3_arcade list ROMS") };
    let mut i = 0;
    with_roms(roms, |roms| {
        pack::stages(roms, &mut |e, _, _| {
            print_line(&format!(
                "{i:2}  {:<34} {:>4} faces  {:<6}  {} floor tiles",
                e["title"].as_str().unwrap_or_default(),
                e["faces"].as_u64().unwrap_or_default(),
                e["lighting"].as_str().unwrap_or_default(),
                e["floor"]["tiles"]
            ));
            i += 1;
            Ok(())
        })
    })?;
    Ok(())
}

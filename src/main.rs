mod il2cpp;
mod metadata;
mod pkg;
mod proto;
mod ztable;

use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use pkg::Package;
use rayon::prelude::*;

#[derive(Clone, Debug, ValueEnum)]
#[value(rename_all = "lower")]
enum Language {
    English,
    Chinese,
    Japanese,
    TraditionalChinese,
    Korean,
    Thai,
    Indonesian,
    German,
    French,
    Spanish,
    Portuguese,
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Extract Blue Protocol: Star Resonance game data from meta.pkg",
    long_about = "Extract localized ZTables and, optionally, every package entry from Blue Protocol: Star Resonance's meta.pkg.\n\nBy default, the tool finds the game's IL2CPP files relative to meta.pkg and writes localized ZTable JSON files to <output>."
)]
struct Args {
    /// Path to the game's meta.pkg file.
    ///
    /// Usually located at:
    /// <game>/*_Data/StreamingAssets/container/meta.pkg
    ///
    /// Unless --dll is provided, this path is also used to locate
    /// <game>/GameAssembly.dll and
    /// <game>/*_Data/il2cpp_data/Metadata/global-metadata.dat.
    #[arg(long, short = 'p')]
    pkg: Utf8PathBuf,
    /// Path to an existing DummyDll directory containing Panda.Table.dll.
    ///
    /// Use this to skip automatic DLL generation from the game's IL2CPP files.
    #[arg(long, short = 'd')]
    dll: Option<Utf8PathBuf>,
    /// Directory where extracted files are written.
    ///
    /// Existing files with the same names are overwritten.
    #[arg(long, short = 'o', default_value = "extracted")]
    output: Utf8PathBuf,
    /// Extract all package entries in addition to localized ZTables.
    ///
    /// Lua files, protobuf descriptors, and unknown entries are written to
    /// separate subdirectories. UnityFS bundles still require --asset-bundles.
    #[arg(long)]
    all: bool,
    /// Include UnityFS asset bundles when extracting all entries.
    ///
    /// Requires --all. Bundles are written to the Bundles subdirectory.
    #[arg(long, requires = "all")]
    asset_bundles: bool,
    /// Language used to localize extracted ZTables.
    ///
    /// The value selects the matching <language>.bytes entry in meta.pkg.
    #[arg(short = 'l', long, default_value = "english")]
    language: Language,
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<()> {
    if !args.pkg.is_file() {
        bail!("PKG file not found: {}", args.pkg);
    }
    let dll = il2cpp::resolve(&args.pkg, args.dll.as_deref())?;
    for dir in ["ZTable", "Proto", "Bundles", "Lua", "Unk"] {
        fs::create_dir_all(args.output.join(dir))?;
    }

    let package = Package::open(&args.pkg)?;
    let language = args.language.to_possible_value().unwrap();
    let localization = package
        .read_by_key(hash33(&format!("{}.bytes", language.get_name())))
        .context("failed to read localization entry")?
        .context("localization entry is missing")?;
    let localization = ztable::Localization::parse(&localization)?;
    let tables = metadata::read_tables(dll.path())?;

    let progress = ProgressBar::new(tables.len() as u64);
    progress.set_style(
        ProgressStyle::with_template(
            "Extracting ZTables [{bar:40.cyan/blue}] {pos}/{len} ({elapsed_precise})",
        )?
        .progress_chars("=>-"),
    );
    let result: Result<()> = tables.into_par_iter().try_for_each(|table| {
        let key = hash33(&format!("{}.ctb", table.name));
        let Some(data) = package.read_by_key(key)? else {
            progress.println(format!("warning: table {} ({key}) is missing", table.name));
            progress.inc(1);
            return Ok(());
        };
        let value = ztable::parse(&data, &table.fields, &localization)
            .with_context(|| format!("failed to parse table {}", table.name))?;
        let path = args
            .output
            .join("ZTable")
            .join(format!("{}.json", table.name));
        fs::write(path, serde_json::to_vec_pretty(&value)?)?;
        progress.inc(1);
        Ok(())
    });
    progress.finish_and_clear();
    result?;

    if args.all {
        extract_all(&package, &args.output, args.asset_bundles)?;
    }
    Ok(())
}

fn extract_all(package: &Package, output: &Utf8Path, bundles: bool) -> Result<()> {
    package.entries().par_iter().try_for_each(|(&key, entry)| {
        let data = package.read(entry)?;
        if data.starts_with(b"UnityFS") {
            if bundles {
                fs::write(output.join("Bundles").join(format!("{key}.ab")), data)?;
            }
        } else if data.starts_with(b"\x1bLua") {
            pkg::write_lua(output, key, &data)?;
        } else {
            if data.windows(6).any(|w| w == b"proto2" || w == b"proto3")
                && let Err(error) = proto::dump(&data, &output.join("Proto"))
            {
                eprintln!("warning: invalid descriptor entry {key}: {error:#}");
            }
            fs::write(output.join("Unk").join(format!("{key}.bin")), data)?;
        }
        Ok(())
    })
}

pub(crate) fn hash33(value: &str) -> u32 {
    value.chars().fold(5381_u32, |hash, ch| {
        hash.wrapping_mul(33).wrapping_add(ch as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::{Language, ValueEnum, hash33};

    #[test]
    fn hash_matches_original() {
        assert_eq!(hash33("AvatarShowTable.ctb"), 3_524_225_204);
    }

    #[test]
    fn language_name_preserves_package_stem() {
        assert_eq!(
            Language::TraditionalChinese
                .to_possible_value()
                .unwrap()
                .get_name(),
            "traditionalchinese"
        );
    }
}

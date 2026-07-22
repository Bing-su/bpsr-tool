mod il2cpp;
mod metadata;
mod pkg;
mod proto;
mod ztable;

use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, ValueEnum};
use pkg::Package;

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
#[command(version, about)]
struct Args {
    /// Path to meta.pkg.
    #[arg(long, short = 'p')]
    pkg: Utf8PathBuf,
    /// Existing DummyDll directory (skips automatic generation).
    #[arg(long, short = 'd')]
    dll: Option<Utf8PathBuf>,
    /// Output directory.
    #[arg(long, short = 'o')]
    output: Utf8PathBuf,
    /// Extract every package entry.
    #[arg(long)]
    all: bool,
    /// Include UnityFS asset bundles (requires --all).
    #[arg(long, requires = "all")]
    asset_bundles: bool,
    /// Localization file name without the .bytes suffix.
    #[arg(long, default_value = "english")]
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

    for table in tables {
        let key = hash33(&format!("{}.ctb", table.name));
        let Some(data) = package.read_by_key(key)? else {
            eprintln!("warning: table {} ({key}) is missing", table.name);
            continue;
        };
        let value = ztable::parse(&data, &table.fields, &localization)
            .with_context(|| format!("failed to parse table {}", table.name))?;
        let path = args
            .output
            .join("ZTable")
            .join(format!("{}.json", table.name));
        fs::write(path, serde_json::to_vec_pretty(&value)?)?;
    }

    if args.all {
        extract_all(&package, &args.output, args.asset_bundles)?;
    }
    Ok(())
}

fn extract_all(package: &Package, output: &Utf8Path, bundles: bool) -> Result<()> {
    for (&key, entry) in package.entries() {
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
    }
    Ok(())
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

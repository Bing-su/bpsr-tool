mod il2cpp;
mod metadata;
mod pkg;
mod proto;
mod ztable;

use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use pkg::Package;
use rayon::prelude::*;
use tracing::{debug, info, warn};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ZtableFormat {
    JsonPretty,
    Json,
    JsonArray,
    Ndjson,
}

impl ZtableFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::JsonPretty | Self::Json | Self::JsonArray => "json",
            Self::Ndjson => "ndjson",
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about = "Extract Blue Protocol: Star Resonance game data")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Extract localized ZTables and package entries from meta.pkg.
    Extract(ExtractArgs),
    /// Generate Panda.Table.dll with the embedded Il2CppInspectorRedux.
    Il2cpp(Il2cppArgs),
}

#[derive(clap::Args, Debug)]
struct ExtractArgs {
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
    /// Output format for localized ZTables.
    #[arg(long, default_value = "json-pretty")]
    format: ZtableFormat,
}

#[derive(clap::Args, Debug)]
struct Il2cppArgs {
    /// Path to the game's meta.pkg file.
    ///
    /// Used to locate <game>/GameAssembly.dll and
    /// <game>/*_Data/il2cpp_data/Metadata/global-metadata.dat.
    #[arg(long, short = 'p')]
    pkg: Utf8PathBuf,
    /// Directory where Panda.Table.dll is written.
    ///
    /// An existing Panda.Table.dll is overwritten.
    #[arg(long, short = 'o')]
    output: Utf8PathBuf,
}

fn main() {
    tracing_subscriber::fmt().with_target(false).init();

    if let Err(error) = run(Args::parse().command) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn output_dirs(all: bool, bundles: bool) -> &'static [&'static str] {
    match (all, bundles) {
        (_, true) => &["ZTable", "Proto", "Lua", "Unk", "Bundles"],
        (true, false) => &["ZTable", "Proto", "Lua", "Unk"],
        (false, false) => &["ZTable"],
    }
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Extract(args) => extract(args),
        Command::Il2cpp(args) => generate_dll(args),
    }
}

fn generate_dll(args: Il2cppArgs) -> Result<()> {
    let dll = il2cpp::resolve(&args.pkg, None)?;
    fs::create_dir_all(&args.output)?;
    let output = args.output.join("Panda.Table.dll");
    fs::copy(dll.path(), &output)
        .with_context(|| format!("failed to write Panda.Table.dll to {output}"))?;
    info!(path = %output, "Panda.Table.dll written");
    Ok(())
}

fn extract(args: ExtractArgs) -> Result<()> {
    info!(
        pkg = %args.pkg,
        output = %args.output,
        language = ?args.language,
        format = ?args.format,
        all = args.all,
        asset_bundles = args.asset_bundles,
        "starting extraction"
    );
    if !args.pkg.is_file() {
        bail!("PKG file not found: {}", args.pkg);
    }

    info!("resolving Panda.Table.dll");
    let dll = il2cpp::resolve(&args.pkg, args.dll.as_deref())?;
    info!(path = %dll.path(), "Panda.Table.dll ready");

    let output_dirs = output_dirs(args.all, args.asset_bundles);
    debug!(?output_dirs, "creating output directories");
    for dir in output_dirs {
        fs::create_dir_all(args.output.join(dir))?;
    }

    info!("loading package index");
    let package = Package::open(&args.pkg)?;
    let language = args.language.to_possible_value().unwrap();
    info!(language = language.get_name(), "loading localization");
    let localization = package
        .read_by_key(hash33(&format!("{}.bytes", language.get_name())))
        .context("failed to read localization entry")?
        .context("localization entry is missing")?;
    let localization = ztable::Localization::parse(&localization)?;
    let tables = metadata::read_tables(dll.path())?;

    info!(tables = tables.len(), "extracting localized ZTables");
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
        let path =
            args.output
                .join("ZTable")
                .join(format!("{}.{}", table.name, args.format.extension()));
        fs::write(path, serialize_ztable(&value, args.format)?)?;
        progress.inc(1);
        Ok(())
    });
    progress.finish();
    result?;
    info!("localized ZTable extraction complete");

    if args.all {
        info!(
            entries = package.entries().len(),
            asset_bundles = args.asset_bundles,
            "extracting package entries"
        );
        extract_all(&package, &args.output, args.asset_bundles)?;
        info!("package entry extraction complete");
    }
    info!("extraction complete");
    Ok(())
}

fn serialize_ztable(value: &serde_json::Value, format: ZtableFormat) -> Result<Vec<u8>> {
    match format {
        ZtableFormat::JsonPretty => Ok(serde_json::to_vec_pretty(value)?),
        ZtableFormat::Json => Ok(serde_json::to_vec(value)?),
        ZtableFormat::JsonArray => Ok(serde_json::to_vec(
            &value
                .as_object()
                .context("ZTable is not an object")?
                .values()
                .collect::<Vec<_>>(),
        )?),
        ZtableFormat::Ndjson => {
            let mut output = Vec::new();
            for row in value
                .as_object()
                .context("ZTable is not an object")?
                .values()
            {
                serde_json::to_writer(
                    &mut output,
                    row.as_object().context("ZTable row is not an object")?,
                )?;
                output.push(b'\n');
            }
            Ok(output)
        }
    }
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
                warn!(key, error = %format!("{error:#}"), "invalid descriptor entry");
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
    use super::*;
    use rstest::rstest;

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

    #[rstest]
    #[case(false, false, &["ZTable"])]
    #[case(true, false, &["ZTable", "Proto", "Lua", "Unk"])]
    #[case(true, true, &["ZTable", "Proto", "Lua", "Unk", "Bundles"])]
    fn output_directories_follow_extraction_options(
        #[case] all: bool,
        #[case] bundles: bool,
        #[case] expected: &[&str],
    ) {
        assert_eq!(output_dirs(all, bundles), expected);
    }

    #[test]
    fn parses_subcommands() {
        let Command::Extract(args) =
            Args::try_parse_from(["bpsr-tool", "extract", "-p", "meta.pkg"])
                .unwrap()
                .command
        else {
            panic!("expected extract command");
        };
        assert_eq!(args.format, ZtableFormat::JsonPretty);
        assert!(matches!(
            Args::try_parse_from(["bpsr-tool", "il2cpp", "-p", "meta.pkg", "-o", "DummyDll"])
                .unwrap()
                .command,
            Command::Il2cpp(_)
        ));
    }

    #[rstest]
    #[case("json-pretty", ZtableFormat::JsonPretty)]
    #[case("json", ZtableFormat::Json)]
    #[case("json-array", ZtableFormat::JsonArray)]
    #[case("ndjson", ZtableFormat::Ndjson)]
    fn parses_ztable_formats(#[case] name: &str, #[case] expected: ZtableFormat) {
        let Command::Extract(args) =
            Args::try_parse_from(["bpsr-tool", "extract", "-p", "meta.pkg", "--format", name])
                .unwrap()
                .command
        else {
            panic!("expected extract command");
        };
        assert_eq!(args.format, expected);
    }

    #[test]
    fn rejects_unknown_ztable_format() {
        assert!(
            Args::try_parse_from(["bpsr-tool", "extract", "-p", "meta.pkg", "--format", "csv"])
                .is_err()
        );
    }

    #[rstest]
    #[case(ZtableFormat::JsonPretty, "{\n  \"42\": {\n    \"Id\": 42\n  }\n}")]
    #[case(ZtableFormat::Json, r#"{"42":{"Id":42}}"#)]
    fn serializes_json_objects(#[case] format: ZtableFormat, #[case] expected: &str) {
        let value = serde_json::json!({"42": {"Id": 42}});

        assert_eq!(
            String::from_utf8(serialize_ztable(&value, format).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn serializes_json_array() {
        let value = serde_json::json!({
            "42": {"Id": 42, "Value": 7},
            "84": {"Id": 84, "Nested": [1, 2]}
        });
        let array: serde_json::Value =
            serde_json::from_slice(&serialize_ztable(&value, ZtableFormat::JsonArray).unwrap())
                .unwrap();
        assert_eq!(
            array,
            serde_json::json!([
                {"Id": 42, "Value": 7},
                {"Id": 84, "Nested": [1, 2]}
            ])
        );
    }

    #[test]
    fn serializes_ndjson() {
        let value = serde_json::json!({
            "42": {"Id": 42, "Value": 7},
            "84": {"Id": 84, "Nested": [1, 2]}
        });

        assert_eq!(
            String::from_utf8(serialize_ztable(&value, ZtableFormat::Ndjson).unwrap()).unwrap(),
            "{\"Id\":42,\"Value\":7}\n{\"Id\":84,\"Nested\":[1,2]}\n"
        );
        assert_eq!(
            serialize_ztable(&serde_json::json!({}), ZtableFormat::Ndjson).unwrap(),
            b""
        );
    }
}

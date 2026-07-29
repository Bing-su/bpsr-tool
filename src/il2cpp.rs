use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

#[cfg(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
use std::{
    fs::{self, File},
    io::{self, Cursor},
    process::Command,
};

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const INSPECTOR: (&[u8], &str, &str) = (
    include_bytes!("../asset/Il2CppInspectorRedux.Legacy.CLI-win-x64.zip"),
    "Il2CppInspectorRedux.Legacy.CLI-win-x64/Il2CppInspector.exe",
    "Il2CppInspector.exe",
);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const INSPECTOR: (&[u8], &str, &str) = (
    include_bytes!("../asset/Il2CppInspectorRedux.Legacy.CLI-linux-x64.zip"),
    "Il2CppInspectorRedux.Legacy.CLI-linux-x64/Il2CppInspector",
    "Il2CppInspector",
);
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const INSPECTOR: (&[u8], &str, &str) = (
    include_bytes!("../asset/Il2CppInspectorRedux.Legacy.CLI-linux-arm64.zip"),
    "Il2CppInspectorRedux.Legacy.CLI-linux-arm64/Il2CppInspector",
    "Il2CppInspector",
);
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const INSPECTOR: (&[u8], &str, &str) = (
    include_bytes!("../asset/Il2CppInspectorRedux.Legacy.CLI-osx-x64.zip"),
    "Il2CppInspectorRedux.Legacy.CLI-osx-x64/Il2CppInspector",
    "Il2CppInspector",
);
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const INSPECTOR: (&[u8], &str, &str) = (
    include_bytes!("../asset/Il2CppInspectorRedux.Legacy.CLI-osx-arm64.zip"),
    "Il2CppInspectorRedux.Legacy.CLI-osx-arm64/Il2CppInspector",
    "Il2CppInspector",
);

pub(crate) struct Dll {
    path: Utf8PathBuf,
    _temp: Option<TempDir>,
}

impl Dll {
    pub(crate) fn path(&self) -> &Utf8Path {
        &self.path
    }
}

pub(crate) fn resolve(pkg: &Utf8Path, directory: Option<&Utf8Path>) -> Result<Dll> {
    if let Some(directory) = directory {
        let path = directory.join("Panda.Table.dll");
        if !path.is_file() {
            bail!("Panda.Table.dll not found in {directory}");
        }
        return Ok(Dll { path, _temp: None });
    }
    generate(pkg)
}

fn game_file_paths(pkg: &Utf8Path) -> Result<(Utf8PathBuf, Utf8PathBuf)> {
    let data = pkg
        .parent()
        .and_then(Utf8Path::parent)
        .and_then(Utf8Path::parent)
        .context("expected meta.pkg under <game>/*_Data/StreamingAssets/container")?;
    let game = data.parent().context("game data directory has no parent")?;
    Ok((
        game.join("GameAssembly.dll"),
        data.join("il2cpp_data/Metadata/global-metadata.dat"),
    ))
}

#[cfg(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
fn generate(pkg: &Utf8Path) -> Result<Dll> {
    let pkg = pkg
        .canonicalize_utf8()
        .with_context(|| format!("failed to resolve PKG path {pkg}"))?;
    let (game_assembly, metadata) = game_file_paths(&pkg)?;
    if !game_assembly.is_file() || !metadata.is_file() {
        bail!("automatic DLL generation requires game files:\n  {game_assembly}\n  {metadata}");
    }

    let (temp, executable, output) = prepare_inspector()?;
    let dll_output = output.join("dll");
    let status = Command::new(&executable)
        .args([
            "-i",
            game_assembly.as_str(),
            "-m",
            metadata.as_str(),
            "--select-outputs",
            "--dll-out",
            dll_output.as_str(),
        ])
        .current_dir(
            executable
                .parent()
                .context("Il2CppInspectorRedux has no parent")?,
        )
        .status()
        .with_context(|| {
            format!("failed to start embedded Il2CppInspectorRedux at {executable}")
        })?;
    let path = validate_inspector_output(status, &output)?;

    Ok(Dll {
        path,
        _temp: Some(temp),
    })
}

#[cfg(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
fn validate_inspector_output(
    status: std::process::ExitStatus,
    output: &Utf8Path,
) -> Result<Utf8PathBuf> {
    if !status.success() {
        bail!("Il2CppInspectorRedux failed with {status}");
    }

    let path = output.join("dll/Panda.Table.dll");
    if !path.is_file() {
        bail!("Il2CppInspectorRedux did not generate {path}");
    }
    Ok(path)
}

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
fn generate(_pkg: &Utf8Path) -> Result<Dll> {
    bail!(
        "automatic Panda.Table.dll generation is unavailable for {}/{}; use --dll",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[cfg(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
fn prepare_inspector() -> Result<(TempDir, Utf8PathBuf, Utf8PathBuf)> {
    let temp =
        tempfile::tempdir().context("failed to create Il2CppInspectorRedux temporary directory")?;
    let root = Utf8PathBuf::try_from(temp.path().to_owned())
        .context("Il2CppInspectorRedux temporary path is not valid UTF-8")?;
    let tool = root.join("tool");
    let output = root.join("output");
    fs::create_dir_all(&tool)?;
    fs::create_dir_all(&output)?;

    let mut archive = zip::ZipArchive::new(Cursor::new(INSPECTOR.0))
        .context("invalid embedded Il2CppInspectorRedux archive")?;
    let executable = tool.join(INSPECTOR.2);
    {
        let mut entry = archive
            .by_name(INSPECTOR.1)
            .context("embedded Il2CppInspectorRedux executable is missing")?;
        let mut file = File::create(&executable)?;
        io::copy(&mut entry, &mut file)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
    }

    Ok((temp, executable, output))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn derives_game_files_from_pkg() {
        let pkg = Utf8Path::new("C:/game/StarASIA_STEAM_Data/StreamingAssets/container/meta.pkg");
        let (assembly, metadata) = game_file_paths(pkg).unwrap();
        assert_eq!(assembly, "C:/game/GameAssembly.dll");
        assert_eq!(
            metadata,
            "C:/game/StarASIA_STEAM_Data/il2cpp_data/Metadata/global-metadata.dat"
        );
    }

    #[test]
    fn accepts_existing_dummy_dll() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Utf8PathBuf::try_from(temp.path().to_owned()).unwrap();
        fs::write(directory.join("Panda.Table.dll"), []).unwrap();
        let dll = resolve(Utf8Path::new("unused"), Some(&directory)).unwrap();
        assert_eq!(dll.path(), directory.join("Panda.Table.dll"));
    }

    #[cfg(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(
            any(target_os = "linux", target_os = "macos"),
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    #[test]
    fn prepares_embedded_inspector() {
        let (_temp, executable, _output) = prepare_inspector().unwrap();
        assert!(executable.is_file());
        let extracted = fs::read_dir(executable.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(extracted, [INSPECTOR.2]);
    }

    #[cfg(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(
            any(target_os = "linux", target_os = "macos"),
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    #[test]
    fn reports_both_missing_game_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::try_from(temp.path().to_owned()).unwrap();
        let pkg = root.join("Game_Data/StreamingAssets/container/meta.pkg");
        fs::create_dir_all(pkg.parent().unwrap()).unwrap();
        fs::write(&pkg, []).unwrap();
        let expected = game_file_paths(&pkg.canonicalize_utf8().unwrap()).unwrap();

        let error = resolve(&pkg, None).err().unwrap();
        let message = format!("{error:#}");
        assert!(message.contains(expected.0.as_str()));
        assert!(message.contains(expected.1.as_str()));
    }

    #[cfg(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(
            any(target_os = "linux", target_os = "macos"),
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    #[test]
    fn reports_inspector_failure() {
        let error = validate_inspector_output(exit_status(7), Utf8Path::new("unused"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("Il2CppInspectorRedux failed"));
    }

    #[cfg(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(
            any(target_os = "linux", target_os = "macos"),
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    #[test]
    fn reports_missing_generated_dll() {
        let temp = tempfile::tempdir().unwrap();
        let output = Utf8PathBuf::try_from(temp.path().to_owned()).unwrap();
        let error = validate_inspector_output(exit_status(0), &output)
            .err()
            .unwrap();
        assert!(error.to_string().contains("did not generate"));
        assert!(error.to_string().contains("Panda.Table.dll"));
    }

    #[cfg(windows)]
    fn exit_status(code: u8) -> std::process::ExitStatus {
        Command::new("cmd.exe")
            .args(["/C", &format!("exit {code}")])
            .status()
            .unwrap()
    }

    #[cfg(unix)]
    fn exit_status(code: u8) -> std::process::ExitStatus {
        Command::new("sh")
            .args(["-c", &format!("exit {code}")])
            .status()
            .unwrap()
    }
}

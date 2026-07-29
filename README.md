# bpsr-tool

Rust data extractor for **Blue Protocol: Star Resonance**.

It can:

- reconstruct localized ZTables from `Panda.Table.dll`
- dump protobuf descriptors
- extract Lua, UnityFS, and unknown assets from PKG containers

## Usage

```bash
bpsr-tool --pkg <meta.pkg> --output <directory> [--dll <DummyDll>] [--all] [--asset-bundles] [--language english]
```

| Option             | Description                                                  |
| ------------------ | ------------------------------------------------------------ |
| `--pkg`, `-p`      | Path to `meta.pkg`                                           |
| `--output`, `-o`   | Output directory (default: `extracted`)                      |
| `--dll`, `-d`      | Existing `DummyDll` directory containing `Panda.Table.dll`   |
| `--all`            | Extract all package entries in addition to localized ZTables |
| `--asset-bundles`  | Include UnityFS bundles; requires `--all`                    |
| `--language`, `-l` | ZTable language (default: `english`)                         |

By default, only localized ZTable JSON files are produced. Use `--all` to
extract the remaining package entries, and add `--asset-bundles` to include
UnityFS bundles.

## DLL generation

When `--dll` is omitted, the tool finds `GameAssembly.dll` and
`global-metadata.dat` relative to `meta.pkg`, then uses the embedded
Il2CppInspectorRedux Legacy CLI to generate a temporary `Panda.Table.dll`.

```text
<game>/GameAssembly.dll
<game>/*_Data/il2cpp_data/Metadata/global-metadata.dat
<game>/*_Data/StreamingAssets/container/meta.pkg
```

Automatic generation supports:

- Windows x64
- Linux x64 and ARM64
- macOS x64 and ARM64

The bundled CLI requires the .NET 10 Runtime. To skip automatic
generation, pass an existing dummy DLL directory with `--dll`. The game
installation is not modified.

## License

This is a GPL-3.0 Rust rewrite of
[PotRooms/StarResonanceTool](https://github.com/PotRooms/StarResonanceTool).
Real game files are not included.

The embedded
[Il2CppInspectorRedux 2026.2](https://github.com/LukeFZ/Il2CppInspectorRedux)
is distributed under the AGPL-3.0 License. See
`asset/Il2CppInspectorRedux-NOTICE` and
`asset/Il2CppInspectorRedux-LICENSE`.

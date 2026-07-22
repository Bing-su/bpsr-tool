# bpsr-tool

Rust data extractor for **Blue Protocol: Star Resonance**. It reads the game's
PKG containers, reconstructs ZTables from `Panda.Table.dll`, dumps protobuf
descriptors, and extracts Lua, UnityFS, and unknown assets.

```text
bpsr-tool --pkg <meta.pkg> --output <directory> [--dll <DummyDll>] [--all] [--asset-bundles] [--language english]
```

Without `--all`, only localized ZTable JSON files are produced. `--all` also
extracts package entries; add `--asset-bundles` to include UnityFS bundles.

When `--dll` is omitted, the tool locates `GameAssembly.dll` and
`global-metadata.dat` from the standard game layout relative to `meta.pkg`,
then generates a temporary `Panda.Table.dll` with the embedded
Il2CppInspectorRedux. Automatic generation supports Windows x64, Linux
x64/ARM64, and macOS x64/ARM64. Pass an existing dummy DLL directory with
`--dll` to skip this step. The bundled CLI requires the ASP.NET Core 10
Runtime. The game installation is not modified.

```text
<game>/GameAssembly.dll
<game>/*_Data/il2cpp_data/Metadata/global-metadata.dat
<game>/*_Data/StreamingAssets/container/meta.pkg
```

The embedded [Il2CppInspectorRedux 2026.2](https://github.com/LukeFZ/Il2CppInspectorRedux)
is distributed under the AGPL-3.0 License; see
`asset/Il2CppInspectorRedux-NOTICE` and
`asset/Il2CppInspectorRedux-LICENSE`.

This is a GPL-3.0 Rust rewrite of
[PotRooms/StarResonanceTool](https://github.com/PotRooms/StarResonanceTool).
Real game files are not included.

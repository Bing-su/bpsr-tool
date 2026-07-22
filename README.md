# bpsr-tool

Rust data extractor for **Blue Protocol: Star Resonance**. It reads the game's
PKG containers, reconstructs ZTables from `Panda.Table.dll`, dumps protobuf
descriptors, and extracts Lua, UnityFS, and unknown assets.

```text
bpsr-tool --pkg <meta.pkg> --dll <DummyDll> --output <directory> [--all] [--asset-bundles] [--language english]
```

Without `--all`, only localized ZTable JSON files are produced. `--all` also
extracts package entries; add `--asset-bundles` to include UnityFS bundles.

This is a GPL-3.0 Rust rewrite of
[PotRooms/StarResonanceTool](https://github.com/PotRooms/StarResonanceTool).
Real game files are not included.

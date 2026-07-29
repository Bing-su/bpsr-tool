# bpsr-game-files.zip
# └┬─ *_Data
#  │  └─ il2cpp_data
#  │     └─ Metadata
#  ├        └─ global-metadata.dat
#  ├── StreamingAssets
#  ├   └─ container
#  ├      ├─ meta.pkg
#  ├      └─ m0.pkg
#  └─ GameAssembly.dll

def main [game_root: path] {
  let game_root = ($game_root | path expand)
  let data_dirs = (
    ls $game_root
    | where { |entry| $entry.type == dir and ($entry.name | str ends-with "_Data") }
    | get name
    | where { |dir|
        [
          "il2cpp_data/Metadata/global-metadata.dat"
          "StreamingAssets/container/meta.pkg"
          "StreamingAssets/container/m0.pkg"
        ]
        | all { |path| ($dir | path join $path) | path exists }
      }
  )

  if ($data_dirs | length) != 1 {
    error make { msg: $"expected one matching *_Data directory under ($game_root)" }
  }

  let files = [
    ($game_root | path join "GameAssembly.dll")
    ($data_dirs.0 | path join "il2cpp_data/Metadata/global-metadata.dat")
    ($data_dirs.0 | path join "StreamingAssets/container/meta.pkg")
    ($data_dirs.0 | path join "StreamingAssets/container/m0.pkg")
  ]
  let missing = ($files | where { |file| not ($file | path exists) })
  if not ($missing | is-empty) {
    error make { msg: $"missing required file: ($missing.0)" }
  }

  let output = ("bpsr-game-files.zip" | path expand)
  if ($output | path exists) {
    error make { msg: $"output already exists: ($output)" }
  }

  let relative_files = ($files | path relative-to $game_root)
  ^tar avcf $output -C $game_root ...$relative_files
  print $output
}

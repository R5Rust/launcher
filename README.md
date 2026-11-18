# R5Rust Launcher

A small Windows desktop launcher for the separate `r5rust` game project. It will let you choose the game folder and start the staged build through that project's `play.ps1` update/launch flow.

## Run

```powershell
cargo run
```

Requires Rust/Cargo and the `r5rust` project. The launcher remembers the selected game folder in your Windows app-data directory.

## Status

Work in progress. The launcher UI and launch flow are being built; no game files are copied into this project.

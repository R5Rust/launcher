# R5Rust Launcher

A small Windows desktop launcher for the separate `r5rust` game project. It checks for a staged build, lets you choose the game folder, and starts the game through `play.ps1` so the game's existing update-and-launch flow stays in charge.

## Run

Requires Rust/Cargo, PowerShell 7 (`pwsh`), and the `r5rust` project.

```powershell
cd C:\Code\apps\r5rust-launcher
cargo run
```

The default game folder is `C:\Code\games\r5rust`. Use **Browse…** to choose another folder; the launcher remembers it in `%APPDATA%\R5Rust Launcher\game_path.txt`. A valid folder contains `play.ps1` and a staged `play\current\r5rust.exe` or `play\next\r5rust.exe`.

If no build is staged, run `tools\publish.ps1` from the R5Rust project before launching.

## Status

The basic launcher UI, folder selection, saved folder, build check, and hidden PowerShell launch are implemented. It does not build R5Rust, manage game settings, or include game assets; those stay in the separate game project.

# R5Rust Launcher

A small Windows desktop launcher for the separate `r5rust` game project. Choose a game mode, check the staged build, and launch through `play.ps1` so the game's existing update-and-launch flow stays in charge.

## Run

Requires Rust/Cargo, PowerShell 7 (`pwsh`), and the `r5rust` project.

```powershell
cd C:\Code\apps\r5rust-launcher
cargo run
```

The default game folder is `C:\Code\games\r5rust`. Use **Change folder** to choose another folder; the launcher remembers it in `%APPDATA%\R5Rust Launcher\game_path.txt`. A valid folder contains `play.ps1` and a staged `play\current\r5rust.exe` or `play\next\r5rust.exe`.

If no build is staged, run `tools\publish.ps1` from the R5Rust project before launching.

## Modes and look

Firing Range is currently the only playable mode. Its selector uses the supplied screenshot as full-bleed background art; **More modes** is shown as a future placeholder. The visual direction borrows from the R5Regenis launcher: flat blue-grey surfaces, quiet navigation, dark art scrims, a cyan active/play accent, and Barlow Condensed display type. The bundled font is under `assets\fonts\OFL.txt`.

The window and executable use the same `assets\icon.png` and `assets\icon.ico` as the game for a matching taskbar/app icon.

## Status

The mode selector, art-led play screen, game-matching icon, saved folder, build check, and hidden PowerShell launch are implemented. The launcher does not build R5Rust, manage game settings, or contain game assets beyond its presentation images/icons.

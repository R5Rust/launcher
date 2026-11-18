# R5Rust Launcher

A small Windows desktop launcher for the separate `r5rust` game project. Choose a game mode, check the staged build, and launch through `play.ps1` so the game's existing update-and-launch flow stays in charge.

## Run

Requires Rust/Cargo, PowerShell 7 (`pwsh`), and the `r5rust` project.

```powershell
cd C:\Code\apps\r5rust-launcher
cargo run
```

The default game folder is `C:\Code\games\r5rust`. Use the **Game folder** picker to choose another folder; the launcher remembers it in `%APPDATA%\R5Rust Launcher\game_path.txt`. A valid folder contains `play.ps1` and a staged `play\current\r5rust.exe` or `play\next\r5rust.exe`.

If no build is staged, run `tools\publish.ps1` from the R5Rust project before launching.

## Modes and look

Firing Range is currently the only playable mode. The **Modes** menu selects it and shows a disabled placeholder for future modes; the supplied screenshot fills the Play page behind the mode title. The layout follows R5Regenis closely: frameless title controls, a 228px navigation rail, full-bleed art, and a bottom status/selection/Play bar. Colors, quiet navigation, dark art scrims, and Barlow Condensed display type follow its launcher look. The bundled font license is `assets\fonts\OFL.txt`.

The window and executable use the same `assets\icon.png` and `assets\icon.ico` as the game for a matching taskbar/app icon.

## Status

The mode menu, art-led Play page, matching icon, saved game folder, build check, and hidden PowerShell launch are implemented. The launcher does not build R5Rust, manage game settings, or contain game assets beyond its presentation images/icons.

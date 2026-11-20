# R5Rust Launcher

A Windows desktop launcher for the separate `r5rust` game project (`C:\Code\games\r5rust`). Pick the mode, see what
build you're on, build the latest code, play, and watch the game's output live. It launches through the game's own
`play.ps1`, so the game's update-and-launch flow stays in charge.

The launcher ships **no game files or game art**. The Play page shows the map's real loading screen read from your own
extracted assets (`<assets>\images\ui\maps_<map>_loadscreen\`), like R5Regenis' and R5Flowstate's launchers do; without
it you get a plain dark backdrop.

## Run

Requires Rust/Cargo, PowerShell 7 (`pwsh`), and the `r5rust` project. Git is optional (Patch Notes and the "new commits"
count use it).

```powershell
cd C:\Code\apps\r5rust-launcher
cargo run --release
```

Flags: `--page=notes|console|settings` opens on that page; `--maximized` starts maximized; `--autoplay` presses Play once it's possible (for scripted
tests). Press **Enter** on any page to play.

## Pages

| Page | What's on it |
|---|---|
| **Play** | the map's loading screen, MODES menu (Firing Range for now), and a "game files missing" card when the extracted map isn't there |
| **Patch Notes** | the game's git log, grouped by day, with PLAYING / STAGED / NOT BUILT chips |
| **Console** | the game's stdout/stderr, live, with a filter, "warnings and errors only", copy, and open log. Shows the last session after a restart |
| **Settings** | **Game**: FOV, sensitivity, auto sprint, mantle boost, invert look, written straight into the game's `r5rust_settings.toml` (binds kept; locked while the game runs). **Graphics**: preset (Lowest..Ultra) + texture quality, sun shadows, anisotropic filtering, anti-aliasing, render scale, FPS cap, V-Sync, written to the game's `[graphics]` table (same preset table as the game's `graphics.rs`; touching a knob makes it Custom). **Launch options** with a preview of the exact command line. **Game folder**, **Game files** (checks `world.glb`, `collision.bin`, `materials.json`, `meta.json`), **Launcher** (open Console on launch), log/settings file shortcuts |

The footer is always there: installed build hash + commit subject, a status line (update staged, N commits not built,
files missing, playing...), **BUILD LATEST** (runs `tools\publish.ps1`, live log panel, never touches the copy you're
playing) and **PLAY**, which becomes **STARTING** / **STOP** while the game runs. Stop kills `play.ps1` and the game
with it (`taskkill /T`).

## Where things live

- Launcher settings: `%APPDATA%\R5Rust Launcher\` (`game_path.txt`, `launch_args.txt`, `console_on_launch.txt`).
- Logs: `%APPDATA%\R5Rust Launcher\game.log` and `build.log`, rewritten each launch / build.
- Game assets root: same rule as the game (`R5RUST_ASSETS` env > `assets_root` in `r5rust.toml` > `D:\R5Rust\assets`).
- Map: `map` in `r5rust.toml`, else the first `bsp` in the game's `content\maps\*.toml`.

## Look

Follows R5Regenis' launcher: flat near-black panels, no corner radius except the switch knob, one cyan accent
(`#3fd8ff`) for the active nav item and Play, Barlow Condensed for the big words, Inter for text, JetBrains Mono for the
console. Fonts are OFL (`assets\fonts\OFL-*.txt`). The window, taskbar and exe use the launcher's own icon,
`assets\app-icon.png` / `app-icon.ico` (the game's white gear+A on a dark square, like R5Flowstate's square launcher icon,
so it can't be mixed up with the game's brown banner); the rail logo is still the game's banner (`assets\icon.png`).

Ideas taken from the reference launchers: R5Regenis (layout, map art from the install, settings card, first-run card),
R5Flowstate (Console tab, Play becomes Stop, launch-args preview, plain-sentence status line), R5Reloaded (verify game
files).

## Code

- `src\main.rs`: the UI (egui 0.28 / eframe).
- `src\game.rs`: everything about the game folder: staged builds (`play\current|next\version.txt`), git history,
  process detection (Toolhelp), the background poller (every 3 s, never on the UI thread), the play/build jobs (output
  drained on threads, written to the log), game settings via `toml_edit`, game-files check.
- `cargo test --release -- --test-threads=1`: 10 tests (jobs run real `pwsh` scripts in temp folders; they never write
  to your `%APPDATA%`).

## Status / unfinished

- Only Firing Range. More modes, a map picker and mods would follow the game.
- No self-update, no installer: run it from cargo or copy `target\release\r5rust-launcher.exe`.
- The Console is read-only; R5Rust has no console commands yet.

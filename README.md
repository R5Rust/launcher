# R5Rust Launcher

**[Download R5Rust Launcher for Windows](https://github.com/R5Rust/launcher/releases/latest/download/R5Rust-Launcher.exe)**
(one exe, always the newest version)

1. Run `R5Rust-Launcher.exe`. It installs itself (Start menu + desktop shortcut, uninstall from Apps & features) and
   opens. Windows may show "Windows protected your PC" because the exe isn't code-signed: **More info > Run anyway**.
2. **Download** gets the newest game build. Later, the same button says **Update** when there's a newer one; the
   launcher updates itself too (the version in the bottom left turns into an **Update** button).
3. **Install** builds the game files from **your own Apex Legends install** (Apex must be closed; R5Rust ships no game
   files). Takes a while, once.
4. **Play**.

Everything lives in `%LOCALAPPDATA%\R5Rust` (game, built files) and `%LOCALAPPDATA%\Programs\R5Rust Launcher`.

---

## For developers

A Windows desktop launcher for the `r5rust` game project. With a dev checkout (`C:\Code\games\r5rust`, git +
`tools\publish.ps1`) it builds the latest code with cargo; for players it downloads builds from GitHub Releases
(player mode, `src\update.rs`). Either way it launches through the game's own `play.ps1`, so the game's
update-and-launch flow stays in charge.

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

Flags: `--page=notes|mods|console|settings` opens on that page (`--mods-tab=browse`, `--mods-view=list` pick the Mods tab / view); `--maximized` starts maximized; `--install` opens the install card; `--install-demo` runs the installer's made-up progress (touches nothing); `--screenshot=<png>` opens without focus, saves one frame once the install check and textures are in, and closes (it ignores all mouse/keyboard input, so a live desktop can't click anything); `--idle` (with `--screenshot`) draws the idle state even while a game runs; `--expand-notes` opens the two newest patch-note updates; `--open-chip=<NAME>` holds a quick chip's menu open; `--open-maps` opens the maps strip, `--open-maps-grid` the All maps grid, `--demo-maps=N` (screenshot mode only) fills the list with N made-up maps to show looping, `--strip-pos=2.5` scrolls the strip; `--custom-open` reveals the custom FPS / resolution fields; `--preview-fps=180`, `--preview-res=1366x768` and `--preview-input=<text>` (screenshot mode only) show a value without saving it; `--scroll=graphics` scrolls Settings to Graphics; `--autoplay` presses Play once it's possible (for scripted
tests). Press **Enter** on any page to play.

Checking the UI: `--screenshot=<png> --idle` plus `--page=...` / `--install-demo` / `--expand-notes`, one run per page.
Build into a separate target dir (`$env:CARGO_TARGET_DIR`) if a launcher is already open, since it locks the exe.

## Pages

| Page | What's on it |
|---|---|
| **Play** | the map's loading screen, MODES menu (Firing Range for now), and the **install card** when the game files aren't there (or opened from Settings): Apex folder (found via the EA app / Steam), where to save, an "Apex is closed" confirmation, **INSTALL**; then a step list with a weighted progress bar, Stop / Try again / Open log. It runs the game's `tools\install.ps1` (see the game README) |
| **Patch Notes** | the game's last 100 commits grouped into one **update per day** (named `UPDATE yy.mm.dd`; the oldest, cut-off day is left out). The newest day is a featured card: map art, headline, up to 4 highlights, READ MORE for every commit. Older days are expandable rows (date tile, headline, "Also: ..."). Each commit shows its subject, its body as sub-notes (`- ` bullets joined), time, and PLAYING / STAGED / NOT BUILT. Headlines are cut from commit subjects (text before `: `, `; `, ` (`); nothing is made up |
| **Mods** | **not working yet: R5Rust doesn't load mods.** The page is laid out for when it does: **Installed** / **Browse** tabs (mods are free, no prices), grid / list view toggle, a search box and sort / filter chips (inert). Both lists are empty (`installed_mods` / `browse_mods`, a `Mod` struct: name, author, version, description, tags, enabled), so each tab shows a "Mods are coming in a future update" banner and a layout preview of grey skeleton cards marked EXAMPLE; every action (enable switch, Install) is disabled with "Coming soon" |
| **Console** | the game's stdout/stderr, live, with a filter, "warnings and errors only", copy, and open log; a strip with line / warning / error counts. Shows the last session after a restart |
| **Settings** | **Game**: FOV, sensitivity, auto sprint, mantle boost, invert look, written straight into the game's `r5rust_settings.toml` (binds kept; locked while the game runs). **Graphics**: preset (Lowest..Ultra) + texture quality, sun shadows, anisotropic filtering, anti-aliasing, render scale, FPS cap, V-Sync, written to the game's `[graphics]` table (same preset table as the game's `graphics.rs`; touching a knob makes it Custom). **Launch options** with a preview of the exact command line. **Game folder**, **Game files** (checks `world.glb`, `collision.bin`, `materials.json`, `meta.json`; install status from `install.ps1 -Check`: installed / update needed after an Apex patch / not finished; Install / Update button), **Launcher** (open Console on launch), log/settings file shortcuts |

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

v0.3 (2026-11-21), from Kainz's mockups with small cues from the EA app and Steam's library: **dark charcoal with one
red accent**. The design tokens sit at the top of `src\main.rs` and every page uses only those:

- Colours: a charcoal ramp (`BG` / `RAIL` / `PANEL` / `RAISED` / `HOVER` / `BORDER`), text in three steps, red `ACCENT`
  `#e22c34` only for real UI accents (active nav item, PLAY / INSTALL, selected segments, switches, sliders, progress,
  small markers). Status colours are separate so "error" never looks like "selected": green `GOOD`, `AMBER`, salmon `BAD`.
- Spacing 4 / 8 / 12 / 16 / 24 / 32 / 48; corners 3 px on controls, 4 px on cards, pills for switches.
- Type: Barlow Condensed (always upper case) for short labels: page titles 50, hero 76, update titles, section titles,
  button labels; Inter for sentences; JetBrains Mono for hashes, paths and the console. Small captions are tracked
  upper case.
- One line-icon set drawn in code (1.6 px stroke at 16 px): play, notes, console, gear, calendar, chevrons, close, doc,
  folder, copy, check, refresh.
- Backdrop: Kainz's grunge texture (`assets\bg-grunge.png`, greyscale, toned down) covering the window, plus a vignette.
  **No decorative shapes** (no diagonal streaks or red background art: Kainz, they read as AI slop).
- The rail logo is still the game's banner (`assets\icon.png`), drawn exactly as in v0.2 (Kainz's call). The window,
  taskbar and exe use `assets\app-icon.png` / `app-icon.ico` (the game's white gear+A on a dark square).

**Maps** (R5Flowstate's map picker, in our look): the **Maps** pill top right of the map card opens a **carousel strip**
under it: map tiles with their names, the selected one raised with a red outline, arrows at both ends, mouse wheel
(one tile per notch), eased scrolling that only repaints while moving. With more maps than fit, it loops (after the
last comes the first again); with fewer (Firing Range's 2), it shrinks to just those tiles, centred, arrows dimmed.
**All maps** in the strip opens a **grid** over the map card (every map of the mode, selected outlined, locked ones
dimmed with a lock and "Coming later"; X or Esc closes, scrolls when there are many). The list is data (`MODES` at the top of `src\main.rs`: mode -> maps with bsp, name, note, playable). Firing Range has
two: `mp_rr_canyonlands_staging_mu1` (the current range, selected) and `mp_rr_canyonlands_staging` (the original range,
dimmed and locked as "Coming later" until it's ported). Tile art is each map's loading screen from the player's files
(`<assets>\images\ui\maps_<bsp>_loadscreen\*.png`, loaded small and off the UI thread when the panel opens); a missing
picture gives a plain tile. The original range's picture was put there by hand
(`D:\R5Rust\assets-retail\images\ui\maps_mp_rr_canyonlands_staging_loadscreen\firing_range_s0.png`); it isn't in this
repo. Picking a different playable map isn't wired up yet (there's only one).

**FPS cap and resolution take presets or any value**, in Settings > Graphics (segments + a CUSTOM segment) and in
the Play chips (menu items + "Custom..."). Custom reveals a small number field: FPS 20 to 1000 (0 / "unlimited" = no
cap), resolution width 320 to 7680 by height 240 to 4320 (the game's own ranges). Enter or clicking away applies;
invalid text turns the field red and isn't saved. A value equal to a preset lights that preset, any other lights
"CUSTOM 180". Saved as `fps_cap` / `stretch = [w, h]` in the game's `[graphics]` table, as before.

Under the map card on Play, **quick chips**: Resolution (stretched res presets + custom), Display (fullscreen /
windowed, `r5rust.toml`), FPS limit (unlimited, 60..360 + custom), Launch (fly mode, show FPS, developer console, open
the menu: edits the launch options). Fonts are OFL (`assets\fonts\OFL-*.txt`).

Ideas taken from the reference launchers: R5Regenis (layout, map art from the install, settings card, first-run card),
R5Flowstate (Console tab, Play becomes Stop, launch-args preview, plain-sentence status line), R5Reloaded (verify game
files).

## Releasing

- **Launcher:** bump `version` in Cargo.toml, then `pwsh tools\release.ps1 -Publish -Notes 'line','line'`. By hand:
  (`$env:CARGO_TARGET_DIR='D:\R5Rust\_build\launcher'; $env:RUSTFLAGS="--remap-path-prefix=$env:USERPROFILE=~ --remap-path-prefix=C:\Code=src"; cargo build --release`),
  then `gh release create v<version> <exe>#R5Rust-Launcher.exe -R R5Rust/launcher` with the asset **named
  `R5Rust-Launcher.exe`** (the download link and the self-updater look for that name / any `.exe`). Installed
  launchers offer the update on their next start.
- **Game build:** `pwsh tools\package.ps1 -Publish` in the game repo (release `build-<date>-<hash>` on R5Rust/R5Rust).
- **Install tools:** `pwsh tools\bundle_tools.ps1` in the game repo makes `r5rust-tools-<n>.zip`; publish it as release
  `tools-<n>` on R5Rust/R5Rust and package game builds with `-Tools <n>`.
- Player mode is used when the game folder isn't a dev checkout. Test it without touching your own setup by pointing
  `APPDATA` / `LOCALAPPDATA` at empty folders and writing `game_path.txt` there; `--portable` skips the self-install,
  `--check-updates` lets a `--screenshot` run reach GitHub, `--download` presses Download once possible.
## Code

- `src\main.rs`: the UI (egui 0.28 / eframe).
- `src\game.rs`: everything about the game folder: staged builds (`play\current|next\version.txt`), git history,
  process detection (Toolhelp), the background poller (every 3 s, never on the UI thread), the play/build jobs (output
  drained on threads, written to the log), game settings via `toml_edit`, game-files check.
- `cargo test --release -- --test-threads=1`: 16 tests (jobs run real `pwsh` scripts in temp folders; they never write
  to your `%APPDATA%`).

## Status / unfinished

- v0.3.0: redesign done and checked on every page with `--screenshot` (Play idle + running, Patch Notes collapsed +
  expanded, Settings, Console, install form + install progress, an open chip menu). Not yet clicked through by hand on
  the new look: hover states, the chip menus, READ MORE / row expand, and resizing are worth a quick manual pass.
- Patch-note "versions" are dates (`UPDATE 26.11.21`): R5Rust has no version numbers, so a day of commits is an update.
  Older updates have a date tile instead of a picture (the launcher only has the one map's loading screen).
- Only Firing Range. More modes, a map picker and mods would follow the game.
- v0.4.0: player mode (downloads, updates, self-install, self-update, uninstall). The exe isn't code-signed, so
  SmartScreen warns on first run. The in-game HUD (RUI host) needs Node + Flowstate's ui.dll, which players don't have:
  the game runs without it.
- Install builds the map from Apex; the first-person items (R-99, arms, heirlooms, sounds, decals, effects) aren't in
  it yet. It still needs the dev tools on this PC (RSX fork, extractor, dxc, Python, cargo): bundling them so a
  friend's PC needs nothing else is the next step (the game's `tools\bin\` is already checked first).
- The Console is read-only; R5Rust has no console commands yet.

//! Everything the launcher knows about the game folder: its staged builds, its git history, whether the game is
//! running, and the long-running jobs it can start (play.ps1, tools\publish.ps1 and tools\install.ps1).

use std::{
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const DEFAULT_GAME_DIR: &str = r"C:\Code\games\r5rust";
const DEFAULT_ASSETS: &str = r"D:\R5Rust\assets";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL_EVERY: Duration = Duration::from_secs(3);
pub const KEEP_LINES: usize = 5000;
/// How many commits Patch Notes reads (the oldest day is left out when the limit cuts it).
pub const HISTORY_LEN: usize = 100;

/// A command with no console window: the launcher is a GUI app, so every child would otherwise flash one.
fn quiet(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// PowerShell 7 for the game's scripts: the portable copy the install tools bring (`tools\bin\pwsh`), else `pwsh`.
pub fn pwsh(root: &Path) -> PathBuf {
    let bundled = root.join("tools").join("bin").join("pwsh").join("pwsh.exe");
    if bundled.is_file() { bundled } else { PathBuf::from("pwsh") }
}

/// A dev checkout (git + tools\publish.ps1: builds come from cargo) rather than a player's downloaded game folder.
pub fn is_dev_root(root: &Path) -> bool {
    root.join(".git").exists() && root.join("tools").join("publish.ps1").is_file()
}

// ---------------------------------------------------------------------------------------------------------------
// Builds and history

/// One `play\<slot>\version.txt`, written by tools\publish.ps1 as `yyyy-MM-dd HH:mm  <hash> <subject>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub date: String,
    pub hash: String,
    pub subject: String,
}

impl Build {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('\u{feff}');
        let (date, rest) = text.split_once("  ")?;
        let rest = rest.trim_start();
        let (hash, subject) = rest.split_once(' ').unwrap_or((rest, ""));
        if hash.is_empty() {
            return None;
        }
        Some(Self { date: date.trim().to_owned(), hash: hash.to_owned(), subject: subject.trim().to_owned() })
    }

    fn read(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        // Windows PowerShell 5.1's Set-Content writes UTF-16; pwsh 7 writes UTF-8.
        let text = if bytes.starts_with(&[0xFF, 0xFE]) {
            let wide: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&wide)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        Self::parse(&text)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub day: String,
    pub time: String,
    pub ago: String,
    pub subject: String,
    /// The commit message's body as notes: one entry per bullet / paragraph line (trailers like Co-Authored-By dropped).
    pub body: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameState {
    Ready,
    #[default]
    FolderMissing,
    InvalidFolder,
    NoBuild,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub state: GameState,
    /// The build `play.ps1` runs now.
    pub current: Option<Build>,
    /// A build staged by publish.ps1; play.ps1 swaps it in on the next launch.
    pub next: Option<Build>,
    /// Commits newer than the newest staged build (None: no git, or the build's commit isn't in the history).
    pub unbuilt: Option<u32>,
    /// Shared so the UI's per-frame snapshot copy doesn't deep-copy 60 commit messages.
    pub commits: Arc<Vec<Commit>>,
    /// An r5rust.exe is running (ours or not).
    pub running: bool,
    /// The extracted game files R5Rust loads (never shipped; made from the player's own Apex install).
    pub files: GameFiles,
    /// A dev checkout (cargo builds, git history) rather than a downloaded game.
    pub dev: bool,
    /// The GitHub release a downloaded game folder was unpacked from (release.txt).
    pub release: Option<String>,
}

/// The map the game loads and whether its extracted files are on disk.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GameFiles {
    pub assets_root: PathBuf,
    pub map: String,
    /// Required files that are missing (empty = good to go).
    pub missing: Vec<String>,
    /// The map's loading-screen picture, if it was extracted.
    pub loadscreen: Option<PathBuf>,
}

impl GameFiles {
    const REQUIRED: [&'static str; 4] = ["world.glb", "collision.bin", "materials.json", "meta.json"];

    pub fn read(root: &Path) -> Self {
        let assets_root = assets_root(root);
        let map = map_bsp(root);
        let map_dir = assets_root.join("maps").join(&map);
        let missing = if map_dir.is_dir() {
            Self::REQUIRED.iter().filter(|f| !map_dir.join(f).is_file()).map(|f| f.to_string()).collect()
        } else {
            vec![format!("maps\\{map}")]
        };
        let loadscreen = first_png(&assets_root.join("images").join("ui").join(format!("maps_{map}_loadscreen")));
        Self { assets_root, map, missing, loadscreen }
    }

    pub fn ok(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Same order as the game's `Config::assets_root`: env R5RUST_ASSETS > r5rust.toml `assets_root` > D:\R5Rust\assets
/// (a player's folder: `%LOCALAPPDATA%\R5Rust\assets`, where tools\install.ps1 puts them by default).
fn assets_root(root: &Path) -> PathBuf {
    if let Some(v) = env::var_os("R5RUST_ASSETS").filter(|v| !v.is_empty()) {
        return PathBuf::from(v);
    }
    for cfg in [root.join("r5rust.toml"), root.join("play").join("current").join("r5rust.toml")] {
        if let Some(v) = fs::read_to_string(cfg)
            .ok()
            .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
            .and_then(|d| d.get("assets_root").and_then(|v| v.as_str()).map(str::to_owned))
        {
            return PathBuf::from(v);
        }
    }
    if is_dev_root(root) { PathBuf::from(DEFAULT_ASSETS) } else { crate::update::data_root().join("assets") }
}

/// The map the game loads: r5rust.toml `map`, else the first `bsp` in content\maps, else the Firing Range.
fn map_bsp(root: &Path) -> String {
    let from_toml = |path: PathBuf, key: &str| {
        fs::read_to_string(path)
            .ok()
            .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
            .and_then(|d| d.get(key).and_then(|v| v.as_str()).map(str::to_owned))
    };
    if let Some(m) = from_toml(root.join("r5rust.toml"), "map") {
        return m;
    }
    let mut maps: Vec<PathBuf> = fs::read_dir(root.join("content").join("maps"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    maps.sort();
    maps.into_iter()
        .find_map(|p| from_toml(p, "bsp"))
        .unwrap_or_else(|| "mp_rr_canyonlands_staging_mu1".to_owned())
}

/// A map's loading-screen picture in the player's files: `<assets>\images\ui\maps_<bsp>_loadscreen\<first>.png`.
pub fn loadscreen_for(assets_root: &Path, bsp: &str) -> Option<PathBuf> {
    first_png(&assets_root.join("images").join("ui").join(format!("maps_{bsp}_loadscreen")))
}

fn first_png(dir: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .collect();
    v.sort();
    v.into_iter().next()
}

// ---------------------------------------------------------------------------------------------------------------
// The game's own settings file (r5rust_settings.toml, next to play.ps1). Edited in place with toml_edit so the
// binds and anything the launcher doesn't know about are kept exactly as the game wrote them.

#[derive(Clone, Debug, PartialEq)]
pub struct GameSettings {
    pub sensitivity: f64,
    pub fov: f64,
    pub invert_y: bool,
    pub auto_sprint: bool,
    pub mantle_boost: bool,
    pub graphics: GraphicsSettings,
}

impl Default for GameSettings {
    /// The game's own defaults (crates/client/src/binds.rs UserSettings).
    fn default() -> Self {
        Self { sensitivity: 1.6, fov: 100.0, invert_y: false, auto_sprint: true, mantle_boost: true, graphics: GraphicsSettings::default() }
    }
}

impl GameSettings {
    pub fn path(root: &Path) -> PathBuf {
        root.join("r5rust_settings.toml")
    }

    pub fn load(root: &Path) -> Self {
        let d = Self::default();
        let Some(doc) = fs::read_to_string(Self::path(root)).ok().and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        else {
            return d;
        };
        let num = |k: &str, def: f64| {
            doc.get(k).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).unwrap_or(def)
        };
        let flag = |k: &str, def: bool| doc.get(k).and_then(|v| v.as_bool()).unwrap_or(def);
        Self {
            sensitivity: num("sensitivity", d.sensitivity),
            fov: num("fov", d.fov),
            invert_y: flag("invert_y", d.invert_y),
            auto_sprint: flag("auto_sprint", d.auto_sprint),
            mantle_boost: flag("mantle_boost", d.mantle_boost),
            graphics: GraphicsSettings::load(&doc),
        }
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = Self::path(root);
        let mut doc = fs::read_to_string(&path)
            .ok()
            .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
            .unwrap_or_default();
        let round = |v: f64, step: f64| (v / step).round() * step;
        // Top-level keys must come before the [[binds]] tables; toml_edit keeps existing positions.
        doc["sensitivity"] = toml_edit::value(round(self.sensitivity, 0.01));
        doc["fov"] = toml_edit::value(round(self.fov, 1.0));
        doc["invert_y"] = toml_edit::value(self.invert_y);
        doc["mantle_boost"] = toml_edit::value(self.mantle_boost);
        doc["auto_sprint"] = toml_edit::value(self.auto_sprint);
        self.graphics.save(&mut doc);
        fs::write(&path, doc.to_string()).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
    }
}

/// The game's `[graphics]` table (r5rust crates/client/src/graphics.rs). Presets fill every knob; touching a knob
/// makes the preset "custom". Strings are the game's serde names.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicsSettings {
    pub preset: String,
    pub texture_quality: String,
    /// Apex's Model Detail (static-prop LOD distance): low / medium / high
    pub model_detail: String,
    pub anisotropic: i64,
    pub sun_shadows: String,
    pub msaa: i64,
    pub render_scale: i64,
    pub vsync: bool,
    pub fps_cap: i64,
    /// Stretched resolution like Apex's (the game renders at this size, stretched to the window); [0, 0] = native.
    pub stretch: [i64; 2],
}

pub const PRESETS: [&str; 5] = ["lowest", "low", "medium", "high", "ultra"];
pub const TEXTURE_LEVELS: [&str; 5] = ["lowest", "low", "medium", "high", "ultra"];
pub const SHADOW_LEVELS: [&str; 3] = ["off", "low", "high"];
pub const MODEL_LEVELS: [&str; 3] = ["low", "medium", "high"];

impl Default for GraphicsSettings {
    fn default() -> Self {
        let mut g = Self { preset: String::new(), texture_quality: String::new(), model_detail: String::new(), anisotropic: 8, sun_shadows: String::new(), msaa: 4, render_scale: 100, vsync: false, fps_cap: 0, stretch: [0, 0] };
        g.apply_preset("high");
        g
    }
}

impl GraphicsSettings {
    /// Same table as the game's `GraphicsSettings::preset` (vsync, fps cap and render scale are kept).
    pub fn apply_preset(&mut self, p: &str) {
        let (tex, model, aniso, shadows, msaa) = match p {
            "lowest" => ("lowest", "low", 1, "off", 1),
            "low" => ("low", "low", 2, "off", 1),
            "medium" => ("medium", "medium", 4, "low", 1),
            "high" => ("high", "high", 8, "high", 4),
            _ => ("ultra", "high", 16, "high", 4),
        };
        self.preset = p.to_owned();
        self.texture_quality = tex.to_owned();
        self.model_detail = model.to_owned();
        self.anisotropic = aniso;
        self.sun_shadows = shadows.to_owned();
        self.msaa = msaa;
    }

    /// The preset the knobs match, else "custom".
    pub fn detect_preset(&self) -> String {
        PRESETS
            .iter()
            .find(|p| {
                let mut g = self.clone();
                g.apply_preset(p);
                g == Self { preset: p.to_string(), ..self.clone() }
            })
            .map(|p| p.to_string())
            .unwrap_or_else(|| "custom".to_owned())
    }

    fn load(doc: &toml_edit::DocumentMut) -> Self {
        let d = Self::default();
        let Some(t) = doc.get("graphics").and_then(|t| t.as_table_like()) else { return d };
        let s = |k: &str, def: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or(def).to_owned();
        let i = |k: &str, def: i64| t.get(k).and_then(|v| v.as_integer()).unwrap_or(def);
        Self {
            preset: s("preset", &d.preset),
            texture_quality: s("texture_quality", &d.texture_quality),
            model_detail: s("model_detail", &d.model_detail),
            anisotropic: i("anisotropic", d.anisotropic),
            sun_shadows: s("sun_shadows", &d.sun_shadows),
            msaa: i("msaa", d.msaa),
            render_scale: i("render_scale", d.render_scale).clamp(50, 100),
            vsync: t.get("vsync").and_then(|v| v.as_bool()).unwrap_or(d.vsync),
            fps_cap: i("fps_cap", d.fps_cap),
            stretch: t
                .get("stretch")
                .and_then(|v| v.as_array())
                .and_then(|a| Some([a.get(0)?.as_integer()?, a.get(1)?.as_integer()?]))
                .unwrap_or(d.stretch),
        }
    }

    fn save(&self, doc: &mut toml_edit::DocumentMut) {
        if !doc.contains_table("graphics") {
            doc["graphics"] = toml_edit::table();
        }
        let t = &mut doc["graphics"];
        t["preset"] = toml_edit::value(self.preset.as_str());
        t["texture_quality"] = toml_edit::value(self.texture_quality.as_str());
        t["model_detail"] = toml_edit::value(self.model_detail.as_str());
        t["anisotropic"] = toml_edit::value(self.anisotropic);
        t["sun_shadows"] = toml_edit::value(self.sun_shadows.as_str());
        t["msaa"] = toml_edit::value(self.msaa);
        t["render_scale"] = toml_edit::value(self.render_scale);
        t["vsync"] = toml_edit::value(self.vsync);
        t["fps_cap"] = toml_edit::value(self.fps_cap);
        let mut stretch = toml_edit::Array::new();
        stretch.push(self.stretch[0]);
        stretch.push(self.stretch[1]);
        t["stretch"] = toml_edit::value(stretch);
    }
}

impl Snapshot {
    pub fn read(root: &Path) -> Self {
        let mut snap = Self { state: inspect_game(root), running: process_running("r5rust.exe"), ..Self::default() };
        if matches!(snap.state, GameState::FolderMissing | GameState::InvalidFolder) {
            return snap;
        }
        snap.files = GameFiles::read(root);
        snap.dev = is_dev_root(root);
        snap.release = fs::read_to_string(root.join("release.txt")).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        snap.current = Build::read(&root.join("play/current/version.txt"));
        snap.next = Build::read(&root.join("play/next/version.txt"))
            .filter(|_| root.join("play/next/r5rust.exe").is_file());
        if !snap.dev {
            // a downloaded build carries its own patch notes
            snap.commits = Arc::new(read_notes(&root.join("notes.json")));
        } else if root.join(".git").exists() {
            snap.commits = Arc::new(git_log(root, HISTORY_LEN));
            if let Some(newest) = snap.newest() {
                snap.unbuilt = git(root, &["rev-list", "--count", &format!("{}..HEAD", newest.hash)])
                    .and_then(|n| n.trim().parse().ok());
            } else if !snap.commits.is_empty() {
                snap.unbuilt = git(root, &["rev-list", "--count", "HEAD"]).and_then(|n| n.trim().parse().ok());
            }
        }
        snap
    }

    /// The newest build on disk: the staged one if there is one, else the installed one.
    pub fn newest(&self) -> Option<&Build> {
        self.next.as_ref().or(self.current.as_ref())
    }

    pub fn is_ready(&self) -> bool {
        self.state == GameState::Ready
    }
}

pub fn inspect_game(root: &Path) -> GameState {
    if !root.is_dir() {
        return GameState::FolderMissing;
    }
    if !root.join("play.ps1").is_file() {
        return GameState::InvalidFolder;
    }
    if root.join("play/current/r5rust.exe").is_file() || root.join("play/next/r5rust.exe").is_file() {
        GameState::Ready
    } else {
        GameState::NoBuild
    }
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = quiet("git").arg("-C").arg(root).args(args).stdin(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_log(root: &Path, count: usize) -> Vec<Commit> {
    // fields split by 0x1f, commits by 0x1e (bodies have newlines)
    let format = "--format=%h%x1f%ad%x1f%ar%x1f%s%x1f%b%x1e";
    let Some(text) =
        git(root, &["log", &format!("-n{count}"), format, "--date=format:%Y-%m-%d %H:%M"])
    else {
        return Vec::new();
    };
    text.split('\u{1e}')
        .filter_map(|record| {
            let mut parts = record.trim_start_matches(['\r', '\n']).split('\u{1f}');
            let hash = parts.next()?.trim().to_owned();
            let (day, time) = parts.next()?.split_once(' ')?;
            Some(Commit {
                hash,
                day: day.to_owned(),
                time: time.to_owned(),
                ago: parts.next()?.to_owned(),
                subject: parts.next().unwrap_or("").to_owned(),
                body: parse_body(parts.next().unwrap_or("")),
            })
        })
        .collect()
}

/// A downloaded build's patch notes (written by the game's tools\package.ps1 from its git log):
/// `[{"hash", "date": "yyyy-MM-dd HH:mm", "subject", "body"}]`, newest first.
fn read_notes(path: &Path) -> Vec<Commit> {
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let today = days_since_epoch_now();
    list.iter()
        .filter_map(|n| {
            let s = |k: &str| n.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
            let date = s("date");
            let (day, time) = date.split_once(' ').unwrap_or((date.as_str(), ""));
            let ago = match (today, days_from_ymd(day)) {
                (Some(t), Some(d)) if t >= d => match t - d {
                    0 => "today".to_owned(),
                    1 => "yesterday".to_owned(),
                    n => format!("{n} days ago"),
                },
                _ => String::new(),
            };
            Some(Commit { hash: s("hash"), day: day.to_owned(), time: time.to_owned(), ago, subject: s("subject"), body: parse_body(&s("body")) })
        })
        .filter(|c| !c.hash.is_empty())
        .collect()
}

fn days_since_epoch_now() -> Option<i64> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(secs.div_euclid(86_400))
}

/// Days since 1970-01-01 for `yyyy-mm-dd` (Howard Hinnant's days_from_civil).
fn days_from_ymd(day: &str) -> Option<i64> {
    let mut it = day.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// A commit body as notes: `- ` bullets with their wrapped continuation lines joined, other lines as their own note.
pub fn parse_body(body: &str) -> Vec<String> {
    let mut notes: Vec<String> = Vec::new();
    let mut continuing = false;
    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if trimmed.is_empty() || lower.starts_with("co-authored-by:") || lower.starts_with("signed-off-by:") {
            continuing = false;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            notes.push(rest.to_owned());
            continuing = true;
        } else if continuing && line.starts_with(char::is_whitespace) {
            let last = notes.last_mut().expect("continuing implies a note");
            last.push(' ');
            last.push_str(trimmed);
        } else {
            notes.push(trimmed.to_owned());
            continuing = false;
        }
    }
    notes
}

#[cfg(windows)]
pub fn process_running(name: &str) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
        },
    };
    // SAFETY: plain Toolhelp enumeration; the entry is zeroed and sized as the API requires, the handle is closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(name) {
                found = true;
                break;
            }
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        found
    }
}

#[cfg(not(windows))]
pub fn process_running(_name: &str) -> bool {
    false
}

// ---------------------------------------------------------------------------------------------------------------
// Background poller: re-reads the folder every few seconds (or when woken) so the UI never touches the disk.

pub struct Poller {
    pub snapshot: Arc<Mutex<Snapshot>>,
    root: Arc<Mutex<PathBuf>>,
    wake: Sender<()>,
}

impl Poller {
    pub fn start(root: PathBuf, ctx: eframe::egui::Context) -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot::read(&root)));
        let root = Arc::new(Mutex::new(root));
        let (wake, rx) = mpsc::channel::<()>();
        let (snap_t, root_t) = (snapshot.clone(), root.clone());
        thread::spawn(move || {
            loop {
                match rx.recv_timeout(POLL_EVERY) {
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    _ => while rx.try_recv().is_ok() {},
                }
                let path = root_t.lock().unwrap().clone();
                let fresh = Snapshot::read(&path);
                let mut shared = snap_t.lock().unwrap();
                if *shared != fresh {
                    *shared = fresh;
                    ctx.request_repaint();
                }
            }
        });
        Self { snapshot, root, wake }
    }

    pub fn get(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }

    pub fn set_root(&self, root: PathBuf) {
        *self.root.lock().unwrap() = root;
        self.refresh();
    }

    pub fn refresh(&self) {
        let _ = self.wake.send(());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Jobs: play.ps1 (lives as long as the game) and tools\publish.ps1 (a cargo build). Output is drained on threads,
// kept in memory for the UI and written to a log file.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Play,
    Build,
    /// tools\install.ps1: builds the game files from the player's own Apex install.
    Install,
    /// tools\push.ps1: releases the build you're playing to players (dev).
    Push,
}

pub struct Job {
    pub started: Instant,
    pub lines: Arc<Mutex<Vec<String>>>,
    /// The `@@` progress lines (install.ps1's machine-readable output), kept whole: `lines` drops its oldest.
    pub marks: Arc<Mutex<Vec<String>>>,
    pub log_path: PathBuf,
    child: Child,
    pub exit: Option<Option<i32>>,
    pub ended: Option<Instant>,
}

impl Job {
    pub fn start(
        kind: JobKind,
        root: &Path,
        extra_args: &[String],
        log_path: PathBuf,
        ctx: eframe::egui::Context,
    ) -> Result<Self, String> {
        let script = match kind {
            JobKind::Play => root.join("play.ps1"),
            JobKind::Build => root.join("tools").join("publish.ps1"),
            JobKind::Install => root.join("tools").join("install.ps1"),
            JobKind::Push => root.join("tools").join("push.ps1"),
        };
        if !script.is_file() {
            return Err(format!("{} is missing", script.display()));
        }
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let log = Arc::new(Mutex::new(fs::File::create(&log_path).ok()));

        let mut command = quiet(pwsh(root));
        command
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .args(extra_args)
            .current_dir(root)
            .env("CARGO_TERM_COLOR", "never")
            .env_remove("NO_COLOR")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| format!("Couldn't start PowerShell 7 (pwsh): {e}"))?;

        let lines = Arc::new(Mutex::new(Vec::new()));
        let marks = Arc::new(Mutex::new(Vec::new()));
        let stdout = child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>);
        let stderr = child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>);
        for stream in [stdout, stderr].into_iter().flatten() {
            let (lines, marks, log, ctx) = (lines.clone(), marks.clone(), log.clone(), ctx.clone());
            thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Some(file) = log.lock().unwrap().as_mut() {
                        let _ = writeln!(file, "{line}");
                    }
                    if line.starts_with("@@") {
                        marks.lock().unwrap().push(line.clone());
                    }
                    let mut lines = lines.lock().unwrap();
                    lines.push(line);
                    if lines.len() > KEEP_LINES {
                        let cut = lines.len() - KEEP_LINES;
                        lines.drain(..cut);
                    }
                    drop(lines);
                    if kind != JobKind::Play {
                        ctx.request_repaint();
                    }
                }
            });
        }
        Ok(Self { started: Instant::now(), lines, marks, log_path, child, exit: None, ended: None })
    }

    /// Checks whether the job finished; returns true the first time it sees the exit.
    pub fn poll(&mut self) -> bool {
        if self.exit.is_some() {
            return false;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.exit = Some(status.code());
                self.ended = Some(Instant::now());
                true
            }
            Ok(None) => false,
            Err(_) => {
                self.exit = Some(None);
                self.ended = Some(Instant::now());
                true
            }
        }
    }

    /// Ends the job and everything it started (play.ps1 waits on r5rust.exe, so the tree takes the game with it).
    pub fn stop(&mut self) {
        let pid = self.child.id().to_string();
        let killed = quiet("taskkill").args(["/PID", &pid, "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        if killed.map(|s| !s.success()).unwrap_or(true) {
            let _ = self.child.kill();
        }
    }

    pub fn running(&self) -> bool {
        self.exit.is_none()
    }

    pub fn succeeded(&self) -> bool {
        matches!(self.exit, Some(Some(0)))
    }

    pub fn elapsed(&self) -> Duration {
        self.ended.unwrap_or_else(Instant::now) - self.started
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        let lines = self.lines.lock().unwrap();
        let mut out: Vec<String> = lines.iter().rev().filter(|l| !l.trim().is_empty()).take(n).cloned().collect();
        out.reverse();
        out
    }

    /// The most useful line to show when the job failed: the last one that looks like an error.
    pub fn failure_line(&self) -> Option<String> {
        let lines = self.lines.lock().unwrap();
        let interesting = |l: &&String| {
            let l = l.to_ascii_lowercase();
            l.contains("error") || l.contains("failed") || l.contains("no build") || l.contains("panicked")
        };
        lines
            .iter()
            .rev()
            .find(interesting)
            .or_else(|| lines.iter().rev().find(|l| !l.trim().is_empty()))
            .map(|l| l.trim().to_owned())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// r5rust.toml: the game's per-machine config (gitignored, next to play.ps1). The launcher edits `fullscreen` in place.

pub fn read_fullscreen(root: &Path) -> bool {
    fs::read_to_string(root.join("r5rust.toml"))
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|d| d.get("fullscreen").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

pub fn write_fullscreen(root: &Path, on: bool) -> Result<(), String> {
    let path = root.join("r5rust.toml");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("r5rust.toml: {e}"))?;
    doc["fullscreen"] = toml_edit::value(on);
    fs::write(&path, doc.to_string()).map_err(|e| format!("Couldn't save r5rust.toml: {e}"))
}

/// Turns a launch option on or off in the launch-args text: `flag` is the option and its values, e.g.
/// `["+cl_showfps", "1"]`. Removing takes the option and as many following tokens as it has values.
pub fn set_launch_flag(args: &str, flag: &[&str], on: bool) -> String {
    let mut tokens = split_args(args);
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].eq_ignore_ascii_case(flag[0]) {
            let end = (i + flag.len()).min(tokens.len());
            tokens.drain(i..end);
        } else {
            i += 1;
        }
    }
    if on {
        tokens.extend(flag.iter().map(|s| s.to_string()));
    }
    tokens.iter().map(|t| if t.contains(' ') { format!("\"{t}\"") } else { t.clone() }).collect::<Vec<_>>().join(" ")
}

pub fn has_launch_flag(args: &str, flag: &str) -> bool {
    split_args(args).iter().any(|t| t.eq_ignore_ascii_case(flag))
}

// ---------------------------------------------------------------------------------------------------------------
// Install: tools\install.ps1 builds the game files from the player's own Apex install. It prints `@@` lines (see its
// header) that the launcher turns into a step list and a progress bar.

#[derive(Clone, Debug, PartialEq)]
pub enum StepState {
    Pending,
    Running,
    Done(u32),
    Skipped(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstallStep {
    pub id: String,
    pub title: String,
    pub weight: f32,
    pub state: StepState,
}

/// What an install job has said so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstallView {
    pub steps: Vec<InstallStep>,
    pub apex: Option<String>,
    pub missing_tools: Vec<String>,
    /// `@@done`: (succeeded, message)
    pub done: Option<(bool, String)>,
}

impl InstallView {
    pub fn parse(marks: &[String]) -> Self {
        let mut v = Self::default();
        for m in marks {
            let (tag, rest) = m.split_once(' ').unwrap_or((m.as_str(), ""));
            let (id, why) = rest.split_once(' ').unwrap_or((rest, ""));
            match tag {
                "@@apex" => v.apex = Some(rest.to_owned()),
                "@@tool" => {
                    let mut parts = rest.splitn(3, ' ');
                    let (name, ok, detail) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                    if ok == "missing" {
                        v.missing_tools.push(format!("{}: {detail}", name.replace('_', " ")));
                    }
                }
                "@@plan" => {
                    v.steps = rest
                        .split('|')
                        .filter(|p| !p.is_empty())
                        .filter_map(|p| {
                            let mut f = p.splitn(3, ':');
                            let (id, w, title) = (f.next()?, f.next()?, f.next()?);
                            Some(InstallStep { id: id.into(), title: title.into(), weight: w.parse().unwrap_or(1.0), state: StepState::Pending })
                        })
                        .collect();
                }
                "@@step" => {
                    if let Some(s) = Self::step(&mut v, id) {
                        *s = StepState::Running;
                    }
                }
                "@@ok" => {
                    if let Some(s) = Self::step(&mut v, id) {
                        *s = StepState::Done(why.trim().parse().unwrap_or(0));
                    }
                }
                "@@skip" => {
                    if let Some(s) = Self::step(&mut v, id) {
                        *s = StepState::Skipped(why.to_owned());
                    }
                }
                "@@fail" => {
                    if let Some(s) = Self::step(&mut v, id) {
                        *s = StepState::Failed(why.to_owned());
                    }
                }
                "@@done" => v.done = Some((id == "ok", why.to_owned())),
                _ => {}
            }
        }
        v
    }

    fn step<'a>(v: &'a mut Self, id: &str) -> Option<&'a mut StepState> {
        v.steps.iter_mut().find(|s| s.id == id).map(|s| &mut s.state)
    }

    /// Weighted share of the plan that's finished (a running step counts half).
    pub fn fraction(&self) -> f32 {
        let total: f32 = self.steps.iter().map(|s| s.weight).sum();
        if total <= 0.0 {
            return if self.done.is_some() { 1.0 } else { 0.0 };
        }
        let done: f32 = self
            .steps
            .iter()
            .map(|s| match s.state {
                StepState::Pending => 0.0,
                StepState::Running => s.weight * 0.5,
                _ => s.weight,
            })
            .sum();
        done / total
    }

    pub fn current(&self) -> Option<(usize, &InstallStep)> {
        self.steps.iter().enumerate().find(|(_, s)| s.state == StepState::Running)
    }
}

/// Whether the game files are installed and current (`install.ps1 -Check`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallStatus {
    Checking,
    Installed(String),
    Partial(String),
    Stale(String),
    Missing(String),
    Unknown(String),
}

fn run_install_script(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let script = root.join("tools").join("install.ps1");
    if !script.is_file() {
        return Err(format!("{} is missing (update the game)", script.display()));
    }
    let out = quiet(pwsh(root))
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't start PowerShell 7 (pwsh): {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect())
}

/// Reads install.json against the Apex files it came from (file sizes and dates only; never the running game).
pub fn check_install(root: &Path, assets: &Path) -> InstallStatus {
    let lines = match run_install_script(root, &["-Check", "-Assets", &assets.to_string_lossy()]) {
        Ok(l) => l,
        Err(e) => return InstallStatus::Unknown(e),
    };
    let Some(line) = lines.iter().find_map(|l| l.strip_prefix("@@status ")) else {
        return InstallStatus::Unknown("the installer said nothing".into());
    };
    let (kind, detail) = line.split_once(' ').unwrap_or((line, ""));
    let detail = detail.to_owned();
    match kind {
        "installed" => InstallStatus::Installed(detail),
        "partial" => InstallStatus::Partial(detail),
        "stale" => InstallStatus::Stale(detail),
        "missing" => InstallStatus::Missing(detail),
        _ => InstallStatus::Unknown(line.to_owned()),
    }
}

/// Apex installs that have the map (EA app registry, the default folder, Steam libraries), newest first.
pub fn detect_apex(root: &Path) -> Vec<String> {
    run_install_script(root, &["-Detect"])
        .map(|l| l.iter().filter_map(|l| l.strip_prefix("@@apex ")).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The arguments for an install job.
pub fn install_args(apex: &str, assets: &str) -> Vec<String> {
    vec!["-ApexIsClosed".into(), "-Apex".into(), apex.into(), "-Assets".into(), assets.into()]
}

// ---------------------------------------------------------------------------------------------------------------
// Settings, kept as small text files in %APPDATA%\R5Rust Launcher (game_path.txt predates this module).

/// Where a job's output is saved: %APPDATA%\R5Rust Launcher\<name>.
pub fn log_path(name: &str) -> PathBuf {
    data_dir().map(|d| d.join(name)).unwrap_or_else(|| env::temp_dir().join(format!("r5rust-launcher-{name}")))
}

pub fn data_dir() -> Option<PathBuf> {
    env::var_os("APPDATA").map(|d| PathBuf::from(d).join("R5Rust Launcher"))
}

pub fn read_setting(name: &str) -> Option<String> {
    let text = fs::read_to_string(data_dir()?.join(name)).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

pub fn write_setting(name: &str, value: &str) -> std::io::Result<()> {
    let Some(dir) = data_dir() else { return Ok(()) };
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(name), value)
}

pub fn load_game_root() -> PathBuf {
    if let Some(saved) = read_setting("game_path.txt") {
        return PathBuf::from(saved);
    }
    if let Some(env_root) = env::var_os("R5RUST_DIR") {
        let path = PathBuf::from(env_root);
        if path.join("play.ps1").is_file() {
            return path;
        }
    }
    if let Ok(exe) = env::current_exe() {
        for ancestor in exe.ancestors() {
            let candidate = ancestor.join("games").join("r5rust");
            if candidate.join("play.ps1").is_file() {
                return candidate;
            }
        }
    }
    // Kainz's dev checkout when it's there; everyone else plays from %LOCALAPPDATA%\R5Rust\game
    if Path::new(DEFAULT_GAME_DIR).join("play.ps1").is_file() {
        return PathBuf::from(DEFAULT_GAME_DIR);
    }
    crate::update::player_game_root()
}

/// The last lines of a log this launcher wrote earlier (game.log / build.log), so the Console isn't empty after a restart.
pub fn read_last_log(name: &str) -> Option<Vec<String>> {
    let text = fs::read(data_dir()?.join(name)).ok()?;
    let text = String::from_utf8_lossy(&text);
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let skip = lines.len().saturating_sub(KEEP_LINES);
    (!lines.is_empty()).then(|| lines[skip..].to_vec())
}

/// A yes/no launcher setting stored as "1" / "0".
pub fn read_flag(name: &str) -> bool {
    read_setting(name).is_some_and(|v| v == "1")
}

pub fn load_launch_args() -> String {
    read_setting("launch_args.txt").unwrap_or_default()
}

/// Splits launch options like a shell would: spaces separate, double quotes group.
pub fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// Opens a folder or file with Explorer's default handler.
pub fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} doesn't exist yet", path.display()));
    }
    Command::new("explorer.exe").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        env::temp_dir().join(format!("r5rust-launcher-{label}-{stamp}"))
    }

    #[test]
    fn commit_body_becomes_notes() {
        let body = "- Cables: 8-sided tube along the rest points,\n  probe lit, shadow casting.\n- Sounds: attach / loop.\nPlain line.\n\nCo-Authored-By: Someone <x@y>\n";
        assert_eq!(
            parse_body(body),
            vec!["Cables: 8-sided tube along the rest points, probe lit, shadow casting.", "Sounds: attach / loop.", "Plain line."]
        );
        assert!(parse_body("Co-Authored-By: Someone <x@y>\n").is_empty());
    }

    #[test]
    fn detects_a_staged_build() {
        let root = temp_root("ready");
        fs::create_dir_all(root.join("play/current")).unwrap();
        fs::create_dir_all(root.join("play/next")).unwrap();
        fs::write(root.join("play.ps1"), "").unwrap();
        fs::write(root.join("play/next/r5rust.exe"), "").unwrap();
        fs::write(root.join("play/next/version.txt"), "2026-11-17 20:17  021f8ff Docs: a b c\n").unwrap();
        assert_eq!(inspect_game(&root), GameState::Ready);
        let snap = Snapshot::read(&root);
        assert_eq!(snap.next.as_ref().map(|b| b.hash.as_str()), Some("021f8ff"));
        assert!(snap.current.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn distinguishes_invalid_folder_and_missing_build() {
        let root = temp_root("empty");
        fs::create_dir_all(&root).unwrap();
        assert_eq!(inspect_game(&root), GameState::InvalidFolder);
        fs::write(root.join("play.ps1"), "").unwrap();
        assert_eq!(inspect_game(&root), GameState::NoBuild);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_version_lines() {
        let b = Build::parse("2026-11-17 16:18  1267102 Content registry: legends, abilities\r\n").unwrap();
        assert_eq!((b.date.as_str(), b.hash.as_str()), ("2026-11-17 16:18", "1267102"));
        assert_eq!(b.subject, "Content registry: legends, abilities");
        assert!(Build::parse("garbage").is_none());
    }

    #[test]
    fn splits_launch_args() {
        assert_eq!(split_args(r#"--fly  --pos "1, 2, 3" --fov 110"#), ["--fly", "--pos", "1, 2, 3", "--fov", "110"]);
        assert!(split_args("   ").is_empty());
    }

    fn wait(job: &mut Job) {
        let t = Instant::now();
        while !job.poll() {
            assert!(t.elapsed() < Duration::from_secs(30), "job never finished");
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(Duration::from_millis(200)); // let the reader threads drain
    }

    #[test]
    fn play_job_passes_args_and_reports_failure() {
        let root = temp_root("job");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("play.ps1"), "Write-Host \"args: $($args -join '|')\"\nWrite-Host 'no build yet: run tools\\publish.ps1'\nexit 1\n").unwrap();
        let mut job = Job::start(JobKind::Play, &root, &split_args("--fly --fov 110"), root.join("game.log"), eframe::egui::Context::default()).unwrap();
        wait(&mut job);
        assert_eq!(job.exit, Some(Some(1)));
        assert!(!job.succeeded());
        assert_eq!(job.tail(5), ["args: --fly|--fov|110", r"no build yet: run tools\publish.ps1"]);
        assert_eq!(job.failure_line().as_deref(), Some(r"no build yet: run tools\publish.ps1"));
        assert!(fs::read_to_string(root.join("game.log")).unwrap().contains("args: --fly"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_ends_the_job_and_its_children() {
        let root = temp_root("stop");
        fs::create_dir_all(&root).unwrap();
        // play.ps1 waits on a child process, like it waits on r5rust.exe.
        fs::write(root.join("play.ps1"), "Write-Host started
& pwsh -NoProfile -Command 'Start-Sleep 60'
").unwrap();
        let mut job = Job::start(JobKind::Play, &root, &[], root.join("game.log"), eframe::egui::Context::default()).unwrap();
        thread::sleep(Duration::from_millis(1500));
        assert!(!job.poll(), "job ended on its own");
        let t = Instant::now();
        job.stop();
        wait(&mut job);
        assert!(t.elapsed() < Duration::from_secs(10));
        assert!(!job.succeeded());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn build_job_streams_stderr_and_succeeds() {
        let root = temp_root("build");
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::write(root.join("tools/publish.ps1"), "[Console]::Error.WriteLine('   Compiling r5rust')\nWrite-Host 'staged: abc'\n").unwrap();
        let mut job = Job::start(JobKind::Build, &root, &[], root.join("build.log"), eframe::egui::Context::default()).unwrap();
        wait(&mut job);
        assert!(job.succeeded());
        let lines = job.tail(5);
        assert!(lines.iter().any(|l| l.contains("Compiling r5rust")), "{lines:?}");
        assert!(lines.iter().any(|l| l == "staged: abc"), "{lines:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn game_settings_round_trip_keeps_binds() {
        let root = temp_root("settings");
        fs::create_dir_all(&root).unwrap();
        let original = "sensitivity = 0.8\nfov = 120.0\ninvert_y = false\nmantle_boost = false\nauto_sprint = true\n\n[[binds]]\naction = \"Forward\"\nslot2 = \"WheelUp\"\n\n[binds.slot1]\nKey = \"KeyW\"\n";
        fs::write(GameSettings::path(&root), original).unwrap();
        let mut s = GameSettings::load(&root);
        assert_eq!((s.sensitivity, s.fov, s.mantle_boost), (0.8, 120.0, false));
        s.fov = 104.4;
        s.invert_y = true;
        s.save(&root).unwrap();
        let text = fs::read_to_string(GameSettings::path(&root)).unwrap();
        assert!(text.contains("fov = 104.0") && text.contains("invert_y = true"), "{text}");
        assert!(text.contains("[[binds]]") && text.contains("Key = \"KeyW\""), "{text}");
        assert!(text.find("fov").unwrap() < text.find("[[binds]]").unwrap(), "{text}");
        assert_eq!(GameSettings::load(&root).fov, 104.0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn graphics_presets_and_save() {
        for p in PRESETS {
            let mut g = GraphicsSettings::default();
            g.apply_preset(p);
            assert_eq!(g.detect_preset(), p);
        }
        let mut g = GraphicsSettings::default();
        g.anisotropic = 2;
        assert_eq!(g.detect_preset(), "custom");
        let root = temp_root("graphics");
        fs::create_dir_all(&root).unwrap();
        fs::write(GameSettings::path(&root), "fov = 110.0\n\n[[binds]]\naction = \"Jump\"\n\n[binds.slot1]\nKey = \"Space\"\n").unwrap();
        let mut s = GameSettings::load(&root);
        assert_eq!(s.graphics.preset, "high");
        s.graphics.apply_preset("lowest");
        s.graphics.fps_cap = 144;
        s.save(&root).unwrap();
        let text = fs::read_to_string(GameSettings::path(&root)).unwrap();
        assert!(text.contains("[graphics]") && text.contains("preset = \"lowest\"") && text.contains("Key = \"Space\""), "{text}");
        let back = GameSettings::load(&root);
        assert_eq!((back.graphics.preset.as_str(), back.graphics.msaa, back.graphics.fps_cap), ("lowest", 1, 144));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_flags_toggle_with_their_values() {
        let a = set_launch_flag("--fov 110", &["+cl_showfps", "1"], true);
        assert_eq!(a, "--fov 110 +cl_showfps 1");
        assert!(has_launch_flag(&a, "+cl_showfps"));
        let a = set_launch_flag(&set_launch_flag(&a, &["--fly"], true), &["+cl_showfps", "1"], false);
        assert_eq!(a, "--fov 110 --fly");
        assert_eq!(set_launch_flag(&a, &["--fly"], false), "--fov 110");
    }

    #[test]
    fn stretch_round_trips_through_the_graphics_table() {
        let mut doc: toml_edit::DocumentMut = "[graphics]
fps_cap = 0
".parse().unwrap();
        let mut g = GraphicsSettings::load(&doc);
        assert_eq!(g.stretch, [0, 0]);
        g.stretch = [1300, 1070];
        g.fps_cap = 240;
        g.save(&mut doc);
        let back = GraphicsSettings::load(&doc.to_string().parse().unwrap());
        assert_eq!((back.stretch, back.fps_cap), ([1300, 1070], 240));
    }

    #[test]
    fn install_marks_become_steps_and_progress() {
        let marks: Vec<String> = [
            "@@apex C:/Apex",
            "@@tool dxc ok C:/dxc.exe",
            "@@tool RSX missing build it",
            "@@plan map:10:Map geometry|sky:2:Sky|reflections:1:Reflections",
            "@@step map",
            "@@ok map 412",
            "@@step sky",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let v = InstallView::parse(&marks);
        assert_eq!(v.apex.as_deref(), Some("C:/Apex"));
        assert_eq!(v.missing_tools, vec!["RSX: build it".to_string()]);
        assert_eq!(v.steps.len(), 3);
        assert_eq!(v.steps[0].state, StepState::Done(412));
        assert_eq!(v.current().map(|(i, s)| (i, s.title.as_str())), Some((1, "Sky")));
        assert!((v.fraction() - 11.0 / 13.0).abs() < 1e-5);
        let mut more = marks.clone();
        more.extend(["@@ok sky 30", "@@skip reflections no cubemaps", "@@done ok Installed to D:/x"].map(String::from));
        let v = InstallView::parse(&more);
        assert_eq!(v.steps[2].state, StepState::Skipped("no cubemaps".into()));
        assert_eq!(v.done, Some((true, "Installed to D:/x".into())));
        assert_eq!(v.fraction(), 1.0);
    }

    #[test]
    fn game_files_report_whats_missing() {
        let root = temp_root("files");
        let assets = root.join("assets");
        fs::create_dir_all(assets.join("maps/mp_test")).unwrap();
        fs::create_dir_all(root.join("content/maps")).unwrap();
        fs::write(root.join("content/maps/test.toml"), "name = \"Test\"\nbsp = \"mp_test\"\n").unwrap();
        fs::write(root.join("r5rust.toml"), format!("assets_root = '{}'\n", assets.display())).unwrap();
        fs::write(assets.join("maps/mp_test/world.glb"), "").unwrap();
        let f = GameFiles::read(&root);
        assert_eq!(f.map, "mp_test");
        assert_eq!(f.missing, ["collision.bin", "materials.json", "meta.json"]);
        assert!(f.loadscreen.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn finds_running_processes() {
        assert!(process_running("explorer.exe"));
        assert!(!process_running("definitely-not-running-r5.exe"));
    }
}

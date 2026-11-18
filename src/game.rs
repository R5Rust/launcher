//! Everything the launcher knows about the game folder: its staged builds, its git history, whether the game is
//! running, and the two long-running jobs it can start (play.ps1 and tools\publish.ps1).

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
const KEEP_LINES: usize = 5000;

/// A command with no console window: the launcher is a GUI app, so every child would otherwise flash one.
fn quiet(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
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
    pub commits: Vec<Commit>,
    /// An r5rust.exe is running (ours or not).
    pub running: bool,
    /// The extracted game files R5Rust loads (never shipped; made from the player's own Apex install).
    pub files: GameFiles,
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

/// Same order as the game's `Config::assets_root`: env R5RUST_ASSETS > r5rust.toml `assets_root` > D:\R5Rust\assets.
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
    PathBuf::from(DEFAULT_ASSETS)
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
}

impl Default for GameSettings {
    /// The game's own defaults (crates/client/src/binds.rs UserSettings).
    fn default() -> Self {
        Self { sensitivity: 1.6, fov: 100.0, invert_y: false, auto_sprint: true, mantle_boost: true }
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
        fs::write(&path, doc.to_string()).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
    }
}

impl Snapshot {
    pub fn read(root: &Path) -> Self {
        let mut snap = Self { state: inspect_game(root), running: process_running("r5rust.exe"), ..Self::default() };
        if matches!(snap.state, GameState::FolderMissing | GameState::InvalidFolder) {
            return snap;
        }
        snap.files = GameFiles::read(root);
        snap.current = Build::read(&root.join("play/current/version.txt"));
        snap.next = Build::read(&root.join("play/next/version.txt"))
            .filter(|_| root.join("play/next/r5rust.exe").is_file());
        if root.join(".git").exists() {
            snap.commits = git_log(root, 60);
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
    let format = "--format=%h%x1f%ad%x1f%ar%x1f%s";
    let Some(text) =
        git(root, &["log", &format!("-n{count}"), format, "--date=format:%Y-%m-%d %H:%M"])
    else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let hash = parts.next()?.to_owned();
            let (day, time) = parts.next()?.split_once(' ')?;
            Some(Commit {
                hash,
                day: day.to_owned(),
                time: time.to_owned(),
                ago: parts.next()?.to_owned(),
                subject: parts.next().unwrap_or("").to_owned(),
            })
        })
        .collect()
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
}

pub struct Job {
    pub started: Instant,
    pub lines: Arc<Mutex<Vec<String>>>,
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
        };
        if !script.is_file() {
            return Err(format!("{} is missing", script.display()));
        }
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let log = Arc::new(Mutex::new(fs::File::create(&log_path).ok()));

        let mut command = quiet("pwsh");
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
        let stdout = child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>);
        let stderr = child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>);
        for stream in [stdout, stderr].into_iter().flatten() {
            let (lines, log, ctx) = (lines.clone(), log.clone(), ctx.clone());
            thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Some(file) = log.lock().unwrap().as_mut() {
                        let _ = writeln!(file, "{line}");
                    }
                    let mut lines = lines.lock().unwrap();
                    lines.push(line);
                    if lines.len() > KEEP_LINES {
                        let cut = lines.len() - KEEP_LINES;
                        lines.drain(..cut);
                    }
                    drop(lines);
                    if kind == JobKind::Build {
                        ctx.request_repaint();
                    }
                }
            });
        }
        Ok(Self { started: Instant::now(), lines, log_path, child, exit: None, ended: None })
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
// Settings, kept as small text files in %APPDATA%\R5Rust Launcher (game_path.txt predates this module).

/// Where a job's output is saved: %APPDATA%\R5Rust Launcher\<name>.
pub fn log_path(name: &str) -> PathBuf {
    data_dir().map(|d| d.join(name)).unwrap_or_else(|| env::temp_dir().join(format!("r5rust-launcher-{name}")))
}

pub fn data_dir() -> Option<PathBuf> {
    env::var_os("APPDATA").map(|d| PathBuf::from(d).join("R5Rust Launcher"))
}

fn read_setting(name: &str) -> Option<String> {
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
    PathBuf::from(DEFAULT_GAME_DIR)
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

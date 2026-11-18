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
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL_EVERY: Duration = Duration::from_secs(3);
const KEEP_LINES: usize = 400;

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
}

impl Snapshot {
    pub fn read(root: &Path) -> Self {
        let mut snap = Self { state: inspect_game(root), running: process_running("r5rust.exe"), ..Self::default() };
        if matches!(snap.state, GameState::FolderMissing | GameState::InvalidFolder) {
            return snap;
        }
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
    pub fn start(kind: JobKind, root: &Path, extra_args: &[String], ctx: eframe::egui::Context) -> Result<Self, String> {
        let script = match kind {
            JobKind::Play => root.join("play.ps1"),
            JobKind::Build => root.join("tools").join("publish.ps1"),
        };
        if !script.is_file() {
            return Err(format!("{} is missing", script.display()));
        }
        let log_path = data_dir()
            .map(|d| d.join(if kind == JobKind::Play { "game.log" } else { "build.log" }))
            .unwrap_or_else(|| env::temp_dir().join("r5rust-launcher.log"));
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
        let mut job = Job::start(JobKind::Play, &root, &split_args("--fly --fov 110"), eframe::egui::Context::default()).unwrap();
        wait(&mut job);
        assert_eq!(job.exit, Some(Some(1)));
        assert!(!job.succeeded());
        assert_eq!(job.tail(5), ["args: --fly|--fov|110", r"no build yet: run tools\publish.ps1"]);
        assert_eq!(job.failure_line().as_deref(), Some(r"no build yet: run tools\publish.ps1"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn build_job_streams_stderr_and_succeeds() {
        let root = temp_root("build");
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::write(root.join("tools/publish.ps1"), "[Console]::Error.WriteLine('   Compiling r5rust')\nWrite-Host 'staged: abc'\n").unwrap();
        let mut job = Job::start(JobKind::Build, &root, &[], eframe::egui::Context::default()).unwrap();
        wait(&mut job);
        assert!(job.succeeded());
        let lines = job.tail(5);
        assert!(lines.iter().any(|l| l.contains("Compiling r5rust")), "{lines:?}");
        assert!(lines.iter().any(|l| l == "staged: abc"), "{lines:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn finds_running_processes() {
        assert!(process_running("explorer.exe"));
        assert!(!process_running("definitely-not-running-r5.exe"));
    }
}

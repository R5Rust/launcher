//! Player mode: game builds, the install toolchain and the launcher itself come from public GitHub Releases.
//!
//! - Game builds: `R5Rust/R5Rust` releases tagged `build-<yyyy.mm.dd>-<hash>`, asset `r5rust-<tag>.zip`. The zip is laid
//!   out like the game folder (play.ps1, content\, tools\...) with the exe under `play\next\`, so play.ps1 swaps it in on
//!   the next launch exactly like a dev build staged by tools\publish.ps1. Its `tools.txt` names the toolchain it needs.
//! - Install toolchain: `R5Rust/R5Rust` releases tagged `tools-<n>`, asset `r5rust-tools-<n>.zip` (portable pwsh, Python,
//!   RSX, dxc, the extractor), unpacked into the game folder; `tools\bin\version.txt` records which one is there.
//! - Launcher: `R5Rust/launcher` releases tagged `v<semver>`, asset `R5Rust-Launcher.exe`.
//!
//! Every download is checked against the asset's SHA-256 when GitHub lists one, and written to a `.part` file first.

use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const GAME_REPO: &str = "R5Rust/R5Rust";
pub const LAUNCHER_REPO: &str = "R5Rust/launcher";
pub const BUILD_PREFIX: &str = "build-";
pub const TOOLS_PREFIX: &str = "tools-";
const USER_AGENT: &str = concat!("R5Rust-Launcher/", env!("CARGO_PKG_VERSION"));
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The installed launcher's file name (also what shortcuts point at).
pub const LAUNCHER_EXE: &str = "R5Rust Launcher.exe";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// lower-case hex, when GitHub reports a digest
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub body: String,
    /// yyyy-mm-dd
    pub date: String,
    pub assets: Vec<Asset>,
}

impl Release {
    /// A game build's commit hash: the last `-` part of `build-<date>-<hash>`.
    pub fn build_hash(&self) -> &str {
        self.tag.rsplit('-').next().unwrap_or("")
    }

    pub fn asset(&self, pred: impl Fn(&str) -> bool) -> Option<&Asset> {
        self.assets.iter().find(|a| pred(&a.name))
    }

    /// `v0.4.1` -> (0, 4, 1)
    pub fn semver(&self) -> Option<(u32, u32, u32)> {
        parse_semver(&self.tag)
    }
}

pub fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

pub fn own_version() -> (u32, u32, u32) {
    parse_semver(env!("CARGO_PKG_VERSION")).unwrap_or_default()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
}

/// The repo's published releases, newest first (drafts and pre-releases left out).
pub fn releases(repo: &str) -> Result<Vec<Release>, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=60");
    let resp = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(403, _) | ureq::Error::Status(429, _) => "GitHub's rate limit (try again in a few minutes)".to_owned(),
            ureq::Error::Status(code, _) => format!("GitHub answered {code} for {repo}"),
            ureq::Error::Transport(t) => format!("no connection to GitHub ({t})"),
        })?;
    let json: serde_json::Value = serde_json::from_reader(resp.into_reader()).map_err(|e| format!("bad answer from GitHub: {e}"))?;
    let list = json.as_array().ok_or("bad answer from GitHub: not a list")?;
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned();
    Ok(list
        .iter()
        .filter(|r| !r.get("draft").and_then(|d| d.as_bool()).unwrap_or(false))
        .filter(|r| !r.get("prerelease").and_then(|d| d.as_bool()).unwrap_or(false))
        .map(|r| Release {
            tag: s(r, "tag_name"),
            name: s(r, "name"),
            body: s(r, "body"),
            date: s(r, "published_at").chars().take(10).collect(),
            assets: r
                .get("assets")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .map(|x| Asset {
                            name: s(x, "name"),
                            url: s(x, "browser_download_url"),
                            size: x.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                            sha256: x
                                .get("digest")
                                .and_then(|d| d.as_str())
                                .and_then(|d| d.strip_prefix("sha256:"))
                                .map(|d| d.to_ascii_lowercase()),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
}

/// The newest release whose tag starts with `prefix` (GitHub lists newest first).
pub fn newest<'a>(list: &'a [Release], prefix: &str) -> Option<&'a Release> {
    list.iter().find(|r| r.tag.starts_with(prefix))
}

// ---------------------------------------------------------------------------------------------------------------
// Progress shared with the UI

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    pub stage: Mutex<String>,
}

impl Progress {
    pub fn set_stage(&self, s: impl Into<String>) {
        *self.stage.lock().unwrap() = s.into();
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
    }

    pub fn stage(&self) -> String {
        self.stage.lock().unwrap().clone()
    }

    /// 0..1, or None while the size isn't known
    pub fn fraction(&self) -> Option<f32> {
        let t = self.total.load(Ordering::Relaxed);
        (t > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / t as f32)
    }

    fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) { Err("cancelled".into()) } else { Ok(()) }
    }
}

/// Downloads `asset` to `dest` (via `dest.part`), checking its SHA-256 when GitHub gave one.
pub fn download(asset: &Asset, dest: &Path, progress: &Progress) -> Result<(), String> {
    use sha2::Digest;
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let part = dest.with_extension("part");
    let resp = agent()
        .get(&asset.url)
        .timeout(Duration::from_secs(60 * 60))
        .call()
        .map_err(|e| format!("download of {} failed: {e}", asset.name))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(asset.size);
    progress.total.store(total, Ordering::Relaxed);
    let mut reader = resp.into_reader();
    let mut file = fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        progress.check()?;
        let n = reader.read(&mut buf).map_err(|e| format!("download of {} broke off: {e}", asset.name))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
        hasher.update(&buf[..n]);
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if let Some(want) = &asset.sha256
        && *want != got
    {
        let _ = fs::remove_file(&part);
        return Err(format!("{} didn't match its checksum (download damaged?)", asset.name));
    }
    let _ = fs::remove_file(dest);
    fs::rename(&part, dest).map_err(|e| format!("{}: {e}", dest.display()))
}

/// Unpacks `zip` into `root` (entries keep their paths; existing files are replaced). Returns the files written.
pub fn unzip(zip: &Path, root: &Path, progress: &Progress) -> Result<usize, String> {
    let file = fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", zip.display()))?;
    progress.total.store(archive.len() as u64, Ordering::Relaxed);
    let mut written = 0;
    for i in 0..archive.len() {
        progress.check()?;
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name: no absolute paths, no `..`
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = root.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        } else {
            if let Some(dir) = out.parent() {
                fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            let tmp = out.with_extension("unzip-part");
            let mut f = fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
            io::copy(&mut entry, &mut f).map_err(|e| format!("{}: {e}", out.display()))?;
            drop(f);
            let _ = fs::remove_file(&out);
            fs::rename(&tmp, &out).map_err(|e| format!("{} (is it open?): {e}", out.display()))?;
            written += 1;
        }
        progress.done.store(i as u64 + 1, Ordering::Relaxed);
    }
    Ok(written)
}

fn read_trim(path: &Path) -> Option<String> {
    let t = fs::read_to_string(path).ok()?;
    let t = t.trim().trim_start_matches('\u{feff}').to_owned();
    (!t.is_empty()).then_some(t)
}

/// Where downloads wait before they're unpacked: `%LOCALAPPDATA%\R5Rust\downloads`.
pub fn downloads_dir() -> PathBuf {
    data_root().join("downloads")
}

/// `%LOCALAPPDATA%\R5Rust`: the player's game folder, files and downloads live under it.
pub fn data_root() -> PathBuf {
    env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(env::temp_dir).join("R5Rust")
}

/// The game folder a player's launcher uses when none was chosen.
pub fn player_game_root() -> PathBuf {
    data_root().join("game")
}

/// Downloads and unpacks game build `release` into `root`, then the toolchain it needs if that isn't there yet.
/// Returns a one-line summary for the UI.
pub fn install_build(root: &Path, release: &Release, all: &[Release], progress: &Progress) -> Result<String, String> {
    let asset = release
        .asset(|n| n.starts_with("r5rust-") && n.ends_with(".zip") && !n.starts_with("r5rust-tools-"))
        .ok_or_else(|| format!("release {} has no game zip", release.tag))?;
    progress.set_stage(format!("Downloading build {}", release.build_hash()));
    let zip = downloads_dir().join(&asset.name);
    download(asset, &zip, progress)?;
    progress.set_stage("Unpacking the build");
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    unzip(&zip, root, progress)?;
    let _ = fs::remove_file(&zip);

    let mut summary = format!("Build {} downloaded. It installs when you press Play.", release.build_hash());
    if let Some(need) = read_trim(&root.join("tools.txt")) {
        let have = read_trim(&root.join("tools").join("bin").join("version.txt"));
        if have.as_deref() != Some(need.as_str()) {
            let tag = format!("{TOOLS_PREFIX}{need}");
            let tools = all.iter().find(|r| r.tag == tag).ok_or_else(|| format!("the build needs install tools {tag}, which isn't published"))?;
            let asset = tools.asset(|n| n.ends_with(".zip")).ok_or_else(|| format!("release {tag} has no zip"))?;
            progress.set_stage(format!("Downloading install tools ({} MB)", asset.size / (1 << 20)));
            let zip = downloads_dir().join(&asset.name);
            download(asset, &zip, progress)?;
            progress.set_stage("Unpacking install tools");
            // a clean copy: old tool versions can leave stray DLLs next to new exes
            let bin = root.join("tools").join("bin");
            for sub in ["pwsh", "python", "rsx", "dxc", "extractor"] {
                let _ = fs::remove_dir_all(bin.join(sub));
            }
            unzip(&zip, root, progress)?;
            let _ = fs::remove_file(&zip);
            fs::write(bin.join("version.txt"), &need).map_err(|e| e.to_string())?;
            summary.push_str(" Install tools updated.");
        }
    }
    Ok(summary)
}

// ---------------------------------------------------------------------------------------------------------------
// Background tasks

pub struct Task {
    pub progress: Arc<Progress>,
    result: Arc<Mutex<Option<Result<String, String>>>>,
}

impl Task {
    pub fn spawn(ctx: eframe::egui::Context, work: impl FnOnce(&Progress) -> Result<String, String> + Send + 'static) -> Self {
        let progress = Arc::new(Progress::default());
        let result = Arc::new(Mutex::new(None));
        let (p, r) = (progress.clone(), result.clone());
        thread::spawn(move || {
            let out = work(&p);
            *r.lock().unwrap() = Some(out);
            ctx.request_repaint();
        });
        Self { progress, result }
    }

    pub fn take_result(&self) -> Option<Result<String, String>> {
        self.result.lock().unwrap().take()
    }

}

/// What the last check found, for the UI.
#[derive(Clone, Debug, Default)]
pub struct Feed {
    pub checked: bool,
    pub error: Option<String>,
    pub game: Vec<Release>,
    pub launcher: Vec<Release>,
}

impl Feed {
    pub fn newest_build(&self) -> Option<&Release> {
        newest(&self.game, BUILD_PREFIX)
    }

    /// A launcher release newer than this one, with an exe to download.
    pub fn launcher_update(&self) -> Option<&Release> {
        self.launcher
            .iter()
            .filter(|r| r.asset(|n| n.ends_with(".exe")).is_some())
            .filter_map(|r| r.semver().map(|v| (v, r)))
            .filter(|(v, _)| *v > own_version())
            .max_by_key(|(v, _)| *v)
            .map(|(_, r)| r)
    }
}

/// Fetches both repos' releases off the UI thread.
pub fn check(ctx: eframe::egui::Context, feed: Arc<Mutex<Feed>>) {
    thread::spawn(move || {
        let game = releases(GAME_REPO);
        let launcher = releases(LAUNCHER_REPO);
        let mut f = feed.lock().unwrap();
        f.checked = true;
        f.error = game.as_ref().err().or(launcher.as_ref().err()).cloned();
        if let Ok(g) = game {
            f.game = g;
        }
        if let Ok(l) = launcher {
            f.launcher = l;
        }
        drop(f);
        ctx.request_repaint();
    });
}

// ---------------------------------------------------------------------------------------------------------------
// The launcher itself: install location, self-update, uninstall

/// `%LOCALAPPDATA%\Programs\R5Rust Launcher`
pub fn install_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(env::temp_dir).join("Programs").join("R5Rust Launcher")
}

/// Running from a cargo `target` folder (Kainz's dev copy): never self-install or self-update.
pub fn is_dev_exe() -> bool {
    env::current_exe()
        .map(|e| e.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target")))
        .unwrap_or(true)
}

fn quiet(program: &str) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// Start menu + desktop shortcuts and an Apps & features entry for the installed exe.
fn register(exe: &Path) {
    // tests in a sandbox (fake LOCALAPPDATA): shortcuts and the registry entry would land on the real desktop
    if env::var_os("R5RUST_TEST_NOREG").is_some() {
        return;
    }
    let exe_s = exe.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "$w = New-Object -ComObject WScript.Shell; \
         foreach ($d in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop'))) {{ \
           $s = $w.CreateShortcut((Join-Path $d 'R5Rust Launcher.lnk')); $s.TargetPath = '{exe_s}'; \
           $s.WorkingDirectory = (Split-Path '{exe_s}'); $s.IconLocation = '{exe_s},0'; $s.Save() }}"
    );
    let _ = quiet("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &ps]).status();
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\R5Rust Launcher";
    let exe_q = format!("\"{}\"", exe.display());
    for (name, kind, value) in [
        ("DisplayName", "REG_SZ", "R5Rust Launcher".to_owned()),
        ("DisplayVersion", "REG_SZ", env!("CARGO_PKG_VERSION").to_owned()),
        ("Publisher", "REG_SZ", "R5Rust".to_owned()),
        ("DisplayIcon", "REG_SZ", exe.display().to_string()),
        ("InstallLocation", "REG_SZ", install_dir().display().to_string()),
        ("UninstallString", "REG_SZ", format!("{exe_q} --uninstall")),
        ("NoModify", "REG_DWORD", "1".to_owned()),
        ("NoRepair", "REG_DWORD", "1".to_owned()),
    ] {
        let _ = quiet("reg").args(["add", key, "/v", name, "/t", kind, "/d", &value, "/f"]).output();
    }
}

/// First run of a downloaded launcher: copy it to the install folder, add shortcuts, start that copy.
/// Returns true when this process should exit (the installed copy is starting).
pub fn self_install() -> bool {
    if is_dev_exe() || env::args().any(|a| a == "--portable") {
        return false;
    }
    let Ok(exe) = env::current_exe() else { return false };
    let dir = install_dir();
    let target = dir.join(LAUNCHER_EXE);
    // cleanup after a self-update
    let _ = fs::remove_file(dir.join("R5Rust Launcher.old.exe"));
    if exe.parent().is_some_and(|p| same_dir(p, &dir)) {
        // keep the Apps & features version current after updates
        register(&exe);
        return false;
    }
    if fs::create_dir_all(&dir).is_err() || fs::copy(&exe, &target).is_err() {
        return false; // can't install: just run from here
    }
    register(&target);
    Command::new(&target).args(env::args().skip(1)).spawn().is_ok()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Downloads launcher `release` next to the running exe, swaps it in (a running exe can be renamed, not overwritten)
/// and returns the new exe's path so the caller can start it and quit.
pub fn self_update(release: &Release, progress: &Progress) -> Result<PathBuf, String> {
    let asset = release.asset(|n| n.ends_with(".exe")).ok_or("that release has no exe")?;
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no exe folder")?.to_path_buf();
    progress.set_stage(format!("Downloading launcher {}", release.tag));
    let new = dir.join("R5Rust Launcher.new.exe");
    download(asset, &new, progress)?;
    let old = dir.join("R5Rust Launcher.old.exe");
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| format!("couldn't move the old launcher aside: {e}"))?;
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(format!("couldn't put the new launcher in place: {e}"));
    }
    Ok(exe)
}

/// `--uninstall` (Apps & features): removes shortcuts, the uninstall entry and the launcher folder; asks whether the
/// game (builds + the files made from Apex) should go too.
pub fn uninstall() {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let sure = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Uninstall R5Rust Launcher")
        .set_description("Remove the R5Rust Launcher from this PC?")
        .set_buttons(MessageButtons::YesNo)
        .show();
    if sure != MessageDialogResult::Yes {
        return;
    }
    let game = data_root();
    let wipe_game = game.exists()
        && MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title("Uninstall R5Rust Launcher")
            .set_description(format!(
                "Also delete the game and the files it built from your Apex install?\n\n{}\n\nNo keeps them (a reinstall picks them up again).",
                game.display()
            ))
            .set_buttons(MessageButtons::YesNo)
            .show()
            == MessageDialogResult::Yes;
    let ps = "foreach ($d in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop'))) { Remove-Item -LiteralPath (Join-Path $d 'R5Rust Launcher.lnk') -ErrorAction SilentlyContinue }";
    let _ = quiet("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", ps]).status();
    let _ = quiet("reg").args(["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\R5Rust Launcher", "/f"]).output();
    if let Some(settings) = env::var_os("APPDATA").map(|d| PathBuf::from(d).join("R5Rust Launcher")) {
        let _ = fs::remove_dir_all(settings);
    }
    // the running exe can't delete its own folder: a detached cmd does it once this process has exited
    let mut dirs = vec![install_dir()];
    if wipe_game {
        dirs.push(game);
    }
    let rm: Vec<String> = dirs.iter().map(|d| format!("rmdir /s /q \"{}\"", d.display())).collect();
    let script = format!("ping -n 3 127.0.0.1 >nul & {}", rm.join(" & "));
    let _ = quiet("cmd.exe").args(["/c", &script]).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_and_build_hash() {
        assert_eq!(parse_semver("v0.4.1"), Some((0, 4, 1)));
        assert_eq!(parse_semver("0.4"), Some((0, 4, 0)));
        assert_eq!(parse_semver("build-x"), None);
        let r = Release { tag: "build-2026.11.22-a11cb1b".into(), name: String::new(), body: String::new(), date: String::new(), assets: vec![] };
        assert_eq!(r.build_hash(), "a11cb1b");
    }

    #[test]
    fn launcher_update_only_when_newer() {
        let rel = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            assets: vec![Asset { name: "R5Rust-Launcher.exe".into(), url: String::new(), size: 1, sha256: None }],
        };
        let mut feed = Feed { launcher: vec![rel("v0.0.1")], ..Feed::default() };
        assert!(feed.launcher_update().is_none());
        feed.launcher.push(rel("v99.0.0"));
        assert_eq!(feed.launcher_update().map(|r| r.tag.as_str()), Some("v99.0.0"));
    }
}

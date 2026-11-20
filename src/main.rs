#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod game;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Key, Layout, Margin, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2,
};
use game::{GameSettings, GameState, InstallStatus, InstallView, Job, JobKind, Poller, Snapshot, StepState};

const RAIL: Color32 = Color32::from_rgb(12, 17, 23);
const BG: Color32 = Color32::from_rgb(10, 14, 19);
const PANEL: Color32 = Color32::from_rgb(17, 23, 33);
const RAISED: Color32 = Color32::from_rgb(21, 29, 40);
const HOVER: Color32 = Color32::from_rgb(26, 35, 49);
const TEXT: Color32 = Color32::from_rgb(232, 239, 247);
const SECONDARY: Color32 = Color32::from_rgb(147, 161, 179);
const MUTED: Color32 = Color32::from_rgb(95, 107, 123);
const BORDER: Color32 = Color32::from_rgb(25, 33, 44);
const CYAN: Color32 = Color32::from_rgb(63, 216, 255);
const CYAN_HI: Color32 = Color32::from_rgb(124, 228, 255);
const INK: Color32 = Color32::from_rgb(8, 19, 26);
const AMBER: Color32 = Color32::from_rgb(255, 186, 82);
const RED: Color32 = Color32::from_rgb(255, 107, 94);

const RAIL_W: f32 = 228.0;
const FOOTER_H: f32 = 94.0;
const PAGE_MARGIN: Margin = Margin { left: 48.0, right: 48.0, top: 58.0, bottom: 0.0 };

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("R5Rust Launcher")
            .with_inner_size([1300.0, 820.0])
            .with_min_inner_size([1080.0, 680.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_icon(load_window_icon())
            // `--screenshot=<png>` opens without taking focus, saves one frame and closes (for checking the UI)
            .with_active(!std::env::args().any(|a| a.starts_with("--screenshot="))),
        ..Default::default()
    };
    eframe::run_native(
        "R5Rust Launcher",
        options,
        Box::new(|cc| {
            set_native_icons(cc);
            Ok(Box::new(LauncherApp::new(&cc.egui_ctx)))
        }),
    )
}

/// eframe's `with_icon` only sets the window's small icon (winit `set_window_icon` = ICON_SMALL), so the taskbar
/// stretched that up and the launcher's taskbar icon looked blurry and unlike the game's. Give Windows both sizes
/// straight from the exe's embedded multi-size app-icon.ico (resource 1, build.rs), each picked at the system's size.
#[cfg(windows)]
fn set_native_icons(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON, SendMessageW,
        WM_SETICON,
    };
    let Ok(handle) = cc.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        for (which, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
            let px = GetSystemMetrics(metric);
            // MAKEINTRESOURCE(1)
            let icon = LoadImageW(module, 1 as _, IMAGE_ICON, px, px, LR_DEFAULTCOLOR);
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, which as usize, icon as isize);
            }
        }
    }
}

#[cfg(not(windows))]
fn set_native_icons(_cc: &eframe::CreationContext<'_>) {}

fn load_window_icon() -> egui::IconData {
    let icon = image::load_from_memory(include_bytes!("../assets/app-icon.png"))
        .expect("the launcher's app icon should be available")
        .resize_exact(256, 256, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    egui::IconData { rgba: icon.into_raw(), width: 256, height: 256 }
}

fn display_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Barlow Condensed".into()))
}

fn color_image(image: image::DynamicImage) -> egui::ColorImage {
    let rgba = image.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Play,
    Notes,
    Console,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Good,
    Bad,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayPhase {
    Idle,
    Starting,
    /// Running, and started by this launcher (so it can be stopped).
    Ours,
    /// An r5rust.exe this launcher didn't start.
    Elsewhere,
}

#[derive(Default)]
struct FooterAction {
    play: bool,
    stop: bool,
    build: bool,
}

struct LauncherApp {
    game_root: PathBuf,
    poller: Poller,
    page: Page,
    notice: Option<(String, Tone)>,
    mode_menu_open: bool,
    launch_args: String,
    play_job: Option<Job>,
    stop_requested: bool,
    build_job: Option<Job>,
    build_panel_open: bool,
    /// tools\install.ps1 (the INSTALL button): builds the game files from the player's own Apex install.
    install_job: Option<Job>,
    /// The install card is open over a working install (Settings > Game files > Install / update).
    install_open: bool,
    install_status: InstallStatus,
    install_rx: Option<Receiver<(InstallStatus, Vec<String>)>>,
    /// Apex installs found on this PC (newest first) and the one the install will read.
    apex_found: Vec<String>,
    apex_dir: String,
    /// Where the files go (defaults to the game's assets_root).
    install_dir: String,
    /// The player confirmed Apex is closed (R5Rust never looks at the running game, so it can't check).
    apex_closed: bool,
    /// Output of the last game session (kept after it ends).
    console: Option<Arc<Mutex<Vec<String>>>>,
    console_filter: String,
    console_problems_only: bool,
    console_on_launch: bool,
    settings: GameSettings,
    /// The map's loading screen from the player's extracted files; nothing of the game ships with the launcher.
    art: Option<(egui::TextureHandle, Vec2)>,
    art_path: Option<PathBuf>,
    art_rx: Option<Receiver<Option<egui::ColorImage>>>,
    logo: egui::TextureHandle,
    /// `--autoplay`: press Play on the first frame it's possible (for scripted tests).
    autoplay: bool,
    /// Our own maximize (the window rect to restore). Windows' maximize of a borderless window spills 8px past every
    /// screen edge, under the taskbar, so the launcher fills the monitor's work area itself instead.
    restore_rect: Option<Rect>,
    /// `--maximized`: start maximized (to the work area).
    start_maximized: bool,
    /// `--screenshot=<png>`: save one frame there, then close.
    screenshot: Option<PathBuf>,
    frames: u32,
}

impl LauncherApp {
    fn new(ctx: &egui::Context) -> Self {
        apply_theme(ctx);
        let logo = ctx.load_texture(
            "r5rust-logo",
            color_image(image::load_from_memory(include_bytes!("../assets/icon.png")).expect("bundled icon")),
            egui::TextureOptions::LINEAR,
        );
        let game_root = game::load_game_root();
        // `--page=notes|console|settings` opens on that page (handy for shortcuts and screenshots).
        let page = match std::env::args().find_map(|a| a.strip_prefix("--page=").map(str::to_owned)).as_deref() {
            Some("notes") => Page::Notes,
            Some("console") => Page::Console,
            Some("settings") => Page::Settings,
            _ => Page::Play,
        };
        Self {
            poller: Poller::start(game_root.clone(), ctx.clone()),
            settings: GameSettings::load(&game_root),
            game_root,
            page,
            notice: None,
            mode_menu_open: false,
            launch_args: game::load_launch_args(),
            play_job: None,
            stop_requested: false,
            build_job: None,
            build_panel_open: false,
            install_job: None,
            // `--install`: open the install card on start (scripted tests, screenshots)
            install_open: std::env::args().any(|a| a == "--install"),
            install_status: InstallStatus::Checking,
            install_rx: None,
            apex_found: Vec::new(),
            apex_dir: game::read_setting("apex_path.txt").unwrap_or_default(),
            install_dir: String::new(),
            apex_closed: false,
            console: game::read_last_log("game.log").map(|l| Arc::new(Mutex::new(l))),
            console_filter: String::new(),
            console_problems_only: false,
            console_on_launch: game::read_flag("console_on_launch.txt"),
            art: None,
            art_path: None,
            art_rx: None,
            logo,
            autoplay: std::env::args().any(|a| a == "--autoplay"),
            restore_rect: None,
            start_maximized: std::env::args().any(|a| a == "--maximized"),
            screenshot: std::env::args().find_map(|a| a.strip_prefix("--screenshot=").map(PathBuf::from)),
            frames: 0,
        }
    }

    fn toggle_maximize(&mut self, ctx: &egui::Context) {
        let (outer, ppp) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().native_pixels_per_point));
        if let Some(saved) = self.restore_rect.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(saved.size()));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(saved.min));
            return;
        }
        let (Some(outer), Some(ppp)) = (outer, ppp) else { return };
        let center = outer.center() * ppp;
        let Some(work) = work_area_at(center.x as i32, center.y as i32) else { return };
        let work = Rect::from_min_max(work.min / ppp, work.max / ppp);
        self.restore_rect = Some(outer);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(work.min));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(work.size()));
    }

    /// Dragging the title bar of a maximized window restores it under the cursor, like Windows does.
    fn drag_from_maximized(&mut self, ctx: &egui::Context, grab: Pos2) {
        let Some(saved) = self.restore_rect.take() else { return };
        let Some(outer) = ctx.input(|i| i.viewport().outer_rect) else { return };
        let frac = ((grab.x - outer.left()) / outer.width()).clamp(0.0, 1.0);
        let screen_grab = outer.min + grab.to_vec2();
        let pos = Pos2::new(screen_grab.x - saved.width() * frac, screen_grab.y - grab.y);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(saved.size()));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    fn browse(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose the r5rust project folder");
        if self.game_root.is_dir() {
            dialog = dialog.set_directory(&self.game_root);
        }
        if let Some(folder) = dialog.pick_folder() {
            self.game_root = folder;
            self.notice = None;
            self.settings = GameSettings::load(&self.game_root);
            self.poller.set_root(self.game_root.clone());
            if let Err(error) = game::write_setting("game_path.txt", &self.game_root.to_string_lossy()) {
                self.notice = Some((format!("Couldn't save this folder: {error}"), Tone::Bad));
            }
        }
    }

    fn launch(&mut self, ctx: &egui::Context) {
        let args = game::split_args(&self.launch_args);
        match Job::start(JobKind::Play, &self.game_root, &args, game::log_path("game.log"), ctx.clone()) {
            Ok(job) => {
                self.console = Some(job.lines.clone());
                self.play_job = Some(job);
                self.stop_requested = false;
                self.notice = None;
                if self.console_on_launch {
                    self.page = Page::Console;
                }
            }
            Err(error) => self.notice = Some((error, Tone::Bad)),
        }
        self.poller.refresh();
    }

    fn stop(&mut self) {
        if let Some(job) = &mut self.play_job {
            job.stop();
            self.stop_requested = true;
        }
    }

    fn build(&mut self, ctx: &egui::Context) {
        match Job::start(JobKind::Build, &self.game_root, &[], game::log_path("build.log"), ctx.clone()) {
            Ok(job) => {
                self.build_job = Some(job);
                self.build_panel_open = true;
                self.notice = None;
            }
            Err(error) => self.notice = Some((error, Tone::Bad)),
        }
    }

    fn building(&self) -> bool {
        self.build_job.as_ref().is_some_and(Job::running)
    }

    fn installing(&self) -> bool {
        self.install_job.as_ref().is_some_and(Job::running)
    }

    /// Asks install.ps1 (off the UI thread) whether the files are current, and which Apex installs it can see.
    fn check_install(&mut self, ctx: &egui::Context, assets: PathBuf) {
        let (tx, rx) = mpsc::channel();
        let (root, ctx) = (self.game_root.clone(), ctx.clone());
        thread::spawn(move || {
            let status = game::check_install(&root, &assets);
            let apex = game::detect_apex(&root);
            let _ = tx.send((status, apex));
            ctx.request_repaint();
        });
        self.install_status = InstallStatus::Checking;
        self.install_rx = Some(rx);
    }

    fn sync_install(&mut self, snap: &Snapshot) {
        if let Some(rx) = &self.install_rx
            && let Ok((status, apex)) = rx.try_recv()
        {
            self.install_status = status;
            self.apex_found = apex;
            self.install_rx = None;
            if self.apex_dir.is_empty() || !Path::new(&self.apex_dir).is_dir() {
                self.apex_dir = self.apex_found.first().cloned().unwrap_or_default();
            }
        }
        if self.install_dir.is_empty() {
            self.install_dir = snap.files.assets_root.display().to_string();
        }
    }

    fn install(&mut self, ctx: &egui::Context) {
        let args = game::install_args(&self.apex_dir, &self.install_dir);
        match Job::start(JobKind::Install, &self.game_root, &args, game::log_path("install.log"), ctx.clone()) {
            Ok(job) => {
                self.install_job = Some(job);
                self.install_open = true;
                self.notice = None;
                let _ = game::write_setting("apex_path.txt", &self.apex_dir);
            }
            Err(error) => self.notice = Some((error, Tone::Bad)),
        }
    }

    fn pick_folder(title: &str, start: &str) -> Option<String> {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if Path::new(start).is_dir() {
            dialog = dialog.set_directory(start);
        }
        dialog.pick_folder().map(|p| p.display().to_string())
    }

    fn phase(&self, snap: &Snapshot) -> PlayPhase {
        match &self.play_job {
            Some(job) if job.running() => {
                if snap.running || game::process_running("r5rust.exe") {
                    PlayPhase::Ours
                } else {
                    PlayPhase::Starting
                }
            }
            _ if snap.running => PlayPhase::Elsewhere,
            _ => PlayPhase::Idle,
        }
    }

    /// Notices when play.ps1 or publish.ps1 finish, and says how it went.
    fn poll_jobs(&mut self, ctx: &egui::Context) {
        if let Some(job) = &mut self.play_job
            && job.poll()
        {
            let ran = job.elapsed();
            self.notice = if self.stop_requested {
                Some(("Stopped. Ready to play.".into(), Tone::Info))
            } else if job.succeeded() && ran > Duration::from_secs(5) {
                Some((format!("Session ended after {}.", fmt_duration(ran)), Tone::Info))
            } else {
                let why = job.failure_line().unwrap_or_else(|| "no output".into());
                Some((format!("The game didn't start: {why}"), Tone::Bad))
            };
            self.play_job = None;
            self.stop_requested = false;
            // The game saves its own settings (menu changes) on the way out.
            self.settings = GameSettings::load(&self.game_root);
            self.poller.refresh();
        }
        if let Some(job) = &mut self.build_job
            && job.poll()
        {
            self.notice = if job.succeeded() {
                Some(("Build staged. It installs the next time you press Play.".into(), Tone::Good))
            } else {
                let why = job.failure_line().unwrap_or_else(|| "see the build log".into());
                Some((format!("Build failed: {why}"), Tone::Bad))
            };
            self.poller.refresh();
        }
        if let Some(job) = &mut self.install_job
            && job.poll()
        {
            let view = InstallView::parse(&job.marks.lock().unwrap());
            if job.succeeded() {
                self.notice = Some(("Game files installed. Ready to play.".into(), Tone::Good));
                self.install_job = None;
                self.install_open = false;
            } else {
                let why = view.done.map(|(_, m)| m).or_else(|| job.failure_line()).unwrap_or_else(|| "see the install log".into());
                self.notice = Some((format!("Install failed: {why}"), Tone::Bad));
            }
            self.poller.refresh();
            self.check_install(ctx, PathBuf::from(&self.install_dir));
        }
        if self.play_job.is_some() || self.building() || self.installing() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Loads the map's loading screen off the UI thread whenever the file it points at changes.
    fn sync_art(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        if let Some(rx) = &self.art_rx
            && let Ok(image) = rx.try_recv()
        {
            self.art = image.map(|img| {
                let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
                (ctx.load_texture("map-art", img, egui::TextureOptions::LINEAR), size)
            });
            self.art_rx = None;
        }
        if self.art_rx.is_none() && snap.files.loadscreen != self.art_path {
            self.art_path = snap.files.loadscreen.clone();
            match self.art_path.clone() {
                None => self.art = None,
                Some(path) => {
                    let (tx, rx) = mpsc::channel();
                    let ctx = ctx.clone();
                    thread::spawn(move || {
                        let image = image::open(&path).ok().map(|i| {
                            let i = if i.width() > 2560 {
                                i.resize(2560, 1440, image::imageops::FilterType::Triangle)
                            } else {
                                i
                            };
                            color_image(i)
                        });
                        let _ = tx.send(image);
                        ctx.request_repaint();
                    });
                    self.art_rx = Some(rx);
                }
            }
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_jobs(ctx);
        if let Some(path) = self.screenshot.clone() {
            self.frames += 1;
            // once the first install check is back (or after ~10 s)
            if self.frames > 20 && (self.install_status != InstallStatus::Checking || self.frames == 330) && self.frames < 1000 {
                self.frames = 1000;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let shot = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
            });
            if let Some(img) = shot {
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, rgba) {
                    let _ = buf.save(&path);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        if self.start_maximized && ctx.input(|i| i.viewport().outer_rect.is_some()) {
            self.start_maximized = false;
            self.toggle_maximize(ctx);
        }
        let snap = self.poller.get();
        self.sync_art(ctx, &snap);
        if self.install_status == InstallStatus::Checking && self.install_rx.is_none() {
            self.check_install(ctx, snap.files.assets_root.clone());
        }
        self.sync_install(&snap);
        let phase = self.phase(&snap);
        let can_play = snap.is_ready() && snap.files.ok() && phase == PlayPhase::Idle && !self.installing();
        let can_build = matches!(snap.state, GameState::Ready | GameState::NoBuild)
            && !self.building()
            && self.game_root.join("tools/publish.ps1").is_file();

        let mut action = FooterAction::default();
        let mut browse_clicked = false;

        self.rail(ctx, &snap, phase);
        egui::TopBottomPanel::bottom("play-footer")
            .exact_height(FOOTER_H)
            .frame(egui::Frame::none().fill(PANEL).inner_margin(Margin::symmetric(30.0, 13.0)))
            .show(ctx, |ui| action = self.footer(ui, &snap, phase, can_play, can_build));

        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG).inner_margin(0.0)).show(ctx, |ui| {
            match self.page {
                Page::Play => browse_clicked = self.play_page(ui, ctx, &snap, phase),
                Page::Notes => self.notes_page(ui, &snap),
                Page::Console => self.console_page(ui, phase),
                Page::Settings => browse_clicked = self.settings_page(ui, &snap, phase),
            }
        });

        if self.build_panel_open && self.build_job.is_some() {
            self.build_panel(ctx);
        }
        match draw_window_chrome(ctx, self.restore_rect.is_some()) {
            ChromeAction::None => {}
            ChromeAction::ToggleMaximize => self.toggle_maximize(ctx),
            ChromeAction::DragFromMaximized(grab) => self.drag_from_maximized(ctx, grab),
        }

        if can_play && !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(Key::Enter)) {
            action.play = true;
        }
        if self.autoplay && can_play {
            self.autoplay = false;
            action.play = true;
        }
        if browse_clicked {
            self.browse();
        }
        if action.build {
            self.build(ctx);
        }
        if action.stop {
            self.stop();
        }
        if action.play {
            self.launch(ctx);
        }
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Rail

#[derive(Clone, Copy)]
enum NavIcon {
    Play,
    Notes,
    Console,
    Gear,
}

impl LauncherApp {
    fn rail(&mut self, ctx: &egui::Context, snap: &Snapshot, phase: PlayPhase) {
        egui::SidePanel::left("launcher-rail")
            .exact_width(RAIL_W)
            .resizable(false)
            .frame(egui::Frame::none().fill(RAIL).inner_margin(0.0))
            .show(ctx, |ui| {
                ui.add_space(38.0);
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    ui.add(egui::Image::new((self.logo.id(), Vec2::splat(46.0))).fit_to_exact_size(Vec2::splat(46.0)));
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(3.0);
                        ui.label(RichText::new("R5RUST").font(display_font(28.0)).color(TEXT));
                        ui.label(RichText::new("LAUNCHER").size(9.0).color(MUTED));
                    });
                });
                ui.add_space(36.0);

                let unbuilt = snap.unbuilt.filter(|n| *n > 0).map(|n| (format!("{n} NEW"), CYAN));
                let live = (phase != PlayPhase::Idle).then(|| ("LIVE".to_owned(), CYAN));
                for (page, label, icon, badge) in [
                    (Page::Play, "Play", NavIcon::Play, None),
                    (Page::Notes, "Patch Notes", NavIcon::Notes, unbuilt),
                    (Page::Console, "Console", NavIcon::Console, live),
                    (Page::Settings, "Settings", NavIcon::Gear, None),
                ] {
                    if nav_item(ui, label, icon, self.page == page, badge) {
                        self.page = page;
                        self.mode_menu_open = false;
                    }
                    ui.add_space(4.0);
                }

                // The version block sits in a band level with the footer, its top edge on the footer's.
                let band_top = ui.max_rect().bottom() - FOOTER_H;
                ui.add_space((band_top - ui.cursor().min.y).max(16.0));
                ui.painter().hline(ui.max_rect().x_range(), band_top, Stroke::new(1.0_f32, BORDER));
                ui.add_space((FOOTER_H - 44.0) / 2.0 - 4.0);
                let build = snap.current.as_ref().map(|b| b.hash.as_str()).unwrap_or("none");
                for (key, value) in [("GAME", build.to_owned()), ("LAUNCHER", format!("v{}", env!("CARGO_PKG_VERSION")))]
                {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(RichText::new(key).size(9.0).color(MUTED));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(24.0);
                            ui.label(RichText::new(value).size(11.0).monospace().color(SECONDARY));
                        });
                    });
                }
            });
    }
}

fn nav_item(ui: &mut egui::Ui, label: &str, icon: NavIcon, active: bool, badge: Option<(String, Color32)>) -> bool {
    let width = ui.available_width() - 20.0;
    let (slot, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
    let rect = Rect::from_min_size(slot.min + Vec2::new(10.0, 0.0), Vec2::new(width, 44.0));
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered() || active, 0.12);
    let painter = ui.painter();
    let fill = if active { RAISED } else { mix(RAIL, HOVER, hover) };
    painter.rect_filled(rect, Rounding::same(0.0), fill);
    if active {
        painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), Rounding::same(0.0), CYAN);
    }
    let color = if active { CYAN } else { mix(SECONDARY, TEXT, hover) };
    let c = Pos2::new(rect.left() + 28.0, rect.center().y);
    let stroke = Stroke::new(1.5_f32, color);
    match icon {
        NavIcon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(c.x - 4.0, c.y - 6.0), Pos2::new(c.x + 6.0, c.y), Pos2::new(c.x - 4.0, c.y + 6.0)],
                color,
                Stroke::NONE,
            ));
        }
        NavIcon::Notes => {
            for (dy, w) in [(-5.0, 12.0), (0.0, 12.0), (5.0, 8.0)] {
                painter.line_segment([Pos2::new(c.x - 6.0, c.y + dy), Pos2::new(c.x - 6.0 + w, c.y + dy)], stroke);
            }
        }
        NavIcon::Console => {
            painter.rect_stroke(Rect::from_center_size(c, Vec2::new(14.0, 11.0)), Rounding::same(0.0), stroke);
            painter.line_segment([c + Vec2::new(-4.0, -2.0), c + Vec2::new(-1.5, 0.0)], stroke);
            painter.line_segment([c + Vec2::new(-1.5, 0.0), c + Vec2::new(-4.0, 2.0)], stroke);
            painter.line_segment([c + Vec2::new(0.5, 2.5), c + Vec2::new(4.0, 2.5)], stroke);
        }
        NavIcon::Gear => {
            painter.circle_stroke(c, 3.2, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = Vec2::new(a.cos(), a.sin());
                painter.line_segment([c + d * 5.0, c + d * 7.0], Stroke::new(2.0_f32, color));
            }
        }
    }
    painter.text(
        Pos2::new(rect.left() + 48.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        if active { TEXT } else { mix(SECONDARY, TEXT, hover) },
    );
    if let Some((badge, color)) = badge {
        painter.text(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            badge,
            FontId::proportional(9.0),
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

// -------------------------------------------------------------------------------------------------------------------
// Footer: what's installed, what's pending, and the two actions.

impl LauncherApp {
    fn footer(
        &self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        phase: PlayPhase,
        can_play: bool,
        can_build: bool,
    ) -> FooterAction {
        let mut action = FooterAction::default();
        let left_w = (ui.available_width() - 196.0 - 200.0 - 48.0).max(200.0);
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(left_w, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                ui.set_width(left_w);
                ui.spacing_mut().item_spacing.y = 3.0;
                let title = match snap.state {
                    GameState::Ready if snap.current.is_some() => "INSTALLED BUILD",
                    GameState::Ready => "STAGED BUILD",
                    GameState::NoBuild => "NO BUILD YET",
                    GameState::FolderMissing | GameState::InvalidFolder => "SETUP REQUIRED",
                };
                ui.label(RichText::new(title).size(10.0).color(MUTED));

                let body = |ui: &mut egui::Ui, text: &str| {
                    ui.add(egui::Label::new(RichText::new(text).size(13.0).color(TEXT)).truncate());
                };
                match (snap.state, snap.current.as_ref().or(snap.next.as_ref())) {
                    (GameState::Ready, Some(build)) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&build.hash).monospace().size(13.0).color(CYAN));
                            body(ui, &build.subject);
                        });
                    }
                    (GameState::Ready, None) => body(ui, "A build is staged (no version info)."),
                    (GameState::NoBuild, _) => body(ui, "Press Build latest to compile the game."),
                    (GameState::FolderMissing, _) => body(ui, "The game folder doesn't exist. Pick it in Settings."),
                    (GameState::InvalidFolder, _) => {
                        body(ui, "That folder has no play.ps1. Pick the r5rust folder in Settings.")
                    }
                }

                let (line, color) = if let Some((text, tone)) = &self.notice {
                    let color = match tone {
                        Tone::Info => MUTED,
                        Tone::Good => CYAN,
                        Tone::Bad => RED,
                    };
                    (text.clone(), color)
                } else if phase == PlayPhase::Starting {
                    ("Starting the game...".to_owned(), CYAN)
                } else if phase == PlayPhase::Ours {
                    ("Playing. Press Stop to quit.".to_owned(), CYAN)
                } else if phase == PlayPhase::Elsewhere {
                    ("R5Rust is already running (started outside the launcher).".to_owned(), CYAN)
                } else if let Some(job) = self.install_job.as_ref().filter(|j| j.running()) {
                    let view = InstallView::parse(&job.marks.lock().unwrap());
                    match view.current() {
                        Some((i, s)) => (format!("Installing game files: {} ({}/{}).", s.title, i + 1, view.steps.len()), CYAN),
                        None => ("Installing game files...".to_owned(), CYAN),
                    }
                } else if snap.is_ready() && !snap.files.ok() {
                    ("Game files missing: press Install on the Play page.".to_owned(), AMBER)
                } else if matches!(self.install_status, InstallStatus::Stale(_)) {
                    ("Apex or the installer changed: update the game files in Settings > Game files.".to_owned(), AMBER)
                } else if let (Some(next), Some(_)) = (&snap.next, &snap.current) {
                    (format!("Update {} staged. It installs when you press Play.", next.hash), CYAN)
                } else if let Some(n) = snap.unbuilt.filter(|n| *n > 0) {
                    (format!("{n} newer commit{} not built yet.", plural(n)), MUTED)
                } else if let Some(b) = snap.newest() {
                    (format!("Ready to play. Built {}.", b.date), MUTED)
                } else {
                    (String::new(), MUTED)
                };
                ui.add(egui::Label::new(RichText::new(line).size(11.0).color(color)).truncate());
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, style, enabled) = match phase {
                    PlayPhase::Ours => ("STOP", BigStyle::Stop, true),
                    PlayPhase::Starting => ("STARTING", BigStyle::Stop, true),
                    PlayPhase::Elsewhere => ("RUNNING", BigStyle::Off, false),
                    PlayPhase::Idle => ("PLAY", if can_play { BigStyle::Primary } else { BigStyle::Off }, can_play),
                };
                let response = big_button(ui, label, style, enabled);
                if phase == PlayPhase::Starting {
                    paint_progress_line(ui, response.rect, CYAN);
                }
                if response.clicked() {
                    match phase {
                        PlayPhase::Idle => action.play = true,
                        PlayPhase::Ours | PlayPhase::Starting => action.stop = true,
                        PlayPhase::Elsewhere => {}
                    }
                }
                ui.add_space(12.0);
                action.build = self.build_button(ui, snap, can_build);
            });
        });
        action
    }

    fn build_button(&self, ui: &mut egui::Ui, snap: &Snapshot, enabled: bool) -> bool {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(200.0, 56.0), Sense::click());
        let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
        let hover = ui.ctx().animate_bool_with_time(response.id, enabled && response.hovered(), 0.12);
        let painter = ui.painter();
        let pending = snap.unbuilt.is_some_and(|n| n > 0) || snap.state == GameState::NoBuild;
        painter.rect_filled(rect, Rounding::same(0.0), mix(RAISED, HOVER, hover));
        if pending && enabled {
            painter.rect_stroke(rect.shrink(0.5), Rounding::same(0.0), Stroke::new(1.0_f32, CYAN.gamma_multiply(0.6)));
        }
        let (title, detail, detail_color) = if let Some(job) = self.build_job.as_ref().filter(|j| j.running()) {
            ("BUILDING", format!("cargo build  {}", fmt_duration(job.elapsed())), CYAN)
        } else if snap.state == GameState::NoBuild {
            ("BUILD LATEST", "no build yet".to_owned(), AMBER)
        } else {
            match snap.unbuilt {
                Some(0) => ("BUILD LATEST", "up to date".to_owned(), MUTED),
                Some(n) => ("BUILD LATEST", format!("{n} new commit{}", plural(n)), CYAN),
                None => ("BUILD LATEST", "compile + stage".to_owned(), MUTED),
            }
        };
        let title_color = if enabled || self.building() { TEXT } else { MUTED };
        painter.text(rect.left_top() + Vec2::new(16.0, 12.0), Align2::LEFT_TOP, title, display_font(18.0), title_color);
        painter.text(
            rect.left_top() + Vec2::new(16.0, 34.0),
            Align2::LEFT_TOP,
            detail,
            FontId::proportional(10.0),
            detail_color,
        );
        if self.building() {
            paint_progress_line(ui, rect, CYAN);
        }
        enabled && response.clicked()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BigStyle {
    Primary,
    Stop,
    Off,
}

/// The one loud thing on screen: 196x56, hover fades in, the press darkens.
fn big_button(ui: &mut egui::Ui, label: &str, style: BigStyle, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(196.0, 56.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id, enabled && response.hovered(), 0.12);
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, text) = match style {
        BigStyle::Primary => (mix(CYAN, CYAN_HI, hover), INK),
        BigStyle::Stop => (mix(RAISED, Color32::from_rgb(120, 40, 36), hover), mix(TEXT, Color32::WHITE, hover)),
        BigStyle::Off => (RAISED, MUTED),
    };
    let fill = if pressed { fill.gamma_multiply(0.85) } else { fill };
    ui.painter().rect_filled(rect, Rounding::same(0.0), fill);
    ui.painter().text(rect.center() + Vec2::new(0.0, 1.0), Align2::CENTER_CENTER, label, display_font(27.0), text);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else if label == "RUNNING" {
        response.on_hover_text("R5Rust is already running.")
    } else {
        response.on_hover_text("Not ready to play yet: see the line on the left.")
    }
}

/// An indeterminate progress sweep along the bottom edge of `rect`.
fn paint_progress_line(ui: &egui::Ui, rect: Rect, color: Color32) {
    let t = ui.input(|i| i.time) as f32;
    let span = rect.width() * 0.3;
    let x = rect.left() - span + (t * 0.8).fract() * (rect.width() + span);
    let bar = Rect::from_min_max(
        Pos2::new(x.max(rect.left()), rect.bottom() - 2.0),
        Pos2::new((x + span).min(rect.right()), rect.bottom()),
    );
    ui.painter().rect_filled(bar, Rounding::same(0.0), color);
    ui.ctx().request_repaint_after(Duration::from_millis(16));
}

fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn plural(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

// -------------------------------------------------------------------------------------------------------------------
// Play page

impl LauncherApp {
    fn paint_hero_background(&self, painter: &egui::Painter, hero: Rect) {
        if let Some((tex, size)) = &self.art {
            painter.image(tex.id(), hero, cover_uv(*size, hero.size()), Color32::WHITE);
            return;
        }
        // No extracted art: a plain dark field (the launcher ships no game imagery).
        paint_scrim_colors(painter, hero, [PANEL, Color32::from_rgb(16, 30, 40), BG, BG]);
    }

    /// Returns true when the files card asked to change the game folder.
    fn play_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snapshot, phase: PlayPhase) -> bool {
        let hero_size = ui.available_size();
        let (hero, _) = ui.allocate_exact_size(hero_size, Sense::hover());
        let painter = ui.painter().clone();
        self.paint_hero_background(&painter, hero);
        paint_vertical_scrim(
            &painter,
            Rect::from_min_max(hero.min, Pos2::new(hero.right(), hero.top() + hero.height() * 0.31)),
            172,
            0,
        );
        paint_vertical_scrim(
            &painter,
            Rect::from_min_max(Pos2::new(hero.left(), hero.top() + hero.height() * 0.43), hero.max),
            0,
            226,
        );
        paint_horizontal_scrim(
            &painter,
            Rect::from_min_max(hero.min, Pos2::new(hero.left() + hero.width() * 0.5, hero.bottom())),
            142,
            0,
        );

        // MODES tab
        let tab = Rect::from_min_size(Pos2::new(hero.left() + 40.0, hero.top() + 42.0), Vec2::new(132.0, 38.0));
        let tab_response = ui.interact(tab, egui::Id::new("mode-tab"), Sense::click());
        let tab_color = if tab_response.hovered() || self.mode_menu_open { TEXT } else { SECONDARY };
        painter.text(Pos2::new(tab.left(), tab.top() + 8.0), Align2::LEFT_TOP, "MODES", display_font(18.0), tab_color);
        let arrow = Pos2::new(tab.left() + 60.0, tab.top() + 19.0);
        let flip = if self.mode_menu_open { -1.0 } else { 1.0 };
        painter.line_segment([Pos2::new(arrow.x - 4.0, arrow.y - 2.0 * flip), arrow], Stroke::new(1.3_f32, tab_color));
        painter.line_segment([arrow, Pos2::new(arrow.x + 4.0, arrow.y - 2.0 * flip)], Stroke::new(1.3_f32, tab_color));
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(tab.left(), tab.bottom() - 2.0), Vec2::new(72.0, 2.0)),
            Rounding::same(0.0),
            CYAN,
        );
        let toggled = tab_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        if toggled {
            self.mode_menu_open = !self.mode_menu_open;
        }

        // Title block
        let x = hero.left() + 42.0;
        let y = hero.bottom() - 168.0;
        painter.text(Pos2::new(x, y), Align2::LEFT_TOP, "TRAINING MODE", FontId::proportional(11.0), CYAN);
        painter.text(Pos2::new(x - 2.0, y + 14.0), Align2::LEFT_TOP, "FIRING RANGE", display_font(70.0), TEXT);
        painter.text(
            Pos2::new(x + 2.0, y + 92.0),
            Align2::LEFT_TOP,
            "Move freely. Practice your movement. Make the range yours.",
            FontId::proportional(14.0),
            SECONDARY,
        );
        let meta = painter.text(
            Pos2::new(x + 2.0, y + 118.0),
            Align2::LEFT_TOP,
            format!("LOCAL SESSION   /   SOLO   /   {}", snap.files.map),
            FontId::monospace(10.0),
            MUTED,
        );
        if phase != PlayPhase::Idle {
            let pulse = 0.55 + 0.45 * (ui.input(|i| i.time) as f32 * 3.0).sin().abs();
            let dot = Pos2::new(meta.right() + 22.0, meta.center().y);
            painter.circle_filled(dot, 3.5, CYAN.gamma_multiply(pulse));
            let text = match (phase, self.play_job.as_ref()) {
                (PlayPhase::Starting, _) => "STARTING".to_owned(),
                (_, Some(job)) => format!("IN GAME  {}", fmt_duration(job.elapsed())),
                _ => "IN GAME".to_owned(),
            };
            painter.text(dot + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, text, FontId::monospace(10.0), CYAN);
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        // Game files: one card, like R5Regenis' first-run card. It's the install form until the files are there, the
        // progress while installing, and comes back from Settings for a reinstall / update.
        let browse = false;
        let show = phase == PlayPhase::Idle && (self.install_job.is_some() || self.install_open || (snap.is_ready() && !snap.files.ok()));
        if show {
            let card = Rect::from_center_size(hero.center() - Vec2::new(0.0, 30.0), Vec2::new(620.0, 372.0));
            ui.allocate_ui_at_rect(card, |ui| {
                egui::Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(12, 17, 24, 240))
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .inner_margin(Margin::same(26.0))
                    .show(ui, |ui| {
                        ui.set_width(card.width() - 52.0);
                        self.install_card(ui, ctx, snap);
                    });
            });
        }

        if self.mode_menu_open {
            let area = egui::Area::new(egui::Id::new("mode-menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(Pos2::new(tab.left(), tab.bottom() + 10.0))
                .show(ctx, |ui| self.mode_menu(ui));
            if area.inner || (!toggled && area.response.clicked_elsewhere()) {
                self.mode_menu_open = false;
            }
        }
        browse
    }

    fn install_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snapshot) {
        if let Some(job) = &self.install_job {
            let view = InstallView::parse(&job.marks.lock().unwrap());
            let (running, elapsed, tail, log) = (job.running(), job.elapsed(), job.tail(3), job.log_path.clone());
            self.install_progress(ui, ctx, &view, running, elapsed, &tail, &log);
            return;
        }
        let (kicker, kicker_color, title) = match &self.install_status {
            InstallStatus::Stale(_) => ("UPDATE AVAILABLE", AMBER, "Update the game files from your Apex install"),
            InstallStatus::Partial(_) => ("INSTALL NOT FINISHED", AMBER, "Finish building the game files"),
            _ if snap.files.ok() => ("REINSTALL", CYAN, "Rebuild the game files from your Apex install"),
            _ => ("GAME FILES MISSING", AMBER, "Build the Firing Range from your Apex install"),
        };
        ui.label(RichText::new(kicker).size(10.0).color(kicker_color));
        ui.add_space(2.0);
        ui.label(RichText::new(title).size(19.0).color(TEXT));
        ui.add_space(4.0);
        let why = match &self.install_status {
            InstallStatus::Stale(d) | InstallStatus::Partial(d) => format!("{d}. "),
            _ => String::new(),
        };
        ui.label(
            RichText::new(format!(
                "{why}R5Rust ships no game files: it makes them from your own copy of Apex, reading its files only (never the running game). Takes about 15 minutes and 5 GB."
            ))
            .size(12.5)
            .color(SECONDARY),
        );
        ui.add_space(14.0);

        let mut pick_apex = false;
        let mut pick_dir = false;
        let apex_ok = !self.apex_dir.is_empty() && Path::new(&self.apex_dir).join("paks").join("Win64").is_dir();
        let checking = self.install_status == InstallStatus::Checking;
        let apex_text = match (self.apex_dir.is_empty(), checking) {
            (false, _) => self.apex_dir.clone(),
            (true, true) => "Looking for Apex...".to_owned(),
            (true, false) => "Not found: choose the Apex folder".to_owned(),
        };
        path_row(ui, "APEX", &apex_text, apex_ok || (checking && self.apex_dir.is_empty()), &mut pick_apex);
        path_row(ui, "SAVE TO", &self.install_dir, true, &mut pick_dir);
        ui.add_space(8.0);
        toggle_row(
            ui,
            &mut self.apex_closed,
            "Apex is closed",
            "R5Rust never looks at the running game (anti-cheat), so it can't check: close Apex first.",
            true,
        );
        ui.add_space(10.0);
        let can = apex_ok && self.apex_closed && !self.install_dir.trim().is_empty() && !self.building();
        let mut start = false;
        let mut close = false;
        ui.horizontal(|ui| {
            let r = big_button(ui, "INSTALL", if can { BigStyle::Primary } else { BigStyle::Off }, can);
            start = r.clicked() && can;
            ui.add_space(10.0);
            if self.install_open && snap.files.ok() {
                close = flat_button(ui, "CANCEL");
            }
        });
        if pick_apex && let Some(d) = Self::pick_folder("Choose the Apex Legends folder (the one with paks\\Win64)", &self.apex_dir) {
            self.apex_dir = d;
        }
        if pick_dir && let Some(d) = Self::pick_folder("Where should R5Rust put its game files?", &self.install_dir) {
            self.install_dir = d;
        }
        if close {
            self.install_open = false;
        }
        if start {
            self.install(ctx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn install_progress(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        view: &InstallView,
        running: bool,
        elapsed: Duration,
        tail: &[String],
        log: &Path,
    ) {
        let failed = !running && !view.done.as_ref().is_some_and(|d| d.0);
        let (kicker, color) = if running {
            (format!("INSTALLING  {}", fmt_duration(elapsed)), CYAN)
        } else if failed {
            ("INSTALL FAILED".to_owned(), RED)
        } else {
            ("INSTALLED".to_owned(), CYAN)
        };
        ui.label(RichText::new(kicker).size(10.0).color(color));
        ui.add_space(2.0);
        let title = match (view.current(), &view.done) {
            (Some((_, s)), _) => s.title.clone(),
            (None, Some((_, m))) => m.clone(),
            _ => "Starting...".to_owned(),
        };
        ui.add(egui::Label::new(RichText::new(title).size(19.0).color(TEXT)).truncate());
        ui.add_space(8.0);

        // progress bar (weighted by how long each step takes)
        let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
        ui.painter().rect_filled(bar, Rounding::same(0.0), RAISED);
        let f = view.fraction().clamp(0.0, 1.0);
        ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * f, bar.height())), Rounding::same(0.0), if failed { RED } else { CYAN });
        ui.add_space(10.0);

        // the steps, two columns
        let rows = view.steps.len().div_ceil(2);
        let col_w = (ui.available_width() - 16.0) / 2.0;
        let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), rows as f32 * 19.0), Sense::hover());
        let p = ui.painter();
        let t = ui.input(|i| i.time) as f32;
        for (i, step) in view.steps.iter().enumerate() {
            let at = area.min + Vec2::new((i / rows) as f32 * (col_w + 16.0), (i % rows) as f32 * 19.0);
            let dot = Rect::from_center_size(at + Vec2::new(4.0, 8.0), Vec2::splat(7.0));
            let (dot_color, right, text_color) = match &step.state {
                StepState::Pending => (BORDER, String::new(), MUTED),
                StepState::Running => (CYAN.gamma_multiply(0.45 + 0.55 * (t * 3.0).sin().abs()), "working".to_owned(), TEXT),
                StepState::Done(secs) => (CYAN, format!("{secs}s"), SECONDARY),
                StepState::Skipped(_) => (AMBER, "skipped".to_owned(), SECONDARY),
                StepState::Failed(_) => (RED, "failed".to_owned(), RED),
            };
            p.rect_filled(dot, Rounding::same(0.0), dot_color);
            p.text(at + Vec2::new(16.0, 1.0), Align2::LEFT_TOP, &step.title, FontId::proportional(12.0), text_color);
            p.text(at + Vec2::new(col_w, 1.0), Align2::RIGHT_TOP, right, FontId::monospace(10.0), MUTED);
        }
        ui.add_space(8.0);
        for tool in &view.missing_tools {
            ui.label(RichText::new(format!("Missing {tool}")).size(11.0).color(AMBER));
        }
        if let Some(why) = view.steps.iter().find_map(|s| if let StepState::Skipped(w) = &s.state { Some(w) } else { None }) {
            ui.label(RichText::new(format!("Skipped: {why}")).size(11.0).color(AMBER));
        }
        for line in tail {
            if !line.starts_with("@@") {
                ui.add(egui::Label::new(RichText::new(line.trim()).monospace().size(10.5).color(MUTED)).truncate());
            }
        }
        ui.add_space(10.0);
        let (mut stop, mut retry, mut close, mut open_log) = (false, false, false, false);
        ui.horizontal(|ui| {
            if running {
                stop = flat_button(ui, "STOP");
            } else {
                if failed {
                    retry = flat_button(ui, "TRY AGAIN");
                }
                close = flat_button(ui, "CLOSE");
            }
            open_log = flat_button(ui, "OPEN LOG");
        });
        if running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if open_log {
            let _ = game::open_path(log);
        }
        if stop && let Some(job) = &mut self.install_job {
            job.stop();
        }
        if retry || close {
            self.install_job = None;
            if close {
                self.install_open = false;
            }
        }
    }

    /// Returns true when the menu should close (a mode was picked).
    fn mode_menu(&self, ui: &mut egui::Ui) -> bool {
        let mut picked = false;
        egui::Frame::none().fill(PANEL).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(312.0);
            ui.label(RichText::new("GAME MODES").size(10.0).color(MUTED));
            ui.add_space(8.0);
            let (row, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 74.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(row, Rounding::same(0.0), if response.hovered() { HOVER } else { RAISED });
            p.rect_filled(Rect::from_min_size(row.left_top(), Vec2::new(2.0, row.height())), Rounding::same(0.0), CYAN);
            let thumb = Rect::from_min_size(row.left_top() + Vec2::new(10.0, 9.0), Vec2::new(88.0, 56.0));
            match &self.art {
                Some((tex, size)) => {
                    p.image(tex.id(), thumb, cover_uv(*size, thumb.size()), Color32::WHITE);
                }
                None => {
                    p.rect_filled(thumb, Rounding::same(0.0), BG);
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    let mark = Rect::from_center_size(thumb.center(), Vec2::splat(34.0));
                    p.image(self.logo.id(), mark, uv, Color32::from_white_alpha(90));
                }
            }
            let text_x = thumb.right() + 12.0;
            p.text(Pos2::new(text_x, row.top() + 22.0), Align2::LEFT_TOP, "Firing Range", FontId::proportional(14.0), TEXT);
            p.text(Pos2::new(text_x, row.top() + 45.0), Align2::LEFT_TOP, "SELECTED", FontId::proportional(9.0), CYAN);
            picked = response.clicked();
            ui.add_space(9.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(RichText::new("MORE MODES").size(9.0).color(MUTED));
            ui.label(RichText::new("Coming later").size(12.0).color(SECONDARY));
        });
        picked
    }

    fn build_panel(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.build_job else { return };
        let screen = ctx.screen_rect();
        let size = Vec2::new(560.0_f32.min(screen.width() - RAIL_W - 56.0), 240.0);
        let pos = Pos2::new(screen.right() - 28.0 - size.x, screen.bottom() - FOOTER_H - 20.0 - size.y);
        let mut close = false;
        let mut open_log = false;
        egui::Area::new(egui::Id::new("build-panel")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, Rounding::same(0.0), Color32::from_rgba_unmultiplied(9, 13, 18, 240));
            p.rect_stroke(rect, Rounding::same(0.0), Stroke::new(1.0_f32, BORDER));
            let (status, color) = match job.exit {
                None => (format!("COMPILING  {}", fmt_duration(job.elapsed())), CYAN),
                Some(Some(0)) => (format!("STAGED IN {}", fmt_duration(job.elapsed())), CYAN),
                Some(_) => ("FAILED".to_owned(), RED),
            };
            p.text(rect.left_top() + Vec2::new(16.0, 13.0), Align2::LEFT_TOP, "BUILD", display_font(18.0), TEXT);
            p.text(rect.left_top() + Vec2::new(64.0, 17.0), Align2::LEFT_TOP, status, FontId::monospace(11.0), color);

            let close_rect = Rect::from_min_size(Pos2::new(rect.right() - 36.0, rect.top() + 6.0), Vec2::splat(30.0));
            let close_r = ui.interact(close_rect, egui::Id::new("build-close"), Sense::click());
            if close_r.hovered() {
                p.rect_filled(close_rect, Rounding::same(0.0), HOVER);
            }
            let c = close_rect.center();
            let s = Stroke::new(1.2_f32, TEXT);
            p.line_segment([c + Vec2::new(-4.5, -4.5), c + Vec2::new(4.5, 4.5)], s);
            p.line_segment([c + Vec2::new(4.5, -4.5), c + Vec2::new(-4.5, 4.5)], s);
            close = close_r.on_hover_text("Hide (the build keeps going)").clicked();

            let log_rect =
                Rect::from_min_size(Pos2::new(close_rect.left() - 76.0, rect.top() + 6.0), Vec2::new(70.0, 30.0));
            let log_r = ui.interact(log_rect, egui::Id::new("build-log"), Sense::click());
            p.text(
                log_rect.center(),
                Align2::CENTER_CENTER,
                "OPEN LOG",
                FontId::proportional(10.0),
                if log_r.hovered() { TEXT } else { SECONDARY },
            );
            open_log = log_r.clicked();

            let body =
                Rect::from_min_max(rect.left_top() + Vec2::new(16.0, 44.0), rect.right_bottom() - Vec2::new(16.0, 12.0));
            let clipped = p.with_clip_rect(body);
            let lines = job.tail(12);
            let line_h = 15.0;
            let start = body.bottom() - line_h * lines.len() as f32;
            for (i, line) in lines.iter().enumerate() {
                clipped.text(
                    Pos2::new(body.left(), start.max(body.top()) + i as f32 * line_h),
                    Align2::LEFT_TOP,
                    line.trim_end(),
                    FontId::monospace(11.0),
                    line_color(line),
                );
            }
            if job.running() {
                paint_progress_line(ui, Rect::from_min_size(rect.left_top(), Vec2::new(rect.width(), 2.0)), CYAN);
            }
        });
        if open_log {
            let _ = game::open_path(&job.log_path);
        }
        if close {
            self.build_panel_open = false;
            if !self.building() {
                self.build_job = None;
            }
        }
    }
}

/// Colour for a line of cargo / env_logger output.
fn line_color(line: &str) -> Color32 {
    let head = line.chars().take(48).collect::<String>().to_ascii_lowercase();
    if head.contains(" error") || head.starts_with("error") || head.contains("panicked") || head.contains("failed") {
        RED
    } else if head.contains(" warn") || head.starts_with("warning") {
        AMBER
    } else if head.starts_with("staged:") || head.contains("finished") {
        CYAN
    } else {
        SECONDARY
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Patch notes: the game's git history, with the installed and staged builds marked.

fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).font(display_font(46.0)).color(TEXT));
    ui.add_space(2.0);
    ui.label(RichText::new(subtitle).size(13.0).color(SECONDARY));
    ui.add_space(22.0);
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32, filled: bool) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(9.0), color);
    let size = galley.size() + Vec2::new(14.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if filled {
        ui.painter().rect_filled(rect, Rounding::same(0.0), color);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(9.0), INK);
    } else {
        ui.painter().rect_stroke(rect.shrink(0.5), Rounding::same(0.0), Stroke::new(1.0_f32, color));
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    }
}

fn same_hash(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

impl LauncherApp {
    fn notes_page(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        egui::Frame::none().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            let subtitle = match snap.unbuilt {
                Some(0) => "Recent changes to the game. Your build has all of them.".to_owned(),
                Some(1) => "Recent changes to the game. The newest one isn't in a build yet: use Build latest.".to_owned(),
                Some(n) => format!("Recent changes to the game. The newest {n} aren't in a build yet: use Build latest."),
                None => "Recent changes to the game, from its git history.".to_owned(),
            };
            page_header(ui, "PATCH NOTES", &subtitle);
            if snap.commits.is_empty() {
                ui.label(RichText::new("No git history in this game folder.").size(13.0).color(MUTED));
                return;
            }
            let playing = snap.current.as_ref().map(|b| b.hash.as_str()).unwrap_or("");
            let staged = snap.next.as_ref().map(|b| b.hash.as_str()).unwrap_or("");
            let unbuilt = snap.unbuilt.unwrap_or(0) as usize;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 8.0);
                let mut day = "";
                for (i, c) in snap.commits.iter().enumerate() {
                    if c.day != day {
                        day = &c.day;
                        ui.add_space(if i == 0 { 0.0 } else { 18.0 });
                        ui.label(RichText::new(&c.day).size(10.0).color(MUTED));
                        ui.add_space(4.0);
                    }
                    let row_top = ui.cursor().min.y;
                    ui.horizontal_top(|ui| {
                        ui.add_space(2.0);
                        ui.allocate_ui_with_layout(Vec2::new(64.0, 18.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_width(64.0);
                            ui.label(RichText::new(&c.hash).monospace().size(12.0).color(MUTED));
                        });
                        let chips_w = 230.0;
                        let text_w = (ui.available_width() - chips_w).max(200.0);
                        ui.allocate_ui_with_layout(Vec2::new(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
                            ui.set_width(text_w);
                            ui.add(egui::Label::new(RichText::new(&c.subject).size(13.0).color(TEXT)).wrap());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            ui.label(RichText::new(format!("{}  {}", c.time, c.ago)).size(10.0).color(MUTED));
                            ui.add_space(6.0);
                            if same_hash(&c.hash, playing) {
                                chip(ui, "PLAYING", CYAN, true);
                            }
                            if same_hash(&c.hash, staged) {
                                chip(ui, "STAGED", CYAN, false);
                            }
                            if i < unbuilt {
                                chip(ui, "NOT BUILT", MUTED, false);
                            }
                        });
                    });
                    ui.add_space(8.0);
                    let y = ui.cursor().min.y - 4.0;
                    if y > row_top {
                        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, BORDER));
                    }
                }
                ui.add_space(24.0);
            });
        });
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Console: the game's own output, live (R5Flowstate's launcher has the same tab).

impl LauncherApp {
    fn console_page(&mut self, ui: &mut egui::Ui, phase: PlayPhase) {
        let log_path = game::data_dir().unwrap_or_default().join("game.log");
        egui::Frame::none().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            let subtitle = match phase {
                PlayPhase::Ours | PlayPhase::Starting => "The game's output, live. Also saved to game.log.",
                PlayPhase::Elsewhere => "R5Rust was started outside the launcher, so its output isn't here.",
                PlayPhase::Idle => "The game's output from the last session started here (game.log).",
            };
            page_header(ui, "CONSOLE", subtitle);

            let lines: Vec<String> = self.console.as_ref().map(|l| l.lock().unwrap().clone()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.console_filter)
                        .hint_text(RichText::new("Filter").color(MUTED))
                        .font(FontId::monospace(12.0))
                        .margin(Margin::symmetric(10.0, 8.0))
                        .desired_width(280.0),
                );
                ui.add_space(8.0);
                toggle(ui, &mut self.console_problems_only, true);
                ui.label(RichText::new("Warnings and errors only").size(12.0).color(SECONDARY));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if flat_button(ui, "OPEN LOG FILE")
                        && let Err(e) = game::open_path(&log_path)
                    {
                        self.notice = Some((e, Tone::Bad));
                    }
                    if flat_button(ui, "COPY") {
                        ui.ctx().copy_text(lines.join("\n"));
                        self.notice = Some((format!("Copied {} lines.", lines.len()), Tone::Info));
                    }
                });
            });
            ui.add_space(12.0);

            let filter = self.console_filter.to_ascii_lowercase();
            let problems_only = self.console_problems_only;
            let shown: Vec<&String> = lines
                .iter()
                .filter(|l| !problems_only || matches!(line_color(l), RED | AMBER))
                .filter(|l| filter.is_empty() || l.to_ascii_lowercase().contains(&filter))
                .collect();

            egui::Frame::none().fill(PANEL).inner_margin(Margin::same(14.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height((ui.available_height() - 24.0).max(120.0));
                if shown.is_empty() {
                    let msg = if lines.is_empty() {
                        "Nothing yet. Press Play and the game's output shows up here."
                    } else {
                        "No lines match."
                    };
                    ui.label(RichText::new(msg).size(13.0).color(MUTED));
                    return;
                }
                let row_h = 17.0;
                egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(
                    ui,
                    row_h,
                    shown.len(),
                    |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for line in &shown[range] {
                            let (prefix, rest) = split_log_prefix(line);
                            ui.horizontal(|ui| {
                                ui.set_height(row_h);
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if !prefix.is_empty() {
                                    let p = RichText::new(prefix).monospace().size(12.0).color(MUTED);
                                    ui.add(egui::Label::new(p).extend());
                                }
                                let r = RichText::new(rest).monospace().size(12.0).color(line_color(line));
                                ui.add(egui::Label::new(r).extend());
                            });
                        }
                    },
                );
            });
        });
    }
}

/// env_logger lines look like `[2026-11-18T01:02:03Z INFO  client] text`: dim the bracket, keep the message.
fn split_log_prefix(line: &str) -> (&str, &str) {
    if line.starts_with('[')
        && let Some(end) = line.find("] ")
        && end < 80
    {
        return (&line[..=end], &line[end + 2..]);
    }
    ("", line)
}

// -------------------------------------------------------------------------------------------------------------------
// Settings

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(title).size(10.0).color(MUTED));
    ui.add_space(6.0);
    egui::Frame::none().fill(PANEL).inner_margin(Margin::same(20.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
    ui.add_space(22.0);
}

/// A labelled path with a CHANGE button (sets `pick` when clicked).
fn path_row(ui: &mut egui::Ui, label: &str, path: &str, ok: bool, pick: &mut bool) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(64.0, 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(64.0);
            ui.label(RichText::new(label).size(9.5).color(MUTED));
        });
        let w = (ui.available_width() - 96.0).max(80.0);
        ui.allocate_ui_with_layout(Vec2::new(w, 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(w);
            ui.add(egui::Label::new(RichText::new(path).monospace().size(12.0).color(if ok { TEXT } else { AMBER })).truncate());
        });
        *pick = flat_button(ui, "CHANGE");
    });
    ui.add_space(4.0);
}

fn flat_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.0).color(TEXT))
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(0.0))
            .min_size(Vec2::new(0.0, 34.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

/// A switch, drawn like R5Regenis' (Theme/Styles.axaml `ToggleButton.switch`): a 42x24 pill with a hairline
/// border and a 16px knob that slides right and turns cyan when on.
fn toggle(ui: &mut egui::Ui, value: &mut bool, enabled: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
    let mut changed = false;
    if enabled && response.clicked() {
        *value = !*value;
        changed = true;
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *value, 0.14);
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), enabled && response.hovered(), 0.12);
    let off_border = mix(Color32::from_rgb(44, 56, 72), Color32::from_rgb(70, 84, 102), hover);
    let (fill, border, knob) = if enabled {
        (
            mix(BG, Color32::from_rgb(14, 52, 64), t),
            mix(off_border, Color32::from_rgb(38, 150, 180), t),
            mix(mix(MUTED, SECONDARY, hover), CYAN, t),
        )
    } else {
        (BG, Color32::from_rgb(32, 40, 52), Color32::from_rgb(58, 66, 78))
    };
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(12.0), fill);
    p.rect_stroke(rect.shrink(0.5), Rounding::same(12.0), Stroke::new(1.0_f32, border));
    let knob_x = egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t);
    p.circle_filled(Pos2::new(knob_x, rect.center().y), 8.0, knob);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    changed
}

fn toggle_row(ui: &mut egui::Ui, value: &mut bool, title: &str, hint: &str, enabled: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed = toggle(ui, value, enabled);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).size(13.0).color(if enabled { TEXT } else { MUTED }));
            ui.label(RichText::new(hint).size(11.0).color(MUTED));
        });
    });
    ui.add_space(6.0);
    changed
}

/// A row of segmented choices (label + options). Returns true when the selection changed.
fn choice_row<T: PartialEq + Clone>(ui: &mut egui::Ui, title: &str, value: &mut T, options: &[(T, &str)], enabled: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(170.0, 20.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(170.0);
            ui.label(RichText::new(title).size(13.0).color(if enabled { TEXT } else { MUTED }));
        });
        ui.spacing_mut().item_spacing.x = 2.0;
        for (v, label) in options {
            let on = *value == *v;
            let text = RichText::new(*label).size(11.0).color(if on { BG } else if enabled { TEXT } else { MUTED });
            let b = egui::Button::new(text)
                .fill(if on { if enabled { CYAN } else { MUTED } } else { BG })
                .stroke(Stroke::NONE)
                .rounding(Rounding::same(0.0))
                .min_size(Vec2::new(0.0, 26.0));
            let r = ui.add_enabled(enabled, b);
            if r.clicked() && !on {
                *value = v.clone();
                changed = true;
            }
        }
    });
    ui.add_space(4.0);
    changed
}

/// Returns true once the value has settled (drag released, or a click / keyboard change).
fn slider_row(
    ui: &mut egui::Ui,
    title: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
    suffix: &str,
    decimals: usize,
    enabled: bool,
) -> bool {
    let mut settled = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(170.0, 20.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(170.0);
            ui.label(RichText::new(title).size(13.0).color(if enabled { TEXT } else { MUTED }));
        });
        let slider = egui::Slider::new(value, range).step_by(step).show_value(false).trailing_fill(true);
        // A smaller knob drawn as a cyan ring (the rail and the knob share the widget fill in egui).
        let r = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = 18.0;
                let w = &mut ui.visuals_mut().widgets;
                w.inactive.fg_stroke = Stroke::new(2.0_f32, CYAN);
                w.hovered.fg_stroke = Stroke::new(2.0_f32, CYAN_HI);
                w.active.fg_stroke = Stroke::new(2.0_f32, CYAN_HI);
                ui.add_enabled(enabled, slider)
            })
            .inner;
        ui.add_space(10.0);
        let shown = format!("{:.*}{suffix}", decimals, *value);
        ui.label(RichText::new(shown).monospace().size(13.0).color(if enabled { TEXT } else { MUTED }));
        settled = r.drag_stopped() || (r.changed() && !r.dragged());
    });
    ui.add_space(4.0);
    settled
}

impl LauncherApp {
    /// Returns true when "Change folder" was pressed (the dialog runs after the frame).
    fn settings_page(&mut self, ui: &mut egui::Ui, snap: &Snapshot, phase: PlayPhase) -> bool {
        let mut browse = false;
        let mut err = None;
        egui::Frame::none().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            page_header(ui, "SETTINGS", "How the game plays, where it lives, and how it starts.");
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(780.0);

                section(ui, "GAME", |ui| {
                    let editable = phase == PlayPhase::Idle;
                    let s = &mut self.settings;
                    let mut save = false;
                    save |= slider_row(ui, "Field of view", &mut s.fov, 70.0..=120.0, 1.0, "°", 0, editable);
                    save |= slider_row(ui, "Mouse sensitivity", &mut s.sensitivity, 0.1..=10.0, 0.05, "", 2, editable);
                    ui.add_space(8.0);
                    save |= toggle_row(
                        ui,
                        &mut s.auto_sprint,
                        "Auto sprint",
                        "Moving forward sprints on its own, like Apex's setting.",
                        editable,
                    );
                    save |= toggle_row(
                        ui,
                        &mut s.mantle_boost,
                        "Mantle boost",
                        "R5Flowstate's jump at the end of a mantle. Not in stock Apex.",
                        editable,
                    );
                    save |= toggle_row(ui, &mut s.invert_y, "Invert look", "Moving the mouse up looks down.", editable);
                    if save && let Err(e) = s.save(&self.game_root) {
                        err = Some(e);
                    }
                    ui.add_space(4.0);
                    let note = if editable {
                        "Saved to r5rust_settings.toml. Keybinds live in the game's menu (Esc)."
                    } else {
                        "Close the game to change these: it saves its own settings when it exits."
                    };
                    ui.label(RichText::new(note).size(11.0).color(if editable { MUTED } else { AMBER }));
                });

                section(ui, "GRAPHICS", |ui| {
                    let editable = phase == PlayPhase::Idle;
                    let g = &mut self.settings.graphics;
                    let mut save = false;
                    let mut preset = g.preset.clone();
                    let presets: Vec<(String, &str)> =
                        [("lowest", "LOWEST"), ("low", "LOW"), ("medium", "MEDIUM"), ("high", "HIGH"), ("ultra", "ULTRA")].iter().map(|(v, l)| (v.to_string(), *l)).collect();
                    if choice_row(ui, "Preset", &mut preset, &presets, editable) {
                        g.apply_preset(&preset);
                        save = true;
                    }
                    if g.preset == "custom" {
                        ui.label(RichText::new("Custom: the knobs below don't match a preset.").size(11.0).color(MUTED));
                    }
                    ui.add_space(6.0);
                    let mut knobs = false;
                    let levels = |n: &[&str]| -> Vec<(String, String)> { n.iter().map(|v| (v.to_string(), v.to_uppercase())).collect() };
                    let tex = levels(&game::TEXTURE_LEVELS);
                    let tex: Vec<(String, &str)> = tex.iter().map(|(v, l)| (v.clone(), l.as_str())).collect();
                    knobs |= choice_row(ui, "Texture quality", &mut g.texture_quality, &tex, editable);
                    let md = levels(&game::MODEL_LEVELS);
                    let md: Vec<(String, &str)> = md.iter().map(|(v, l)| (v.clone(), l.as_str())).collect();
                    knobs |= choice_row(ui, "Model detail", &mut g.model_detail, &md, editable);
                    let sh = levels(&game::SHADOW_LEVELS);
                    let sh: Vec<(String, &str)> = sh.iter().map(|(v, l)| (v.clone(), l.as_str())).collect();
                    knobs |= choice_row(ui, "Sun shadows", &mut g.sun_shadows, &sh, editable);
                    knobs |= choice_row(ui, "Anisotropic", &mut g.anisotropic, &[(1, "OFF"), (2, "2X"), (4, "4X"), (8, "8X"), (16, "16X")], editable);
                    knobs |= choice_row(ui, "Anti-aliasing", &mut g.msaa, &[(1, "OFF"), (4, "4X MSAA")], editable);
                    let mut scale = g.render_scale as f64;
                    if slider_row(ui, "Render scale", &mut scale, 50.0..=100.0, 5.0, "%", 0, editable) {
                        g.render_scale = scale.round() as i64;
                        save = true;
                    }
                    save |= choice_row(ui, "FPS cap", &mut g.fps_cap, &[(0, "NONE"), (60, "60"), (120, "120"), (144, "144"), (165, "165"), (240, "240"), (360, "360")], editable);
                    save |= toggle_row(ui, &mut g.vsync, "V-Sync", "Waits for the display: no tearing, a frame of extra input lag.", editable);
                    if knobs {
                        g.preset = g.detect_preset();
                        save = true;
                    }
                    if save && let Err(e) = self.settings.save(&self.game_root) {
                        err = Some(e);
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Lighting and effects always render like Apex: presets only trade detail and resolution. Texture quality applies on the next launch, the rest live in game too (Esc > Video).")
                            .size(11.0)
                            .color(MUTED),
                    );
                });

                section(ui, "LAUNCH OPTIONS", |ui| {
                    let edit = egui::TextEdit::singleline(&mut self.launch_args)
                        .hint_text(RichText::new("e.g. --fly --fov 110").color(MUTED))
                        .font(FontId::monospace(13.0))
                        .margin(Margin::symmetric(10.0, 9.0))
                        .desired_width(f32::INFINITY);
                    if ui.add(edit).changed()
                        && let Err(e) = game::write_setting("launch_args.txt", &self.launch_args)
                    {
                        err = Some(format!("Couldn't save launch options: {e}"));
                    }
                    ui.add_space(6.0);
                    let args = game::split_args(&self.launch_args)
                        .iter()
                        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() })
                        .collect::<Vec<_>>()
                        .join(" ");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("RUNS").size(9.0).color(MUTED));
                        ui.label(RichText::new(format!("r5rust.exe {args}")).monospace().size(12.0).color(CYAN));
                    });
                    ui.add_space(8.0);
                    for (flag, what) in [
                        ("--fly", "free-fly camera, no collision"),
                        ("--fov <deg>", "field of view for this run only"),
                        ("--pos x,y,z", "spawn at a map position (feet)"),
                        ("--yaw <deg>  --pitch <deg>", "spawn view angles"),
                        ("--menu", "open the in-game menu at start"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(200.0, 16.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_width(200.0);
                                    ui.label(RichText::new(flag).monospace().size(11.0).color(TEXT));
                                },
                            );
                            ui.label(RichText::new(what).size(11.0).color(MUTED));
                        });
                    }
                });

                section(ui, "GAME FOLDER", |ui| {
                    ui.label(RichText::new(self.game_root.display().to_string()).monospace().size(13.0).color(TEXT));
                    let (state, color) = match snap.state {
                        GameState::Ready => ("R5Rust project with a build ready to play", CYAN),
                        GameState::NoBuild => ("R5Rust project, nothing built yet", AMBER),
                        GameState::FolderMissing => ("This folder doesn't exist", RED),
                        GameState::InvalidFolder => ("Not an R5Rust project (no play.ps1)", RED),
                    };
                    ui.label(RichText::new(state).size(11.0).color(color));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        browse = flat_button(ui, "CHANGE FOLDER");
                        if flat_button(ui, "OPEN FOLDER") {
                            err = game::open_path(&self.game_root).err();
                        }
                    });
                });

                section(ui, "GAME FILES", |ui| {
                    let f = &snap.files;
                    let dir = f.assets_root.join("maps").join(&f.map);
                    ui.label(RichText::new(dir.display().to_string()).monospace().size(13.0).color(TEXT));
                    let (state, color) = if f.ok() {
                        ("Map extracted: world, collision, materials and lighting found.".to_owned(), CYAN)
                    } else {
                        let missing = f.missing.join(", ");
                        (format!("Missing {missing}. Install builds them from your own Apex install."), AMBER)
                    };
                    ui.label(RichText::new(state).size(11.0).color(color));
                    let art = if f.loadscreen.is_some() {
                        "Loading screen found (shown on the Play page)."
                    } else {
                        "No loading screen extracted: the Play page shows a plain backdrop."
                    };
                    ui.label(RichText::new(art).size(11.0).color(MUTED));
                    ui.add_space(10.0);
                    let (install, install_color) = match &self.install_status {
                        InstallStatus::Checking => ("Checking the install...".to_owned(), MUTED),
                        InstallStatus::Installed(d) => (format!("Installed {d}"), MUTED),
                        InstallStatus::Partial(d) => (format!("Install not finished: {d}"), AMBER),
                        InstallStatus::Stale(d) => (format!("Update needed: {d}"), AMBER),
                        InstallStatus::Missing(_) => ("Not installed yet.".to_owned(), AMBER),
                        InstallStatus::Unknown(d) => (format!("Couldn't check the install: {d}"), MUTED),
                    };
                    ui.label(RichText::new(install).size(11.0).color(install_color));
                    ui.add_space(10.0);
                    let mut recheck = false;
                    ui.horizontal(|ui| {
                        let label = if matches!(self.install_status, InstallStatus::Stale(_) | InstallStatus::Partial(_)) { "UPDATE GAME FILES" } else { "INSTALL / REINSTALL" };
                        if flat_button(ui, label) && !self.installing() {
                            self.install_open = true;
                            self.page = Page::Play;
                        }
                        if flat_button(ui, "OPEN ASSETS FOLDER") {
                            err = game::open_path(&f.assets_root).err();
                        }
                        if flat_button(ui, "CHECK AGAIN") {
                            self.poller.refresh();
                            recheck = true;
                        }
                    });
                    if recheck {
                        self.check_install(ui.ctx(), snap.files.assets_root.clone());
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Install builds the files from your Apex install (tools\\install.ps1) and points the game at them (assets_root in r5rust.toml).")
                            .size(11.0)
                            .color(MUTED),
                    );
                });

                section(ui, "LAUNCHER", |ui| {
                    if toggle_row(
                        ui,
                        &mut self.console_on_launch,
                        "Open the Console when the game starts",
                        "Jump to the game's live output on every launch.",
                        true,
                    ) && let Err(e) =
                        game::write_setting("console_on_launch.txt", if self.console_on_launch { "1" } else { "0" })
                    {
                        err = Some(format!("Couldn't save: {e}"));
                    }
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if flat_button(ui, "GAME SETTINGS FILE") {
                            err = game::open_path(&GameSettings::path(&self.game_root)).err();
                        }
                        let dir = game::data_dir().unwrap_or_default();
                        if flat_button(ui, "LAST GAME LOG") {
                            err = game::open_path(&dir.join("game.log")).err();
                        }
                        if flat_button(ui, "LAST BUILD LOG") {
                            err = game::open_path(&dir.join("build.log")).err();
                        }
                    });
                });

                section(ui, "ABOUT", |ui| {
                    ui.label(
                        RichText::new(format!(
                            "R5Rust Launcher v{}. Play runs play.ps1, which installs a staged build first. Build latest runs tools\\publish.ps1 (cargo build + stage) and never touches the copy you're playing. Press Enter to play from any page.",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .size(12.0)
                        .color(SECONDARY),
                    );
                });
                ui.add_space(24.0);
            });
        });
        if let Some(e) = err {
            self.notice = Some((e, Tone::Bad));
        }
        browse
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Window chrome and painting helpers

enum ChromeAction {
    None,
    ToggleMaximize,
    /// The title bar was dragged while maximized; the point is where it was grabbed (window points).
    DragFromMaximized(Pos2),
}

fn draw_window_chrome(ctx: &egui::Context, maximized: bool) -> ChromeAction {
    let screen = ctx.screen_rect();
    let bar_size = Vec2::new(screen.width(), 36.0);
    let mut action = ChromeAction::None;

    egui::Area::new(egui::Id::new("window-chrome"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .default_size(bar_size)
        .show(ctx, |ui| {
            let (bar, _) = ui.allocate_exact_size(bar_size, Sense::hover());
            let close = Rect::from_min_size(Pos2::new(bar.right() - 40.0, bar.top() + 3.0), Vec2::new(36.0, 30.0));
            let maximize = Rect::from_min_size(Pos2::new(close.left() - 38.0, bar.top() + 3.0), Vec2::new(36.0, 30.0));
            let minimize =
                Rect::from_min_size(Pos2::new(maximize.left() - 38.0, bar.top() + 3.0), Vec2::new(36.0, 30.0));
            let drag_area = Rect::from_min_max(bar.min, Pos2::new(minimize.left(), bar.bottom()));
            let drag = ui.interact(drag_area, egui::Id::new("window-drag-area"), Sense::click_and_drag());
            if drag.drag_started() {
                match (maximized, drag.interact_pointer_pos()) {
                    (true, Some(grab)) => action = ChromeAction::DragFromMaximized(grab),
                    _ => ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag),
                }
            }
            if drag.double_clicked() {
                action = ChromeAction::ToggleMaximize;
            }

            let min_r = ui.interact(minimize, egui::Id::new("window-minimize"), Sense::click());
            let max_r = ui.interact(maximize, egui::Id::new("window-maximize"), Sense::click());
            let close_r = ui.interact(close, egui::Id::new("window-close"), Sense::click());
            let painter = ui.painter();
            if min_r.hovered() {
                painter.rect_filled(minimize, Rounding::same(0.0), HOVER);
            }
            if max_r.hovered() {
                painter.rect_filled(maximize, Rounding::same(0.0), HOVER);
            }
            if close_r.hovered() {
                painter.rect_filled(close, Rounding::same(0.0), Color32::from_rgb(224, 68, 58));
            }
            let s = Stroke::new(1.2_f32, TEXT);
            let m = minimize.center();
            painter.line_segment([Pos2::new(m.x - 5.0, m.y + 3.0), Pos2::new(m.x + 5.0, m.y + 3.0)], s);
            if maximized {
                // Restore: two overlapping squares.
                let front = Rect::from_center_size(maximize.center() + Vec2::new(-1.5, 1.5), Vec2::splat(9.0));
                let back = front.translate(Vec2::new(3.0, -3.0));
                painter.line_segment([Pos2::new(back.left(), back.top()), Pos2::new(back.right(), back.top())], s);
                painter.line_segment([Pos2::new(back.right(), back.top()), Pos2::new(back.right(), back.bottom())], s);
                painter.rect_stroke(front, Rounding::same(0.0), s);
            } else {
                painter.rect_stroke(maximize.shrink(10.0), Rounding::same(0.0), s);
            }
            let c = close.center();
            painter.line_segment([Pos2::new(c.x - 5.0, c.y - 5.0), Pos2::new(c.x + 5.0, c.y + 5.0)], s);
            painter.line_segment([Pos2::new(c.x + 5.0, c.y - 5.0), Pos2::new(c.x - 5.0, c.y + 5.0)], s);

            if min_r.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            if max_r.clicked() {
                action = ChromeAction::ToggleMaximize;
            }
            if close_r.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    action
}

/// The work area (screen minus taskbar) of the monitor under a point, in physical pixels.
#[cfg(windows)]
fn work_area_at(x: i32, y: i32) -> Option<Rect> {
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    };
    // SAFETY: MonitorFromPoint never fails with DEFAULTTONEAREST; MONITORINFO is zeroed and sized as required.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let w = info.rcWork;
        Some(Rect::from_min_max(
            Pos2::new(w.left as f32, w.top as f32),
            Pos2::new(w.right as f32, w.bottom as f32),
        ))
    }
}

#[cfg(not(windows))]
fn work_area_at(_x: i32, _y: i32) -> Option<Rect> {
    None
}

fn paint_vertical_scrim(painter: &egui::Painter, bounds: Rect, top_alpha: u8, bottom_alpha: u8) {
    let (t, b) = (Color32::from_black_alpha(top_alpha), Color32::from_black_alpha(bottom_alpha));
    paint_scrim_colors(painter, bounds, [t, t, b, b]);
}

fn paint_horizontal_scrim(painter: &egui::Painter, bounds: Rect, left_alpha: u8, right_alpha: u8) {
    let (l, r) = (Color32::from_black_alpha(left_alpha), Color32::from_black_alpha(right_alpha));
    paint_scrim_colors(painter, bounds, [l, r, r, l]);
}

/// Colours for the corners top-left, top-right, bottom-right, bottom-left.
fn paint_scrim_colors(painter: &egui::Painter, b: Rect, c: [Color32; 4]) {
    let mut mesh = egui::Mesh::default();
    for (pos, color) in [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()].into_iter().zip(c) {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn cover_uv(image_size: Vec2, target_size: Vec2) -> Rect {
    let image_aspect = image_size.x / image_size.y;
    let target_aspect = target_size.x / target_size.y;
    if image_aspect > target_aspect {
        let crop = (1.0 - target_aspect / image_aspect) * 0.5;
        Rect::from_min_max(Pos2::new(crop, 0.0), Pos2::new(1.0 - crop, 1.0))
    } else {
        let crop = (1.0 - image_aspect / target_aspect) * 0.5;
        Rect::from_min_max(Pos2::new(0.0, crop), Pos2::new(1.0, 1.0 - crop))
    }
}

fn apply_theme(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("barlow-condensed-bold", &include_bytes!("../assets/fonts/BarlowCondensed-Bold.ttf")[..]),
        ("inter", &include_bytes!("../assets/fonts/Inter.ttf")[..]),
        ("jetbrains-mono", &include_bytes!("../assets/fonts/JetBrainsMono.ttf")[..]),
    ] {
        fonts.font_data.insert(name.to_owned(), egui::FontData::from_static(bytes));
    }
    fonts.families.insert(egui::FontFamily::Name("Barlow Condensed".into()), vec!["barlow-condensed-bold".to_owned()]);
    fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "inter".to_owned());
    fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "jetbrains-mono".to_owned());
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = RAISED;
    v.override_text_color = Some(TEXT);
    // Text selection and the slider's trailing fill share this colour.
    v.selection.bg_fill = Color32::from_rgb(40, 140, 168);
    v.selection.stroke = Stroke::new(1.0_f32, CYAN);
    v.text_cursor.stroke = Stroke::new(2.0_f32, CYAN);
    v.slider_trailing_fill = true;
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    for (w, fill, fg) in [
        (&mut v.widgets.inactive, RAISED, SECONDARY),
        (&mut v.widgets.hovered, HOVER, TEXT),
        (&mut v.widgets.active, HOVER, TEXT),
        (&mut v.widgets.open, HOVER, TEXT),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke.color = fg;
        w.rounding = Rounding::same(0.0);
        w.expansion = 0.0;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, CYAN);
    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    style.spacing.slider_width = 360.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.interact_size.y = 22.0;
    ctx.set_style(style);
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod game;
mod presence;
mod update;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Key, Layout, Margin, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2,
};
use game::{Commit, GameSettings, GameState, InstallStatus, InstallView, Job, JobKind, Poller, Snapshot, StepState};

// -------------------------------------------------------------------------------------------------------------------
// Design tokens. One charcoal ramp, one red accent, three status colours; every page uses these and nothing else.

/// Surfaces, darkest to lightest.
const BG: Color32 = Color32::from_rgb(11, 12, 14);
const RAIL: Color32 = Color32::from_rgb(8, 9, 11);
const PANEL: Color32 = Color32::from_rgb(20, 22, 25);
const RAISED: Color32 = Color32::from_rgb(31, 34, 39);
const HOVER: Color32 = Color32::from_rgb(44, 48, 55);
const BORDER: Color32 = Color32::from_rgb(45, 49, 56);
/// Text: primary, secondary, muted (captions, hints).
const TEXT: Color32 = Color32::from_rgb(243, 244, 246);
const SECONDARY: Color32 = Color32::from_rgb(188, 192, 199);
const MUTED: Color32 = Color32::from_rgb(128, 134, 143);
/// The accent: the active page, Play, selections, progress.
const ACCENT: Color32 = Color32::from_rgb(226, 44, 52);
const ACCENT_HI: Color32 = Color32::from_rgb(246, 74, 80);
/// Behind active / selected things that aren't the primary action (the active nav item).
const ACCENT_TINT: Color32 = Color32::from_rgb(58, 18, 22);
/// Status (never the accent, so "error" and "selected" don't look alike).
const GOOD: Color32 = Color32::from_rgb(98, 208, 142);
const AMBER: Color32 = Color32::from_rgb(242, 180, 76);
const BAD: Color32 = Color32::from_rgb(255, 128, 112);
const INK: Color32 = Color32::from_rgb(10, 12, 14);
/// The rail logo's text keeps its v0.2 colours exactly (the logo block is unchanged on purpose).
const LOGO_TEXT: Color32 = Color32::from_rgb(242, 246, 250);
const LOGO_MUTED: Color32 = Color32::from_rgb(138, 152, 171);

/// Spacing scale (px): 4 / 8 / 12 / 16 / 24 / 32 / 48.
const SP_XS: f32 = 4.0;
const SP_S: f32 = 8.0;
const SP_M: f32 = 12.0;
const SP_L: f32 = 16.0;
const SP_XL: f32 = 24.0;
const SP_2XL: f32 = 32.0;
const SP_3XL: f32 = 48.0;

/// Corners: controls (buttons, chips, fields) and cards. Switches are pills.
const R_CTRL: f32 = 3.0;
const R_CARD: f32 = 4.0;

/// Type scale. Barlow Condensed (display, always upper case) for short labels; Inter for sentences.
const T_HERO: f32 = 76.0;
const T_PAGE: f32 = 50.0;
const T_UPDATE: f32 = 30.0;
const T_SECTION: f32 = 21.0;
const T_TITLE: f32 = 18.0;
const T_BODY: f32 = 14.0;
const T_SMALL: f32 = 12.5;
const T_CAPTION: f32 = 11.0;
const T_MONO: f32 = 12.5;

const RAIL_W: f32 = 232.0;
const FOOTER_H: f32 = 92.0;
/// Every page's content gutter and top edge (under the window buttons).
const GUTTER: f32 = 32.0;
const PAGE_TOP: f32 = 48.0;
const PAGE_MARGIN: Margin = Margin { left: GUTTER, right: GUTTER, top: PAGE_TOP, bottom: 0.0 };
/// Settings label column.
const LABEL_W: f32 = 180.0;

fn main() -> eframe::Result<()> {
    // Apps & features > Uninstall
    if std::env::args().any(|a| a == "--uninstall") {
        update::uninstall();
        return Ok(());
    }
    // first run of a downloaded exe: install to %LOCALAPPDATA%\Programs\R5Rust Launcher, add shortcuts, start that copy
    if update::self_install() {
        return Ok(());
    }
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

/// The game's white gear mark (from the launcher icon) as a white-on-transparent texture, so it can sit on any colour.
fn mark_image() -> egui::ColorImage {
    let icon = image::load_from_memory(include_bytes!("../assets/app-icon.png")).expect("bundled icon").to_rgba8();
    let pixels = icon
        .pixels()
        .map(|p| {
            let lum = (p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11) * p[3] as f32 / 255.0;
            let a = ((lum - 40.0) / 190.0).clamp(0.0, 1.0);
            Color32::from_white_alpha((a * 255.0) as u8)
        })
        .collect();
    egui::ColorImage { size: [icon.width() as usize, icon.height() as usize], pixels }
}

fn display_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Barlow Condensed".into()))
}

/// Small upper-case label with a little tracking (section kickers, field labels).
fn caption(text: &str) -> RichText {
    RichText::new(text.to_uppercase()).size(T_CAPTION).extra_letter_spacing(0.9)
}

fn caption_galley(painter: &egui::Painter, text: &str, size: f32, color: Color32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat { font_id: FontId::proportional(size), extra_letter_spacing: 0.9, color, ..Default::default() },
    );
    painter.layout_job(job)
}

/// Paints a caption anchored at `pos`; returns where it went.
fn paint_caption(painter: &egui::Painter, pos: Pos2, align: Align2, text: &str, color: Color32) -> Rect {
    let galley = caption_galley(painter, text, T_CAPTION, color);
    let rect = align.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

fn color_image(image: image::DynamicImage) -> egui::ColorImage {
    let rgba = image.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba)
}

/// One map in a mode's list. `playable: false` is shown (art, name) but can't be picked yet.
struct MapDef {
    /// The map's bsp name: its loading screen is read from `<assets>\images\ui\maps_<bsp>_loadscreen\`.
    bsp: &'static str,
    name: &'static str,
    note: &'static str,
    playable: bool,
}

/// A game mode and its maps. More modes / maps: add entries here, the Play page and maps panel follow.
struct ModeDef {
    name: &'static str,
    kicker: &'static str,
    maps: &'static [MapDef],
}

const MODES: &[ModeDef] = &[ModeDef {
    name: "Firing Range",
    kicker: "Training mode",
    maps: &[
        MapDef { bsp: "mp_rr_canyonlands_staging_mu1", name: "Firing Range", note: "The current range", playable: true },
        MapDef {
            bsp: "mp_rr_canyonlands_staging",
            name: "Firing Range (original)",
            note: "The launch-era range. Not ported yet",
            playable: false,
        },
    ],
}];

/// Maps carousel: tile size, gap, pitch (tile + gap) and the arrow buttons' width.
const STRIP_TILE_W: f32 = 176.0;
const STRIP_TILE_H: f32 = 99.0;
const STRIP_GAP: f32 = 10.0;
const STRIP_PITCH: f32 = STRIP_TILE_W + STRIP_GAP;
const STRIP_ARROW: f32 = 30.0;

/// What the maps strip asked for this frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StripAction {
    None,
    Close,
    OpenGrid,
}

/// FPS cap presets (0 = unlimited). Any other value 20..=1000 is a custom cap (the game's own range).
const FPS_PRESETS: [(i64, &str); 7] =
    [(0, "Unlimited"), (60, "60"), (120, "120"), (144, "144"), (165, "165"), (240, "240"), (360, "360")];
const FPS_RANGE: std::ops::RangeInclusive<i64> = 20..=1000;
/// Stretched-resolution presets ([0, 0] = native). Custom: any width x height in the game's range.
const RES_PRESETS: [([i64; 2], &str); 9] = [
    ([0, 0], "Native"),
    ([1920, 1080], "1920 x 1080"),
    ([1728, 1080], "1728 x 1080"),
    ([1680, 1050], "1680 x 1050"),
    ([1600, 900], "1600 x 900"),
    ([1440, 1080], "1440 x 1080"),
    ([1300, 1070], "1300 x 1070"),
    ([1280, 960], "1280 x 960"),
    ([1024, 768], "1024 x 768"),
];
const RES_W: std::ops::RangeInclusive<i64> = 320..=7680;
const RES_H: std::ops::RangeInclusive<i64> = 240..=4320;

/// "180", "180 fps", "0" / "none" / "unlimited" (= 0). None when out of range or not a number.
fn parse_fps(text: &str) -> Option<i64> {
    let t = text.trim().to_ascii_lowercase();
    if matches!(t.as_str(), "none" | "unlimited" | "off") {
        return Some(0);
    }
    let v: i64 = t.trim_end_matches("fps").trim().parse().ok()?;
    (v == 0 || FPS_RANGE.contains(&v)).then_some(v)
}

fn parse_dim(text: &str, range: &std::ops::RangeInclusive<i64>) -> Option<i64> {
    text.trim().parse().ok().filter(|v| range.contains(v))
}

/// The custom-value fields shared by the Settings rows and the Play chips (one value, edited in either place).
#[derive(Default)]
struct CustomInputs {
    fps_open: bool,
    fps: String,
    res_open: bool,
    res: [String; 2],
    /// Focus the field on the frame it's revealed.
    focus: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Play,
    Notes,
    Mods,
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
    push: bool,
}

/// The Console's filtered view, kept between frames: only new lines are scanned while the filter stays the same.
#[derive(Default)]
struct ConsoleView {
    source: usize,
    len: usize,
    filter: String,
    problems_only: bool,
    shown: Vec<usize>,
    warnings: usize,
    errors: usize,
}

struct LauncherApp {
    game_root: PathBuf,
    poller: Poller,
    page: Page,
    notice: Option<(String, Tone)>,
    mode_menu_open: bool,
    /// Index into MODES.
    mode: usize,
    /// The maps carousel strip (top right of the map card) and the "All maps" grid over the map card.
    maps_open: bool,
    maps_grid_open: bool,
    /// Carousel scroll in tiles: `strip_pos` eases towards `strip_target` (unbounded: the strip loops).
    strip_pos: f32,
    strip_target: f32,
    /// Centre the selected map the next time the strip lays out (set when it opens).
    strip_center: bool,
    wheel_acc: f32,
    /// `--demo-maps=N` (screenshots only): a longer made-up list to show the looping carousel.
    demo_maps: Option<&'static [MapDef]>,
    /// Loading screens of the current mode's other maps (None: not extracted), loaded off the UI thread on demand.
    thumbs: std::collections::HashMap<String, Option<(egui::TextureHandle, Vec2)>>,
    thumbs_pending: HashSet<String>,
    thumbs_tx: mpsc::Sender<(String, Option<egui::ColorImage>)>,
    thumbs_rx: Receiver<(String, Option<egui::ColorImage>)>,
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
    console_view: ConsoleView,
    settings: GameSettings,
    /// The map's loading screen from the player's extracted files; nothing of the game ships with the launcher.
    art: Option<(egui::TextureHandle, Vec2)>,
    art_path: Option<PathBuf>,
    art_rx: Option<Receiver<Option<egui::ColorImage>>>,
    /// The launcher's own backdrop texture (assets\bg-grunge.png), decoded off the UI thread.
    backdrop: Option<(egui::TextureHandle, Vec2)>,
    backdrop_rx: Option<Receiver<egui::ColorImage>>,
    /// Patch-note days the player opened (the newest one shows its highlights either way).
    notes_open: HashSet<String>,
    /// Mods page: tab, grid / list view, search text, and the mod lists (empty until the game loads mods).
    mods_tab: ModsTab,
    mods_view: ModsView,
    mods_search: String,
    installed_mods: Vec<Mod>,
    browse_mods: Vec<Mod>,
    /// Display mode (r5rust.toml `fullscreen`) and the custom-value fields of the quick chips.
    fullscreen: bool,
    custom: CustomInputs,
    /// The rail logo: the game's banner (assets\icon.png), unchanged from v0.2.
    logo: egui::TextureHandle,
    /// The game's gear mark, white on transparent (placeholders when there's no map art).
    mark: egui::TextureHandle,
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
    /// Player mode: what GitHub Releases has (game builds, install tools, launcher versions).
    feed: Arc<Mutex<update::Feed>>,
    /// A download running in the background (a game build, or a launcher update).
    update_task: Option<(UpdateKind, update::Task)>,
    autodownloaded: bool,
    /// player mode: last GitHub check, and the updates already tried automatically (once each)
    last_check: Instant,
    auto_tried: HashSet<String>,
    /// dev mode: Push build (the dialog, its patch notes, and the push job)
    push_open: bool,
    push_notes: String,
    /// ping the Update Ping role for this push (big updates only; off every time the dialog opens)
    push_ping: bool,
    push_job: Option<Job>,
    /// frames spent waiting on a scripted --download / --update-launcher in --screenshot mode
    test_wait: u32,
    /// Discord Rich Presence while the game runs (Settings > Launcher; discord_rpc.txt, on unless "0").
    presence: presence::Presence,
    rpc_on: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdateKind {
    Game,
    Launcher,
}

impl LauncherApp {
    fn new(ctx: &egui::Context) -> Self {
        apply_theme(ctx);
        let logo = ctx.load_texture(
            "r5rust-logo",
            color_image(image::load_from_memory(include_bytes!("../assets/icon.png")).expect("bundled icon")),
            egui::TextureOptions::LINEAR,
        );
        let mark = ctx.load_texture("r5rust-mark", mark_image(), egui::TextureOptions::LINEAR);
        let game_root = game::load_game_root();
        // `--page=notes|console|settings` opens on that page (handy for shortcuts and screenshots).
        let page = match std::env::args().find_map(|a| a.strip_prefix("--page=").map(str::to_owned)).as_deref() {
            Some("notes") => Page::Notes,
            Some("console") => Page::Console,
            Some("settings") => Page::Settings,
            Some("mods") => Page::Mods,
            _ => Page::Play,
        };
        let (tx, backdrop_rx) = mpsc::channel();
        let repaint = ctx.clone();
        thread::spawn(move || {
            if let Ok(img) = image::load_from_memory(include_bytes!("../assets/bg-grunge.png")) {
                let _ = tx.send(color_image(img));
                repaint.request_repaint();
            }
        });
        let (thumbs_tx, thumbs_rx) = mpsc::channel();
        let mut app = Self {
            mode: 0,
            // `--open-maps`: open the maps panel on start (screenshots)
            maps_open: std::env::args().any(|a| a == "--open-maps" || a == "--open-maps-grid"),
            // `--open-maps-grid`: open the All maps grid on start (screenshots)
            maps_grid_open: std::env::args().any(|a| a == "--open-maps-grid"),
            strip_pos: 0.0,
            strip_target: 0.0,
            strip_center: true,
            wheel_acc: 0.0,
            demo_maps: None,
            thumbs: Default::default(),
            thumbs_pending: HashSet::new(),
            thumbs_tx,
            thumbs_rx,
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
            console_view: ConsoleView::default(),
            art: None,
            art_path: None,
            art_rx: None,
            backdrop: None,
            backdrop_rx: Some(backdrop_rx),
            notes_open: HashSet::new(),
            // `--mods-tab=browse` / `--mods-view=list`: open the Mods page that way (screenshots)
            mods_tab: if std::env::args().any(|a| a == "--mods-tab=browse") { ModsTab::Browse } else { ModsTab::Installed },
            mods_view: if std::env::args().any(|a| a == "--mods-view=list") { ModsView::List } else { ModsView::Grid },
            mods_search: String::new(),
            installed_mods: Vec::new(),
            browse_mods: Vec::new(),
            fullscreen: game::read_fullscreen(&game::load_game_root()),
            custom: CustomInputs::default(),
            logo,
            mark,
            autoplay: std::env::args().any(|a| a == "--autoplay"),
            restore_rect: None,
            start_maximized: std::env::args().any(|a| a == "--maximized"),
            screenshot: std::env::args().find_map(|a| a.strip_prefix("--screenshot=").map(PathBuf::from)),
            frames: 0,
            feed: Arc::new(Mutex::new(update::Feed::default())),
            update_task: None,
            autodownloaded: false,
            last_check: Instant::now(),
            auto_tried: HashSet::new(),
            // `--open-push`: open the Push build dialog on start (screenshots)
            push_open: std::env::args().any(|a| a == "--open-push"),
            push_notes: String::new(),
            push_ping: false,
            push_job: None,
            test_wait: 0,
            presence: presence::Presence::start(),
            rpc_on: game::read_setting("discord_rpc.txt").is_none_or(|v| v != "0"),
        };
        // what's on GitHub (screenshots stay offline unless asked: `--check-updates`)
        if app.screenshot.is_none() || std::env::args().any(|a| a == "--check-updates") {
            update::check(ctx.clone(), app.feed.clone());
        }
        if app.screenshot.is_some() {
            // `--demo-maps=N`: N made-up maps (cycling the real ones' art) to show the looping carousel
            if let Some(n) = std::env::args().find_map(|a| a.strip_prefix("--demo-maps=").and_then(|v| v.parse::<usize>().ok())) {
                let base = MODES[app.mode].maps;
                let list: Vec<MapDef> = (0..n.max(1))
                    .map(|i| {
                        let b = &base[i % base.len()];
                        let name: &'static str = Box::leak(format!("Demo map {}", i + 1).into_boxed_str());
                        MapDef { bsp: b.bsp, name, note: b.note, playable: b.playable }
                    })
                    .collect();
                app.demo_maps = Some(Box::leak(list.into_boxed_slice()));
            }
            // `--strip-pos=2.5`: carousel scrolled to that tile offset (mid-scroll screenshots)
            if let Some(p) = std::env::args().find_map(|a| a.strip_prefix("--strip-pos=").and_then(|v| v.parse::<f32>().ok())) {
                (app.strip_pos, app.strip_target, app.strip_center) = (p, p, false);
            }
            // `--preview-fps=180` / `--preview-res=1366x768`: show a value in the UI without saving it (screenshots)
            for arg in std::env::args() {
                if let Some(v) = arg.strip_prefix("--preview-fps=").and_then(parse_fps) {
                    app.settings.graphics.fps_cap = v;
                }
                if let Some((w, h)) = arg.strip_prefix("--preview-res=").and_then(|v| v.split_once('x')) {
                    if let (Some(w), Some(h)) = (parse_dim(w, &RES_W), parse_dim(h, &RES_H)) {
                        app.settings.graphics.stretch = [w, h];
                    }
                }
            }
        }
        // `--custom-open`: reveal both custom fields (screenshots)
        if std::env::args().any(|a| a == "--custom-open") {
            let g = &app.settings.graphics;
            let (fps, stretch) = (g.fps_cap, g.stretch);
            app.custom.toggle_fps(fps);
            app.custom.toggle_res(stretch);
            app.custom.focus = false;
            // `--preview-input=<text>`: what's typed in the FPS field (checking the invalid state in a screenshot)
            if let Some(text) = std::env::args().find_map(|a| a.strip_prefix("--preview-input=").map(str::to_owned)) {
                app.custom.fps = text;
            }
        }
        app
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

    fn updating(&self, kind: UpdateKind) -> bool {
        self.update_task.as_ref().is_some_and(|(k, _)| *k == kind)
    }

    /// Player mode: the newest published build when it's newer than the one on disk (or none is there yet).
    fn available_build(&self, snap: &Snapshot) -> Option<update::Release> {
        let feed = self.feed.lock().unwrap();
        let latest = feed.newest_build()?;
        // the release tag the folder was unpacked from (release.txt); older zips had none: compare the commit hash
        if let Some(have) = &snap.release {
            return (*have != latest.tag).then(|| latest.clone());
        }
        let have = snap.newest().map(|b| b.hash.as_str()).unwrap_or("");
        (have.is_empty() || !same_hash(have, latest.build_hash())).then(|| latest.clone())
    }

    /// Player mode, hands off: re-check GitHub every 30 minutes, take a new launcher at start, download new builds
    /// as soon as they're out (play.ps1 swaps them in on the next Play). The buttons stay for doing it by hand.
    fn auto_update(&mut self, ctx: &egui::Context, snap: &Snapshot, phase: PlayPhase) {
        if snap.dev || self.screenshot.is_some() {
            return;
        }
        if self.update_task.is_none() && self.last_check.elapsed() > Duration::from_secs(30 * 60) {
            self.last_check = Instant::now();
            update::check(ctx.clone(), self.feed.clone());
        }
        if self.update_task.is_some() || self.installing() || phase != PlayPhase::Idle {
            return;
        }
        let (launcher_tag, build_tag) = {
            let f = self.feed.lock().unwrap();
            (f.launcher_update().map(|r| r.tag.clone()), f.newest_build().map(|r| r.tag.clone()))
        };
        // a newer launcher first (it restarts into itself), once per version so a failure can't loop
        if let Some(tag) = launcher_tag.filter(|t| !update::is_dev_exe() && self.auto_tried.insert(format!("launcher {t}"))) {
            let _ = tag;
            self.update_launcher(ctx);
            return;
        }
        if self.available_build(snap).is_some() && build_tag.is_some_and(|t| self.auto_tried.insert(format!("build {t}"))) {
            self.update_game(ctx);
        }
    }

    /// Downloads the newest build (and the install tools it needs) into the game folder, off the UI thread.
    fn update_game(&mut self, ctx: &egui::Context) {
        let (release, all) = {
            let feed = self.feed.lock().unwrap();
            (feed.newest_build().cloned(), feed.game.clone())
        };
        let Some(release) = release else {
            self.notice = Some(("No game build is published yet.".into(), Tone::Bad));
            return;
        };
        let root = self.game_root.clone();
        let task = update::Task::spawn(ctx.clone(), move |p| update::install_build(&root, &release, &all, p));
        self.update_task = Some((UpdateKind::Game, task));
        self.notice = None;
    }

    /// Downloads the newer launcher, swaps it in, and restarts into it.
    fn update_launcher(&mut self, ctx: &egui::Context) {
        let Some(release) = self.feed.lock().unwrap().launcher_update().cloned() else { return };
        let task = update::Task::spawn(ctx.clone(), move |p| update::self_update(&release, p).map(|exe| exe.display().to_string()));
        self.update_task = Some((UpdateKind::Launcher, task));
    }

    fn poll_update(&mut self, ctx: &egui::Context) {
        let Some((kind, task)) = &self.update_task else { return };
        let Some(result) = task.take_result() else {
            ctx.request_repaint_after(Duration::from_millis(200));
            return;
        };
        let kind = *kind;
        self.update_task = None;
        match (kind, result) {
            (UpdateKind::Game, Ok(summary)) => {
                self.notice = Some((summary, Tone::Good));
                self.settings = GameSettings::load(&self.game_root);
                self.poller.refresh();
                self.check_install(ctx, self.poller.get().files.assets_root.clone());
            }
            (UpdateKind::Launcher, Ok(exe)) => {
                if std::process::Command::new(&exe).spawn().is_ok() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    self.notice = Some(("Launcher updated: restart it to use the new version.".into(), Tone::Good));
                }
            }
            (UpdateKind::Game, Err(e)) => self.notice = Some((format!("Download failed: {e}"), Tone::Bad)),
            (UpdateKind::Launcher, Err(e)) => self.notice = Some((format!("Launcher update failed: {e}"), Tone::Bad)),
        }
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
        // `--screenshot=... --idle`: draw the idle state even while a game runs (checking the UI; nothing is launched)
        if self.screenshot.is_some() && std::env::args().any(|a| a == "--idle") {
            return PlayPhase::Idle;
        }
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
        if let Some(job) = &mut self.push_job
            && job.poll()
        {
            self.notice = if job.succeeded() {
                Some(("Build pushed: players' launchers pick it up by themselves, and #patch-notes has it.".into(), Tone::Good))
            } else {
                let why = job.failure_line().unwrap_or_else(|| "see push.log".into());
                Some((format!("Push failed: {why}"), Tone::Bad))
            };
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
        if self.play_job.is_some() || self.building() || self.installing() || self.push_job.as_ref().is_some_and(Job::running) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// While the maps panel is open: loads the other maps' loading screens (small, off the UI thread, once each).
    fn sync_thumbs(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        while let Ok((bsp, img)) = self.thumbs_rx.try_recv() {
            self.thumbs_pending.remove(&bsp);
            let tex = img.map(|img| {
                let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
                (ctx.load_texture(format!("map-thumb-{bsp}"), img, egui::TextureOptions::LINEAR), size)
            });
            self.thumbs.insert(bsp, tex);
        }
        if !self.maps_open {
            return;
        }
        for map in self.maps() {
            if self.thumbs.contains_key(map.bsp) || self.thumbs_pending.contains(map.bsp) {
                continue;
            }
            self.thumbs_pending.insert(map.bsp.to_owned());
            let (tx, ctx, bsp, assets) = (self.thumbs_tx.clone(), ctx.clone(), map.bsp.to_owned(), snap.files.assets_root.clone());
            thread::spawn(move || {
                let img = game::loadscreen_for(&assets, &bsp)
                    .and_then(|p| image::open(p).ok())
                    // about twice the biggest tile, so the (mipmap-less) downscale stays clean
                    .map(|i| color_image(i.resize_to_fill(512, 288, image::imageops::FilterType::Lanczos3)));
                let _ = tx.send((bsp, img));
                ctx.request_repaint();
            });
        }
    }

    /// Loads the map's loading screen off the UI thread whenever the file it points at changes.
    fn sync_art(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        if let Some(rx) = &self.backdrop_rx
            && let Ok(img) = rx.try_recv()
        {
            let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
            self.backdrop = Some((ctx.load_texture("backdrop", img, egui::TextureOptions::LINEAR), size));
            self.backdrop_rx = None;
        }
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
                            let i = if i.width() > 2560 { i.resize(2560, 1440, image::imageops::FilterType::Triangle) } else { i };
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
    /// `--screenshot`: the window takes no input at all, so a mouse that happens to be over it on the live desktop
    /// can't scroll, hover, select or click anything (and can never press Play).
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.screenshot.is_some() {
            raw.events.retain(|e| matches!(e, egui::Event::Screenshot { .. }));
            raw.hovered_files.clear();
            raw.dropped_files.clear();
            raw.focused = false;
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_jobs(ctx);
        // `--install-demo`: run install.ps1 -Demo (made-up progress, touches nothing) to check the install view
        if self.frames == 0 && self.install_job.is_none() && std::env::args().any(|a| a == "--install-demo") {
            if let Ok(job) = Job::start(JobKind::Install, &self.game_root, &["-Demo".to_owned()], game::log_path("install-demo.log"), ctx.clone()) {
                self.install_job = Some(job);
            }
            self.frames = self.frames.max(1);
        }
        if let Some(path) = self.screenshot.clone() {
            self.frames += 1;
            // once the first install check and the backdrop are in (or after ~10 s)
            // a scripted `--download` / `--update-launcher`: wait until it has started and finished (up to ~30 min)
            let want_dl = std::env::args().any(|a| a == "--download");
            let want_up = std::env::args().any(|a| a == "--update-launcher");
            let waiting = (want_dl || want_up) && self.test_wait < 60_000 && {
                let snap0 = self.poller.get();
                let (checked, launcher_new) = {
                    let f = self.feed.lock().unwrap();
                    (f.checked, f.launcher_update().is_some())
                };
                let trigger_pending = !self.autodownloaded
                    && (!checked || (want_dl && self.available_build(&snap0).is_some()) || (want_up && launcher_new));
                self.update_task.is_some() || trigger_pending
            };
            if waiting {
                self.frames = self.frames.min(20);
                self.test_wait += 1;
            }
            let ready = self.install_status != InstallStatus::Checking
                && self.backdrop_rx.is_none()
                && self.art_rx.is_none()
                && self.thumbs_pending.is_empty()
                && !waiting;
            if self.frames > 20 && (ready || self.frames == 330) && self.frames < 1000 {
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
        self.sync_thumbs(ctx, &snap);
        if self.install_status == InstallStatus::Checking && self.install_rx.is_none() {
            self.check_install(ctx, snap.files.assets_root.clone());
        }
        self.sync_install(&snap);
        self.paint_backdrop(ctx);
        let phase = self.phase(&snap);
        // Discord: "Playing R5Rust / Firing Range / Loading into Game" while it starts, "In Game" once it's up
        let rpc = (self.rpc_on && self.screenshot.is_none() && phase != PlayPhase::Idle).then(|| presence::Playing {
            map: MODES[self.mode].maps.iter().find(|m| m.playable).map_or(MODES[self.mode].name, |m| m.name).to_owned(),
            state: if phase == PlayPhase::Starting { "Loading into Game" } else { "In Game" }.to_owned(),
        });
        self.presence.set(rpc);
        let can_play = snap.is_ready() && snap.files.ok() && phase == PlayPhase::Idle && !self.installing();
        self.poll_update(ctx);
        self.auto_update(ctx, &snap, phase);
        let can_build = if snap.dev {
            matches!(snap.state, GameState::Ready | GameState::NoBuild)
                && !self.building()
                && self.game_root.join("tools/publish.ps1").is_file()
        } else {
            // player: download when there's something newer and nothing else is busy
            self.update_task.is_none() && !self.installing() && phase == PlayPhase::Idle && self.available_build(&snap).is_some()
        };

        let mut action = FooterAction::default();
        let mut browse_clicked = false;

        self.rail(ctx, &snap, phase);
        egui::TopBottomPanel::bottom("play-footer")
            .exact_height(FOOTER_H)
            .show_separator_line(false)
            .frame(egui::Frame::none().fill(glass(PANEL, 240)).inner_margin(Margin::symmetric(GUTTER, 18.0)))
            .show(ctx, |ui| {
                let r = ui.max_rect().expand2(Vec2::new(GUTTER, 18.0));
                ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0_f32, BORDER));
                action = self.footer(ui, &snap, phase, can_play, can_build);
            });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::TRANSPARENT).inner_margin(0.0)).show(ctx, |ui| {
            match self.page {
                Page::Play => browse_clicked = self.play_page(ui, ctx, &snap, phase),
                Page::Notes => self.notes_page(ui, &snap),
                Page::Console => self.console_page(ui, phase),
                Page::Mods => self.mods_page(ui),
                Page::Settings => browse_clicked = self.settings_page(ui, &snap, phase),
            }
        });

        if self.build_panel_open && self.build_job.is_some() {
            self.build_panel(ctx);
        }
        if self.push_open {
            self.push_dialog(ctx, &snap);
        }
        match draw_window_chrome(ctx, self.restore_rect.is_some()) {
            ChromeAction::None => {}
            ChromeAction::ToggleMaximize => self.toggle_maximize(ctx),
            ChromeAction::DragFromMaximized(grab) => self.drag_from_maximized(ctx, grab),
        }

        if can_play && !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(Key::Enter)) {
            action.play = true;
        }
        // `--download`: press Download/Update once it's possible (scripted tests of player mode)
        if can_build && !snap.dev && self.update_task.is_none() && std::env::args().any(|a| a == "--download") && !self.autodownloaded {
            self.autodownloaded = true;
            action.build = true;
        }
        // `--update-launcher`: take a newer launcher once the check finds one (scripted tests of the self-update)
        if self.update_task.is_none() && !self.autodownloaded && std::env::args().any(|a| a == "--update-launcher") && !update::is_dev_exe() {
            let has = self.feed.lock().unwrap().launcher_update().is_some();
            if has {
                self.autodownloaded = true;
                self.update_launcher(ctx);
            }
        }
        if self.autoplay && can_play {
            self.autoplay = false;
            action.play = true;
        }
        if browse_clicked {
            self.browse();
        }
        if action.build {
            if snap.dev { self.build(ctx) } else { self.update_game(ctx) }
        }
        if action.stop {
            self.stop();
        }
        if action.push {
            self.push_open = true;
            self.push_ping = false;
        }
        if action.play {
            self.launch(ctx);
        }
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Backdrop: Kainz's grunge texture (toned down) stretched to cover the window, and a vignette. No decorative shapes:
// red is only for real UI accents (Kainz, 2026-11-21: diagonal streaks read as AI slop).

impl LauncherApp {
    fn paint_backdrop(&self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        let p = ctx.layer_painter(egui::LayerId::background());
        p.rect_filled(screen, Rounding::ZERO, BG);
        if let Some((tex, size)) = &self.backdrop {
            p.image(tex.id(), screen, cover_uv(*size, screen.size()), Color32::from_gray(200));
        }
        // vignette
        let edge = 180.0;
        let dark = Color32::from_black_alpha(110);
        let clear = Color32::TRANSPARENT;
        let s = screen;
        paint_gradient(&p, Rect::from_min_max(s.min, Pos2::new(s.right(), s.top() + edge)), [dark, dark, clear, clear]);
        paint_gradient(&p, Rect::from_min_max(Pos2::new(s.left(), s.bottom() - edge), s.max), [clear, clear, dark, dark]);
        paint_gradient(&p, Rect::from_min_max(Pos2::new(s.right() - edge, s.top()), s.max), [clear, dark, dark, clear]);
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Icons: one line style (1.6 px at 16 px, round-ish joins), drawn on a 16-unit grid centred on `c`.

#[derive(Clone, Copy)]
enum Icon {
    Play,
    Notes,
    Console,
    Gear,
    Calendar,
    ChevronDown,
    ChevronUp,
    ChevronRight,
    ChevronLeft,
    Close,
    Doc,
    Folder,
    Copy,
    Check,
    Refresh,
    Grid,
    Lock,
    Box,
    Search,
}

fn paint_icon(p: &egui::Painter, icon: Icon, c: Pos2, size: f32, color: Color32) {
    let k = size / 16.0;
    let s = Stroke::new(1.6 * k.max(0.8), color);
    let v = |x: f32, y: f32| c + Vec2::new(x * k, y * k);
    let line = |pts: Vec<Pos2>| p.add(egui::Shape::line(pts, s));
    match icon {
        Icon::Play => {
            p.add(egui::Shape::closed_line(vec![v(-4.5, -6.5), v(6.5, 0.0), v(-4.5, 6.5)], s));
        }
        Icon::Notes => {
            for y in [-5.0, 0.0, 5.0] {
                p.circle_filled(v(-6.0, y), 1.1 * k, color);
                line(vec![v(-2.5, y), v(7.0, y)]);
            }
        }
        Icon::Console => {
            p.rect_stroke(Rect::from_min_max(v(-7.0, -6.0), v(7.0, 6.0)), Rounding::same(2.0 * k), s);
            line(vec![v(-4.0, -2.0), v(-1.5, 0.5), v(-4.0, 3.0)]);
            line(vec![v(0.5, 3.0), v(4.0, 3.0)]);
        }
        Icon::Gear => {
            let mut pts = Vec::with_capacity(32);
            for i in 0..8 {
                let a0 = i as f32 * std::f32::consts::TAU / 8.0;
                for (da, r) in [(-0.23, 5.0), (-0.14, 7.2), (0.14, 7.2), (0.23, 5.0)] {
                    let a = a0 + da;
                    pts.push(v(a.cos() * r, a.sin() * r));
                }
            }
            p.add(egui::Shape::closed_line(pts, s));
            p.circle_stroke(c, 2.2 * k, s);
        }
        Icon::Calendar => {
            p.rect_stroke(Rect::from_min_max(v(-6.5, -4.5), v(6.5, 6.5)), Rounding::same(1.5 * k), s);
            line(vec![v(-6.5, -0.5), v(6.5, -0.5)]);
            line(vec![v(-3.0, -7.0), v(-3.0, -3.0)]);
            line(vec![v(3.0, -7.0), v(3.0, -3.0)]);
        }
        Icon::ChevronDown => {
            line(vec![v(-4.5, -2.0), v(0.0, 2.5), v(4.5, -2.0)]);
        }
        Icon::ChevronUp => {
            line(vec![v(-4.5, 2.0), v(0.0, -2.5), v(4.5, 2.0)]);
        }
        Icon::ChevronRight => {
            line(vec![v(-2.0, -4.5), v(2.5, 0.0), v(-2.0, 4.5)]);
        }
        Icon::ChevronLeft => {
            line(vec![v(2.0, -4.5), v(-2.5, 0.0), v(2.0, 4.5)]);
        }
        Icon::Close => {
            line(vec![v(-5.0, -5.0), v(5.0, 5.0)]);
            line(vec![v(5.0, -5.0), v(-5.0, 5.0)]);
        }
        Icon::Doc => {
            p.add(egui::Shape::closed_line(vec![v(-5.0, -7.0), v(2.0, -7.0), v(5.5, -3.5), v(5.5, 7.0), v(-5.0, 7.0)], s));
            line(vec![v(-2.0, 0.0), v(2.5, 0.0)]);
            line(vec![v(-2.0, 3.5), v(2.5, 3.5)]);
        }
        Icon::Folder => {
            p.add(egui::Shape::closed_line(
                vec![v(-7.0, -5.0), v(-2.5, -5.0), v(-1.0, -3.0), v(7.0, -3.0), v(7.0, 6.0), v(-7.0, 6.0)],
                s,
            ));
        }
        Icon::Copy => {
            p.rect_stroke(Rect::from_min_max(v(-2.5, -2.5), v(6.5, 6.5)), Rounding::same(1.5 * k), s);
            line(vec![v(-6.5, 2.5), v(-6.5, -6.5), v(2.5, -6.5)]);
        }
        Icon::Check => {
            line(vec![v(-5.5, 0.5), v(-1.5, 4.5), v(5.5, -4.0)]);
        }
        Icon::Box => {
            p.add(egui::Shape::closed_line(vec![v(0.0, -7.0), v(6.5, -3.5), v(6.5, 3.5), v(0.0, 7.0), v(-6.5, 3.5), v(-6.5, -3.5)], s));
            line(vec![v(-6.5, -3.5), v(0.0, 0.0), v(6.5, -3.5)]);
            line(vec![v(0.0, 0.0), v(0.0, 7.0)]);
        }
        Icon::Search => {
            p.circle_stroke(v(-1.5, -1.5), 5.0 * k, s);
            line(vec![v(2.2, 2.2), v(6.5, 6.5)]);
        }
        Icon::Grid => {
            for (x, y) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                p.rect_stroke(Rect::from_center_size(v(x, y), Vec2::splat(5.0 * k)), Rounding::same(1.0 * k), s);
            }
        }
        Icon::Lock => {
            p.rect_stroke(Rect::from_min_max(v(-5.5, -1.0), v(5.5, 7.0)), Rounding::same(1.5 * k), s);
            let mut pts = vec![v(-3.2, -1.0)];
            pts.extend((0..=12).map(|i| {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
                v(a.cos() * 3.2, -4.0 + a.sin() * 3.2)
            }));
            pts.push(v(3.2, -1.0));
            line(pts);
            p.circle_filled(v(0.0, 3.0), 1.0 * k, color);
        }
        Icon::Refresh => {
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = -0.6 + i as f32 / 20.0 * 4.9;
                    v(a.cos() * 6.0, a.sin() * 6.0)
                })
                .collect();
            let end = *pts.last().unwrap();
            line(pts);
            p.add(egui::Shape::line(vec![end + Vec2::new(-3.5, -0.8) * k, end, end + Vec2::new(0.6, -3.6) * k], s));
        }
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Rail

impl LauncherApp {
    fn rail(&mut self, ctx: &egui::Context, snap: &Snapshot, phase: PlayPhase) {
        egui::SidePanel::left("launcher-rail")
            .exact_width(RAIL_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::none().fill(glass(RAIL, 222)).inner_margin(0.0))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                let p = ui.painter().clone();
                p.vline(full.right() - 0.5, full.y_range(), Stroke::new(1.0_f32, BORDER));

                // brand: the game's banner logo, drawn exactly as in v0.2 (Kainz's call: keep it, don't recolour it)
                ui.add_space(38.0);
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    ui.add(egui::Image::new((self.logo.id(), Vec2::splat(46.0))).fit_to_exact_size(Vec2::splat(46.0)));
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(3.0);
                        ui.label(RichText::new("R5RUST").font(display_font(28.0)).color(LOGO_TEXT));
                        ui.label(RichText::new("LAUNCHER").size(10.0).color(LOGO_MUTED));
                    });
                });
                ui.add_space(36.0);
                let unbuilt = snap.unbuilt.filter(|n| *n > 0).map(|n| Badge::New(format!("{n} NEW")));
                let live = (phase != PlayPhase::Idle).then_some(Badge::Live);
                for (page, label, icon, badge) in [
                    (Page::Play, "Play", Icon::Play, None),
                    (Page::Notes, "Patch Notes", Icon::Notes, unbuilt),
                    (Page::Mods, "Mods", Icon::Box, None),
                    (Page::Console, "Console", Icon::Console, live),
                    (Page::Settings, "Settings", Icon::Gear, None),
                ] {
                    if nav_item(ui, label, icon, self.page == page, badge) {
                        self.page = page;
                        self.mode_menu_open = false;
                    }
                    ui.add_space(SP_XS);
                }

                // The version block sits in a band level with the footer, its top edge on the footer's.
                let band_top = full.bottom() - FOOTER_H;
                p.rect_filled(Rect::from_min_max(Pos2::new(full.left(), band_top), full.max), Rounding::ZERO, glass(RAIL, 200));
                p.hline(full.x_range(), band_top + 0.5, Stroke::new(1.0_f32, BORDER));
                let build = snap.current.as_ref().map(|b| b.hash.as_str()).unwrap_or("none");
                let launcher = format!("v{}", env!("CARGO_PKG_VERSION"));
                // a newer launcher on GitHub: the version turns into a button (installed copies only, not cargo runs)
                let newer = (!update::is_dev_exe()).then(|| self.feed.lock().unwrap().launcher_update().map(|r| r.tag.clone())).flatten();
                for (i, (key, value)) in [("Game", build), ("Launcher", launcher.as_str())].into_iter().enumerate() {
                    let y = band_top + FOOTER_H / 2.0 + (i as f32 - 0.5) * 24.0;
                    paint_caption(&p, Pos2::new(full.left() + SP_XL, y), Align2::LEFT_CENTER, key, MUTED);
                    match (&newer, i) {
                        (Some(tag), 1) => {
                            let text = if self.updating(UpdateKind::Launcher) { "UPDATING...".to_owned() } else { format!("UPDATE {tag}") };
                            let galley = p.layout_no_wrap(text, FontId::proportional(T_SMALL), ACCENT_HI);
                            let r = Rect::from_min_size(Pos2::new(full.right() - SP_XL - galley.size().x - 16.0, y - 11.0), Vec2::new(galley.size().x + 16.0, 22.0));
                            let resp = ui.interact(r, egui::Id::new("launcher-update"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                            p.rect(r, Rounding::same(R_CTRL), if resp.hovered() { ACCENT_TINT } else { Color32::TRANSPARENT }, Stroke::new(1.0_f32, ACCENT));
                            p.galley(r.center() - galley.size() / 2.0, galley, ACCENT_HI);
                            if resp.clicked() && self.update_task.is_none() {
                                self.update_launcher(ui.ctx());
                            }
                        }
                        _ => {
                            p.text(Pos2::new(full.right() - SP_XL, y), Align2::RIGHT_CENTER, value, FontId::monospace(T_MONO), SECONDARY);
                        }
                    }
                }
            });
    }
}

enum Badge {
    New(String),
    Live,
}

fn nav_item(ui: &mut egui::Ui, label: &str, icon: Icon, active: bool, badge: Option<Badge>) -> bool {
    let (slot, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
    let rect = slot.shrink2(Vec2::new(SP_M, 0.0));
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered() && !active, 0.12);
    let p = ui.painter();
    if active {
        p.rect_filled(rect, Rounding::same(R_CTRL), ACCENT_TINT);
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
            Rounding { nw: R_CTRL, sw: R_CTRL, ne: 0.0, se: 0.0 },
            ACCENT,
        );
    } else if hover > 0.0 {
        p.rect_filled(rect, Rounding::same(R_CTRL), glass(HOVER, (hover * 150.0) as u8));
    }
    let color = if active { TEXT } else { mix(SECONDARY, TEXT, hover) };
    paint_icon(p, icon, Pos2::new(rect.left() + 26.0, rect.center().y), 16.0, if active { ACCENT_HI } else { color });
    p.text(Pos2::new(rect.left() + 48.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(T_BODY + 0.5), color);
    match badge {
        Some(Badge::New(text)) => {
            pill(p, Pos2::new(rect.right() - SP_M, rect.center().y), Align2::RIGHT_CENTER, &text, PillStyle::Accent);
        }
        Some(Badge::Live) => {
            let r = paint_caption(p, Pos2::new(rect.right() - SP_M, rect.center().y), Align2::RIGHT_CENTER, "Live", GOOD);
            p.circle_filled(Pos2::new(r.left() - 8.0, r.center().y), 3.0, GOOD);
        }
        None => {}
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// `c` at `alpha`: see-through surfaces over the backdrop.
fn glass(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PillStyle {
    /// Accent fill, white text (LATEST, N NEW).
    Accent,
    /// Status colour fill, dark text (PLAYING).
    Solid(Color32),
    /// Hairline outline in a status colour (STAGED, NOT BUILT).
    Outline(Color32),
}

/// A small status label in a 20 px pill-ish box (control corners). Returns its rect.
fn pill(p: &egui::Painter, pos: Pos2, align: Align2, text: &str, style: PillStyle) -> Rect {
    let (fg, fill, stroke) = match style {
        PillStyle::Accent => (Color32::WHITE, ACCENT, Stroke::NONE),
        PillStyle::Solid(c) => (INK, c, Stroke::NONE),
        PillStyle::Outline(c) => (c, Color32::TRANSPARENT, Stroke::new(1.0_f32, c.gamma_multiply(0.8))),
    };
    let galley = caption_galley(p, text, 10.0, fg);
    let size = Vec2::new(galley.size().x + 14.0, 20.0);
    let rect = align.anchor_size(pos, size);
    p.rect(rect.shrink(0.5), Rounding::same(R_CTRL), fill, stroke);
    p.galley(rect.center() - galley.size() / 2.0, galley, fg);
    rect
}

fn ui_pill(ui: &mut egui::Ui, text: &str, style: PillStyle) {
    let galley = caption_galley(ui.painter(), text, 10.0, TEXT);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 14.0, 20.0), Sense::hover());
    pill(ui.painter(), rect.center(), Align2::CENTER_CENTER, text, style);
}

// -------------------------------------------------------------------------------------------------------------------
// Footer: what's installed, what's pending, and the two actions.

impl LauncherApp {
    fn footer(&self, ui: &mut egui::Ui, snap: &Snapshot, phase: PlayPhase, can_play: bool, can_build: bool) -> FooterAction {
        let mut action = FooterAction::default();
        let left_w = (ui.available_width() - 200.0 - 200.0 - SP_M - SP_3XL).max(200.0);
        ui.horizontal(|ui| {
            let (bar, _) = ui.allocate_exact_size(Vec2::new(3.0, ui.available_height()), Sense::hover());
            ui.painter().rect_filled(bar, Rounding::same(1.5), ACCENT);
            ui.add_space(SP_M);
            ui.allocate_ui_with_layout(Vec2::new(left_w, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                ui.set_width(left_w);
                ui.spacing_mut().item_spacing.y = 3.0;
                let not_downloaded = !snap.dev && !matches!(snap.state, GameState::Ready);
                let title = match snap.state {
                    _ if not_downloaded => "Not downloaded yet",
                    GameState::Ready if snap.current.is_some() => "Installed build",
                    GameState::Ready => "Staged build",
                    GameState::NoBuild => "No build yet",
                    GameState::FolderMissing | GameState::InvalidFolder => "Setup required",
                };
                ui.label(caption(title).color(ACCENT_HI));

                let body = |ui: &mut egui::Ui, text: &str| {
                    ui.add(egui::Label::new(RichText::new(text).size(T_BODY + 0.5).color(TEXT)).truncate());
                };
                match (snap.state, snap.current.as_ref().or(snap.next.as_ref())) {
                    _ if not_downloaded => body(ui, "Press Download to get the latest build."),
                    (GameState::Ready, Some(build)) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&build.hash).monospace().size(T_BODY).color(ACCENT_HI));
                            body(ui, &build.subject);
                        });
                    }
                    (GameState::Ready, None) => body(ui, "A build is staged (no version info)."),
                    (GameState::NoBuild, _) => body(ui, "Press Build latest to compile the game."),
                    (GameState::FolderMissing, _) => body(ui, "The game folder doesn't exist. Pick it in Settings."),
                    (GameState::InvalidFolder, _) => body(ui, "That folder has no play.ps1. Pick the r5rust folder in Settings."),
                }

                let (line, color) = self.status_line(snap, phase);
                ui.add(egui::Label::new(RichText::new(line).size(T_SMALL).color(color)).truncate());
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
                    paint_progress_line(ui, response.rect, ACCENT_HI);
                }
                if response.clicked() {
                    match phase {
                        PlayPhase::Idle => action.play = true,
                        PlayPhase::Ours | PlayPhase::Starting => action.stop = true,
                        PlayPhase::Elsewhere => {}
                    }
                }
                ui.add_space(SP_M);
                action.build = self.build_button(ui, snap, can_build);
                if snap.dev {
                    ui.add_space(SP_M);
                    action.push = self.push_button(ui, snap);
                }
            });
        });
        action
    }

    /// Dev: "PUSH BUILD": sends players the build you're playing (opens the patch-notes dialog first).
    fn push_button(&self, ui: &mut egui::Ui, snap: &Snapshot) -> bool {
        let pushing = self.push_job.as_ref().is_some_and(Job::running);
        let enabled = snap.current.is_some() && !pushing;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(150.0, 56.0), Sense::click());
        let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
        let hover = ui.ctx().animate_bool_with_time(response.id, enabled && response.hovered(), 0.12);
        let p = ui.painter();
        p.rect(rect.shrink(0.5), Rounding::same(R_CTRL), mix(RAISED, HOVER, hover), Stroke::new(1.0_f32, mix(BORDER, Color32::from_rgb(70, 76, 86), hover)));
        let (title, detail, dc) = match (&self.push_job, &snap.current) {
            (Some(j), _) if j.running() => ("PUSHING", fmt_duration(j.elapsed()), SECONDARY),
            (_, Some(b)) => ("PUSH BUILD", format!("{} to players", b.hash), MUTED),
            (_, None) => ("PUSH BUILD", "play a build first".to_owned(), MUTED),
        };
        p.text(rect.left_top() + Vec2::new(SP_L, 10.0), Align2::LEFT_TOP, title, display_font(T_TITLE + 2.0), if enabled || pushing { TEXT } else { MUTED });
        p.text(rect.left_top() + Vec2::new(SP_L, 34.0), Align2::LEFT_TOP, detail, FontId::proportional(11.5), dc);
        if pushing {
            paint_progress_line(ui, rect, ACCENT_HI);
        }
        enabled && response.clicked()
    }

    /// The Push build dialog: which build goes out, its patch notes (one short line each), Push / Cancel.
    fn push_dialog(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        let Some(build) = snap.current.clone() else {
            self.push_open = false;
            return;
        };
        let screen = ctx.screen_rect();
        let size = Vec2::new(560.0, 360.0);
        let pos = screen.center() - size / 2.0;
        let (mut push, mut cancel) = (false, false);
        egui::Area::new(egui::Id::new("push-dim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, Rounding::ZERO, Color32::from_black_alpha(150));
            cancel |= resp.clicked();
        });
        egui::Area::new(egui::Id::new("push-dialog")).order(egui::Order::Tooltip).fixed_pos(pos).show(ctx, |ui| {
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(14, 15, 18, 250))
                .stroke(Stroke::new(1.0_f32, BORDER))
                .rounding(Rounding::same(R_CARD))
                .inner_margin(Margin::same(SP_L))
                .show(ui, |ui| {
                    ui.set_width(size.x - 2.0 * SP_L);
                    ui.spacing_mut().item_spacing.y = SP_S;
                    ui.label(RichText::new("PUSH BUILD TO PLAYERS").font(display_font(T_TITLE + 4.0)).color(TEXT));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&build.hash).monospace().size(T_BODY).color(ACCENT_HI));
                        ui.add(egui::Label::new(RichText::new(&build.subject).size(T_BODY).color(SECONDARY)).truncate());
                    });
                    ui.label(
                        RichText::new("The build you're playing goes out exactly as it is. Players' launchers download it by themselves; R5Rust Bot posts it in #patch-notes.")
                            .size(T_SMALL)
                            .color(MUTED),
                    );
                    if let Some(next) = &snap.next {
                        ui.label(
                            RichText::new(format!(
                                "A newer build ({}) is staged but you haven't played it yet. To push that one: close this, press Play, try it, then push.",
                                next.hash
                            ))
                            .size(T_SMALL)
                            .color(AMBER),
                        );
                    }
                    ui.add_space(SP_XS);
                    ui.label(caption("PATCH NOTES").color(SECONDARY));
                    ui.add(
                        egui::TextEdit::multiline(&mut self.push_notes)
                            .hint_text("One short line per change, like:\nFixed the sun flickering\nZiplines are faster to grab")
                            .desired_rows(6)
                            .desired_width(f32::INFINITY),
                    );
                    toggle_row(
                        ui,
                        &mut self.push_ping,
                        "Ping players",
                        "Big updates only: pings everyone with Update Ping. Off, the build is posted quietly.",
                        true,
                    );
                    ui.add_space(SP_XS);
                    ui.horizontal(|ui| {
                        push = big_button(ui, "PUSH", BigStyle::Primary, true).clicked();
                        ui.add_space(SP_M);
                        cancel |= ghost_button(ui, "Cancel", None);
                    });
                });
        });
        if cancel || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.push_open = false;
        }
        if push {
            self.push_open = false;
            self.start_push(ctx);
        }
    }

    fn start_push(&mut self, ctx: &egui::Context) {
        let notes = game::log_path("push-notes.txt");
        if let Err(e) = std::fs::write(&notes, &self.push_notes) {
            self.notice = Some((format!("Couldn't save the patch notes: {e}"), Tone::Bad));
            return;
        }
        let mut args = vec!["-NotesFile".to_owned(), notes.display().to_string()];
        if self.push_ping {
            args.push("-Ping".to_owned());
        }
        self.push_ping = false;
        match Job::start(JobKind::Push, &self.game_root, &args, game::log_path("push.log"), ctx.clone()) {
            Ok(job) => {
                self.push_job = Some(job);
                self.notice = Some(("Pushing the build to players...".into(), Tone::Info));
            }
            Err(e) => self.notice = Some((e, Tone::Bad)),
        }
    }

    /// The footer's one-sentence status, most important first.
    fn status_line(&self, snap: &Snapshot, phase: PlayPhase) -> (String, Color32) {
        if let Some((text, tone)) = &self.notice {
            let color = match tone {
                Tone::Info => SECONDARY,
                Tone::Good => GOOD,
                Tone::Bad => BAD,
            };
            (text.clone(), color)
        } else if phase == PlayPhase::Starting {
            ("Starting the game...".to_owned(), GOOD)
        } else if phase == PlayPhase::Ours {
            ("Playing. Press Stop to quit.".to_owned(), GOOD)
        } else if phase == PlayPhase::Elsewhere {
            ("R5Rust is already running (started outside the launcher).".to_owned(), GOOD)
        } else if let Some(job) = self.install_job.as_ref().filter(|j| j.running()) {
            let view = InstallView::parse(&job.marks.lock().unwrap());
            match view.current() {
                Some((i, s)) => (format!("Installing game files: {} ({}/{}).", s.title, i + 1, view.steps.len()), SECONDARY),
                None => ("Installing game files...".to_owned(), SECONDARY),
            }
        } else if let Some((_, task)) = &self.update_task {
            let p = &task.progress;
            let pct = p.fraction().map(|f| format!(" {:.0}%", f * 100.0)).unwrap_or_default();
            (format!("{}...{pct}", p.stage()), SECONDARY)
        } else if !snap.dev && !snap.is_ready() {
            let feed = self.feed.lock().unwrap();
            match (&feed.error, feed.checked, feed.newest_build()) {
                (_, _, Some(b)) => (format!("Build {} ({}) is ready to download.", b.build_hash(), b.date), MUTED),
                (Some(e), _, None) => (format!("Couldn't reach GitHub: {e}"), AMBER),
                (None, true, None) => ("No game build is published yet.".to_owned(), AMBER),
                (None, false, None) => ("Checking for builds...".to_owned(), MUTED),
            }
        } else if snap.is_ready() && !snap.files.ok() {
            ("Game files missing: press Install on the Play page.".to_owned(), AMBER)
        } else if matches!(self.install_status, InstallStatus::Stale(_)) {
            ("Apex or the installer changed: update the game files in Settings > Game files.".to_owned(), AMBER)
        } else if let (Some(next), Some(_)) = (&snap.next, &snap.current) {
            (format!("Update {} staged. It installs when you press Play.", next.hash), GOOD)
        } else if let Some(b) = self.available_build(snap).filter(|_| !snap.dev) {
            (format!("Update {} ({}) is available: press Update.", b.build_hash(), b.date), ACCENT_HI)
        } else if let Some(n) = snap.unbuilt.filter(|n| *n > 0) {
            (format!("{n} newer commit{} not built yet.", plural(n)), MUTED)
        } else if let Some(b) = snap.newest() {
            (format!("Ready to play. Built {}.", b.date), MUTED)
        } else {
            (String::new(), MUTED)
        }
    }

    fn build_button(&self, ui: &mut egui::Ui, snap: &Snapshot, enabled: bool) -> bool {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(200.0, 56.0), Sense::click());
        let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
        let hover = ui.ctx().animate_bool_with_time(response.id, enabled && response.hovered(), 0.12);
        let p = ui.painter();
        let pending = if snap.dev { snap.unbuilt.is_some_and(|n| n > 0) || snap.state == GameState::NoBuild } else { enabled };
        let border = if pending && enabled { ACCENT } else { mix(BORDER, Color32::from_rgb(70, 76, 86), hover) };
        p.rect(rect.shrink(0.5), Rounding::same(R_CTRL), mix(RAISED, HOVER, hover), Stroke::new(1.0_f32, border));
        let downloading = self.updating(UpdateKind::Game);
        let (title, detail, detail_color) = if !snap.dev {
            let first = !snap.is_ready();
            let verb = if first { "DOWNLOAD" } else { "UPDATE" };
            if let Some((_, task)) = self.update_task.as_ref().filter(|_| downloading) {
                let p = &task.progress;
                let mb = |b: u64| b as f64 / (1u64 << 20) as f64;
                let detail = match p.fraction() {
                    Some(_) if p.stage().starts_with("Unpacking") => "unpacking".to_owned(),
                    Some(_) => format!("{:.0} / {:.0} MB", mb(p.done.load(std::sync::atomic::Ordering::Relaxed)), mb(p.total.load(std::sync::atomic::Ordering::Relaxed))),
                    None => "starting".to_owned(),
                };
                ("DOWNLOADING", detail, SECONDARY)
            } else {
                match self.available_build(snap) {
                    Some(b) => (verb, format!("build {} · {}", b.build_hash(), b.date), ACCENT_HI),
                    None if !self.feed.lock().unwrap().checked => (verb, "checking...".to_owned(), MUTED),
                    None if first => (verb, "nothing published yet".to_owned(), AMBER),
                    None => ("UP TO DATE", "latest build".to_owned(), MUTED),
                }
            }
        } else if let Some(job) = self.build_job.as_ref().filter(|j| j.running()) {
            ("BUILDING", format!("cargo build  {}", fmt_duration(job.elapsed())), SECONDARY)
        } else if snap.state == GameState::NoBuild {
            ("BUILD LATEST", "no build yet".to_owned(), AMBER)
        } else {
            match snap.unbuilt {
                Some(0) => ("BUILD LATEST", "up to date".to_owned(), MUTED),
                Some(n) => ("BUILD LATEST", format!("{n} new commit{}", plural(n)), ACCENT_HI),
                None => ("BUILD LATEST", "compile + stage".to_owned(), MUTED),
            }
        };
        let title_color = if enabled || self.building() { TEXT } else { MUTED };
        p.text(rect.left_top() + Vec2::new(SP_L, 10.0), Align2::LEFT_TOP, title, display_font(T_TITLE + 2.0), title_color);
        p.text(rect.left_top() + Vec2::new(SP_L, 34.0), Align2::LEFT_TOP, detail, FontId::proportional(11.5), detail_color);
        if self.building() {
            paint_progress_line(ui, rect, ACCENT_HI);
        }
        if let Some((_, task)) = self.update_task.as_ref().filter(|_| downloading) {
            match task.progress.fraction() {
                Some(f) => {
                    let bar = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), Pos2::new(rect.left() + rect.width() * f.clamp(0.0, 1.0), rect.bottom()));
                    ui.painter().rect_filled(bar, Rounding::ZERO, ACCENT_HI);
                }
                None => paint_progress_line(ui, rect, ACCENT_HI),
            }
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

/// The one loud thing on screen: 200x56, hover brightens, the press darkens.
fn big_button(ui: &mut egui::Ui, label: &str, style: BigStyle, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(200.0, 56.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id, enabled && response.hovered(), 0.12);
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, stroke, text) = match style {
        BigStyle::Primary => (mix(ACCENT, ACCENT_HI, hover), Stroke::NONE, Color32::WHITE),
        BigStyle::Stop => (mix(RAISED, ACCENT_TINT, hover), Stroke::new(1.0_f32, ACCENT), TEXT),
        BigStyle::Off => (RAISED, Stroke::new(1.0_f32, BORDER), MUTED),
    };
    let fill = if pressed { fill.gamma_multiply(0.85) } else { fill };
    ui.painter().rect(rect.shrink(0.5), Rounding::same(R_CTRL), fill, stroke);
    ui.painter().text(rect.center() + Vec2::new(0.0, 1.0), Align2::CENTER_CENTER, label, display_font(30.0), text);
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
    ui.painter().rect_filled(bar, Rounding::ZERO, color);
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
    /// The map art (or a plain placeholder: the launcher ships no game imagery) filling `rect`.
    fn paint_art(&self, ui: &egui::Ui, rect: Rect, rounding: f32) {
        self.paint_cover(ui.painter(), self.art.as_ref(), rect, rounding, Color32::WHITE);
    }

    /// `art` covering `rect` (cropped to fill), tinted, through `p` (so it honours `p`'s clip); the placeholder when
    /// there's none.
    fn paint_cover(&self, p: &egui::Painter, art: Option<&(egui::TextureHandle, Vec2)>, rect: Rect, rounding: f32, tint: Color32) {
        match art {
            Some((tex, size)) => {
                let mut shape = egui::epaint::RectShape::filled(rect, Rounding::same(rounding), tint);
                shape.fill_texture_id = tex.id();
                shape.uv = cover_uv(*size, rect.size());
                p.add(shape);
            }
            None => {
                p.rect_filled(rect, Rounding::same(rounding), RAISED);
                let clip = p.with_clip_rect(rect.intersect(p.clip_rect()));
                let m = rect.height().min(rect.width()) * 0.34;
                clip.image(
                    self.mark.id(),
                    Rect::from_center_size(rect.center(), Vec2::splat(m)),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::from_white_alpha(28),
                );
            }
        }
    }

    /// Returns true when the files card asked to change the game folder.
    fn play_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snapshot, phase: PlayPhase) -> bool {
        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, Sense::hover());
        let chips_h = 44.0;
        let hero = Rect::from_min_max(
            full.min + Vec2::new(GUTTER, PAGE_TOP),
            Pos2::new(full.right() - GUTTER, full.bottom() - SP_XL - chips_h - SP_L),
        );
        let bar = Rect::from_min_size(Pos2::new(hero.left(), hero.bottom() + SP_L), Vec2::new(hero.width(), chips_h));
        let painter = ui.painter().clone();
        self.paint_art(ui, hero, R_CARD);
        // scrims: a light one under the MODES tab, a heavy one under the title, and a left one for the text column
        let shade = |a: u8| Color32::from_rgba_unmultiplied(6, 7, 9, a);
        let clip = painter.with_clip_rect(hero.shrink(1.0));
        let top = Rect::from_min_max(hero.min, Pos2::new(hero.right(), hero.top() + hero.height() * 0.3));
        paint_gradient(&clip, top, [shade(120), shade(120), shade(0), shade(0)]);
        let low = Rect::from_min_max(Pos2::new(hero.left(), hero.top() + hero.height() * 0.42), hero.max);
        paint_gradient(&clip, low, [shade(0), shade(0), shade(225), shade(225)]);
        let left = Rect::from_min_max(hero.min, Pos2::new(hero.left() + hero.width() * 0.6, hero.bottom()));
        paint_gradient(&clip, left, [shade(130), shade(0), shade(0), shade(130)]);
        painter.rect_stroke(hero, Rounding::same(R_CARD), Stroke::new(1.0_f32, glass(Color32::WHITE, 22)));
        self.quick_bar(ui, bar, phase == PlayPhase::Idle);

        // MODES tab
        let tab = Rect::from_min_size(Pos2::new(hero.left() + SP_2XL, hero.top() + SP_XL), Vec2::new(112.0, 38.0));
        let tab_response = ui.interact(tab, egui::Id::new("mode-tab"), Sense::click());
        let tab_color = if tab_response.hovered() || self.mode_menu_open { TEXT } else { SECONDARY };
        let label = painter.text(Pos2::new(tab.left(), tab.top() + 4.0), Align2::LEFT_TOP, "MODES", display_font(24.0), tab_color);
        let chevron = if self.mode_menu_open { Icon::ChevronUp } else { Icon::ChevronDown };
        paint_icon(&painter, chevron, Pos2::new(label.right() + 14.0, label.center().y + 1.0), 14.0, tab_color);
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(tab.left(), label.bottom() + 5.0), Vec2::new(label.width() + 24.0, 3.0)),
            Rounding::same(1.5),
            ACCENT,
        );
        let toggled = tab_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        if toggled {
            self.mode_menu_open = !self.mode_menu_open;
            self.maps_open = false;
        }

        // The install card (below) covers the art; the title block stays out of its way then.
        let show = phase == PlayPhase::Idle && (self.install_job.is_some() || self.install_open || (snap.is_ready() && !snap.files.ok()));
        // Title block, built up from the bottom edge
        let x = hero.left() + SP_2XL + 2.0;
        if !show {
            let meta_y = hero.bottom() - SP_2XL;
            let meta = painter.text(
                Pos2::new(x, meta_y),
                Align2::LEFT_BOTTOM,
                format!("LOCAL SESSION   /   SOLO   /   {}", snap.files.map),
                FontId::monospace(11.5),
                MUTED,
            );
            let tagline = painter.text(
                Pos2::new(x, meta.top() - SP_M),
                Align2::LEFT_BOTTOM,
                "Move freely. Practice your movement. Make the range yours.",
                FontId::proportional(15.5),
                SECONDARY,
            );
            let mode = &MODES[self.mode];
            let title = painter.text(Pos2::new(x - 3.0, tagline.top() + 8.0), Align2::LEFT_BOTTOM, mode.name.to_uppercase(), display_font(T_HERO), TEXT);
            paint_caption(&painter, Pos2::new(x, title.top() + 12.0), Align2::LEFT_BOTTOM, mode.kicker, ACCENT_HI);
            if phase != PlayPhase::Idle {
                let pulse = 0.55 + 0.45 * (ui.input(|i| i.time) as f32 * 3.0).sin().abs();
                let dot = Pos2::new(meta.right() + 24.0, meta.center().y);
                painter.circle_filled(dot, 3.5, GOOD.gamma_multiply(pulse));
                let text = match (phase, self.play_job.as_ref()) {
                    (PlayPhase::Starting, _) => "STARTING".to_owned(),
                    (_, Some(job)) => format!("IN GAME  {}", fmt_duration(job.elapsed())),
                    _ => "IN GAME".to_owned(),
                };
                painter.text(dot + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, text, FontId::monospace(11.5), GOOD);
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }

        // Game files: one card, like R5Regenis' first-run card. It's the install form until the files are there, the
        // progress while installing, and comes back from Settings for a reinstall / update.
        let browse = false;
        if show {
            painter.rect_filled(hero, Rounding::same(R_CARD), shade(120));
            // centred on the art, using last frame's height (the card grows while installing)
            let h_id = egui::Id::new("install-card-height");
            let h: f32 = ctx.data(|d| d.get_temp(h_id)).unwrap_or(400.0);
            let top = (hero.center().y - h / 2.0).max(hero.top() + SP_XL);
            let card = Rect::from_min_size(Pos2::new(hero.center().x - 320.0, top), Vec2::new(640.0, hero.bottom() - top));
            ui.allocate_ui_at_rect(card, |ui| {
                let frame = card_frame(248).inner_margin(Margin::same(28.0));
                let r = frame.show(ui, |ui| {
                    ui.set_width(card.width() - 56.0);
                    self.install_card(ui, ctx, snap);
                });
                ctx.data_mut(|d| d.insert_temp(h_id, r.response.rect.height()));
            });
        }

        // Maps: a pill in the top-right corner of the art opens the mode's maps as a carousel strip under it; the strip's
        // "All maps" opens a grid over the whole map card (R5Flowstate's map picker, in our look)
        if !show {
            let galley = painter.layout_no_wrap("Maps".to_owned(), FontId::proportional(T_BODY - 0.5), TEXT);
            let pill_rect = Rect::from_min_size(
                Pos2::new(hero.right() - SP_XL - (galley.size().x + 48.0), hero.top() + SP_XL + 2.0),
                Vec2::new(galley.size().x + 48.0, 34.0),
            );
            let r = ui.interact(pill_rect, egui::Id::new("maps-pill"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let hover = ctx.animate_bool_with_time(r.id, r.hovered() || self.maps_open, 0.12);
            painter.rect(
                pill_rect,
                Rounding::same(17.0),
                Color32::from_rgba_unmultiplied(10, 11, 13, (170.0 + hover * 60.0) as u8),
                Stroke::new(1.0_f32, glass(Color32::WHITE, (28.0 + hover * 40.0) as u8)),
            );
            paint_icon(&painter, Icon::Grid, Pos2::new(pill_rect.left() + 22.0, pill_rect.center().y), 14.0, TEXT);
            painter.galley(Pos2::new(pill_rect.left() + 36.0, pill_rect.center().y - galley.size().y / 2.0), galley, TEXT);
            let pill_clicked = r.clicked();
            if pill_clicked {
                self.maps_open = !self.maps_open;
                self.maps_grid_open = false;
                self.strip_center = true;
                self.mode_menu_open = false;
            }
            let esc = ctx.input(|i| i.key_pressed(Key::Escape));
            if self.maps_grid_open {
                let area = egui::Area::new(egui::Id::new("maps-grid"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(hero.min)
                    .show(ctx, |ui| self.maps_grid(ui, &snap.files.map, hero.size()));
                if area.inner || esc || (!pill_clicked && area.response.clicked_elsewhere()) {
                    self.maps_grid_open = false;
                }
            } else if self.maps_open {
                // a looping strip as wide as the card allows; few maps (they fit) shrink it to just those, centred
                let n = self.maps().len() as f32;
                let max_inner = (hero.width() - 2.0 * SP_XL).min(920.0) - 2.0 * (SP_M + STRIP_ARROW + SP_S);
                let content = n * STRIP_PITCH - STRIP_GAP;
                let looping = content > max_inner;
                let inner = if looping { max_inner } else { content.max(2.0 * STRIP_PITCH - STRIP_GAP) };
                let w = inner + 2.0 * (SP_M + STRIP_ARROW + SP_S);
                let pos = Pos2::new(hero.right() - SP_XL - w, pill_rect.bottom() + SP_S + 2.0);
                let area = egui::Area::new(egui::Id::new("maps-strip"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(pos)
                    .show(ctx, |ui| self.map_strip(ui, &snap.files.map, inner, looping));
                match area.inner {
                    StripAction::OpenGrid => self.maps_grid_open = true,
                    StripAction::Close => self.maps_open = false,
                    StripAction::None if esc || (!pill_clicked && area.response.clicked_elsewhere()) => self.maps_open = false,
                    StripAction::None => {}
                }
            }
        } else {
            self.maps_open = false;
            self.maps_grid_open = false;
        }

        if self.mode_menu_open {
            let area = egui::Area::new(egui::Id::new("mode-menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(Pos2::new(tab.left(), tab.bottom() + SP_M))
                .show(ctx, |ui| self.mode_menu(ui));
            if area.inner || (!toggled && area.response.clicked_elsewhere()) {
                self.mode_menu_open = false;
            }
        }
        browse
    }

    /// Resolution / display / FPS limit / launch options, one chip each, under the map card. They write the same
    /// settings as the Settings page (r5rust_settings.toml [graphics], r5rust.toml, launch_args.txt).
    fn quick_bar(&mut self, ui: &mut egui::Ui, bar: Rect, editable: bool) {
        let mut err = None;
        ui.allocate_ui_at_rect(bar, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = SP_S;
                let g = self.settings.graphics.clone();

                // Resolution (Apex-style stretched res)
                let res = if g.stretch[0] > 0 { format!("{} x {}", g.stretch[0], g.stretch[1]) } else { "Native".to_owned() };
                let mut pick_res: Option<[i64; 2]> = None;
                quick_chip(ui, "Resolution", &res, editable, |ui| {
                    menu_hint(ui, "Stretched resolution: the game renders at this size, stretched to the window (like Apex).");
                    for (v, label) in RES_PRESETS {
                        let label = if v[0] == 0 { "Native (window size)" } else { label };
                        if menu_row(ui, label, g.stretch == v) {
                            pick_res = Some(v);
                        }
                    }
                    let custom_on = !RES_PRESETS.iter().any(|(v, _)| *v == g.stretch);
                    let label = if custom_on { format!("Custom  {} x {}", g.stretch[0], g.stretch[1]) } else { "Custom...".to_owned() };
                    if menu_row(ui, &label, custom_on) {
                        self.custom.toggle_res(g.stretch);
                    }
                    if self.custom.res_open {
                        ui.add_space(SP_XS);
                        if let Some(v) = self.custom.res_field(ui, true) {
                            pick_res = Some(v);
                        }
                        ui.add_space(SP_XS);
                    }
                });
                if let Some(v) = pick_res {
                    self.custom.res_open = false;
                    self.settings.graphics.stretch = v;
                    err = self.settings.save(&self.game_root).err();
                    ui.memory_mut(|m| m.close_popup());
                }

                // Display mode
                let mut pick_fs: Option<bool> = None;
                quick_chip(ui, "Display", if self.fullscreen { "Fullscreen" } else { "Windowed" }, editable, |ui| {
                    if menu_row(ui, "Fullscreen (borderless)", self.fullscreen) {
                        pick_fs = Some(true);
                    }
                    if menu_row(ui, "Windowed", !self.fullscreen) {
                        pick_fs = Some(false);
                    }
                    menu_hint(ui, "F11 switches in game.");
                });
                if let Some(on) = pick_fs {
                    match game::write_fullscreen(&self.game_root, on) {
                        Ok(()) => self.fullscreen = on,
                        Err(e) => err = Some(e),
                    }
                    ui.memory_mut(|m| m.close_popup());
                }

                // FPS limit
                let fps = if g.fps_cap > 0 { format!("{} FPS", g.fps_cap) } else { "Unlimited".to_owned() };
                let mut pick_fps: Option<i64> = None;
                quick_chip(ui, "FPS limit", &fps, editable, |ui| {
                    for (v, label) in FPS_PRESETS {
                        if menu_row(ui, label, g.fps_cap == v) {
                            pick_fps = Some(v);
                        }
                    }
                    let custom_on = !FPS_PRESETS.iter().any(|(v, _)| *v == g.fps_cap);
                    let label = if custom_on { format!("Custom  {}", g.fps_cap) } else { "Custom...".to_owned() };
                    if menu_row(ui, &label, custom_on) {
                        self.custom.toggle_fps(g.fps_cap);
                    }
                    if self.custom.fps_open {
                        ui.add_space(SP_XS);
                        if let Some(v) = self.custom.fps_field(ui, true) {
                            pick_fps = Some(v);
                        }
                        ui.add_space(SP_XS);
                    }
                });
                if let Some(v) = pick_fps {
                    self.custom.fps_open = false;
                    self.settings.graphics.fps_cap = v;
                    err = self.settings.save(&self.game_root).err();
                    ui.memory_mut(|m| m.close_popup());
                }

                // Launch options: quick toggles that edit the launch-args line
                const FLAGS: [(&[&str], &str, &str); 4] = [
                    (&["--fly"], "Fly mode", "free camera, no collision"),
                    (&["+cl_showfps", "1"], "Show FPS", "frame-rate counter"),
                    (&["-console"], "Developer console", "opens with the game"),
                    (&["--menu"], "Open the menu", "the game starts in its menu"),
                ];
                let on = FLAGS.iter().filter(|(f, ..)| game::has_launch_flag(&self.launch_args, f[0])).count();
                let value = if on == 0 { "Default".to_owned() } else { format!("{on} on") };
                let mut args = self.launch_args.clone();
                quick_chip(ui, "Launch", &value, true, |ui| {
                    ui.add_space(SP_XS);
                    for (flag, title, hint) in FLAGS {
                        let mut v = game::has_launch_flag(&args, flag[0]);
                        if toggle_row(ui, &mut v, title, hint, true) {
                            args = game::set_launch_flag(&args, flag, v);
                        }
                    }
                    menu_hint(ui, "More in Settings > Launch options.");
                });
                if args != self.launch_args {
                    self.launch_args = args;
                    if let Err(e) = game::write_setting("launch_args.txt", &self.launch_args) {
                        err = Some(format!("Couldn't save launch options: {e}"));
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let note = if editable { "Applies the next time the game starts" } else { "Close the game to change these" };
                    ui.label(RichText::new(note).size(T_SMALL).color(if editable { MUTED } else { AMBER }));
                });
            });
        });
        if let Some(e) = err {
            self.notice = Some((e, Tone::Bad));
        }
    }

    fn install_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snapshot) {
        if let Some(job) = &self.install_job {
            let view = InstallView::parse(&job.marks.lock().unwrap());
            let (running, elapsed, tail, log) = (job.running(), job.elapsed(), job.tail(3), job.log_path.clone());
            self.install_progress(ui, ctx, &view, running, elapsed, &tail, &log);
            return;
        }
        let (kicker, kicker_color, title) = match &self.install_status {
            InstallStatus::Stale(_) => ("Update available", AMBER, "Update the game files from your Apex install"),
            InstallStatus::Partial(_) => ("Install not finished", AMBER, "Finish building the game files"),
            _ if snap.files.ok() => ("Reinstall", ACCENT_HI, "Rebuild the game files from your Apex install"),
            _ => ("Game files missing", AMBER, "Build the Firing Range from your Apex install"),
        };
        ui.label(caption(kicker).color(kicker_color));
        ui.add_space(SP_XS);
        ui.label(RichText::new(title).size(T_TITLE + 2.0).color(TEXT));
        ui.add_space(SP_XS);
        let why = match &self.install_status {
            InstallStatus::Stale(d) | InstallStatus::Partial(d) => format!("{d}. "),
            _ => String::new(),
        };
        ui.label(
            RichText::new(format!(
                "{why}R5Rust ships no game files: it makes them from your own copy of Apex, reading its files only (never the running game). Takes about 15 minutes and 5 GB."
            ))
            .size(T_BODY)
            .color(SECONDARY),
        );
        ui.add_space(SP_L);

        let mut pick_apex = false;
        let mut pick_dir = false;
        let apex_ok = !self.apex_dir.is_empty() && Path::new(&self.apex_dir).join("paks").join("Win64").is_dir();
        let checking = self.install_status == InstallStatus::Checking;
        let apex_text = match (self.apex_dir.is_empty(), checking) {
            (false, _) => self.apex_dir.clone(),
            (true, true) => "Looking for Apex...".to_owned(),
            (true, false) => "Not found: choose the Apex folder".to_owned(),
        };
        path_row(ui, "Apex", &apex_text, apex_ok || (checking && self.apex_dir.is_empty()), &mut pick_apex);
        path_row(ui, "Save to", &self.install_dir, true, &mut pick_dir);
        ui.add_space(SP_S);
        toggle_row(
            ui,
            &mut self.apex_closed,
            "Apex is closed",
            "R5Rust never looks at the running game (anti-cheat), so it can't check: close Apex first.",
            true,
        );
        ui.add_space(SP_M);
        let can = apex_ok && self.apex_closed && !self.install_dir.trim().is_empty() && !self.building();
        let mut start = false;
        let mut close = false;
        ui.horizontal(|ui| {
            let r = big_button(ui, "INSTALL", if can { BigStyle::Primary } else { BigStyle::Off }, can);
            start = r.clicked() && can;
            ui.add_space(SP_M);
            if self.install_open && snap.files.ok() {
                ui.allocate_ui_with_layout(Vec2::new(120.0, 56.0), Layout::left_to_right(Align::Center), |ui| {
                    close = ghost_button(ui, "Cancel", None);
                });
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
            (format!("Installing  {}", fmt_duration(elapsed)), ACCENT_HI)
        } else if failed {
            ("Install failed".to_owned(), BAD)
        } else {
            ("Installed".to_owned(), GOOD)
        };
        ui.label(caption(&kicker).color(color));
        ui.add_space(SP_XS);
        let title = match (view.current(), &view.done) {
            (Some((_, s)), _) => s.title.clone(),
            (None, Some((_, m))) => m.clone(),
            _ => "Starting...".to_owned(),
        };
        ui.add(egui::Label::new(RichText::new(title).size(T_TITLE + 2.0).color(TEXT)).truncate());
        ui.add_space(SP_M);

        // progress bar (weighted by how long each step takes)
        let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
        ui.painter().rect_filled(bar, Rounding::same(3.0), RAISED);
        let f = view.fraction().clamp(0.0, 1.0);
        if f > 0.0 {
            let fill = Rect::from_min_size(bar.min, Vec2::new((bar.width() * f).max(6.0), bar.height()));
            ui.painter().rect_filled(fill, Rounding::same(3.0), if failed { BAD } else { ACCENT });
        }
        ui.add_space(SP_L);

        // the steps, two columns
        let rows = view.steps.len().div_ceil(2);
        let row_h = 24.0;
        let col_gap = SP_XL;
        let col_w = (ui.available_width() - col_gap) / 2.0;
        let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), rows as f32 * row_h), Sense::hover());
        let p = ui.painter();
        let t = ui.input(|i| i.time) as f32;
        for (i, step) in view.steps.iter().enumerate() {
            let at = area.min + Vec2::new((i / rows) as f32 * (col_w + col_gap), (i % rows) as f32 * row_h);
            let ic = at + Vec2::new(7.0, row_h / 2.0);
            let (right, text_color) = match &step.state {
                StepState::Pending => {
                    p.circle_stroke(ic, 4.5, Stroke::new(1.4_f32, BORDER));
                    (String::new(), MUTED)
                }
                StepState::Running => {
                    p.circle_filled(ic, 4.5, ACCENT_HI.gamma_multiply(0.45 + 0.55 * (t * 3.0).sin().abs()));
                    ("working".to_owned(), TEXT)
                }
                StepState::Done(secs) => {
                    paint_icon(p, Icon::Check, ic, 13.0, GOOD);
                    (format!("{secs}s"), SECONDARY)
                }
                StepState::Skipped(_) => {
                    p.circle_stroke(ic, 4.5, Stroke::new(1.4_f32, AMBER));
                    ("skipped".to_owned(), SECONDARY)
                }
                StepState::Failed(_) => {
                    paint_icon(p, Icon::Close, ic, 12.0, BAD);
                    ("failed".to_owned(), BAD)
                }
            };
            p.text(at + Vec2::new(22.0, row_h / 2.0), Align2::LEFT_CENTER, &step.title, FontId::proportional(13.5), text_color);
            p.text(at + Vec2::new(col_w, row_h / 2.0), Align2::RIGHT_CENTER, right, FontId::monospace(11.0), MUTED);
        }
        ui.add_space(SP_S);
        for tool in &view.missing_tools {
            ui.label(RichText::new(format!("Missing {tool}")).size(T_SMALL).color(AMBER));
        }
        if let Some(why) = view.steps.iter().find_map(|s| if let StepState::Skipped(w) = &s.state { Some(w) } else { None }) {
            ui.label(RichText::new(format!("Skipped: {why}")).size(T_SMALL).color(AMBER));
        }
        let shown: Vec<&String> = tail.iter().filter(|l| !l.starts_with("@@")).collect();
        if !shown.is_empty() {
            ui.add_space(SP_XS);
            egui::Frame::none().fill(glass(BG, 230)).rounding(Rounding::same(R_CTRL)).inner_margin(Margin::symmetric(SP_M, SP_S)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                for line in shown {
                    ui.add(egui::Label::new(RichText::new(line.trim()).monospace().size(11.5).color(MUTED)).truncate());
                }
            });
        }
        ui.add_space(SP_L);
        let (mut stop, mut retry, mut close, mut open_log) = (false, false, false, false);
        ui.horizontal(|ui| {
            if running {
                stop = ghost_button(ui, "Stop", Some(Icon::Close));
            } else {
                if failed {
                    retry = ghost_button(ui, "Try again", Some(Icon::Refresh));
                }
                close = ghost_button(ui, "Close", None);
            }
            open_log = ghost_button(ui, "Open log", Some(Icon::Doc));
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

    /// The selected mode's maps (or the `--demo-maps` list in screenshots).
    fn maps(&self) -> &'static [MapDef] {
        self.demo_maps.unwrap_or(MODES[self.mode].maps)
    }

    /// One map tile: art (dimmed when not playable), a shade under the name, the name, a lock badge, and the
    /// selected / hover outline. Drawn through `p`, so the strip can clip tiles at its edges.
    fn paint_map_tile(&self, p: &egui::Painter, rect: Rect, map: &MapDef, selected: bool, hover: f32, name_size: f32) {
        let art = self.thumbs.get(map.bsp).and_then(Option::as_ref).or(if selected { self.art.as_ref() } else { None });
        let tint = if map.playable { Color32::WHITE } else { Color32::from_gray(92) };
        self.paint_cover(p, art, rect, R_CTRL, tint);
        let shade = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.45), rect.max).shrink2(Vec2::new(0.5, 0.0));
        let dark = Color32::from_rgba_unmultiplied(6, 7, 9, 215);
        paint_gradient(p, shade, [Color32::TRANSPARENT, Color32::TRANSPARENT, dark, dark]);
        let pad = (rect.height() * 0.09).clamp(8.0, 14.0);
        let name_color = if map.playable { TEXT } else { SECONDARY };
        p.text(rect.left_bottom() + Vec2::new(pad, -pad + 1.0), Align2::LEFT_BOTTOM, map.name, FontId::proportional(name_size), name_color);
        if !map.playable {
            let c = Pos2::new(rect.right() - pad - 9.0, rect.top() + pad + 9.0);
            p.circle(c, 13.0, Color32::from_rgba_unmultiplied(10, 11, 13, 215), Stroke::new(1.0_f32, glass(Color32::WHITE, 40)));
            paint_icon(p, Icon::Lock, c, 13.0, SECONDARY);
        }
        let border = if selected {
            Stroke::new(2.0_f32, ACCENT)
        } else {
            Stroke::new(1.0_f32, glass(Color32::WHITE, (24.0 + hover * 70.0) as u8))
        };
        p.rect_stroke(rect, Rounding::same(R_CTRL), border);
    }

    /// The carousel: header with "All maps", arrows at both ends, mouse wheel, eased scrolling. With more maps than
    /// fit it loops (slot j shows map j mod n, the scroll position is unbounded); with fewer, they sit centred.
    fn map_strip(&mut self, ui: &mut egui::Ui, current: &str, inner: f32, looping: bool) -> StripAction {
        let maps = self.maps();
        let n = maps.len();
        let mut action = StripAction::None;
        card_frame(246).inner_margin(Margin::same(SP_M)).show(ui, |ui| {
            ui.set_width(inner + 2.0 * (STRIP_ARROW + SP_S));
            ui.horizontal(|ui| {
                ui.add_space(SP_XS);
                ui.label(caption(&format!("{} maps", MODES[self.mode].name)).color(SECONDARY));
                ui.label(RichText::new(n.to_string()).size(T_SMALL).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (rect, r) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                    if r.hovered() {
                        ui.painter().rect_filled(rect, Rounding::same(R_CTRL), HOVER);
                    }
                    paint_icon(ui.painter(), Icon::Close, rect.center(), 12.0, if r.hovered() { TEXT } else { SECONDARY });
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        action = StripAction::Close;
                    }
                    if ghost_button(ui, "All maps", Some(Icon::Grid)) {
                        action = StripAction::OpenGrid;
                    }
                });
            });
            ui.add_space(SP_S);
            let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STRIP_TILE_H + 8.0), Sense::hover());
            let area = Rect::from_min_max(Pos2::new(row.left() + STRIP_ARROW + SP_S, row.top()), Pos2::new(row.right() - STRIP_ARROW - SP_S, row.bottom()));
            let tiles_y = row.top() + 5.0;

            // arrows (only live when the strip loops)
            for (dir, rect, icon) in [
                (-1.0, Rect::from_min_size(row.min, Vec2::new(STRIP_ARROW, row.height())), Icon::ChevronLeft),
                (1.0, Rect::from_min_size(Pos2::new(row.right() - STRIP_ARROW, row.top()), Vec2::new(STRIP_ARROW, row.height())), Icon::ChevronRight),
            ] {
                let r = ui.interact(rect, ui.id().with(("strip-arrow", dir as i32)), if looping { Sense::click() } else { Sense::hover() });
                let p = ui.painter();
                if looping && r.hovered() {
                    p.rect_filled(rect, Rounding::same(R_CTRL), HOVER);
                }
                let color = if !looping { BORDER } else if r.hovered() { TEXT } else { SECONDARY };
                paint_icon(p, icon, rect.center(), 16.0, color);
                if looping && r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.strip_target += dir;
                }
            }

            let visible = (area.width() / STRIP_PITCH).floor().max(1.0);
            let selected_index = maps.iter().position(|m| m.bsp == current).unwrap_or(0) as f32;
            if self.strip_center {
                self.strip_center = false;
                let start = if looping { selected_index - ((visible - 1.0) / 2.0).floor() } else { 0.0 };
                (self.strip_pos, self.strip_target) = (start, start);
            }
            if looping {
                // mouse wheel over the strip: one tile per notch (40 px of trackpad travel)
                if ui.rect_contains_pointer(row) {
                    let d = ui.input(|i| i.raw_scroll_delta);
                    self.wheel_acc += d.y + d.x;
                    while self.wheel_acc <= -40.0 {
                        self.strip_target += 1.0;
                        self.wheel_acc += 40.0;
                    }
                    while self.wheel_acc >= 40.0 {
                        self.strip_target -= 1.0;
                        self.wheel_acc -= 40.0;
                    }
                }
                // ease towards the target; repaint only while moving
                let dt = ui.input(|i| i.stable_dt).min(0.05);
                let gap = self.strip_target - self.strip_pos;
                if gap.abs() < 0.001 {
                    self.strip_pos = self.strip_target;
                } else {
                    self.strip_pos += gap * (1.0 - (-14.0 * dt).exp());
                    ui.ctx().request_repaint();
                }
            }

            // tiles: slot j at x = (j - pos) * pitch; clipped to the area (with room for the raised selection)
            let clip = ui.painter().with_clip_rect(area.expand2(Vec2::new(0.0, 4.0)).intersect(ui.clip_rect()));
            let slots: Vec<(i64, f32)> = if looping {
                let first = self.strip_pos.floor() as i64 - 1;
                (first..first + visible as i64 + 3).map(|j| (j, area.left() + (j as f32 - self.strip_pos) * STRIP_PITCH)).collect()
            } else {
                let x0 = area.left() + (area.width() - (n as f32 * STRIP_PITCH - STRIP_GAP)) / 2.0;
                (0..n as i64).map(|j| (j, x0 + j as f32 * STRIP_PITCH)).collect()
            };
            for (j, x) in slots {
                let idx = j.rem_euclid(n as i64) as usize;
                let map = &maps[idx];
                let selected = idx as f32 == selected_index && maps[idx].bsp == current;
                let mut rect = Rect::from_min_size(Pos2::new(x, tiles_y), Vec2::new(STRIP_TILE_W, STRIP_TILE_H));
                let hit = rect.intersect(area);
                let r = if hit.width() > 4.0 {
                    let sense = if map.playable { Sense::click() } else { Sense::hover() };
                    let r = ui.interact(hit, ui.id().with(("strip-tile", j)), sense);
                    let r = if map.playable { r.on_hover_cursor(egui::CursorIcon::PointingHand) } else { r };
                    Some(r.on_hover_text(map.note))
                } else {
                    None
                };
                let hover = r.as_ref().map_or(0.0, |r| ui.ctx().animate_bool_with_time(r.id, map.playable && r.hovered(), 0.12));
                if selected {
                    rect = rect.translate(Vec2::new(0.0, -3.0));
                }
                self.paint_map_tile(&clip, rect, map, selected, hover, T_SMALL + 0.5);
                if r.is_some_and(|r| map.playable && r.clicked()) {
                    action = StripAction::Close;
                }
            }
        });
        action
    }

    /// "All maps": every map of the mode as a grid over the map card (scrolls when there are many). Returns true
    /// when it should close (X, or a playable map picked).
    fn maps_grid(&self, ui: &mut egui::Ui, current: &str, size: Vec2) -> bool {
        let maps = self.maps();
        let mut close = false;
        let selected_index = maps.iter().position(|m| m.bsp == current);
        card_frame(255).inner_margin(Margin::same(SP_XL)).show(ui, |ui| {
            let inner = size - Vec2::splat(2.0 * SP_XL);
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(caption("Choose map").color(SECONDARY));
                    let ready = maps.iter().filter(|m| m.playable).count();
                    ui.label(RichText::new(format!("{}  ·  {} maps, {ready} playable", MODES[self.mode].name, maps.len())).size(T_SMALL).color(MUTED));
                });
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    let (rect, r) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
                    if r.hovered() {
                        ui.painter().rect_filled(rect, Rounding::same(R_CTRL), HOVER);
                    }
                    paint_icon(ui.painter(), Icon::Close, rect.center(), 13.0, if r.hovered() { TEXT } else { SECONDARY });
                    close |= r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Close (Esc)").clicked();
                });
            });
            ui.add_space(SP_L);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let w = ui.available_width() - SP_S;
                let gap = SP_M;
                let cols = (((w + gap) / (205.0 + gap)).floor() as usize).max(1);
                let tile_w = (w - gap * (cols - 1) as f32) / cols as f32;
                let tile_h = (tile_w * 9.0 / 16.0).round();
                for (r_i, row) in maps.chunks(cols).enumerate() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for (c_i, map) in row.iter().enumerate() {
                            let sense = if map.playable { Sense::click() } else { Sense::hover() };
                            let (rect, r) = ui.allocate_exact_size(Vec2::new(tile_w, tile_h), sense);
                            let hover = ui.ctx().animate_bool_with_time(r.id, map.playable && r.hovered(), 0.12);
                            let selected = selected_index == Some(r_i * cols + c_i);
                            self.paint_map_tile(ui.painter(), rect, map, selected, hover, T_BODY);
                            if !map.playable {
                                pill(ui.painter(), rect.left_top() + Vec2::new(12.0, 12.0), Align2::LEFT_TOP, "Coming later", PillStyle::Outline(SECONDARY));
                            } else if selected {
                                pill(ui.painter(), rect.left_top() + Vec2::new(12.0, 12.0), Align2::LEFT_TOP, "Selected", PillStyle::Accent);
                            }
                            let r = if map.playable { r.on_hover_cursor(egui::CursorIcon::PointingHand) } else { r };
                            close |= map.playable && r.on_hover_text(map.note).clicked();
                        }
                    });
                    ui.add_space(gap - SP_S);
                }
            });
        });
        close
    }
    /// Returns true when the menu should close (a mode was picked).
    fn mode_menu(&self, ui: &mut egui::Ui) -> bool {
        let mut picked = false;
        card_frame(250).inner_margin(Margin::same(SP_L)).show(ui, |ui| {
            ui.set_width(320.0);
            ui.label(caption("Game modes").color(MUTED));
            ui.add_space(SP_XS);
            let (row, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 76.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(row, Rounding::same(R_CTRL), if response.hovered() { HOVER } else { RAISED });
            p.rect_filled(
                Rect::from_min_size(row.left_top(), Vec2::new(3.0, row.height())),
                Rounding { nw: R_CTRL, sw: R_CTRL, ne: 0.0, se: 0.0 },
                ACCENT,
            );
            let thumb = Rect::from_min_size(row.left_top() + Vec2::new(SP_M, 10.0), Vec2::new(98.0, 56.0));
            self.paint_art(ui, thumb, R_CTRL);
            let text_x = thumb.right() + SP_M;
            p.text(Pos2::new(text_x, row.top() + 20.0), Align2::LEFT_TOP, MODES[self.mode].name, FontId::proportional(T_BODY + 1.0), TEXT);
            paint_caption(p, Pos2::new(text_x, row.top() + 44.0), Align2::LEFT_TOP, "Selected", ACCENT_HI);
            paint_icon(p, Icon::Check, Pos2::new(row.right() - 20.0, row.center().y), 14.0, ACCENT_HI);
            picked = response.clicked();
            ui.add_space(SP_S);
            divider(ui);
            ui.add_space(SP_S);
            ui.label(caption("More modes").color(MUTED));
            ui.label(RichText::new("Coming later").size(T_BODY).color(SECONDARY));
        });
        picked
    }

    fn build_panel(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.build_job else { return };
        let screen = ctx.screen_rect();
        let size = Vec2::new(580.0_f32.min(screen.width() - RAIL_W - 2.0 * GUTTER), 248.0);
        let pos = Pos2::new(screen.right() - GUTTER - size.x, screen.bottom() - FOOTER_H - SP_L - size.y);
        let mut close = false;
        let mut open_log = false;
        egui::Area::new(egui::Id::new("build-panel")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let p = ui.painter();
            p.add(egui::epaint::Shadow { offset: Vec2::new(0.0, 8.0), blur: 24.0, spread: 0.0, color: Color32::from_black_alpha(140) }.as_shape(rect, Rounding::same(R_CARD)));
            p.rect(rect, Rounding::same(R_CARD), Color32::from_rgba_unmultiplied(12, 13, 16, 246), Stroke::new(1.0_f32, BORDER));
            let (status, color) = match job.exit {
                None => (format!("COMPILING  {}", fmt_duration(job.elapsed())), ACCENT_HI),
                Some(Some(0)) => (format!("STAGED IN {}", fmt_duration(job.elapsed())), GOOD),
                Some(_) => ("FAILED".to_owned(), BAD),
            };
            let head_y = rect.top() + 22.0;
            let t = p.text(Pos2::new(rect.left() + SP_L, head_y), Align2::LEFT_CENTER, "BUILD", display_font(T_TITLE + 2.0), TEXT);
            p.text(Pos2::new(t.right() + SP_M, head_y + 1.0), Align2::LEFT_CENTER, status, FontId::monospace(11.5), color);

            let close_rect = Rect::from_center_size(Pos2::new(rect.right() - 24.0, head_y), Vec2::splat(30.0));
            let close_r = ui.interact(close_rect, egui::Id::new("build-close"), Sense::click());
            if close_r.hovered() {
                p.rect_filled(close_rect, Rounding::same(R_CTRL), HOVER);
            }
            paint_icon(p, Icon::Close, close_rect.center(), 12.0, if close_r.hovered() { TEXT } else { SECONDARY });
            close = close_r.on_hover_text("Hide (the build keeps going)").clicked();

            let log_rect = Rect::from_min_max(Pos2::new(close_rect.left() - 96.0, close_rect.top()), Pos2::new(close_rect.left() - 4.0, close_rect.bottom()));
            let log_r = ui.interact(log_rect, egui::Id::new("build-log"), Sense::click());
            if log_r.hovered() {
                p.rect_filled(log_rect, Rounding::same(R_CTRL), HOVER);
            }
            let lc = if log_r.hovered() { TEXT } else { SECONDARY };
            paint_icon(p, Icon::Doc, Pos2::new(log_rect.left() + 14.0, log_rect.center().y), 13.0, lc);
            paint_caption(p, Pos2::new(log_rect.left() + 26.0, log_rect.center().y), Align2::LEFT_CENTER, "Open log", lc);
            open_log = log_r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();

            p.hline(rect.x_range().shrink(1.0), rect.top() + 44.0, Stroke::new(1.0_f32, BORDER));
            let body = Rect::from_min_max(rect.left_top() + Vec2::new(SP_L, 54.0), rect.right_bottom() - Vec2::new(SP_L, SP_M));
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
                paint_progress_line(ui, Rect::from_min_size(Pos2::new(rect.left() + 1.0, rect.top() + 43.0), Vec2::new(rect.width() - 2.0, 2.0)), ACCENT);
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
        BAD
    } else if head.contains(" warn") || head.starts_with("warning") {
        AMBER
    } else if head.starts_with("staged:") || head.contains("finished") {
        GOOD
    } else {
        SECONDARY
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Shared page pieces

fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let r = ui.label(RichText::new(title).font(display_font(T_PAGE)).color(TEXT)).rect;
    let (bar, _) = ui.allocate_exact_size(Vec2::new(r.width(), 3.0), Sense::hover());
    ui.painter().rect_filled(Rect::from_min_size(bar.min - Vec2::new(0.0, 2.0), Vec2::new(44.0, 3.0)), Rounding::same(1.5), ACCENT);
    ui.add_space(SP_M);
    ui.label(RichText::new(subtitle).size(T_BODY + 0.5).color(SECONDARY));
    ui.add_space(SP_XL + SP_XS);
    ui.spacing_mut().item_spacing.y = SP_S;
}

/// The card surface every page uses: panel fill (see-through by `alpha`), hairline border, card corners.
fn card_frame(alpha: u8) -> egui::Frame {
    egui::Frame::none().fill(glass(PANEL, alpha)).stroke(Stroke::new(1.0_f32, BORDER)).rounding(Rounding::same(R_CARD))
}

/// A 3 px accent strip down a card's left edge.
fn accent_edge(p: &egui::Painter, card: Rect) {
    p.rect_filled(
        Rect::from_min_size(card.min, Vec2::new(3.0, card.height())),
        Rounding { nw: R_CARD, sw: R_CARD, ne: 0.0, se: 0.0 },
        ACCENT,
    );
}

fn divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, BORDER));
}

fn same_hash(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

// -------------------------------------------------------------------------------------------------------------------
// Patch notes: the game's git history, one update per day (newest first), each commit a note with its body as
// sub-notes. Titles and highlights are cut from the commit subjects; nothing is written that isn't in the history.

struct Update<'a> {
    day: &'a str,
    /// (index in the history, commit): the index says whether it's built yet.
    commits: Vec<(usize, &'a Commit)>,
}

fn group_updates(commits: &[Commit], limit_hit: bool) -> Vec<Update<'_>> {
    let mut out: Vec<Update> = Vec::new();
    for (i, c) in commits.iter().enumerate() {
        match out.last_mut() {
            Some(u) if u.day == c.day => u.commits.push((i, c)),
            _ => out.push(Update { day: &c.day, commits: vec![(i, c)] }),
        }
    }
    // the oldest day is probably cut off by the history limit: leave it out rather than show it half
    if limit_hit && out.len() > 1 {
        out.pop();
    }
    out
}

/// A commit subject's lead: "Ziplines v1: map cables (retail material), ..." -> "Ziplines v1: map cables".
fn headline(subject: &str) -> String {
    fn clause(s: &str) -> &str {
        let cut = [": ", "; ", " (", " - "].iter().filter_map(|p| s.find(p)).min().unwrap_or(s.len());
        s[..cut].trim()
    }
    let lead = match subject.split_once(": ") {
        Some((topic, rest)) if topic.split_whitespace().count() <= 2 && !rest.trim().is_empty() => format!("{topic}: {}", clause(rest)),
        Some((topic, _)) => clause(topic).to_owned(),
        None => clause(subject).to_owned(),
    };
    let lead = if lead.chars().count() < 3 { subject.trim().to_owned() } else { lead };
    shorten(&lead, 78)
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut);
    format!("{}...", cut.trim_end_matches([',', ';', ':']))
}

/// "2026-11-21" -> ("26.11.21", "Saturday, November 21", "21", "NOV").
fn day_labels(day: &str) -> (String, String, String, String) {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let mut it = day.split('-').map(|p| p.parse::<i32>().ok());
    let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (it.next(), it.next(), it.next()) else {
        return (day.to_owned(), day.to_owned(), String::new(), String::new());
    };
    if !(1..=12).contains(&m) {
        return (day.to_owned(), day.to_owned(), String::new(), String::new());
    }
    // Sakamoto's day of the week
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    let wd = (yy + yy / 4 - yy / 100 + yy / 400 + T[(m - 1) as usize] + d).rem_euclid(7) as usize;
    let month = MONTHS[(m - 1) as usize];
    (
        format!("{:02}.{:02}.{:02}", y % 100, m, d),
        format!("{}, {} {}", DAYS[wd], month, d),
        d.to_string(),
        month[..3].to_uppercase(),
    )
}

impl LauncherApp {
    fn notes_page(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        egui::Frame::none().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            let subtitle = match snap.unbuilt {
                Some(0) => "Recent changes to the game. Your build has all of them.".to_owned(),
                Some(1) => "Recent changes to the game. The newest one isn't in a build yet: use Build latest.".to_owned(),
                Some(n) => format!("Recent changes to the game. The newest {n} aren't in a build yet: use Build latest."),
                None => "Recent changes to the game, from its git history.".to_owned(),
            };
            page_header(ui, "PATCH NOTES", &subtitle);
            if snap.commits.is_empty() {
                ui.label(RichText::new("No git history in this game folder.").size(T_BODY).color(MUTED));
                return;
            }
            let updates = group_updates(&snap.commits, snap.commits.len() >= game::HISTORY_LEN);
            let playing = snap.current.as_ref().map(|b| b.hash.as_str()).unwrap_or("");
            let staged = snap.next.as_ref().map(|b| b.hash.as_str()).unwrap_or("");
            let unbuilt = snap.unbuilt.unwrap_or(0) as usize;
            let ctx = NoteCtx { playing, staged, unbuilt };
            // `--expand-notes`: open the newest two updates (screenshots)
            if std::env::args().any(|a| a == "--expand-notes") && self.notes_open.is_empty() {
                self.notes_open.extend(updates.iter().take(2).map(|u| u.day.to_owned()));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width((ui.available_width() - SP_M).min(1180.0));
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, update) in updates.iter().enumerate() {
                    let open = self.notes_open.contains(update.day);
                    let toggle = if i == 0 { self.featured_update(ui, update, open, &ctx) } else { update_row(ui, update, open, &ctx) };
                    if toggle && !self.notes_open.remove(update.day) {
                        self.notes_open.insert(update.day.to_owned());
                    }
                    ui.add_space(if i == 0 { SP_XL } else { SP_M });
                    if i == 0 && updates.len() > 1 {
                        ui.label(caption("Earlier updates").color(MUTED));
                        ui.add_space(SP_S + 2.0);
                    }
                }
                ui.add_space(SP_XL);
            });
        });
    }

    /// The newest day as a big card: art, headline, highlights, READ MORE. Returns true when READ MORE was pressed.
    fn featured_update(&self, ui: &mut egui::Ui, u: &Update, open: bool, nc: &NoteCtx) -> bool {
        let (version, long_date, _, _) = day_labels(u.day);
        let mut toggle = false;
        let newest = u.commits[0].1;
        let r = card_frame(238).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // header
            egui::Frame::none().inner_margin(Margin { left: SP_XL + 4.0, right: SP_XL, top: SP_L, bottom: SP_M }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SP_S;
                    ui.label(RichText::new("UPDATE").font(display_font(T_UPDATE + 4.0)).color(TEXT));
                    ui.label(RichText::new(&version).font(display_font(T_UPDATE + 4.0)).color(ACCENT_HI));
                    ui.add_space(SP_L);
                    icon_label(ui, Icon::Calendar, &long_date, SECONDARY);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui_pill(ui, "Latest", PillStyle::Accent);
                        update_pills(ui, u, nc);
                    });
                });
            });
            divider(ui);
            egui::Frame::none().inner_margin(Margin { left: SP_XL + 4.0, right: SP_XL, top: SP_L + 2.0, bottom: SP_L + 2.0 }).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = SP_XL;
                    let (art, _) = ui.allocate_exact_size(Vec2::new(300.0, 169.0), Sense::hover());
                    self.paint_art(ui, art, R_CTRL);
                    ui.painter().rect_stroke(art, Rounding::same(R_CTRL), Stroke::new(1.0_f32, glass(Color32::WHITE, 20)));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = SP_XS;
                        ui.add(egui::Label::new(RichText::new(headline(&newest.subject)).size(T_TITLE + 2.0).color(TEXT)).wrap());
                        let n = u.commits.len();
                        ui.label(
                            RichText::new(format!("{n} change{}  ·  newest {} ({})  ·  {}", if n == 1 { "" } else { "s" }, newest.time, newest.ago, newest.hash))
                                .size(T_SMALL)
                                .color(MUTED),
                        );
                        ui.add_space(SP_M);
                        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
                        ui.painter().rect_filled(
                            Rect::from_min_size(Pos2::new(row.left(), row.center().y - 7.0), Vec2::new(3.0, 14.0)),
                            Rounding::same(1.5),
                            ACCENT,
                        );
                        paint_caption(ui.painter(), Pos2::new(row.left() + 12.0, row.center().y), Align2::LEFT_CENTER, "Patch highlights", SECONDARY);
                        ui.add_space(2.0);
                        let mut seen: Vec<String> = Vec::new();
                        for (_, c) in &u.commits {
                            let h = headline(&c.subject);
                            if seen.contains(&h) {
                                continue;
                            }
                            seen.push(h);
                            if seen.len() > 4 {
                                break;
                            }
                        }
                        for h in seen.iter().take(4) {
                            bullet(ui, h, T_BODY, SECONDARY, true);
                        }
                        ui.add_space(SP_S);
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let label = if open { "Show less" } else { "Read more" };
                            toggle = ghost_button(ui, label, Some(if open { Icon::ChevronUp } else { Icon::ChevronRight }));
                        });
                    });
                });
            });
            if open {
                divider(ui);
                egui::Frame::none().inner_margin(Margin { left: SP_XL + 4.0, right: SP_XL, top: SP_S, bottom: SP_M }).show(ui, |ui| {
                    commit_list(ui, u, nc);
                });
            }
        });
        accent_edge(ui.painter(), r.response.rect);
        toggle
    }
}

struct NoteCtx<'a> {
    playing: &'a str,
    staged: &'a str,
    unbuilt: usize,
}

/// PLAYING / STAGED / NOT BUILT for a whole update (right-to-left layout).
fn update_pills(ui: &mut egui::Ui, u: &Update, nc: &NoteCtx) {
    if u.commits.iter().any(|(_, c)| same_hash(&c.hash, nc.playing)) {
        ui_pill(ui, "Playing", PillStyle::Solid(GOOD));
    }
    if u.commits.iter().any(|(_, c)| same_hash(&c.hash, nc.staged)) && !same_hash(nc.staged, nc.playing) {
        ui_pill(ui, "Staged", PillStyle::Outline(GOOD));
    }
    if u.commits.iter().any(|(i, _)| *i < nc.unbuilt) {
        ui_pill(ui, "Not built", PillStyle::Outline(MUTED));
    }
}

/// One older update: a date tile, its headline and the rest of its topics; a click opens its notes.
fn update_row(ui: &mut egui::Ui, u: &Update, open: bool, nc: &NoteCtx) -> bool {
    let (version, long_date, day_num, month) = day_labels(u.day);
    let id = ui.make_persistent_id(("update", u.day));
    let hovered = ui.ctx().animate_bool_with_time(id, ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false)), 0.12);
    let head_h = 76.0;
    let r = card_frame(232).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Frame::none().inner_margin(Margin { left: SP_L + 4.0, right: SP_XL, top: SP_M, bottom: SP_M }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(head_h - 2.0 * SP_M);
                ui.spacing_mut().item_spacing.x = SP_L;
                // date tile
                let (tile, _) = ui.allocate_exact_size(Vec2::new(52.0, 52.0), Sense::hover());
                let p = ui.painter();
                p.rect(tile, Rounding::same(R_CTRL), RAISED, Stroke::new(1.0_f32, BORDER));
                p.text(tile.center() - Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, &day_num, display_font(26.0), TEXT);
                paint_caption(p, tile.center() + Vec2::new(0.0, 13.0), Align2::CENTER_CENTER, &month, MUTED);

                let right_w = 330.0;
                let text_w = (ui.available_width() - right_w).max(200.0);
                ui.allocate_ui_with_layout(Vec2::new(text_w, head_h - 2.0 * SP_M), Layout::top_down(Align::Min).with_main_align(Align::Center), |ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(format!("UPDATE {version}")).font(display_font(T_TITLE)).color(SECONDARY));
                    ui.add(egui::Label::new(RichText::new(headline(&u.commits[0].1.subject)).size(T_BODY + 1.5).color(TEXT)).truncate());
                    let rest: Vec<String> = {
                        let mut seen = vec![headline(&u.commits[0].1.subject)];
                        for (_, c) in u.commits.iter().skip(1) {
                            let h = headline(&c.subject);
                            if !seen.contains(&h) {
                                seen.push(h);
                            }
                        }
                        seen.into_iter().skip(1).collect()
                    };
                    if !rest.is_empty() {
                        ui.add(egui::Label::new(RichText::new(format!("Also: {}", rest.join("  ·  "))).size(T_SMALL).color(MUTED)).truncate());
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = SP_S;
                    let (chev, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    paint_icon(ui.painter(), if open { Icon::ChevronUp } else { Icon::ChevronDown }, chev.center(), 16.0, mix(SECONDARY, TEXT, hovered));
                    ui.add_space(SP_S);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = SP_XS;
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| icon_label(ui, Icon::Calendar, &long_date, SECONDARY));
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let n = u.commits.len();
                            ui.label(RichText::new(format!("{n} change{}", if n == 1 { "" } else { "s" })).size(T_SMALL).color(MUTED));
                            update_pills(ui, u, nc);
                        });
                    });
                });
            });
        });
        if open {
            divider(ui);
            egui::Frame::none().inner_margin(Margin { left: SP_L + 4.0, right: SP_XL, top: SP_S, bottom: SP_M }).show(ui, |ui| {
                commit_list(ui, u, nc);
            });
        }
    });
    let card = r.response.rect;
    let head = Rect::from_min_size(card.min, Vec2::new(card.width(), head_h));
    let resp = ui.interact(head, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.ctx().data_mut(|d| d.insert_temp(id, resp.hovered()));
    if hovered > 0.0 {
        ui.painter().rect_stroke(card, Rounding::same(R_CARD), Stroke::new(1.0_f32, glass(Color32::from_rgb(90, 96, 108), (hovered * 255.0) as u8)));
    }
    resp.clicked()
}

/// Every commit of an update: hash, subject, body notes, time and its build state.
fn commit_list(ui: &mut egui::Ui, u: &Update, nc: &NoteCtx) {
    for (n, (i, c)) in u.commits.iter().enumerate() {
        if n > 0 {
            divider(ui);
        }
        ui.add_space(SP_M);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = SP_L;
            ui.allocate_ui_with_layout(Vec2::new(64.0, 20.0), Layout::left_to_right(Align::Center), |ui| {
                ui.set_width(64.0);
                ui.label(RichText::new(&c.hash).monospace().size(T_MONO).color(MUTED));
            });
            let right_w = 210.0;
            let text_w = (ui.available_width() - right_w).max(200.0);
            ui.allocate_ui_with_layout(Vec2::new(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = SP_XS;
                ui.add(egui::Label::new(RichText::new(&c.subject).size(T_BODY).color(TEXT)).wrap());
                if !c.body.is_empty() {
                    ui.add_space(2.0);
                    for note in &c.body {
                        bullet(ui, note, T_SMALL, SECONDARY, false);
                    }
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = SP_S;
                ui.label(RichText::new(&c.time).monospace().size(T_MONO).color(MUTED));
                if same_hash(&c.hash, nc.playing) {
                    ui_pill(ui, "Playing", PillStyle::Solid(GOOD));
                }
                if same_hash(&c.hash, nc.staged) && !same_hash(nc.staged, nc.playing) {
                    ui_pill(ui, "Staged", PillStyle::Outline(GOOD));
                }
                if *i < nc.unbuilt {
                    ui_pill(ui, "Not built", PillStyle::Outline(MUTED));
                }
            });
        });
        ui.add_space(SP_M);
    }
}

/// A bulleted line: a small square (accent for highlights) and wrapping text.
fn bullet(ui: &mut egui::Ui, text: &str, size: f32, color: Color32, accent: bool) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = SP_S + 2.0;
        let (slot, _) = ui.allocate_exact_size(Vec2::new(6.0, size + 4.0), Sense::hover());
        let sq = Rect::from_center_size(Pos2::new(slot.center().x, slot.top() + size * 0.62), Vec2::splat(if accent { 5.0 } else { 4.0 }));
        ui.painter().rect_filled(sq, Rounding::ZERO, if accent { ACCENT } else { MUTED });
        ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap());
    });
}

/// An icon followed by a short text, both in `color` (dates, counts).
fn icon_label(ui: &mut egui::Ui, icon: Icon, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(T_SMALL + 0.5), color);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0 + galley.size().x, galley.size().y.max(16.0)), Sense::hover());
    paint_icon(ui.painter(), icon, Pos2::new(rect.left() + 7.0, rect.center().y), 14.0, color);
    ui.painter().galley(Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

// -------------------------------------------------------------------------------------------------------------------
// Mods: the page is laid out, but the game doesn't load mods yet. Both lists are empty; below the "coming" banner the
// grid / list shows clearly marked examples (grey skeletons, every action disabled with "Coming soon").

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModsTab {
    Installed,
    Browse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModsView {
    Grid,
    List,
}

/// A mod as the page shows it. Mods are free: there's no price anywhere. Real mods plug into `installed_mods` /
/// `browse_mods`; both are empty until R5Rust can load them.
struct Mod {
    name: String,
    author: String,
    version: String,
    description: String,
    tags: Vec<String>,
    enabled: bool,
}

/// How many example skeletons the empty tabs show.
const MOD_EXAMPLES: usize = 6;
const COMING_SOON: &str = "Coming soon: R5Rust doesn't load mods yet";

/// A grey placeholder bar (skeleton text).
fn skeleton_bar(p: &egui::Painter, min: Pos2, w: f32, h: f32, color: Color32) {
    p.rect_filled(Rect::from_min_size(min, Vec2::new(w, h)), Rounding::same(h / 2.0), color);
}

/// A button that can't be pressed yet: muted, hairline border, "Coming soon" on hover.
fn soon_button(ui: &mut egui::Ui, text: &str, icon: Option<Icon>) -> egui::Response {
    let galley = caption_galley(ui.painter(), text, T_CAPTION + 0.5, MUTED);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let (rect, r) = ui.allocate_exact_size(Vec2::new(galley.size().x + icon_w + 2.0 * SP_M + 2.0, 30.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect.shrink(0.5), Rounding::same(R_CTRL), RAISED, Stroke::new(1.0_f32, BORDER));
    let mut x = rect.left() + SP_M + 1.0;
    if let Some(icon) = icon {
        paint_icon(p, icon, Pos2::new(x + 7.0, rect.center().y), 13.0, MUTED);
        x += icon_w;
    }
    p.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, MUTED);
    r.on_hover_text(COMING_SOON)
}

/// A page tab: display-font label, a muted count, the accent underline when active (like the MODES tab).
fn page_tab(ui: &mut egui::Ui, label: &str, count: usize, active: bool) -> bool {
    let font = display_font(T_SECTION + 1.0);
    let lw = ui.painter().layout_no_wrap(label.to_uppercase(), font.clone(), TEXT).size().x;
    let count_text = count.to_string();
    let cw = ui.painter().layout_no_wrap(count_text.clone(), FontId::proportional(T_SMALL), MUTED).size().x;
    let (rect, r) = ui.allocate_exact_size(Vec2::new(lw + SP_S + cw + 2.0, 40.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(r.id, r.hovered() || active, 0.12);
    let p = ui.painter();
    let color = mix(SECONDARY, TEXT, hover);
    let t = p.text(Pos2::new(rect.left(), rect.center().y - 2.0), Align2::LEFT_CENTER, label.to_uppercase(), font, color);
    p.text(Pos2::new(t.right() + SP_S, t.center().y + 1.0), Align2::LEFT_CENTER, count_text, FontId::proportional(T_SMALL), MUTED);
    if active {
        p.rect_filled(Rect::from_min_size(Pos2::new(rect.left(), rect.bottom() - 3.0), Vec2::new(rect.width(), 3.0)), Rounding::same(1.5), ACCENT);
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A small square icon button that shows whether it's the current choice (grid / list view).
fn icon_choice(ui: &mut egui::Ui, icon: Icon, on: bool, tip: &str) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(34.0, 32.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(r.id, r.hovered(), 0.1);
    let p = ui.painter();
    if on {
        p.rect(rect.shrink(0.5), Rounding::same(R_CTRL), RAISED, Stroke::new(1.0_f32, Color32::from_rgb(74, 80, 90)));
    } else if hover > 0.0 {
        p.rect_filled(rect, Rounding::same(R_CTRL), glass(HOVER, (hover * 200.0) as u8));
    }
    paint_icon(p, icon, rect.center(), 15.0, if on { TEXT } else { mix(MUTED, SECONDARY, hover) });
    r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
}

impl LauncherApp {
    fn mods_page(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            page_header(ui, "MODS", "Free add-ons for R5Rust: install them, switch each one on or off.");

            // tabs + view toggle, on a hairline
            let row = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SP_XL;
                    for (tab, label, count) in
                        [(ModsTab::Installed, "Installed", self.installed_mods.len()), (ModsTab::Browse, "Browse", self.browse_mods.len())]
                    {
                        if page_tab(ui, label, count, self.mods_tab == tab) {
                            self.mods_tab = tab;
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = SP_XS;
                        if icon_choice(ui, Icon::Notes, self.mods_view == ModsView::List, "List view") {
                            self.mods_view = ModsView::List;
                        }
                        if icon_choice(ui, Icon::Grid, self.mods_view == ModsView::Grid, "Grid view") {
                            self.mods_view = ModsView::Grid;
                        }
                    });
                })
                .response
                .rect;
            ui.painter().hline(row.x_range(), row.bottom() + 0.5, Stroke::new(1.0_f32, BORDER));
            ui.add_space(SP_L);

            // search + sort / filter (ready for later: the search box types, the chips are inert)
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SP_XS;
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.mods_search)
                        .hint_text(RichText::new("Search mods").color(MUTED))
                        .font(FontId::proportional(T_BODY - 0.5))
                        .margin(Margin { left: 32.0, right: SP_M, top: 7.0, bottom: 7.0 })
                        .desired_width(280.0),
                );
                // (the response rect is the text area, inside the margin)
                paint_icon(ui.painter(), Icon::Search, Pos2::new(r.rect.left() - 15.0, r.rect.center().y), 14.0, MUTED);
                ui.add_space(SP_XL);
                ui.label(caption("Sort").color(MUTED));
                ui.add_space(SP_XS);
                let sorts: &[&str] = match self.mods_tab {
                    ModsTab::Installed => &["Name", "Recently added"],
                    ModsTab::Browse => &["Popular", "Newest", "Name"],
                };
                for (i, s) in sorts.iter().enumerate() {
                    segment(ui, s, i == 0, false).on_hover_text(COMING_SOON);
                }
                ui.add_space(SP_XL);
                ui.label(caption("Show").color(MUTED));
                ui.add_space(SP_XS);
                let filters: &[&str] = match self.mods_tab {
                    ModsTab::Installed => &["All", "Enabled", "Disabled"],
                    ModsTab::Browse => &["All", "Maps", "Gameplay", "Visual"],
                };
                for (i, f) in filters.iter().enumerate() {
                    segment(ui, f, i == 0, false).on_hover_text(COMING_SOON);
                }
            });
            ui.add_space(SP_L);

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - SP_S);
                let tab = self.mods_tab;
                let list = match tab {
                    ModsTab::Installed => &self.installed_mods,
                    ModsTab::Browse => &self.browse_mods,
                };
                if list.is_empty() {
                    self.mods_banner(ui, tab);
                    ui.add_space(SP_XL);
                    ui.horizontal(|ui| {
                        ui.label(caption("Layout preview").color(SECONDARY));
                        ui.label(RichText::new("Examples only, not real mods.").size(T_SMALL).color(MUTED));
                    });
                    ui.add_space(SP_M);
                }
                let items: Vec<Option<&Mod>> = if list.is_empty() { vec![None; MOD_EXAMPLES] } else { list.iter().map(Some).collect() };
                match self.mods_view {
                    ModsView::Grid => self.mods_grid(ui, &items, tab),
                    ModsView::List => {
                        for m in &items {
                            let w = ui.available_width();
                            ui.allocate_ui_with_layout(Vec2::new(w, 0.0), Layout::top_down(Align::Min), |ui| self.mod_row(ui, *m, tab));
                            ui.add_space(SP_S);
                        }
                    }
                }
                ui.add_space(SP_XL);
            });
        });
    }

    fn mods_banner(&self, ui: &mut egui::Ui, tab: ModsTab) {
        card_frame(238).inner_margin(Margin::symmetric(SP_XL, SP_L + 2.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SP_L;
                let (tile, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                ui.painter().rect(tile, Rounding::same(R_CARD), RAISED, Stroke::new(1.0_f32, BORDER));
                paint_icon(ui.painter(), Icon::Box, tile.center(), 22.0, SECONDARY);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = SP_XS;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mods are coming in a future update").size(T_TITLE).color(TEXT));
                        ui.add_space(SP_XS);
                        ui_pill(ui, "Coming soon", PillStyle::Outline(SECONDARY));
                    });
                    let body = match tab {
                        ModsTab::Installed => "R5Rust doesn't load mods yet. Once it does, the mods you install show up here, each with its own on / off switch.",
                        ModsTab::Browse => "Nothing to browse yet. When mod support lands, free mods show up here, ready to install with one click.",
                    };
                    ui.label(RichText::new(body).size(T_BODY - 0.5).color(SECONDARY));
                });
            });
        });
    }

    /// Cards in as many columns as fit (min 250 px wide).
    fn mods_grid(&self, ui: &mut egui::Ui, items: &[Option<&Mod>], tab: ModsTab) {
        let w = ui.available_width();
        let gap = SP_L;
        let cols = (((w + gap) / (250.0 + gap)).floor() as usize).max(1);
        let card_w = (w - gap * (cols - 1) as f32) / cols as f32;
        for row in items.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for m in row {
                    // its own sub-ui, so the action area inside can't move this row's cursor
                    ui.allocate_ui_with_layout(Vec2::new(card_w, 0.0), Layout::top_down(Align::Min), |ui| self.mod_card(ui, *m, tab, card_w));
                }
            });
            ui.add_space(gap);
        }
    }

    /// A grid card: cover, name, author, description, tags, and the action. `None` = an example skeleton.
    fn mod_card(&self, ui: &mut egui::Ui, m: Option<&Mod>, tab: ModsTab, w: f32) {
        let cover_h = (w * 0.42).round();
        let body_h = 156.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, cover_h + body_h), Sense::hover());
        let p = ui.painter();
        p.rect(rect, Rounding::same(R_CARD), glass(PANEL, 238), Stroke::new(1.0_f32, BORDER));
        let cover = Rect::from_min_size(rect.min, Vec2::new(w, cover_h)).shrink(1.0);
        p.rect_filled(cover, Rounding { nw: R_CARD, ne: R_CARD, sw: 0.0, se: 0.0 }, RAISED);
        let m_size = cover_h * 0.42;
        p.image(
            self.mark.id(),
            Rect::from_center_size(cover.center(), Vec2::splat(m_size)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::from_white_alpha(18),
        );
        let x = rect.left() + SP_L;
        let mut y = cover.bottom() + SP_M + 2.0;
        match m {
            None => {
                pill(p, cover.left_top() + Vec2::new(SP_M, SP_M), Align2::LEFT_TOP, "Example", PillStyle::Outline(MUTED));
                skeleton_bar(p, Pos2::new(x, y), w * 0.5, 12.0, HOVER);
                skeleton_bar(p, Pos2::new(x, y + 20.0), w * 0.28, 8.0, RAISED);
                skeleton_bar(p, Pos2::new(x, y + 40.0), w - 2.0 * SP_L, 8.0, RAISED);
                skeleton_bar(p, Pos2::new(x, y + 54.0), (w - 2.0 * SP_L) * 0.7, 8.0, RAISED);
                for (i, tw) in [46.0, 58.0].into_iter().enumerate() {
                    let tx = x + i as f32 * (46.0 + SP_XS + 4.0);
                    p.rect_filled(Rect::from_min_size(Pos2::new(tx, y + 74.0), Vec2::new(tw, 18.0)), Rounding::same(R_CTRL), RAISED);
                }
            }
            Some(m) => {
                p.text(Pos2::new(x, y), Align2::LEFT_TOP, &m.name, FontId::proportional(T_BODY + 1.0), TEXT);
                p.text(Pos2::new(x, y + 20.0), Align2::LEFT_TOP, format!("by {}  ·  v{}", m.author, m.version), FontId::proportional(T_SMALL), MUTED);
                let desc = shorten(&m.description, 90);
                p.text(Pos2::new(x, y + 40.0), Align2::LEFT_TOP, desc, FontId::proportional(T_SMALL), SECONDARY);
                let mut tx = x;
                for tag in m.tags.iter().take(3) {
                    let r = pill(p, Pos2::new(tx, y + 74.0), Align2::LEFT_TOP, tag, PillStyle::Outline(SECONDARY));
                    tx = r.right() + SP_XS;
                }
            }
        }
        y = rect.bottom() - SP_M - 30.0;
        let action = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w - 2.0 * SP_L, 30.0));
        ui.allocate_ui_at_rect(action, |ui| {
            ui.horizontal(|ui| match tab {
                ModsTab::Installed => {
                    let mut on = m.is_some_and(|m| m.enabled);
                    ui.scope(|ui| toggle(ui, &mut on, false)).response.on_hover_text(COMING_SOON);
                    ui.label(RichText::new(if on { "Enabled" } else { "Disabled" }).size(T_SMALL).color(MUTED));
                }
                ModsTab::Browse => {
                    soon_button(ui, "Install", None);
                }
            });
        });
    }

    /// A list row: thumbnail, name + author, description, version, and the action. `None` = an example skeleton.
    fn mod_row(&self, ui: &mut egui::Ui, m: Option<&Mod>, tab: ModsTab) {
        let w = ui.available_width();
        let h = 76.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        let p = ui.painter();
        p.rect(rect, Rounding::same(R_CARD), glass(PANEL, 238), Stroke::new(1.0_f32, BORDER));
        let thumb = Rect::from_min_size(rect.min + Vec2::new(SP_M, SP_M), Vec2::new(92.0, h - 2.0 * SP_M));
        p.rect_filled(thumb, Rounding::same(R_CTRL), RAISED);
        if m.is_some() {
            p.image(
                self.mark.id(),
                Rect::from_center_size(thumb.center(), Vec2::splat(thumb.height() * 0.6)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::from_white_alpha(18),
            );
        }
        let x = thumb.right() + SP_L;
        let action_w = 170.0;
        let text_w = (rect.right() - action_w - x - SP_XL).max(120.0);
        let cy = rect.center().y;
        match m {
            None => {
                skeleton_bar(p, Pos2::new(x, cy - 22.0), (text_w * 0.32).min(200.0), 11.0, HOVER);
                skeleton_bar(p, Pos2::new(x + (text_w * 0.32).min(200.0) + SP_S, cy - 20.0), 70.0, 8.0, RAISED);
                skeleton_bar(p, Pos2::new(x, cy - 1.0), text_w * 0.85, 8.0, RAISED);
                skeleton_bar(p, Pos2::new(x, cy + 13.0), text_w * 0.55, 8.0, RAISED);
                skeleton_bar(p, Pos2::new(rect.right() - action_w - SP_XL - 40.0, cy - 4.0), 36.0, 8.0, RAISED);
                pill(p, thumb.center(), Align2::CENTER_CENTER, "Example", PillStyle::Outline(MUTED));
            }
            Some(m) => {
                let t = p.text(Pos2::new(x, cy - 24.0), Align2::LEFT_TOP, &m.name, FontId::proportional(T_BODY + 0.5), TEXT);
                p.text(Pos2::new(t.right() + SP_S, cy - 23.0), Align2::LEFT_TOP, format!("by {}", m.author), FontId::proportional(T_SMALL), MUTED);
                p.text(Pos2::new(x, cy), Align2::LEFT_TOP, shorten(&m.description, 110), FontId::proportional(T_SMALL), SECONDARY);
                p.text(Pos2::new(rect.right() - action_w - SP_XL, cy), Align2::RIGHT_CENTER, format!("v{}", m.version), FontId::monospace(T_MONO), MUTED);
            }
        }
        let action = Rect::from_min_size(Pos2::new(rect.right() - action_w - SP_M, cy - 15.0), Vec2::new(action_w, 30.0));
        ui.allocate_ui_at_rect(action, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| match tab {
                ModsTab::Installed => {
                    let mut on = m.is_some_and(|m| m.enabled);
                    ui.label(RichText::new(if on { "Enabled" } else { "Disabled" }).size(T_SMALL).color(MUTED));
                    ui.scope(|ui| toggle(ui, &mut on, false)).response.on_hover_text(COMING_SOON);
                }
                ModsTab::Browse => {
                    soon_button(ui, "Install", None);
                }
            });
        });
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Console: the game's own output, live (R5Flowstate's launcher has the same tab).

impl LauncherApp {
    /// Brings the filtered index list up to date (only the new lines while the filter is unchanged).
    fn refresh_console_view(&mut self, lines: &[String]) {
        let source = self.console.as_ref().map(|a| Arc::as_ptr(a) as usize).unwrap_or(0);
        let v = &mut self.console_view;
        let same_query = v.source == source && v.filter == self.console_filter && v.problems_only == self.console_problems_only;
        // at the line cap the oldest lines drop off the front, so indices shift: rescan then
        let append_only = same_query && lines.len() >= v.len && lines.len() < game::KEEP_LINES;
        if append_only && lines.len() == v.len {
            return;
        }
        if !append_only {
            *v = ConsoleView {
                source,
                filter: self.console_filter.clone(),
                problems_only: self.console_problems_only,
                ..ConsoleView::default()
            };
        }
        let filter = v.filter.to_ascii_lowercase();
        for (i, line) in lines.iter().enumerate().skip(v.len) {
            let color = line_color(line);
            if color == BAD {
                v.errors += 1;
            } else if color == AMBER {
                v.warnings += 1;
            }
            let problem = color == BAD || color == AMBER;
            if (!v.problems_only || problem) && (filter.is_empty() || line.to_ascii_lowercase().contains(&filter)) {
                v.shown.push(i);
            }
        }
        v.len = lines.len();
    }

    fn console_page(&mut self, ui: &mut egui::Ui, phase: PlayPhase) {
        let log_path = game::data_dir().unwrap_or_default().join("game.log");
        egui::Frame::none().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            let subtitle = match phase {
                PlayPhase::Ours | PlayPhase::Starting => "The game's output, live. Also saved to game.log.",
                PlayPhase::Elsewhere => "R5Rust was started outside the launcher, so its output isn't here.",
                PlayPhase::Idle => "The game's output from the last session started here (game.log).",
            };
            page_header(ui, "CONSOLE", subtitle);

            let source = self.console.clone();
            let empty = Mutex::new(Vec::new());
            let guard = match &source {
                Some(arc) => arc.lock().unwrap(),
                None => empty.lock().unwrap(),
            };
            let lines: &[String] = &guard;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SP_S;
                ui.add(
                    egui::TextEdit::singleline(&mut self.console_filter)
                        .hint_text(RichText::new("Filter lines").color(MUTED))
                        .font(FontId::monospace(T_MONO))
                        .margin(Margin::symmetric(SP_M, SP_S))
                        .desired_width(300.0),
                );
                ui.add_space(SP_M);
                toggle(ui, &mut self.console_problems_only, true);
                ui.label(RichText::new("Warnings and errors only").size(T_BODY - 0.5).color(SECONDARY));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost_button(ui, "Open log file", Some(Icon::Doc))
                        && let Err(e) = game::open_path(&log_path)
                    {
                        self.notice = Some((e, Tone::Bad));
                    }
                    if ghost_button(ui, "Copy", Some(Icon::Copy)) {
                        ui.ctx().copy_text(lines.join("\n"));
                        self.notice = Some((format!("Copied {} lines.", lines.len()), Tone::Info));
                    }
                });
            });
            ui.add_space(SP_L);
            self.refresh_console_view(lines);
            let v = &self.console_view;

            card_frame(236).fill(Color32::from_rgba_unmultiplied(9, 10, 12, 240)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                // status strip
                egui::Frame::none().inner_margin(Margin::symmetric(SP_L, SP_S + 2.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SP_L;
                        let live = matches!(phase, PlayPhase::Ours | PlayPhase::Starting);
                        if live {
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 3.5, GOOD);
                        }
                        ui.label(caption(if live { "Live" } else { "Last session" }).color(if live { GOOD } else { SECONDARY }));
                        ui.label(RichText::new(format!("{} lines", lines.len())).size(T_SMALL).color(MUTED));
                        ui.label(RichText::new(format!("{} warnings", v.warnings)).size(T_SMALL).color(if v.warnings > 0 { AMBER } else { MUTED }));
                        ui.label(RichText::new(format!("{} errors", v.errors)).size(T_SMALL).color(if v.errors > 0 { BAD } else { MUTED }));
                        if v.shown.len() != lines.len() {
                            ui.label(RichText::new(format!("{} shown", v.shown.len())).size(T_SMALL).color(SECONDARY));
                        }
                    });
                });
                divider(ui);
                egui::Frame::none().inner_margin(Margin::same(SP_L)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height((ui.available_height() - SP_XL - 2.0 * SP_L).max(120.0));
                    if v.shown.is_empty() {
                        let msg = if lines.is_empty() { "Nothing yet. Press Play and the game's output shows up here." } else { "No lines match." };
                        ui.label(RichText::new(msg).size(T_BODY).color(MUTED));
                        return;
                    }
                    // rows must be exactly row_h tall, or show_rows picks the wrong range and leaves a gap
                    // (show_rows adds item_spacing.y per row, so that has to be 0 before the call, not inside it)
                    let row_h = ui.fonts(|f| f.row_height(&FontId::monospace(T_MONO + 0.5))).ceil() + 2.0;
                    ui.spacing_mut().item_spacing.y = 0.0;
                    egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(ui, row_h, v.shown.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for &idx in &v.shown[range] {
                            let line = &lines[idx];
                            let (prefix, rest) = split_log_prefix(line);
                            ui.horizontal(|ui| {
                                ui.set_height(row_h);
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if !prefix.is_empty() {
                                    ui.add(egui::Label::new(RichText::new(prefix).monospace().size(T_MONO + 0.5).color(MUTED)).extend());
                                }
                                let r = RichText::new(rest).monospace().size(T_MONO + 0.5).color(line_color(line));
                                ui.add(egui::Label::new(r).extend());
                            });
                        }
                    });
                });
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
// Settings and the shared controls

/// A settings group: an accent-barred display title, then a card; `note` is the card's footer strip.
/// Returns the title row's rect (for scrolling to a section).
fn section(ui: &mut egui::Ui, title: &str, note: Option<(&str, Color32)>, body: impl FnOnce(&mut egui::Ui)) -> Rect {
    let head = ui.horizontal(|ui| {
        let (bar, _) = ui.allocate_exact_size(Vec2::new(3.0, 20.0), Sense::hover());
        ui.painter().rect_filled(bar, Rounding::same(1.5), ACCENT);
        ui.add_space(2.0);
        ui.label(RichText::new(title).font(display_font(T_SECTION)).color(TEXT));
    })
    .response
    .rect;
    ui.add_space(SP_S);
    card_frame(236).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::Frame::none().inner_margin(Margin::symmetric(SP_XL, SP_L + 2.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = SP_S;
            body(ui);
        });
        if let Some((text, color)) = note {
            divider(ui);
            egui::Frame::none().fill(glass(BG, 90)).inner_margin(Margin::symmetric(SP_XL, SP_M)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(text).size(T_SMALL).color(color));
            });
        }
    });
    ui.add_space(SP_2XL);
    head
}

/// The fixed-width label column of a settings row.
fn row_label(ui: &mut egui::Ui, title: &str, enabled: bool) {
    ui.allocate_ui_with_layout(Vec2::new(LABEL_W, 28.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(LABEL_W);
        ui.label(RichText::new(title).size(T_BODY).color(if enabled { TEXT } else { MUTED }));
    });
}

/// A chip: small caption, the current value, a chevron; a click opens `menu` above it.
fn quick_chip(ui: &mut egui::Ui, label: &str, value: &str, enabled: bool, menu: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(("quick-chip", label));
    // `--open-chip=<LABEL>`: keep that menu open (screenshots of the menus)
    if std::env::args().any(|a| a.strip_prefix("--open-chip=").is_some_and(|l| l.eq_ignore_ascii_case(label))) {
        ui.memory_mut(|m| m.open_popup(id));
    }
    let open = ui.memory(|m| m.is_popup_open(id));
    let lg = caption_galley(ui.painter(), label, T_CAPTION, MUTED);
    let vf = FontId::proportional(T_BODY + 0.5);
    let vw = ui.painter().layout_no_wrap(value.to_owned(), vf.clone(), TEXT).size().x;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(SP_L + lg.size().x + 10.0 + vw + 34.0, 44.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id, enabled && (response.hovered() || open), 0.12);
    let p = ui.painter();
    p.rect(
        rect.shrink(0.5),
        Rounding::same(R_CTRL),
        mix(glass(PANEL, 255), HOVER, hover * 0.7),
        Stroke::new(1.0_f32, if open { ACCENT } else { mix(BORDER, Color32::from_rgb(70, 76, 86), hover) }),
    );
    let cy = rect.center().y;
    let lx = rect.left() + SP_L;
    p.galley(Pos2::new(lx, cy - lg.size().y / 2.0), lg.clone(), MUTED);
    p.text(Pos2::new(lx + lg.size().x + 10.0, cy), Align2::LEFT_CENTER, value, vf, if enabled { TEXT } else { SECONDARY });
    paint_icon(p, if open { Icon::ChevronDown } else { Icon::ChevronUp }, Pos2::new(rect.right() - 17.0, cy), 13.0, if open { ACCENT_HI } else { SECONDARY });
    let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response.on_hover_text("Close the game to change this") };
    if enabled && response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup::popup_above_or_below_widget(
        ui,
        id,
        &response,
        egui::AboveOrBelow::Above,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(300.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            menu(ui);
        },
    );
}

fn menu_hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(SP_XS);
    ui.label(RichText::new(text).size(T_SMALL - 0.5).color(MUTED));
    ui.add_space(SP_XS);
}

/// One choice in a chip's menu: full width, an accent bar and a check when it's the current one.
fn menu_row(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    let p = ui.painter();
    if r.hovered() || selected {
        p.rect_filled(rect, Rounding::same(R_CTRL), if r.hovered() { HOVER } else { RAISED });
    }
    if selected {
        p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), Rounding { nw: R_CTRL, sw: R_CTRL, ne: 0.0, se: 0.0 }, ACCENT);
        paint_icon(p, Icon::Check, Pos2::new(rect.right() - 16.0, rect.center().y), 13.0, ACCENT_HI);
    }
    p.text(Pos2::new(rect.left() + SP_M + 2.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(T_BODY - 0.5), TEXT);
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A labelled path with a CHANGE button (sets `pick` when clicked).
fn path_row(ui: &mut egui::Ui, label: &str, path: &str, ok: bool, pick: &mut bool) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(72.0, 32.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(72.0);
            ui.label(caption(label).color(MUTED));
        });
        let w = (ui.available_width() - 112.0).max(80.0);
        ui.allocate_ui_with_layout(Vec2::new(w, 32.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(w);
            ui.add(egui::Label::new(RichText::new(path).monospace().size(T_MONO + 0.5).color(if ok { TEXT } else { AMBER })).truncate());
        });
        *pick = ghost_button(ui, "Change", Some(Icon::Folder));
    });
}

/// The secondary button everywhere: raised fill, hairline border, optional icon, tracked caption text.
fn ghost_button(ui: &mut egui::Ui, text: &str, icon: Option<Icon>) -> bool {
    let galley = caption_galley(ui.painter(), text, T_CAPTION + 0.5, TEXT);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let size = Vec2::new(galley.size().x + icon_w + 2.0 * SP_M + 2.0, 32.0);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let hover = ui.ctx().animate_bool_with_time(r.id, r.hovered(), 0.1);
    let pressed = r.is_pointer_button_down_on();
    let fill = mix(RAISED, HOVER, hover);
    let p = ui.painter();
    p.rect(
        rect.shrink(0.5),
        Rounding::same(R_CTRL),
        if pressed { fill.gamma_multiply(0.85) } else { fill },
        Stroke::new(1.0_f32, mix(BORDER, Color32::from_rgb(74, 80, 90), hover)),
    );
    let mut x = rect.left() + SP_M + 1.0;
    let color = mix(SECONDARY, TEXT, hover.max(0.6));
    if let Some(icon) = icon {
        paint_icon(p, icon, Pos2::new(x + 7.0, rect.center().y), 14.0, color);
        x += icon_w;
    }
    p.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, color);
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A switch: a 40x22 pill; off is dark with a grey knob, on is accent with a white knob.
fn toggle(ui: &mut egui::Ui, value: &mut bool, enabled: bool) -> bool {
    let (slot, response) = ui.allocate_exact_size(Vec2::new(40.0, 24.0), Sense::click());
    let rect = Rect::from_center_size(slot.center(), Vec2::new(40.0, 22.0));
    let mut changed = false;
    if enabled && response.clicked() {
        *value = !*value;
        changed = true;
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *value, 0.14);
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), enabled && response.hovered(), 0.12);
    let (fill, border, knob) = if enabled {
        (
            mix(mix(BG, RAISED, hover), ACCENT, t),
            mix(mix(Color32::from_rgb(60, 65, 74), Color32::from_rgb(86, 92, 102), hover), ACCENT, t),
            mix(mix(MUTED, SECONDARY, hover), Color32::WHITE, t),
        )
    } else {
        (BG, BORDER, Color32::from_rgb(64, 68, 76))
    };
    let p = ui.painter();
    p.rect(rect, Rounding::same(11.0), fill, Stroke::new(1.0_f32, border));
    let knob_x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    p.circle_filled(Pos2::new(knob_x, rect.center().y), 7.0, knob);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    changed
}

fn toggle_row(ui: &mut egui::Ui, value: &mut bool, title: &str, hint: &str, enabled: bool) -> bool {
    let mut changed = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = SP_M;
        changed = toggle(ui, value, enabled);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).size(T_BODY).color(if enabled { TEXT } else { MUTED }));
            ui.label(RichText::new(hint).size(T_SMALL).color(MUTED));
        });
    });
    ui.add_space(SP_XS);
    changed
}

/// One segment of a segmented control.
fn segment(ui: &mut egui::Ui, label: &str, on: bool, enabled: bool) -> egui::Response {
    let galley = caption_galley(ui.painter(), label, T_CAPTION + 0.5, TEXT);
    let size = Vec2::new((galley.size().x + 2.0 * SP_M + 2.0).max(48.0), 30.0);
    let (rect, r) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let hover = ui.ctx().animate_bool_with_time(r.id, enabled && r.hovered() && !on, 0.1);
    let (fill, stroke, text) = match (on, enabled) {
        (true, true) => (ACCENT, Stroke::NONE, Color32::WHITE),
        (true, false) => (HOVER, Stroke::NONE, SECONDARY),
        (false, true) => (mix(RAISED, HOVER, hover), Stroke::new(1.0_f32, mix(BORDER, Color32::from_rgb(74, 80, 90), hover)), mix(SECONDARY, TEXT, hover)),
        (false, false) => (RAISED, Stroke::new(1.0_f32, BORDER), MUTED),
    };
    let p = ui.painter();
    p.rect(rect.shrink(0.5), Rounding::same(R_CTRL), fill, stroke);
    p.galley(rect.center() - galley.size() / 2.0, galley, text);
    if enabled { r.on_hover_cursor(egui::CursorIcon::PointingHand) } else { r }
}

/// A row of segmented choices (label + options). Returns true when the selection changed.
fn choice_row<T: PartialEq + Clone>(ui: &mut egui::Ui, title: &str, value: &mut T, options: &[(T, &str)], enabled: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        row_label(ui, title, enabled);
        choice_wrap(ui, |ui| {
            for (v, label) in options {
                let on = *value == *v;
                if segment(ui, label, on, enabled).clicked() && !on {
                    *value = v.clone();
                    changed = true;
                }
            }
        });
    });
    changed
}

/// Returns true once the value has settled (drag released, or a click / keyboard change).
#[allow(clippy::too_many_arguments)]
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
        row_label(ui, title, enabled);
        let slider = egui::Slider::new(value, range).step_by(step).show_value(false).trailing_fill(true);
        // the knob is a ring in the accent over a dark centre; the filled part of the rail is the accent
        let r = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = 18.0;
                ui.visuals_mut().selection.bg_fill = if enabled { ACCENT } else { HOVER };
                let w = &mut ui.visuals_mut().widgets;
                for v in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                    v.bg_fill = BG;
                    v.weak_bg_fill = BG;
                }
                w.inactive.fg_stroke = Stroke::new(2.5_f32, ACCENT);
                w.hovered.fg_stroke = Stroke::new(2.5_f32, ACCENT_HI);
                w.active.fg_stroke = Stroke::new(2.5_f32, ACCENT_HI);
                ui.add_enabled(enabled, slider)
            })
            .inner;
        ui.add_space(SP_M);
        let shown = format!("{:.*}{suffix}", decimals, *value);
        ui.label(RichText::new(shown).monospace().size(T_BODY).color(if enabled { TEXT } else { MUTED }));
        settled = r.drag_stopped() || (r.changed() && !r.dragged());
    });
    settled
}

/// A compact number field. Returns true when it commits (Enter, or focus moving away); `invalid` paints it in the
/// error colour (the caller doesn't save invalid text).
fn num_field(ui: &mut egui::Ui, buf: &mut String, hint: &str, width: f32, invalid: bool, focus: bool, enabled: bool) -> bool {
    let r = ui
        .scope(|ui| {
            if invalid {
                let v = ui.visuals_mut();
                for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
                    w.bg_stroke = Stroke::new(1.0_f32, BAD);
                }
                v.selection.stroke = Stroke::new(1.0_f32, BAD);
            }
            let edit = egui::TextEdit::singleline(buf)
                .hint_text(RichText::new(hint).color(MUTED))
                .font(FontId::monospace(T_MONO + 0.5))
                .text_color(if invalid { BAD } else { TEXT })
                .margin(Margin::symmetric(SP_S, 6.0))
                .desired_width(width)
                .char_limit(9);
            ui.add_enabled(enabled, edit)
        })
        .inner;
    if focus {
        r.request_focus();
    }
    r.lost_focus()
}

impl CustomInputs {
    /// Shows / hides the custom FPS field, filled with the current cap when that's already a custom one.
    fn toggle_fps(&mut self, current: i64) {
        self.fps_open = !self.fps_open;
        if self.fps_open {
            let custom = !FPS_PRESETS.iter().any(|(v, _)| *v == current);
            self.fps = if custom { current.to_string() } else { String::new() };
            self.focus = true;
        }
    }

    fn toggle_res(&mut self, current: [i64; 2]) {
        self.res_open = !self.res_open;
        if self.res_open {
            let custom = !RES_PRESETS.iter().any(|(v, _)| *v == current);
            self.res = if custom { [current[0].to_string(), current[1].to_string()] } else { Default::default() };
            self.focus = true;
        }
    }

    /// The FPS field + its hint. Some(cap) when a valid value was committed.
    fn fps_field(&mut self, ui: &mut egui::Ui, enabled: bool) -> Option<i64> {
        let invalid = !self.fps.trim().is_empty() && parse_fps(&self.fps).is_none();
        let mut out = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SP_S;
            if num_field(ui, &mut self.fps, "180", 64.0, invalid, std::mem::take(&mut self.focus), enabled) {
                out = parse_fps(&self.fps);
            }
            let (hint, color) = if invalid { ("20 to 1000 FPS, or 0 for unlimited", BAD) } else { ("FPS  ·  Enter applies", MUTED) };
            ui.label(RichText::new(hint).size(T_SMALL).color(color));
        });
        out
    }

    /// Width x height fields + hint. Some([w, h]) when both are valid and one of them committed.
    fn res_field(&mut self, ui: &mut egui::Ui, enabled: bool) -> Option<[i64; 2]> {
        let bad = |s: &str, r: &std::ops::RangeInclusive<i64>| !s.trim().is_empty() && parse_dim(s, r).is_none();
        let (bad_w, bad_h) = (bad(&self.res[0], &RES_W), bad(&self.res[1], &RES_H));
        let mut commit = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SP_S;
            commit |= num_field(ui, &mut self.res[0], "1920", 64.0, bad_w, std::mem::take(&mut self.focus), enabled);
            ui.label(RichText::new("x").size(T_SMALL).color(MUTED));
            commit |= num_field(ui, &mut self.res[1], "1080", 64.0, bad_h, false, enabled);
            let (hint, color) =
                if bad_w || bad_h { ("Width 320 to 7680, height 240 to 4320", BAD) } else { ("Enter applies", MUTED) };
            ui.label(RichText::new(hint).size(T_SMALL).color(color));
        });
        if !commit {
            return None;
        }
        Some([parse_dim(&self.res[0], &RES_W)?, parse_dim(&self.res[1], &RES_H)?])
    }
}

/// The wrapping area right of a settings label (segmented choices wrap under themselves, not under the label).
fn choice_wrap<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 30.0), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(SP_XS, SP_XS);
        add(ui)
    })
    .inner
}

/// A coloured dot and a sentence: the state lines in Settings.
fn status_dot(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SP_S;
        let (dot, _) = ui.allocate_exact_size(Vec2::new(8.0, 16.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 3.5, color);
        ui.label(RichText::new(text).size(T_SMALL + 0.5).color(if color == MUTED { SECONDARY } else { color }));
    });
}

impl LauncherApp {
    /// Returns true when "Change folder" was pressed (the dialog runs after the frame).
    fn settings_page(&mut self, ui: &mut egui::Ui, snap: &Snapshot, phase: PlayPhase) -> bool {
        let mut browse = false;
        let mut err = None;
        egui::Frame::none().inner_margin(PAGE_MARGIN).show(ui, |ui| {
            page_header(ui, "SETTINGS", "How the game plays, where it lives, and how it starts.");
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(860.0);
                let editable = phase == PlayPhase::Idle;

                let game_note = if editable {
                    ("Saved to r5rust_settings.toml. Keybinds live in the game's menu (Esc).", MUTED)
                } else {
                    ("Close the game to change these: it saves its own settings when it exits.", AMBER)
                };
                section(ui, "GAME", Some(game_note), |ui| {
                    let s = &mut self.settings;
                    let mut save = false;
                    save |= slider_row(ui, "Field of view", &mut s.fov, 70.0..=120.0, 1.0, "°", 0, editable);
                    save |= slider_row(ui, "Mouse sensitivity", &mut s.sensitivity, 0.1..=10.0, 0.05, "", 2, editable);
                    ui.add_space(SP_S);
                    save |= toggle_row(ui, &mut s.auto_sprint, "Auto sprint", "Moving forward sprints on its own, like Apex's setting.", editable);
                    save |= toggle_row(ui, &mut s.mantle_boost, "Mantle boost", "R5Flowstate's jump at the end of a mantle. Not in stock Apex.", editable);
                    save |= toggle_row(ui, &mut s.invert_y, "Invert look", "Moving the mouse up looks down.", editable);
                    if save && let Err(e) = s.save(&self.game_root) {
                        err = Some(e);
                    }
                });

                let gfx_note = "Lighting and effects always render like Apex: presets only trade detail and resolution. Texture quality applies on the next launch, the rest live in game too (Esc > Video).";
                let scroll_gfx = self.screenshot.is_some() && self.frames < 60 && std::env::args().any(|a| a == "--scroll=graphics");
                let gfx_head = section(ui, "GRAPHICS", Some((gfx_note, MUTED)), |ui| {
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
                        ui.horizontal(|ui| {
                            ui.add_space(LABEL_W + SP_S);
                            ui.label(RichText::new("Custom: the settings below don't match a preset.").size(T_SMALL).color(MUTED));
                        });
                    }
                    ui.add_space(SP_XS);
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
                    // Resolution (stretched) and FPS cap: presets plus a Custom segment that reveals a number field
                    let custom = &mut self.custom;
                    ui.horizontal(|ui| {
                        row_label(ui, "Resolution", editable);
                        choice_wrap(ui, |ui| {
                            for (v, label) in RES_PRESETS {
                                if segment(ui, label, g.stretch == v, editable).clicked() && g.stretch != v {
                                    g.stretch = v;
                                    custom.res_open = false;
                                    save = true;
                                }
                            }
                            let on = !RES_PRESETS.iter().any(|(v, _)| *v == g.stretch);
                            let label = if on { format!("Custom {} x {}", g.stretch[0], g.stretch[1]) } else { "Custom".to_owned() };
                            if segment(ui, &label, on, editable).clicked() {
                                custom.toggle_res(g.stretch);
                            }
                        });
                    });
                    if custom.res_open {
                        ui.horizontal(|ui| {
                            ui.add_space(LABEL_W + SP_S);
                            if let Some(v) = custom.res_field(ui, editable) {
                                g.stretch = v;
                                custom.res_open = false;
                                save = true;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        row_label(ui, "FPS cap", editable);
                        choice_wrap(ui, |ui| {
                            for (v, label) in FPS_PRESETS {
                                let label = if v == 0 { "None" } else { label };
                                if segment(ui, label, g.fps_cap == v, editable).clicked() && g.fps_cap != v {
                                    g.fps_cap = v;
                                    custom.fps_open = false;
                                    save = true;
                                }
                            }
                            let on = !FPS_PRESETS.iter().any(|(v, _)| *v == g.fps_cap);
                            let label = if on { format!("Custom {}", g.fps_cap) } else { "Custom".to_owned() };
                            if segment(ui, &label, on, editable).clicked() {
                                custom.toggle_fps(g.fps_cap);
                            }
                        });
                    });
                    if custom.fps_open {
                        ui.horizontal(|ui| {
                            ui.add_space(LABEL_W + SP_S);
                            if let Some(v) = custom.fps_field(ui, editable) {
                                g.fps_cap = v;
                                custom.fps_open = false;
                                save = true;
                            }
                        });
                    }
                    ui.add_space(SP_XS);
                    save |= toggle_row(ui, &mut g.vsync, "V-Sync", "Waits for the display: no tearing, a frame of extra input lag.", editable);
                    if knobs {
                        g.preset = g.detect_preset();
                        save = true;
                    }
                    if save && let Err(e) = self.settings.save(&self.game_root) {
                        err = Some(e);
                    }
                });

                if scroll_gfx {
                    ui.scroll_to_rect(gfx_head, Some(Align::Min));
                }

                section(ui, "LAUNCH OPTIONS", None, |ui| {
                    let edit = egui::TextEdit::singleline(&mut self.launch_args)
                        .hint_text(RichText::new("e.g. --fly --fov 110").color(MUTED))
                        .font(FontId::monospace(T_MONO + 0.5))
                        .margin(Margin::symmetric(SP_M, SP_S + 1.0))
                        .desired_width(f32::INFINITY);
                    if ui.add(edit).changed()
                        && let Err(e) = game::write_setting("launch_args.txt", &self.launch_args)
                    {
                        err = Some(format!("Couldn't save launch options: {e}"));
                    }
                    let args = game::split_args(&self.launch_args)
                        .iter()
                        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() })
                        .collect::<Vec<_>>()
                        .join(" ");
                    ui.horizontal(|ui| {
                        ui.label(caption("Runs").color(MUTED));
                        ui.label(RichText::new(format!("r5rust.exe {args}")).monospace().size(T_MONO + 0.5).color(ACCENT_HI));
                    });
                    ui.add_space(SP_XS);
                    divider(ui);
                    ui.add_space(SP_XS);
                    ui.spacing_mut().item_spacing.y = SP_XS;
                    for (flag, what) in [
                        ("--fly", "free-fly camera, no collision"),
                        ("--fov <deg>", "field of view for this run only"),
                        ("--pos x,y,z", "spawn at a map position (feet)"),
                        ("--yaw <deg>  --pitch <deg>", "spawn view angles"),
                        ("--menu", "open the in-game menu at start"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(Vec2::new(LABEL_W + 30.0, 18.0), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_width(LABEL_W + 30.0);
                                ui.label(RichText::new(flag).monospace().size(T_MONO).color(TEXT));
                            });
                            ui.label(RichText::new(what).size(T_SMALL).color(MUTED));
                        });
                    }
                });

                section(ui, "GAME FOLDER", None, |ui| {
                    ui.label(RichText::new(self.game_root.display().to_string()).monospace().size(T_BODY).color(TEXT));
                    let (state, color) = match snap.state {
                        GameState::Ready => ("R5Rust project with a build ready to play", GOOD),
                        GameState::NoBuild => ("R5Rust project, nothing built yet", AMBER),
                        GameState::FolderMissing => ("This folder doesn't exist", BAD),
                        GameState::InvalidFolder => ("Not an R5Rust project (no play.ps1)", BAD),
                    };
                    status_dot(ui, state, color);
                    ui.add_space(SP_XS);
                    ui.horizontal(|ui| {
                        browse = ghost_button(ui, "Change folder", Some(Icon::Folder));
                        if ghost_button(ui, "Open folder", Some(Icon::ChevronRight)) {
                            err = game::open_path(&self.game_root).err();
                        }
                    });
                });

                let files_note = "Install builds the files from your Apex install (tools\\install.ps1) and points the game at them (assets_root in r5rust.toml).";
                section(ui, "GAME FILES", Some((files_note, MUTED)), |ui| {
                    let f = &snap.files;
                    let dir = f.assets_root.join("maps").join(&f.map);
                    ui.label(RichText::new(dir.display().to_string()).monospace().size(T_BODY).color(TEXT));
                    if f.ok() {
                        status_dot(ui, "Map extracted: world, collision, materials and lighting found.", GOOD);
                    } else {
                        let missing = f.missing.join(", ");
                        status_dot(ui, &format!("Missing {missing}. Install builds them from your own Apex install."), AMBER);
                    }
                    let art = if f.loadscreen.is_some() {
                        "Loading screen found (shown on the Play page)."
                    } else {
                        "No loading screen extracted: the Play page shows a plain backdrop."
                    };
                    status_dot(ui, art, MUTED);
                    let (install, install_color) = match &self.install_status {
                        InstallStatus::Checking => ("Checking the install...".to_owned(), MUTED),
                        InstallStatus::Installed(d) => (format!("Installed {d}"), GOOD),
                        InstallStatus::Partial(d) => (format!("Install not finished: {d}"), AMBER),
                        InstallStatus::Stale(d) => (format!("Update needed: {d}"), AMBER),
                        InstallStatus::Missing(_) => ("Not installed yet.".to_owned(), AMBER),
                        InstallStatus::Unknown(d) => (format!("Couldn't check the install: {d}"), MUTED),
                    };
                    status_dot(ui, &install, install_color);
                    ui.add_space(SP_XS);
                    let mut recheck = false;
                    ui.horizontal(|ui| {
                        let label = if matches!(self.install_status, InstallStatus::Stale(_) | InstallStatus::Partial(_)) { "Update game files" } else { "Install / reinstall" };
                        if ghost_button(ui, label, Some(Icon::Refresh)) && !self.installing() {
                            self.install_open = true;
                            self.page = Page::Play;
                        }
                        if ghost_button(ui, "Open assets folder", Some(Icon::Folder)) {
                            err = game::open_path(&f.assets_root).err();
                        }
                        if ghost_button(ui, "Check again", Some(Icon::Check)) {
                            self.poller.refresh();
                            recheck = true;
                        }
                    });
                    if recheck {
                        self.check_install(ui.ctx(), snap.files.assets_root.clone());
                    }
                });

                section(ui, "LAUNCHER", None, |ui| {
                    if toggle_row(
                        ui,
                        &mut self.rpc_on,
                        "Discord Rich Presence",
                        "Show \"Playing R5Rust\" on your Discord profile while the game runs.",
                        true,
                    ) && let Err(e) = game::write_setting("discord_rpc.txt", if self.rpc_on { "1" } else { "0" })
                    {
                        err = Some(format!("Couldn't save: {e}"));
                    }
                    if toggle_row(
                        ui,
                        &mut self.console_on_launch,
                        "Open the Console when the game starts",
                        "Jump to the game's live output on every launch.",
                        true,
                    ) && let Err(e) = game::write_setting("console_on_launch.txt", if self.console_on_launch { "1" } else { "0" })
                    {
                        err = Some(format!("Couldn't save: {e}"));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ghost_button(ui, "Game settings file", Some(Icon::Doc)) {
                            err = game::open_path(&GameSettings::path(&self.game_root)).err();
                        }
                        let dir = game::data_dir().unwrap_or_default();
                        if ghost_button(ui, "Last game log", Some(Icon::Doc)) {
                            err = game::open_path(&dir.join("game.log")).err();
                        }
                        if ghost_button(ui, "Last build log", Some(Icon::Doc)) {
                            err = game::open_path(&dir.join("build.log")).err();
                        }
                    });
                });

                section(ui, "ABOUT", None, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "R5Rust Launcher v{}. Play runs play.ps1, which installs a staged build first. Build latest runs tools\\publish.ps1 (cargo build + stage) and never touches the copy you're playing. Press Enter to play from any page.",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .size(T_BODY - 0.5)
                        .color(SECONDARY),
                    );
                });
                ui.add_space(SP_XL);
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
            let btn = Vec2::new(44.0, 32.0);
            let close = Rect::from_min_size(Pos2::new(bar.right() - btn.x, bar.top()), btn);
            let maximize = close.translate(Vec2::new(-btn.x, 0.0));
            let minimize = maximize.translate(Vec2::new(-btn.x, 0.0));
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
                painter.rect_filled(minimize, Rounding::ZERO, glass(HOVER, 220));
            }
            if max_r.hovered() {
                painter.rect_filled(maximize, Rounding::ZERO, glass(HOVER, 220));
            }
            if close_r.hovered() {
                painter.rect_filled(close, Rounding::ZERO, ACCENT);
            }
            let s = Stroke::new(1.2_f32, TEXT);
            let m = minimize.center();
            painter.line_segment([Pos2::new(m.x - 5.0, m.y), Pos2::new(m.x + 5.0, m.y)], s);
            if maximized {
                // Restore: two overlapping squares.
                let front = Rect::from_center_size(maximize.center() + Vec2::new(-1.5, 1.5), Vec2::splat(9.0));
                let back = front.translate(Vec2::new(3.0, -3.0));
                painter.line_segment([Pos2::new(back.left(), back.top()), Pos2::new(back.right(), back.top())], s);
                painter.line_segment([Pos2::new(back.right(), back.top()), Pos2::new(back.right(), back.bottom())], s);
                painter.rect_stroke(front, Rounding::same(1.0), s);
            } else {
                painter.rect_stroke(Rect::from_center_size(maximize.center(), Vec2::splat(10.0)), Rounding::same(1.0), s);
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
        Some(Rect::from_min_max(Pos2::new(w.left as f32, w.top as f32), Pos2::new(w.right as f32, w.bottom as f32)))
    }
}

#[cfg(not(windows))]
fn work_area_at(_x: i32, _y: i32) -> Option<Rect> {
    None
}

/// A rect with a colour per corner: top-left, top-right, bottom-right, bottom-left.
fn paint_gradient(painter: &egui::Painter, b: Rect, c: [Color32; 4]) {
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
    style.spacing.menu_margin = Margin::same(SP_S);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_rounding = Rounding::same(R_CARD);
    v.menu_rounding = Rounding::same(R_CARD);
    v.popup_shadow = egui::epaint::Shadow { offset: Vec2::new(0.0, 8.0), blur: 24.0, spread: 0.0, color: Color32::from_black_alpha(150) };
    v.extreme_bg_color = Color32::from_rgb(8, 9, 11);
    v.override_text_color = Some(TEXT);
    // text selection (the slider's trailing fill is set per slider)
    v.selection.bg_fill = Color32::from_rgb(110, 30, 36);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT_HI);
    v.text_cursor.stroke = Stroke::new(2.0_f32, ACCENT_HI);
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
        w.bg_stroke = Stroke::new(1.0_f32, BORDER);
        w.fg_stroke.color = fg;
        w.rounding = Rounding::same(R_CTRL);
        w.expansion = 0.0;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(74, 80, 90));
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(SP_S, SP_S);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    style.spacing.slider_width = 380.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.interact_size.y = 22.0;
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_fps_and_resolution_are_validated() {
        assert_eq!(parse_fps("180"), Some(180));
        assert_eq!(parse_fps(" 180 fps "), Some(180));
        assert_eq!(parse_fps("0"), Some(0));
        assert_eq!(parse_fps("Unlimited"), Some(0));
        assert_eq!(parse_fps("12"), None);
        assert_eq!(parse_fps("1001"), None);
        assert_eq!(parse_fps("abc"), None);
        assert_eq!(parse_dim("1366", &RES_W), Some(1366));
        assert_eq!(parse_dim("200", &RES_H), None);
        assert_eq!(parse_dim("8000", &RES_W), None);
    }
}

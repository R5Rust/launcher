#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod game;

use std::{path::PathBuf, time::Duration};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Key, Layout, Margin, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2,
};
use game::{GameState, Job, JobKind, Poller, Snapshot};

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
const INK: Color32 = Color32::from_rgb(8, 19, 26);
const AMBER: Color32 = Color32::from_rgb(255, 186, 82);
const RED: Color32 = Color32::from_rgb(255, 107, 94);

const RAIL_W: f32 = 228.0;
const FOOTER_H: f32 = 94.0;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("R5Rust Launcher")
            .with_inner_size([1300.0, 820.0])
            .with_min_inner_size([1080.0, 680.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_icon(load_window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "R5Rust Launcher",
        options,
        Box::new(|cc| Ok(Box::new(LauncherApp::new(&cc.egui_ctx)))),
    )
}

fn load_window_icon() -> egui::IconData {
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("the R5Rust app icon should be available")
        .resize_exact(256, 256, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    egui::IconData { rgba: icon.into_raw(), width: 256, height: 256 }
}

fn display_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Barlow Condensed".into()))
}

fn texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> (egui::TextureHandle, Vec2) {
    let image = image::load_from_memory(bytes).expect("bundled image should decode");
    let size = Vec2::new(image.width() as f32, image.height() as f32);
    let rgba = image.to_rgba8();
    let handle = ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied([size.x as usize, size.y as usize], &rgba),
        egui::TextureOptions::LINEAR,
    );
    (handle, size)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Play,
    Notes,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Good,
    Bad,
}

impl Tone {
    fn color(self) -> Color32 {
        match self {
            Tone::Info => SECONDARY,
            Tone::Good => CYAN,
            Tone::Bad => RED,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayPhase {
    Idle,
    Starting,
    Running,
}

struct LauncherApp {
    game_root: PathBuf,
    poller: Poller,
    page: Page,
    notice: Option<(String, Tone)>,
    mode_menu_open: bool,
    launch_args: String,
    play_job: Option<Job>,
    build_job: Option<Job>,
    build_panel_open: bool,
    cover: egui::TextureHandle,
    cover_size: Vec2,
    logo: egui::TextureHandle,
}

impl LauncherApp {
    fn new(ctx: &egui::Context) -> Self {
        apply_theme(ctx);
        let (cover, cover_size) = texture(ctx, "firing-range-cover", include_bytes!("../assets/firing-range.jpg"));
        let (logo, _) = texture(ctx, "r5rust-logo", include_bytes!("../assets/icon.png"));
        let game_root = game::load_game_root();
        Self {
            poller: Poller::start(game_root.clone(), ctx.clone()),
            game_root,
            // `--page=notes` / `--page=settings` opens on that page (handy for shortcuts and screenshots).
            page: match std::env::args().find_map(|a| a.strip_prefix("--page=").map(str::to_owned)).as_deref() {
                Some("notes") => Page::Notes,
                Some("settings") => Page::Settings,
                _ => Page::Play,
            },
            notice: None,
            mode_menu_open: false,
            launch_args: game::load_launch_args(),
            play_job: None,
            build_job: None,
            build_panel_open: false,
            cover,
            cover_size,
            logo,
        }
    }

    fn browse(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose the r5rust project folder");
        if self.game_root.is_dir() {
            dialog = dialog.set_directory(&self.game_root);
        }
        if let Some(folder) = dialog.pick_folder() {
            self.game_root = folder;
            self.notice = None;
            self.poller.set_root(self.game_root.clone());
            if let Err(error) = game::write_setting("game_path.txt", &self.game_root.to_string_lossy()) {
                self.notice = Some((format!("Couldn't save this folder: {error}"), Tone::Bad));
            }
        }
    }

    fn launch(&mut self, ctx: &egui::Context) {
        let args = game::split_args(&self.launch_args);
        match Job::start(JobKind::Play, &self.game_root, &args, ctx.clone()) {
            Ok(job) => {
                self.play_job = Some(job);
                self.notice = None;
            }
            Err(error) => self.notice = Some((error, Tone::Bad)),
        }
        self.poller.refresh();
    }

    fn build(&mut self, ctx: &egui::Context) {
        match Job::start(JobKind::Build, &self.game_root, &[], ctx.clone()) {
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

    fn phase(&self, snap: &Snapshot) -> PlayPhase {
        match &self.play_job {
            Some(job) if job.running() => {
                if snap.running || game::process_running("r5rust.exe") {
                    PlayPhase::Running
                } else {
                    PlayPhase::Starting
                }
            }
            _ if snap.running => PlayPhase::Running,
            _ => PlayPhase::Idle,
        }
    }

    /// Notices when play.ps1 or publish.ps1 finish, and says how it went.
    fn poll_jobs(&mut self, ctx: &egui::Context) {
        if let Some(job) = &mut self.play_job
            && job.poll()
        {
            let ran = job.elapsed();
            self.notice = if job.succeeded() && ran > Duration::from_secs(5) {
                Some((format!("Session ended after {}.", fmt_duration(ran)), Tone::Info))
            } else {
                let why = job.failure_line().unwrap_or_else(|| "no output".into());
                Some((format!("The game didn't start: {why}"), Tone::Bad))
            };
            self.play_job = None;
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
        if self.play_job.is_some() || self.building() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_jobs(ctx);
        let snap = self.poller.get();
        let phase = self.phase(&snap);
        let can_play = snap.is_ready() && phase == PlayPhase::Idle;
        let can_build = matches!(snap.state, GameState::Ready | GameState::NoBuild)
            && !self.building()
            && self.game_root.join("tools/publish.ps1").is_file();

        let mut launch_clicked = false;
        let mut build_clicked = false;
        let mut browse_clicked = false;

        self.rail(ctx, &snap);
        egui::TopBottomPanel::bottom("play-footer")
            .exact_height(FOOTER_H)
            .frame(egui::Frame::none().fill(PANEL).inner_margin(Margin::symmetric(30.0, 13.0)))
            .show(ctx, |ui| {
                let (play, build) = self.footer(ui, &snap, phase, can_play, can_build);
                launch_clicked = play;
                build_clicked = build;
            });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG).inner_margin(0.0)).show(ctx, |ui| {
            match self.page {
                Page::Play => self.play_page(ui, ctx, phase),
                Page::Notes => self.notes_page(ui, &snap),
                Page::Settings => browse_clicked = self.settings_page(ui, &snap),
            }
        });

        if self.build_panel_open && self.build_job.is_some() {
            self.build_panel(ctx);
        }
        draw_window_chrome(ctx);

        if can_play && !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(Key::Enter)) {
            launch_clicked = true;
        }
        if browse_clicked {
            self.browse();
        }
        if build_clicked {
            self.build(ctx);
        }
        if launch_clicked {
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
    Gear,
}

impl LauncherApp {
    fn rail(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        egui::SidePanel::left("launcher-rail")
            .exact_width(RAIL_W)
            .resizable(false)
            .frame(egui::Frame::none().fill(RAIL).inner_margin(0.0))
            .show(ctx, |ui| {
                ui.add_space(40.0);
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.add(egui::Image::new((self.logo.id(), Vec2::splat(36.0))).fit_to_exact_size(Vec2::splat(36.0)));
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("R5RUST").font(display_font(22.0)).color(TEXT));
                        ui.label(RichText::new("LAUNCHER").size(9.0).strong().color(MUTED));
                    });
                });
                ui.add_space(34.0);

                let unbuilt = snap.unbuilt.filter(|n| *n > 0).map(|n| format!("{n} NEW"));
                for (page, label, icon, badge) in [
                    (Page::Play, "PLAY", NavIcon::Play, None),
                    (Page::Notes, "PATCH NOTES", NavIcon::Notes, unbuilt),
                    (Page::Settings, "SETTINGS", NavIcon::Gear, None),
                ] {
                    if nav_item(ui, label, icon, self.page == page, badge.as_deref()) {
                        self.page = page;
                        self.mode_menu_open = false;
                    }
                    ui.add_space(2.0);
                }

                ui.add_space((ui.available_height() - 66.0).max(24.0));
                let line = ui.cursor().min.y;
                ui.painter().hline(ui.max_rect().x_range(), line, Stroke::new(1.0_f32, BORDER));
                ui.add_space(16.0);
                let build = snap.current.as_ref().map(|b| b.hash.as_str()).unwrap_or("none");
                for (key, value) in [("GAME", build.to_owned()), ("LAUNCHER", format!("v{}", env!("CARGO_PKG_VERSION")))]
                {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(RichText::new(key).size(9.0).strong().color(MUTED));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(24.0);
                            ui.label(RichText::new(value).size(11.0).monospace().color(SECONDARY));
                        });
                    });
                }
            });
    }
}

fn nav_item(ui: &mut egui::Ui, label: &str, icon: NavIcon, active: bool, badge: Option<&str>) -> bool {
    let width = ui.available_width() - 20.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
    let rect = Rect::from_min_size(rect.min + Vec2::new(10.0, 0.0), Vec2::new(width, 44.0));
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, Rounding::same(0.0), RAISED);
        painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), Rounding::same(0.0), CYAN);
    } else if response.hovered() {
        painter.rect_filled(rect, Rounding::same(0.0), HOVER);
    }
    let color = if active { CYAN } else if response.hovered() { TEXT } else { SECONDARY };
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
        FontId::proportional(13.0),
        if active || response.hovered() { TEXT } else { SECONDARY },
    );
    if let Some(badge) = badge {
        painter.text(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            badge,
            FontId::proportional(9.0),
            CYAN,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
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
    ) -> (bool, bool) {
        let mut play = false;
        let mut build = false;
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
                ui.label(RichText::new(title).size(10.0).strong().color(MUTED));

                match (snap.state, snap.current.as_ref().or(snap.next.as_ref())) {
                    (GameState::Ready, Some(build)) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&build.hash).monospace().size(13.0).color(CYAN));
                            ui.add(
                                egui::Label::new(RichText::new(&build.subject).size(13.0).color(TEXT)).truncate(),
                            );
                        });
                    }
                    (GameState::Ready, None) => {
                        ui.label(RichText::new("A build is staged (no version info).").size(13.0).color(TEXT));
                    }
                    (GameState::NoBuild, _) => {
                        ui.label(RichText::new("Press Build latest to compile the game.").size(13.0).color(TEXT));
                    }
                    (GameState::FolderMissing, _) => {
                        ui.label(RichText::new("The game folder doesn't exist. Pick it in Settings.").size(13.0).color(TEXT));
                    }
                    (GameState::InvalidFolder, _) => {
                        ui.label(RichText::new("That folder has no play.ps1. Pick the r5rust folder in Settings.").size(13.0).color(TEXT));
                    }
                }

                let (line, tone) = if let Some((text, tone)) = &self.notice {
                    (text.clone(), *tone)
                } else if phase == PlayPhase::Running {
                    ("R5Rust is running.".to_owned(), Tone::Good)
                } else if let (Some(next), Some(_)) = (&snap.next, &snap.current) {
                    (format!("Update {} staged. It installs when you press Play.", next.hash), Tone::Good)
                } else if let Some(n) = snap.unbuilt.filter(|n| *n > 0) {
                    (format!("{n} newer commit{} not built yet.", if n == 1 { "" } else { "s" }), Tone::Info)
                } else if let Some(b) = snap.newest() {
                    (format!("Up to date with the game's code. Built {}.", b.date), Tone::Info)
                } else {
                    (String::new(), Tone::Info)
                };
                let color = if tone == Tone::Info { MUTED } else { tone.color() };
                ui.add(egui::Label::new(RichText::new(line).size(11.0).color(color)).truncate());
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, fill, text) = match phase {
                    PlayPhase::Running => ("RUNNING", RAISED, CYAN),
                    PlayPhase::Starting => ("STARTING", RAISED, SECONDARY),
                    PlayPhase::Idle if can_play => ("PLAY", CYAN, INK),
                    PlayPhase::Idle => ("PLAY", RAISED, MUTED),
                };
                let button = egui::Button::new(RichText::new(label).font(display_font(26.0)).color(text))
                    .fill(fill)
                    .stroke(Stroke::NONE)
                    .rounding(Rounding::same(0.0))
                    .min_size(Vec2::new(196.0, 56.0));
                let response = ui.add_enabled(can_play, button);
                if phase == PlayPhase::Starting {
                    paint_progress_line(ui, response.rect, CYAN);
                }
                play = response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                ui.add_space(12.0);
                build = self.build_button(ui, snap, can_build);
            });
        });
        (play, build)
    }

    fn build_button(&self, ui: &mut egui::Ui, snap: &Snapshot, enabled: bool) -> bool {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(200.0, 56.0), Sense::click());
        let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
        let painter = ui.painter();
        let hovered = enabled && response.hovered();
        let pending = snap.unbuilt.is_some_and(|n| n > 0) || snap.state == GameState::NoBuild;
        painter.rect_filled(rect, Rounding::same(0.0), if hovered { HOVER } else { RAISED });
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
                Some(n) => ("BUILD LATEST", format!("{n} new commit{}", if n == 1 { "" } else { "s" }), CYAN),
                None => ("BUILD LATEST", "compile + stage".to_owned(), MUTED),
            }
        };
        let title_color = if enabled || self.building() { TEXT } else { MUTED };
        painter.text(rect.left_top() + Vec2::new(16.0, 12.0), Align2::LEFT_TOP, title, display_font(17.0), title_color);
        painter.text(
            rect.left_top() + Vec2::new(16.0, 33.0),
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

// -------------------------------------------------------------------------------------------------------------------
// Play page

impl LauncherApp {
    fn play_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, phase: PlayPhase) {
        let hero_size = ui.available_size();
        let (hero, _) = ui.allocate_exact_size(hero_size, Sense::hover());
        let painter = ui.painter().clone();
        painter.image(self.cover.id(), hero, cover_uv(self.cover_size, hero_size), Color32::WHITE);
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

        // MODES tab + menu
        let tab = Rect::from_min_size(Pos2::new(hero.left() + 40.0, hero.top() + 42.0), Vec2::new(132.0, 38.0));
        let tab_response = ui.interact(tab, egui::Id::new("mode-tab"), Sense::click());
        let tab_color = if tab_response.hovered() || self.mode_menu_open { TEXT } else { SECONDARY };
        painter.text(Pos2::new(tab.left(), tab.top() + 8.0), Align2::LEFT_TOP, "MODES", display_font(17.0), tab_color);
        let arrow = Pos2::new(tab.left() + 58.0, tab.top() + 18.0);
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
        painter.text(Pos2::new(x - 2.0, y + 15.0), Align2::LEFT_TOP, "FIRING RANGE", display_font(68.0), TEXT);
        painter.text(
            Pos2::new(x + 2.0, y + 91.0),
            Align2::LEFT_TOP,
            "Move freely. Practice your movement. Make the range yours.",
            FontId::proportional(14.0),
            SECONDARY,
        );
        let meta = painter.text(
            Pos2::new(x + 2.0, y + 117.0),
            Align2::LEFT_TOP,
            "LOCAL SESSION   /   SOLO",
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

        if self.mode_menu_open {
            let area = egui::Area::new(egui::Id::new("mode-menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(Pos2::new(tab.left(), tab.bottom() + 10.0))
                .show(ctx, |ui| self.mode_menu(ui));
            if area.inner || (!toggled && area.response.clicked_elsewhere()) {
                self.mode_menu_open = false;
            }
        }
    }

    /// Returns true when the menu should close (a mode was picked).
    fn mode_menu(&self, ui: &mut egui::Ui) -> bool {
        let mut picked = false;
        egui::Frame::none().fill(PANEL).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(312.0);
            ui.label(RichText::new("GAME MODES").size(10.0).strong().color(MUTED));
            ui.add_space(8.0);
            let (row, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 74.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(row, Rounding::same(0.0), if response.hovered() { HOVER } else { RAISED });
            p.rect_filled(Rect::from_min_size(row.left_top(), Vec2::new(2.0, row.height())), Rounding::same(0.0), CYAN);
            let thumb = Rect::from_min_size(row.left_top() + Vec2::new(10.0, 9.0), Vec2::new(88.0, 56.0));
            p.image(self.cover.id(), thumb, cover_uv(self.cover_size, thumb.size()), Color32::WHITE);
            p.text(Pos2::new(thumb.right() + 12.0, row.top() + 23.0), Align2::LEFT_TOP, "Firing Range", FontId::proportional(13.0), TEXT);
            p.text(Pos2::new(thumb.right() + 12.0, row.top() + 45.0), Align2::LEFT_TOP, "SELECTED", FontId::proportional(9.0), CYAN);
            picked = response.clicked();
            ui.add_space(9.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(RichText::new("MORE MODES").size(9.0).strong().color(MUTED));
            ui.label(RichText::new("Coming later").size(11.0).color(SECONDARY));
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
            p.rect_filled(rect, Rounding::same(0.0), Color32::from_rgba_unmultiplied(9, 13, 18, 238));
            p.rect_stroke(rect, Rounding::same(0.0), Stroke::new(1.0_f32, BORDER));
            let (status, color) = match job.exit {
                None => (format!("COMPILING  {}", fmt_duration(job.elapsed())), CYAN),
                Some(Some(0)) => (format!("STAGED IN {}", fmt_duration(job.elapsed())), CYAN),
                Some(_) => ("FAILED".to_owned(), RED),
            };
            p.text(rect.left_top() + Vec2::new(16.0, 14.0), Align2::LEFT_TOP, "BUILD", display_font(17.0), TEXT);
            p.text(rect.left_top() + Vec2::new(62.0, 17.0), Align2::LEFT_TOP, status, FontId::monospace(11.0), color);

            let close_rect = Rect::from_min_size(Pos2::new(rect.right() - 36.0, rect.top() + 6.0), Vec2::splat(30.0));
            let close_r = ui.interact(close_rect, egui::Id::new("build-close"), Sense::click());
            if close_r.hovered() {
                p.rect_filled(close_rect, Rounding::same(0.0), HOVER);
            }
            let c = close_rect.center();
            let s = Stroke::new(1.2_f32, if job.running() { MUTED } else { TEXT });
            p.line_segment([c + Vec2::new(-4.5, -4.5), c + Vec2::new(4.5, 4.5)], s);
            p.line_segment([c + Vec2::new(4.5, -4.5), c + Vec2::new(-4.5, 4.5)], s);
            close = close_r.clicked();

            let log_rect = Rect::from_min_size(Pos2::new(close_rect.left() - 76.0, rect.top() + 6.0), Vec2::new(70.0, 30.0));
            let log_r = ui.interact(log_rect, egui::Id::new("build-log"), Sense::click());
            p.text(log_rect.center(), Align2::CENTER_CENTER, "OPEN LOG", FontId::proportional(10.0), if log_r.hovered() { TEXT } else { SECONDARY });
            open_log = log_r.clicked();

            let body = Rect::from_min_max(rect.left_top() + Vec2::new(16.0, 44.0), rect.right_bottom() - Vec2::new(16.0, 12.0));
            let clipped = p.with_clip_rect(body);
            let lines = job.tail(12);
            let line_h = 15.0;
            let start = body.bottom() - line_h * lines.len() as f32;
            for (i, line) in lines.iter().enumerate() {
                let lower = line.to_ascii_lowercase();
                let color = if lower.starts_with("error") || lower.contains("failed") {
                    RED
                } else if lower.starts_with("warning") {
                    AMBER
                } else if lower.starts_with("staged:") || lower.contains("finished") {
                    CYAN
                } else {
                    SECONDARY
                };
                clipped.text(
                    Pos2::new(body.left(), start.max(body.top()) + i as f32 * line_h),
                    Align2::LEFT_TOP,
                    line.trim_end(),
                    FontId::monospace(11.0),
                    color,
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
        egui::Frame::none().inner_margin(Margin { left: 48.0, right: 48.0, top: 58.0, bottom: 0.0 }).show(ui, |ui| {
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
                        ui.label(RichText::new(&c.day).size(10.0).strong().color(MUTED));
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
// Settings

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(title).size(10.0).strong().color(MUTED));
    ui.add_space(6.0);
    egui::Frame::none().fill(PANEL).inner_margin(Margin::same(20.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
    ui.add_space(22.0);
}

fn flat_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.0).strong().color(TEXT))
            .fill(RAISED)
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(0.0))
            .min_size(Vec2::new(0.0, 34.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

impl LauncherApp {
    /// Returns true when "Change folder" was pressed (the dialog runs after the frame).
    fn settings_page(&mut self, ui: &mut egui::Ui, snap: &Snapshot) -> bool {
        let mut browse = false;
        let mut open_err = None;
        egui::Frame::none().inner_margin(Margin { left: 48.0, right: 48.0, top: 58.0, bottom: 0.0 }).show(ui, |ui| {
            page_header(ui, "SETTINGS", "Where the game lives and how it starts.");
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(760.0);
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
                            open_err = game::open_path(&self.game_root).err();
                        }
                    });
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
                        open_err = Some(format!("Couldn't save launch options: {e}"));
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("Added to every launch. Game flags:").size(11.0).color(SECONDARY));
                    ui.add_space(2.0);
                    for (flag, what) in [
                        ("--fly", "free-fly camera, no collision"),
                        ("--fov <deg>", "field of view for this run"),
                        ("--pos x,y,z", "spawn at a map position (feet)"),
                        ("--yaw <deg>  --pitch <deg>", "spawn view angles"),
                        ("--menu", "open the in-game menu at start"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(Vec2::new(190.0, 16.0), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_width(190.0);
                                ui.label(RichText::new(flag).monospace().size(11.0).color(TEXT));
                            });
                            ui.label(RichText::new(what).size(11.0).color(MUTED));
                        });
                    }
                });
                section(ui, "FILES", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if flat_button(ui, "GAME SETTINGS FILE") {
                            open_err = game::open_path(&self.game_root.join("r5rust_settings.toml")).err();
                        }
                        let dir = game::data_dir().unwrap_or_default();
                        if flat_button(ui, "LAST GAME LOG") {
                            open_err = game::open_path(&dir.join("game.log")).err();
                        }
                        if flat_button(ui, "LAST BUILD LOG") {
                            open_err = game::open_path(&dir.join("build.log")).err();
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Keybinds, sensitivity and FOV are also in the game's own menu (Esc).")
                            .size(11.0)
                            .color(MUTED),
                    );
                });
                section(ui, "ABOUT", |ui| {
                    ui.label(
                        RichText::new(format!(
                            "R5Rust Launcher v{}. Play runs play.ps1, which installs a staged build first. Build latest runs tools\\publish.ps1 (cargo build + stage); it never touches the copy you're playing.",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .size(12.0)
                        .color(SECONDARY),
                    );
                });
            });
        });
        if let Some(e) = open_err {
            self.notice = Some((e, Tone::Bad));
        }
        browse
    }
}

// -------------------------------------------------------------------------------------------------------------------
// Window chrome and painting helpers

fn draw_window_chrome(ctx: &egui::Context) {
    let screen = ctx.screen_rect();
    let bar_size = Vec2::new(screen.width(), 36.0);
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));

    egui::Area::new(egui::Id::new("window-chrome"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .default_size(bar_size)
        .show(ctx, |ui| {
            let (bar, _) = ui.allocate_exact_size(bar_size, Sense::hover());
            let close = Rect::from_min_size(Pos2::new(bar.right() - 40.0, bar.top() + 3.0), Vec2::new(36.0, 30.0));
            let maximize = Rect::from_min_size(Pos2::new(close.left() - 38.0, bar.top() + 3.0), Vec2::new(36.0, 30.0));
            let minimize = Rect::from_min_size(Pos2::new(maximize.left() - 38.0, bar.top() + 3.0), Vec2::new(36.0, 30.0));
            let drag_area = Rect::from_min_max(bar.min, Pos2::new(minimize.left(), bar.bottom()));
            let drag = ui.interact(drag_area, egui::Id::new("window-drag-area"), Sense::click_and_drag());
            if drag.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
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
            painter.rect_stroke(maximize.shrink(10.0), Rounding::same(0.0), s);
            let c = close.center();
            painter.line_segment([Pos2::new(c.x - 5.0, c.y - 5.0), Pos2::new(c.x + 5.0, c.y + 5.0)], s);
            painter.line_segment([Pos2::new(c.x + 5.0, c.y - 5.0), Pos2::new(c.x - 5.0, c.y + 5.0)], s);

            if min_r.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            if max_r.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if close_r.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
}

fn paint_vertical_scrim(painter: &egui::Painter, bounds: Rect, top_alpha: u8, bottom_alpha: u8) {
    paint_scrim(painter, bounds, [top_alpha, top_alpha, bottom_alpha, bottom_alpha]);
}

fn paint_horizontal_scrim(painter: &egui::Painter, bounds: Rect, left_alpha: u8, right_alpha: u8) {
    paint_scrim(painter, bounds, [left_alpha, right_alpha, right_alpha, left_alpha]);
}

/// Alphas for the corners top-left, top-right, bottom-right, bottom-left.
fn paint_scrim(painter: &egui::Painter, b: Rect, a: [u8; 4]) {
    let mut mesh = egui::Mesh::default();
    for (pos, alpha) in [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom()].into_iter().zip(a) {
        mesh.colored_vertex(pos, Color32::from_black_alpha(alpha));
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
    fonts.font_data.insert(
        "barlow-condensed-bold".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/BarlowCondensed-Bold.ttf")),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name("Barlow Condensed".into()), vec!["barlow-condensed-bold".to_owned()]);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = RAISED;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(63, 216, 255, 48);
    v.selection.stroke = Stroke::new(1.0_f32, CYAN);
    v.text_cursor.stroke = Stroke::new(2.0_f32, CYAN);
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
    ctx.set_style(style);
}

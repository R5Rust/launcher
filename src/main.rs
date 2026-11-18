#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Rounding, Sense, Stroke,
    Vec2,
};

const DEFAULT_GAME_DIR: &str = r"C:\Code\games\r5rust";
const SIDE: Color32 = Color32::from_rgb(14, 16, 21);
const BG: Color32 = Color32::from_rgb(20, 22, 27);
const CARD: Color32 = Color32::from_rgb(27, 29, 34);
const ROW: Color32 = Color32::from_rgb(34, 36, 41);
const TEXT: Color32 = Color32::from_rgb(228, 230, 235);
const MUTED: Color32 = Color32::from_rgb(138, 140, 145);
const DIM: Color32 = Color32::from_rgb(91, 93, 98);
const CLAY: Color32 = Color32::from_rgb(217, 119, 87);
const OK: Color32 = Color32::from_rgb(143, 207, 156);
const WARN: Color32 = Color32::from_rgb(255, 184, 108);
const LINE: Color32 = Color32::from_rgb(48, 50, 55);

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("R5Rust Launcher")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([900.0, 640.0])
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
    egui::IconData {
        rgba: icon.into_raw(),
        width: 256,
        height: 256,
    }
}

struct LauncherApp {
    game_root: PathBuf,
    notice: Option<(String, bool)>,
    selected_mode: GameMode,
    cover: egui::TextureHandle,
    cover_size: Vec2,
    logo: egui::TextureHandle,
}

impl LauncherApp {
    fn new(ctx: &egui::Context) -> Self {
        apply_theme(ctx);

        let image = image::load_from_memory(include_bytes!("../assets/firing-range.jpg"))
            .expect("the Firing Range image should be available");
        let cover_size = Vec2::new(image.width() as f32, image.height() as f32);
        let cover_rgba = image.to_rgba8();
        let cover = ctx.load_texture(
            "firing-range-cover",
            egui::ColorImage::from_rgba_unmultiplied(
                [cover_size.x as usize, cover_size.y as usize],
                &cover_rgba,
            ),
            egui::TextureOptions::LINEAR,
        );

        let logo_image = image::load_from_memory(include_bytes!("../assets/icon.png"))
            .expect("the R5Rust app icon should be available");
        let logo_size = Vec2::new(logo_image.width() as f32, logo_image.height() as f32);
        let logo_rgba = logo_image.to_rgba8();
        let logo = ctx.load_texture(
            "r5rust-logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [logo_size.x as usize, logo_size.y as usize],
                &logo_rgba,
            ),
            egui::TextureOptions::LINEAR,
        );

        Self {
            game_root: load_game_root(),
            notice: None,
            selected_mode: GameMode::FiringRange,
            cover,
            cover_size,
            logo,
        }
    }

    fn state(&self) -> GameState {
        inspect_game(&self.game_root)
    }

    fn browse(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose the r5rust project folder");
        if self.game_root.is_dir() {
            dialog = dialog.set_directory(&self.game_root);
        }

        if let Some(folder) = dialog.pick_folder() {
            self.game_root = folder;
            self.notice = None;
            if let Err(error) = save_game_root(&self.game_root) {
                self.notice = Some((format!("Couldn't save this folder: {error}"), true));
            }
        }
    }

    fn launch(&mut self) {
        match self.selected_mode {
            GameMode::FiringRange => self.launch_game_script(),
        }
    }

    fn launch_game_script(&mut self) {
        let script = self.game_root.join("play.ps1");
        let mut command = Command::new("pwsh");
        command
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-File",
            ])
            .arg(script)
            .current_dir(&self.game_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW

        match command.spawn() {
            Ok(_) => self.notice = Some(("Starting Firing Range…".to_owned(), false)),
            Err(error) => {
                self.notice = Some((format!("Couldn't start PowerShell: {error}"), true));
            }
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let state = self.state();
        let mut browse_clicked = false;
        let mut launch_clicked = false;
        let mut select_firing_range = false;

        egui::SidePanel::left("launcher-sidebar")
            .exact_width(238.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(SIDE).inner_margin(18.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new((self.logo.id(), Vec2::splat(36.0)))
                            .fit_to_exact_size(Vec2::splat(36.0))
                            .rounding(Rounding::same(8.0)),
                    );
                    ui.add_space(9.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("R5Rust").size(16.0).strong().color(TEXT));
                        ui.label(RichText::new("LAUNCHER").size(9.0).strong().color(CLAY));
                    });
                });

                ui.add_space(30.0);
                ui.label(RichText::new("GAME MODES").size(10.0).strong().color(DIM));
                ui.add_space(10.0);

                if mode_tile(
                    ui,
                    &self.cover,
                    self.cover_size,
                    self.selected_mode == GameMode::FiringRange,
                ) {
                    select_firing_range = true;
                }

                ui.add_space(9.0);
                egui::Frame::none()
                    .fill(Color32::from_rgb(22, 24, 29))
                    .stroke(Stroke::new(1.0_f32, LINE))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(11.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("+ ").size(15.0).strong().color(DIM));
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("More modes").size(12.0).strong().color(MUTED),
                                );
                                ui.label(RichText::new("Coming later").size(10.0).color(DIM));
                            });
                        });
                    });

                let footer_space = (ui.available_height() - 170.0).max(24.0);
                ui.add_space(footer_space);
                ui.separator();
                ui.add_space(12.0);
                ui.label(RichText::new("GAME INSTALL").size(10.0).strong().color(DIM));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, state.color());
                    ui.label(
                        RichText::new(state.sidebar_status())
                            .size(11.0)
                            .strong()
                            .color(TEXT),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(self.game_root.display().to_string())
                        .size(9.0)
                        .monospace()
                        .color(DIM),
                );
                ui.add_space(9.0);
                if ui
                    .add_sized(
                        Vec2::new(ui.available_width(), 34.0),
                        egui::Button::new(RichText::new("Change game folder").size(11.0)),
                    )
                    .clicked()
                {
                    browse_clicked = true;
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(32.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PLAY").size(10.0).strong().color(DIM));
                    ui.label(RichText::new("/").size(10.0).color(DIM));
                    ui.label(
                        RichText::new("MODE SELECT")
                            .size(10.0)
                            .strong()
                            .color(MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        status_pill(ui, state);
                    });
                });

                ui.add_space(23.0);
                ui.label(
                    RichText::new("Choose your mode")
                        .size(29.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new(
                        "Jump into the range. More modes will appear here as they arrive.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(20.0);

                let card_width = ui.available_width();
                let card_height = (card_width * 0.46).clamp(290.0, 380.0);
                let card_size = Vec2::new(card_width, card_height);
                let card = egui::Image::new((self.cover.id(), self.cover_size))
                    .fit_to_exact_size(card_size)
                    .uv(cover_uv(self.cover_size, card_size))
                    .rounding(Rounding::same(12.0))
                    .sense(Sense::click());
                let response = ui.add(card);
                let rect = response.rect;
                let painter = ui.painter();

                let badge = Rect::from_min_size(
                    rect.left_top() + Vec2::new(17.0, 17.0),
                    Vec2::new(132.0, 25.0),
                );
                painter.rect_filled(badge, Rounding::same(6.0), Color32::from_black_alpha(175));
                painter.text(
                    badge.center(),
                    Align2::CENTER_CENTER,
                    "AVAILABLE MODE",
                    FontId::proportional(9.0),
                    TEXT,
                );

                let selected_badge = Rect::from_min_size(
                    Pos2::new(rect.right() - 102.0, rect.top() + 17.0),
                    Vec2::new(85.0, 25.0),
                );
                painter.rect_filled(
                    selected_badge,
                    Rounding::same(6.0),
                    Color32::from_black_alpha(175),
                );
                painter.circle_filled(
                    Pos2::new(selected_badge.left() + 14.0, selected_badge.center().y),
                    3.0,
                    OK,
                );
                painter.text(
                    Pos2::new(selected_badge.left() + 24.0, selected_badge.center().y),
                    Align2::LEFT_CENTER,
                    "SELECTED",
                    FontId::proportional(9.0),
                    TEXT,
                );

                let info = Rect::from_min_max(
                    Pos2::new(rect.left() + 12.0, rect.bottom() - 120.0),
                    Pos2::new(rect.right() - 12.0, rect.bottom() - 12.0),
                );
                painter.rect_filled(info, Rounding::same(9.0), Color32::from_black_alpha(198));
                painter.text(
                    Pos2::new(info.left() + 18.0, info.top() + 17.0),
                    Align2::LEFT_TOP,
                    "FIRING RANGE",
                    FontId::proportional(24.0),
                    TEXT,
                );
                painter.text(
                    Pos2::new(info.left() + 18.0, info.top() + 50.0),
                    Align2::LEFT_TOP,
                    "Movement practice  /  Free roam  /  Solo",
                    FontId::proportional(12.0),
                    Color32::from_rgb(205, 207, 212),
                );
                painter.rect_stroke(
                    rect,
                    Rounding::same(12.0),
                    Stroke::new(1.0_f32, if response.hovered() { CLAY } else { LINE }),
                );

                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("SELECTED MODE").size(9.0).strong().color(DIM));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(self.selected_mode.title())
                                .size(15.0)
                                .strong()
                                .color(TEXT),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let play = egui::Button::new(
                            RichText::new("Launch Firing Range  ›")
                                .size(13.0)
                                .strong()
                                .color(BG),
                        )
                        .fill(CLAY)
                        .stroke(Stroke::NONE)
                        .min_size(Vec2::new(198.0, 46.0));
                        launch_clicked = ui.add_enabled(state.is_ready(), play).clicked();
                    });
                });

                if let Some((message, is_error)) = &self.notice {
                    ui.add_space(12.0);
                    ui.label(RichText::new(message).size(11.0).color(if *is_error {
                        WARN
                    } else {
                        OK
                    }));
                }
            });

        if select_firing_range {
            self.selected_mode = GameMode::FiringRange;
        }
        if browse_clicked {
            self.browse();
        }
        if launch_clicked {
            self.launch();
        }
    }
}

fn mode_tile(
    ui: &mut egui::Ui,
    cover: &egui::TextureHandle,
    cover_size: Vec2,
    selected: bool,
) -> bool {
    let mut clicked = false;
    egui::Frame::none()
        .fill(if selected { ROW } else { CARD })
        .stroke(Stroke::new(1.0_f32, if selected { CLAY } else { LINE }))
        .rounding(Rounding::same(10.0))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let thumb_size = Vec2::new(54.0, 48.0);
                ui.add(
                    egui::Image::new((cover.id(), cover_size))
                        .fit_to_exact_size(thumb_size)
                        .uv(cover_uv(cover_size, thumb_size))
                        .rounding(Rounding::same(6.0)),
                );
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    if ui
                        .add_sized(
                            Vec2::new(ui.available_width(), 24.0),
                            egui::Button::new(
                                RichText::new("Firing Range")
                                    .size(12.0)
                                    .strong()
                                    .color(TEXT),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        clicked = true;
                    }
                    ui.label(RichText::new("AVAILABLE NOW").size(9.0).strong().color(OK));
                });
            });
        });
    clicked
}

fn cover_uv(image_size: Vec2, target_size: Vec2) -> Rect {
    let image_aspect = image_size.x / image_size.y;
    let target_aspect = target_size.x / target_size.y;
    if image_aspect > target_aspect {
        let visible_width = target_aspect / image_aspect;
        let crop = (1.0 - visible_width) * 0.5;
        Rect::from_min_max(Pos2::new(crop, 0.0), Pos2::new(1.0 - crop, 1.0))
    } else {
        let visible_height = image_aspect / target_aspect;
        let crop = (1.0 - visible_height) * 0.5;
        Rect::from_min_max(Pos2::new(0.0, crop), Pos2::new(1.0, 1.0 - crop))
    }
}

fn status_pill(ui: &mut egui::Ui, state: GameState) {
    let background = if state.is_ready() {
        Color32::from_rgb(31, 43, 36)
    } else {
        Color32::from_rgb(48, 39, 31)
    };
    egui::Frame::none()
        .fill(background)
        .rounding(Rounding::same(20.0))
        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.0, state.color());
                ui.label(
                    RichText::new(state.status_label())
                        .size(9.0)
                        .strong()
                        .color(state.color()),
                );
            });
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameMode {
    FiringRange,
}

impl GameMode {
    fn title(self) -> &'static str {
        match self {
            Self::FiringRange => "Firing Range",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameState {
    Ready { update_waiting: bool },
    FolderMissing,
    InvalidFolder,
    NoBuild,
}

impl GameState {
    fn is_ready(self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    fn sidebar_status(self) -> &'static str {
        match self {
            Self::Ready { .. } => "Build ready",
            Self::FolderMissing => "Folder not found",
            Self::InvalidFolder => "Invalid folder",
            Self::NoBuild => "Build not staged",
        }
    }

    fn status_label(self) -> &'static str {
        match self {
            Self::Ready {
                update_waiting: true,
            } => "UPDATE STAGED",
            Self::Ready {
                update_waiting: false,
            } => "READY TO PLAY",
            Self::FolderMissing | Self::InvalidFolder | Self::NoBuild => "SETUP NEEDED",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Ready { .. } => OK,
            Self::NoBuild => WARN,
            Self::FolderMissing | Self::InvalidFolder => DIM,
        }
    }
}

fn inspect_game(root: &Path) -> GameState {
    if !root.is_dir() {
        return GameState::FolderMissing;
    }
    if !root.join("play.ps1").is_file() {
        return GameState::InvalidFolder;
    }

    let current_build = root.join("play/current/r5rust.exe").is_file();
    let update_waiting = root.join("play/next/r5rust.exe").is_file();
    if current_build || update_waiting {
        GameState::Ready { update_waiting }
    } else {
        GameState::NoBuild
    }
}

fn load_game_root() -> PathBuf {
    if let Some(config) = config_path()
        && let Ok(saved) = fs::read_to_string(config)
    {
        let saved = saved.trim();
        if !saved.is_empty() {
            return PathBuf::from(saved);
        }
    }

    if let Some(env_root) = env::var_os("R5RUST_DIR") {
        let path = PathBuf::from(env_root);
        if path.join("play.ps1").is_file() {
            return path;
        }
    }

    if Path::new(DEFAULT_GAME_DIR).join("play.ps1").is_file() {
        return PathBuf::from(DEFAULT_GAME_DIR);
    }

    if let Ok(exe) = env::current_exe() {
        for ancestor in exe.ancestors() {
            let candidate = ancestor.join("games/r5rust");
            if candidate.join("play.ps1").is_file() {
                return candidate;
            }
        }
    }

    PathBuf::from(DEFAULT_GAME_DIR)
}

fn config_path() -> Option<PathBuf> {
    env::var_os("APPDATA").map(|app_data| {
        PathBuf::from(app_data)
            .join("R5Rust Launcher")
            .join("game_path.txt")
    })
}

fn save_game_root(root: &Path) -> std::io::Result<()> {
    let Some(config) = config_path() else {
        return Ok(());
    };
    let Some(parent) = config.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    fs::write(config, root.to_string_lossy().as_bytes())
}

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = CLAY;
    visuals.widgets.noninteractive.bg_fill = CARD;
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(36, 38, 43);
    visuals.widgets.inactive.fg_stroke.color = TEXT;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 50, 55);
    visuals.widgets.hovered.fg_stroke.color = TEXT;
    visuals.widgets.active.bg_fill = CLAY;
    visuals.widgets.active.fg_stroke.color = BG;
    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 8.0);
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        env::temp_dir().join(format!("r5rust-launcher-{label}-{stamp}"))
    }

    #[test]
    fn detects_a_staged_build_and_pending_update() {
        let root = temp_root("ready");
        fs::create_dir_all(root.join("play/current")).unwrap();
        fs::create_dir_all(root.join("play/next")).unwrap();
        fs::write(root.join("play.ps1"), "").unwrap();
        fs::write(root.join("play/current/r5rust.exe"), "").unwrap();
        fs::write(root.join("play/next/r5rust.exe"), "").unwrap();

        assert_eq!(
            inspect_game(&root),
            GameState::Ready {
                update_waiting: true
            }
        );
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
}

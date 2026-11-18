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
    egui::IconData {
        rgba: icon.into_raw(),
        width: 256,
        height: 256,
    }
}

fn display_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Barlow Condensed".into()))
}

struct LauncherApp {
    game_root: PathBuf,
    notice: Option<(String, bool)>,
    selected_mode: GameMode,
    mode_menu_open: bool,
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
            mode_menu_open: false,
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
            Err(error) => self.notice = Some((format!("Couldn't start PowerShell: {error}"), true)),
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let state = self.state();
        let mut browse_clicked = false;
        let mut launch_clicked = false;
        let mut select_firing_range = false;
        let mut toggle_modes_menu = false;
        let mut mode_menu_position = Pos2::ZERO;

        egui::SidePanel::left("launcher-rail")
            .exact_width(228.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(RAIL).inner_margin(0.0))
            .show(ctx, |ui| {
                ui.add_space(40.0);
                let brand_width = 140.0;
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - brand_width) * 0.5).max(0.0));
                    ui.add(
                        egui::Image::new((self.logo.id(), Vec2::splat(36.0)))
                            .fit_to_exact_size(Vec2::splat(36.0)),
                    );
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("R5RUST").font(display_font(21.0)).color(TEXT));
                        ui.label(RichText::new("LAUNCHER").size(9.0).strong().color(MUTED));
                    });
                });

                ui.add_space(34.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let nav_width = (ui.available_width() - 10.0).max(0.0);
                    let (nav_rect, nav_response) =
                        ui.allocate_exact_size(Vec2::new(nav_width, 44.0), Sense::click());
                    let nav_painter = ui.painter();
                    if nav_response.hovered() {
                        nav_painter.rect_filled(nav_rect, Rounding::same(0.0), HOVER);
                    }
                    nav_painter.rect_filled(
                        Rect::from_min_size(nav_rect.left_top(), Vec2::new(3.0, nav_rect.height())),
                        Rounding::same(0.0),
                        CYAN,
                    );
                    let icon_center = Pos2::new(nav_rect.left() + 28.0, nav_rect.center().y);
                    nav_painter.add(egui::Shape::convex_polygon(
                        vec![
                            Pos2::new(icon_center.x - 4.0, icon_center.y - 6.0),
                            Pos2::new(icon_center.x + 6.0, icon_center.y),
                            Pos2::new(icon_center.x - 4.0, icon_center.y + 6.0),
                        ],
                        CYAN,
                        Stroke::NONE,
                    ));
                    nav_painter.text(
                        Pos2::new(nav_rect.left() + 48.0, nav_rect.center().y),
                        Align2::LEFT_CENTER,
                        "PLAY",
                        FontId::proportional(13.0),
                        TEXT,
                    );
                    if nav_response.clicked() {
                        select_firing_range = true;
                    }
                    ui.add_space(10.0);
                });

                ui.add_space((ui.available_height() - 50.0).max(24.0));
                ui.separator();
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(
                        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .size(11.0)
                            .monospace()
                            .color(MUTED),
                    );
                });
            });

        egui::TopBottomPanel::bottom("play-footer")
            .exact_height(94.0)
            .frame(
                egui::Frame::none()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(30.0, 13.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(state.footer_title())
                                .size(11.0)
                                .strong()
                                .color(SECONDARY),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(self.game_root.display().to_string())
                                .size(10.0)
                                .monospace()
                                .color(MUTED),
                        );
                        if let Some((message, is_error)) = &self.notice {
                            ui.label(RichText::new(message).size(10.0).color(if *is_error {
                                TEXT
                            } else {
                                SECONDARY
                            }));
                        }
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let play_button = egui::Button::new(
                            RichText::new("PLAY")
                                .font(display_font(24.0))
                                .color(Color32::from_rgb(8, 19, 26)),
                        )
                        .fill(if state.is_ready() { CYAN } else { RAISED })
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(0.0))
                        .min_size(Vec2::new(196.0, 56.0));
                        launch_clicked = ui.add_enabled(state.is_ready(), play_button).clicked();

                        ui.add_space(12.0);
                        let folder_name = self
                            .game_root
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Choose a folder".to_owned());
                        let (folder_rect, folder_response) =
                            ui.allocate_exact_size(Vec2::new(228.0, 56.0), Sense::click());
                        let folder_painter = ui.painter();
                        folder_painter.rect_filled(
                            folder_rect,
                            Rounding::same(0.0),
                            if folder_response.hovered() {
                                HOVER
                            } else {
                                RAISED
                            },
                        );
                        folder_painter.text(
                            Pos2::new(folder_rect.left() + 16.0, folder_rect.top() + 14.0),
                            Align2::LEFT_TOP,
                            "GAME FOLDER",
                            FontId::proportional(9.0),
                            MUTED,
                        );
                        folder_painter.text(
                            Pos2::new(folder_rect.left() + 16.0, folder_rect.top() + 34.0),
                            Align2::LEFT_TOP,
                            folder_name,
                            FontId::proportional(13.0),
                            TEXT,
                        );
                        let chevron =
                            Pos2::new(folder_rect.right() - 18.0, folder_rect.center().y + 1.0);
                        folder_painter.line_segment(
                            [Pos2::new(chevron.x - 4.0, chevron.y - 2.0), chevron],
                            Stroke::new(1.2_f32, SECONDARY),
                        );
                        folder_painter.line_segment(
                            [chevron, Pos2::new(chevron.x + 4.0, chevron.y - 2.0)],
                            Stroke::new(1.2_f32, SECONDARY),
                        );
                        browse_clicked = folder_response.clicked();
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(0.0))
            .show(ctx, |ui| {
                let hero_size = ui.available_size();
                let (hero_rect, _) = ui.allocate_exact_size(hero_size, Sense::hover());
                let painter = ui.painter();
                painter.image(
                    self.cover.id(),
                    hero_rect,
                    cover_uv(self.cover_size, hero_size),
                    Color32::WHITE,
                );

                let top_scrim = Rect::from_min_max(
                    hero_rect.min,
                    Pos2::new(
                        hero_rect.right(),
                        hero_rect.top() + hero_rect.height() * 0.31,
                    ),
                );
                paint_vertical_scrim(painter, top_scrim, 172, 0);

                let bottom_scrim = Rect::from_min_max(
                    Pos2::new(
                        hero_rect.left(),
                        hero_rect.top() + hero_rect.height() * 0.43,
                    ),
                    hero_rect.max,
                );
                paint_vertical_scrim(painter, bottom_scrim, 0, 226);

                let left_scrim = Rect::from_min_max(
                    hero_rect.min,
                    Pos2::new(
                        hero_rect.left() + hero_rect.width() * 0.50,
                        hero_rect.bottom(),
                    ),
                );
                paint_horizontal_scrim(painter, left_scrim, 142, 0);

                let mode_tab = Rect::from_min_size(
                    Pos2::new(hero_rect.left() + 40.0, hero_rect.top() + 42.0),
                    Vec2::new(132.0, 38.0),
                );
                let mode_response =
                    ui.interact(mode_tab, egui::Id::new("mode-tab"), Sense::click());
                painter.text(
                    Pos2::new(mode_tab.left(), mode_tab.top() + 8.0),
                    Align2::LEFT_TOP,
                    "MODES",
                    FontId::new(17.0, egui::FontFamily::Name("Barlow Condensed".into())),
                    TEXT,
                );
                let arrow = Pos2::new(mode_tab.left() + 78.0, mode_tab.top() + 18.0);
                painter.line_segment(
                    [Pos2::new(arrow.x - 4.0, arrow.y - 2.0), arrow],
                    Stroke::new(1.3_f32, TEXT),
                );
                painter.line_segment(
                    [arrow, Pos2::new(arrow.x + 4.0, arrow.y - 2.0)],
                    Stroke::new(1.3_f32, TEXT),
                );
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(mode_tab.left(), mode_tab.bottom() - 2.0),
                        Vec2::new(72.0, 2.0),
                    ),
                    Rounding::same(0.0),
                    CYAN,
                );
                mode_menu_position = Pos2::new(mode_tab.left(), mode_tab.bottom() + 10.0);
                if mode_response.clicked() {
                    toggle_modes_menu = true;
                }

                painter.text(
                    Pos2::new(hero_rect.right() - 36.0, hero_rect.top() + 55.0),
                    Align2::RIGHT_CENTER,
                    state.status_label(),
                    FontId::proportional(10.0),
                    SECONDARY,
                );

                let title_x = hero_rect.left() + 42.0;
                let title_y = hero_rect.bottom() - 168.0;
                painter.text(
                    Pos2::new(title_x, title_y),
                    Align2::LEFT_TOP,
                    "TRAINING MODE",
                    FontId::proportional(11.0),
                    CYAN,
                );
                painter.text(
                    Pos2::new(title_x - 2.0, title_y + 15.0),
                    Align2::LEFT_TOP,
                    self.selected_mode.title().to_uppercase(),
                    display_font(68.0),
                    TEXT,
                );
                painter.text(
                    Pos2::new(title_x + 2.0, title_y + 91.0),
                    Align2::LEFT_TOP,
                    "Move freely. Practice your movement. Make the range yours.",
                    FontId::proportional(14.0),
                    SECONDARY,
                );
                painter.text(
                    Pos2::new(title_x + 2.0, title_y + 117.0),
                    Align2::LEFT_TOP,
                    "LOCAL SESSION   /   SOLO",
                    FontId::monospace(10.0),
                    MUTED,
                );
            });

        if toggle_modes_menu {
            self.mode_menu_open = !self.mode_menu_open;
        }

        let mut choose_mode = false;
        if self.mode_menu_open {
            egui::Area::new(egui::Id::new("mode-menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(mode_menu_position)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.set_width(312.0);
                            ui.label(RichText::new("GAME MODES").size(10.0).strong().color(MUTED));
                            ui.add_space(8.0);

                            let (row_rect, row_response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 74.0),
                                Sense::click(),
                            );
                            let menu_painter = ui.painter();
                            menu_painter.rect_filled(
                                row_rect,
                                Rounding::same(0.0),
                                if row_response.hovered() {
                                    HOVER
                                } else {
                                    RAISED
                                },
                            );
                            menu_painter.rect_filled(
                                Rect::from_min_size(
                                    row_rect.left_top(),
                                    Vec2::new(2.0, row_rect.height()),
                                ),
                                Rounding::same(0.0),
                                CYAN,
                            );
                            let thumb_rect = Rect::from_min_size(
                                row_rect.left_top() + Vec2::new(10.0, 9.0),
                                Vec2::new(88.0, 56.0),
                            );
                            menu_painter.image(
                                self.cover.id(),
                                thumb_rect,
                                cover_uv(self.cover_size, thumb_rect.size()),
                                Color32::WHITE,
                            );
                            menu_painter.text(
                                Pos2::new(thumb_rect.right() + 12.0, row_rect.top() + 23.0),
                                Align2::LEFT_TOP,
                                "Firing Range",
                                FontId::proportional(13.0),
                                TEXT,
                            );
                            menu_painter.text(
                                Pos2::new(thumb_rect.right() + 12.0, row_rect.top() + 45.0),
                                Align2::LEFT_TOP,
                                "AVAILABLE",
                                FontId::proportional(9.0),
                                SECONDARY,
                            );
                            if row_response.clicked() {
                                choose_mode = true;
                            }

                            ui.add_space(9.0);
                            ui.separator();
                            ui.add_space(8.0);
                            ui.label(RichText::new("MORE MODES").size(9.0).strong().color(MUTED));
                            ui.label(RichText::new("Coming later").size(11.0).color(SECONDARY));
                        });
                });
        }

        if choose_mode {
            select_firing_range = true;
            self.mode_menu_open = false;
        }
        if select_firing_range {
            self.selected_mode = GameMode::FiringRange;
        }
        draw_window_chrome(ctx);
        if browse_clicked {
            self.browse();
        }
        if launch_clicked {
            self.launch();
        }
    }
}

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
            let close = Rect::from_min_size(
                Pos2::new(bar.right() - 40.0, bar.top() + 3.0),
                Vec2::new(36.0, 30.0),
            );
            let maximize = Rect::from_min_size(
                Pos2::new(close.left() - 38.0, bar.top() + 3.0),
                Vec2::new(36.0, 30.0),
            );
            let minimize = Rect::from_min_size(
                Pos2::new(maximize.left() - 38.0, bar.top() + 3.0),
                Vec2::new(36.0, 30.0),
            );
            let drag_area = Rect::from_min_max(bar.min, Pos2::new(minimize.left(), bar.bottom()));
            let drag = ui.interact(
                drag_area,
                egui::Id::new("window-drag-area"),
                Sense::click_and_drag(),
            );
            if drag.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }

            let minimize_response =
                ui.interact(minimize, egui::Id::new("window-minimize"), Sense::click());
            let maximize_response =
                ui.interact(maximize, egui::Id::new("window-maximize"), Sense::click());
            let close_response = ui.interact(close, egui::Id::new("window-close"), Sense::click());
            let painter = ui.painter();

            if minimize_response.hovered() {
                painter.rect_filled(minimize, Rounding::same(0.0), HOVER);
            }
            if maximize_response.hovered() {
                painter.rect_filled(maximize, Rounding::same(0.0), HOVER);
            }
            if close_response.hovered() {
                painter.rect_filled(close, Rounding::same(0.0), Color32::from_rgb(224, 68, 58));
            }

            let icon_stroke = Stroke::new(1.2_f32, TEXT);
            painter.line_segment(
                [
                    Pos2::new(minimize.center().x - 5.0, minimize.center().y + 3.0),
                    Pos2::new(minimize.center().x + 5.0, minimize.center().y + 3.0),
                ],
                icon_stroke,
            );
            let max_icon = maximize.shrink(10.0);
            painter.rect_stroke(max_icon, Rounding::same(0.0), icon_stroke);
            let close_center = close.center();
            painter.line_segment(
                [
                    Pos2::new(close_center.x - 5.0, close_center.y - 5.0),
                    Pos2::new(close_center.x + 5.0, close_center.y + 5.0),
                ],
                icon_stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(close_center.x + 5.0, close_center.y - 5.0),
                    Pos2::new(close_center.x - 5.0, close_center.y + 5.0),
                ],
                icon_stroke,
            );

            if minimize_response.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            if maximize_response.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if close_response.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
}

fn paint_vertical_scrim(painter: &egui::Painter, bounds: Rect, top_alpha: u8, bottom_alpha: u8) {
    let mut mesh = egui::Mesh::default();
    let top = Color32::from_black_alpha(top_alpha);
    let bottom = Color32::from_black_alpha(bottom_alpha);
    mesh.colored_vertex(bounds.left_top(), top);
    mesh.colored_vertex(bounds.right_top(), top);
    mesh.colored_vertex(bounds.right_bottom(), bottom);
    mesh.colored_vertex(bounds.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_horizontal_scrim(painter: &egui::Painter, bounds: Rect, left_alpha: u8, right_alpha: u8) {
    let mut mesh = egui::Mesh::default();
    let left = Color32::from_black_alpha(left_alpha);
    let right = Color32::from_black_alpha(right_alpha);
    mesh.colored_vertex(bounds.left_top(), left);
    mesh.colored_vertex(bounds.right_top(), right);
    mesh.colored_vertex(bounds.right_bottom(), right);
    mesh.colored_vertex(bounds.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
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

    fn status_label(self) -> &'static str {
        match self {
            Self::Ready {
                update_waiting: true,
            } => "UPDATE STAGED",
            Self::Ready {
                update_waiting: false,
            } => "GAME BUILD READY",
            Self::FolderMissing => "FOLDER NOT FOUND",
            Self::InvalidFolder => "NOT AN R5RUST FOLDER",
            Self::NoBuild => "NO STAGED BUILD",
        }
    }

    fn footer_title(self) -> &'static str {
        match self {
            Self::Ready { .. } => "READY TO PLAY",
            Self::FolderMissing | Self::InvalidFolder | Self::NoBuild => "SETUP REQUIRED",
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
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "barlow-condensed-bold".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/BarlowCondensed-Bold.ttf")),
    );
    fonts.families.insert(
        egui::FontFamily::Name("Barlow Condensed".into()),
        vec!["barlow-condensed-bold".to_owned()],
    );
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(63, 216, 255, 32);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(0.0_f32, Color32::TRANSPARENT);
    visuals.widgets.inactive.fg_stroke.color = SECONDARY;
    visuals.widgets.inactive.rounding = Rounding::same(0.0);
    visuals.widgets.hovered.bg_fill = HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(0.0_f32, Color32::TRANSPARENT);
    visuals.widgets.hovered.fg_stroke.color = TEXT;
    visuals.widgets.hovered.rounding = Rounding::same(0.0);
    visuals.widgets.active.bg_fill = CYAN;
    visuals.widgets.active.bg_stroke = Stroke::new(0.0_f32, Color32::TRANSPARENT);
    visuals.widgets.active.fg_stroke.color = BG;
    visuals.widgets.active.rounding = Rounding::same(0.0);
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

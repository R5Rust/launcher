#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use eframe::egui::{self, Align, Color32, Layout, RichText, Stroke, Vec2};

const DEFAULT_GAME_DIR: &str = r"C:\Code\games\r5rust";
const BG: Color32 = Color32::from_rgb(20, 22, 27);
const CARD: Color32 = Color32::from_rgb(27, 29, 34);
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
            .with_inner_size([560.0, 410.0])
            .with_min_inner_size([460.0, 360.0]),
        ..Default::default()
    };

    eframe::run_native(
        "R5Rust Launcher",
        options,
        Box::new(|cc| Ok(Box::new(LauncherApp::new(&cc.egui_ctx)))),
    )
}

struct LauncherApp {
    game_root: PathBuf,
    notice: Option<(String, bool)>,
}

impl LauncherApp {
    fn new(ctx: &egui::Context) -> Self {
        apply_theme(ctx);
        Self {
            game_root: load_game_root(),
            notice: None,
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
            Ok(_) => self.notice = Some(("Starting R5Rust…".to_owned(), false)),
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

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(24.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("R5Rust").size(16.0).strong().color(TEXT));
                    ui.label(RichText::new("LAUNCHER").size(10.0).strong().color(CLAY));
                });

                ui.add_space(20.0);
                ui.label(
                    RichText::new("Back to the range.")
                        .size(26.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("Launch your local R5Rust build.")
                        .size(13.0)
                        .color(MUTED),
                );
                ui.add_space(20.0);

                egui::Frame::group(ui.style())
                    .fill(CARD)
                    .stroke(Stroke::new(1.0_f32, LINE))
                    .inner_margin(18.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());

                        ui.horizontal(|ui| {
                            let (dot, _) =
                                ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.0, state.color());
                            ui.label(RichText::new(state.title()).size(15.0).strong().color(TEXT));
                        });

                        ui.add_space(4.0);
                        ui.label(RichText::new(state.detail()).size(12.0).color(MUTED));
                        ui.add_space(16.0);

                        ui.horizontal(|ui| {
                            ui.label(RichText::new("GAME FOLDER").size(10.0).strong().color(DIM));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button(RichText::new("Browse…").color(TEXT)).clicked() {
                                    browse_clicked = true;
                                }
                            });
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(self.game_root.display().to_string())
                                .size(11.0)
                                .monospace()
                                .color(MUTED),
                        );

                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            let play =
                                egui::Button::new(RichText::new("Play R5Rust").strong().color(BG))
                                    .fill(CLAY)
                                    .stroke(Stroke::NONE)
                                    .min_size(Vec2::new(142.0, 40.0));
                            launch_clicked = ui.add_enabled(state.is_ready(), play).clicked();
                        });
                    });

                if let Some((message, is_error)) = &self.notice {
                    ui.add_space(12.0);
                    ui.label(RichText::new(message).size(12.0).color(if *is_error {
                        WARN
                    } else {
                        OK
                    }));
                }
            });

        if browse_clicked {
            self.browse();
        }
        if launch_clicked {
            self.launch();
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

    fn title(self) -> &'static str {
        match self {
            Self::Ready { .. } => "Ready to launch",
            Self::FolderMissing => "Choose your game folder",
            Self::InvalidFolder => "That isn't the r5rust folder",
            Self::NoBuild => "No game build found",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Ready {
                update_waiting: true,
            } => "A newer staged build will be applied when you launch.",
            Self::Ready {
                update_waiting: false,
            } => "Your staged build is ready.",
            Self::FolderMissing => "Select the folder that contains r5rust's play.ps1.",
            Self::InvalidFolder => "Choose the r5rust project folder; it should contain play.ps1.",
            Self::NoBuild => "Build and stage the game with tools\\publish.ps1 first.",
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

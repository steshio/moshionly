//! The [`eframe::App`] that owns the editor window.

use std::sync::Arc;

use egui::{IconData, ViewportBuilder, ViewportCommand};

use crate::{
    APP_NAME,
    commands::CommandId,
    dialogs,
    settings::Settings,
    shell::{Shell, ShellOutput},
    theme::{self, brand},
    widgets::Toast,
};

/// The identifier of the app, used by the operating system to group its
/// windows and find its settings.
pub const APP_ID: &str = "org.moshionly.app";

/// Room the macOS window buttons take at the left of the top bar, in points.
const MAC_WINDOW_BUTTONS: f32 = 76.0;

/// Side of the window icon, in pixels.
const ICON_SIZE: u32 = 256;

/// The window icon: Moshionly's mark.
pub fn window_icon() -> IconData {
    IconData {
        rgba: brand::icon_rgba(ICON_SIZE),
        width: ICON_SIZE,
        height: ICON_SIZE,
    }
}

/// How the editor window is created: its size, icon and title bar.
///
/// On macOS the content fills the whole window and the system title bar is
/// transparent, so the editor's own top bar sits where the title would be,
/// beside the window buttons.
pub fn native_options() -> eframe::NativeOptions {
    let mut viewport = ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id(APP_ID)
        .with_icon(Arc::new(window_icon()))
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([960.0, 600.0]);
    if cfg!(target_os = "macos") {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}

/// The window title for a project: its name, then the app's.
pub fn window_title(project_name: &str) -> String {
    let name = project_name.trim();
    if name.is_empty() {
        APP_NAME.to_owned()
    } else {
        format!("{name} – {APP_NAME}")
    }
}

/// The running editor: one window, one document.
pub struct MoshionlyApp {
    shell: Shell,
    /// The title the window was last given.
    title: String,
}

impl MoshionlyApp {
    /// Creates the app. Called once by `eframe` when the window is ready.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        // ⌘+ and ⌘- zoom the canvas here, not the whole interface.
        cc.egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        let settings = Settings::load(cc.storage);
        theme::set_mode(&cc.egui_ctx, settings.theme);
        let mut shell = Shell::untitled(settings);
        shell.set_dialogs(Box::new(dialogs::NativeDialogs));
        if cfg!(target_os = "macos") {
            shell.use_top_bar_as_title_bar(MAC_WINDOW_BUTTONS);
        }
        Self {
            shell,
            title: APP_NAME.to_owned(),
        }
    }

    /// Carries out the commands the shell handed up: the ones that need a
    /// file dialog.
    fn handle(&mut self, output: ShellOutput) {
        for id in output.commands {
            // TODO: these stop at the dialog until the shell is bound to the
            // document engine and the exporter.
            let chosen = match id {
                CommandId::OpenProject => dialogs::open_project(),
                CommandId::SaveAs => dialogs::save_project(&self.shell.project_name()),
                _ => {
                    self.shell.toast(Toast::info(format!(
                        "\"{}\" is not available yet",
                        id.name().trim_end_matches('…')
                    )));
                    continue;
                }
            };
            if let Some(path) = chosen {
                self.shell.toast(Toast::info(format!(
                    "Chose {}. Projects are not connected yet.",
                    path.file_name()
                        .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy())
                )));
            }
        }
    }
}

impl eframe::App for MoshionlyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let output = self.shell.show(ui);
        self.handle(output);

        let title = window_title(&self.shell.project_name());
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.shell.settings().save(storage);
    }

    /// The shell keeps its own state; egui's memory of open popups and
    /// scroll positions is better started fresh.
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_title_names_the_project_first() {
        assert_eq!(window_title("Launch teaser"), "Launch teaser – Moshionly");
        assert_eq!(window_title("  "), "Moshionly");
        assert_eq!(window_title(""), "Moshionly");
    }

    #[test]
    fn the_window_icon_is_a_square_of_pixels() {
        let icon = window_icon();
        assert_eq!((icon.width, icon.height), (ICON_SIZE, ICON_SIZE));
        assert_eq!(icon.rgba.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        assert_eq!(icon.width % 4, 0);
    }

    #[test]
    fn the_window_opens_at_a_usable_size() {
        let options = native_options();
        let size = options.viewport.inner_size.unwrap();
        let min = options.viewport.min_inner_size.unwrap();
        assert!(size.x >= min.x && size.y >= min.y);
        assert_eq!(options.viewport.title.as_deref(), Some(APP_NAME));
        assert!(options.viewport.icon.is_some());
    }
}

//! The [`eframe::App`] that owns the editor window.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{IconData, ViewportBuilder, ViewportCommand};

use crate::{
    APP_NAME, dialogs,
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

/// The window title for a project: its name, marked when it has unsaved
/// changes, then the app's.
pub fn window_title(project_name: &str, unsaved: bool) -> String {
    let name = project_name.trim();
    match (name.is_empty(), unsaved) {
        (true, _) => APP_NAME.to_owned(),
        (false, false) => format!("{name} – {APP_NAME}"),
        (false, true) => format!("{name} (edited) – {APP_NAME}"),
    }
}

/// The project named on the command line (`moshionly path/to/file.moshi`):
/// the first argument that is not an option. `args` leaves out the
/// program's own name.
pub fn project_argument(args: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    args.into_iter()
        .find(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
}

/// The running editor: one window, one document.
pub struct MoshionlyApp {
    shell: Shell,
    /// The title the window was last given.
    title: String,
}

impl MoshionlyApp {
    /// Creates the app. Called once by `eframe` when the window is ready.
    /// `project` is a file to open straight away, for example one given on
    /// the command line.
    pub fn new(cc: &eframe::CreationContext<'_>, project: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        // ⌘+ and ⌘- zoom the canvas here, not the whole interface.
        cc.egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        let settings = Settings::load(cc.storage);
        theme::set_mode(&cc.egui_ctx, settings.theme);
        let mut shell = Shell::untitled(settings);
        shell.set_dialogs(Box::new(dialogs::NativeDialogs));
        // Sound plays through the computer's speakers.
        shell.set_audio(Box::new(crate::audio::DeviceAudio::new()));
        if cfg!(target_os = "macos") {
            shell.use_top_bar_as_title_bar(MAC_WINDOW_BUTTONS);
        }
        if let Some(project) = project {
            shell.open_path(&project);
        }
        // Agents outside the window, and the Figma plugin, reach the open
        // document here unless the settings turn it off.
        shell.serve_agents(crate::connect::DEFAULT_PORT);
        // The AI chat, with the person's AI settings and keys in the
        // system's keychain. Nothing is checked until the panel is shown.
        shell.set_chat(crate::chat::Chat::new());
        Self {
            shell,
            title: APP_NAME.to_owned(),
        }
    }

    /// Carries out what the shell handed up: closing the window, and the
    /// commands that are not available yet.
    fn handle(&mut self, ctx: &egui::Context, output: ShellOutput) {
        if output.close_window {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        for id in output.commands {
            self.shell.toast(Toast::info(format!(
                "\"{}\" is not available yet",
                id.name().trim_end_matches('…')
            )));
        }
    }
}

impl eframe::App for MoshionlyApp {
    /// Runs before every frame and also while the window is covered or
    /// minimized, when `ui` does not: a design sent from Figma is then still
    /// added, a covered window comes forward, and a minimized or hidden one
    /// has its Dock icon ask for attention.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shell.receive_imports(Some(ctx));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Closing with unsaved changes asks first; the shell closes the
        // window itself once the person has answered.
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && !self.shell.may_close() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        let output = self.shell.show(ui);
        self.handle(&ctx, output);

        let title = window_title(
            &self.shell.project_name(),
            self.shell.editor().has_unsaved_changes(),
        );
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.shell.settings().save(storage);
    }

    /// `main` ends the process as soon as the window has closed, so what
    /// runs in the background is stopped and cleaned up here.
    fn on_exit(&mut self) {
        self.shell.prepare_to_quit();
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
        assert_eq!(
            window_title("Launch teaser", false),
            "Launch teaser – Moshionly"
        );
        assert_eq!(
            window_title("Launch teaser", true),
            "Launch teaser (edited) – Moshionly"
        );
        assert_eq!(window_title("  ", true), "Moshionly");
        assert_eq!(window_title("", false), "Moshionly");
    }

    #[test]
    fn the_project_to_open_comes_from_the_command_line() {
        let args = |list: &[&str]| project_argument(list.iter().map(|s| s.to_string()));
        assert_eq!(args(&[]), None);
        assert_eq!(args(&["teaser.moshi"]), Some(PathBuf::from("teaser.moshi")));
        // Options are not files.
        assert_eq!(
            args(&["--verbose", "a.moshi"]),
            Some(PathBuf::from("a.moshi"))
        );
        assert_eq!(args(&["-psn_0_1234"]), None);
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

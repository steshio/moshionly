//! The [`eframe::App`] that owns the editor window.

use crate::{
    commands::CommandId,
    dialogs,
    settings::Settings,
    shell::{Shell, ShellOutput},
    theme,
    widgets::Toast,
};

/// The running editor: one window, one document.
pub struct MoshionlyApp {
    shell: Shell,
}

impl MoshionlyApp {
    /// Creates the app. Called once by `eframe` when the window is ready.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let settings = Settings::load(cc.storage);
        theme::set_mode(&cc.egui_ctx, settings.theme);
        Self {
            shell: Shell::new(settings),
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
                CommandId::SaveAs => dialogs::save_project("Untitled"),
                CommandId::ImportImage => {
                    dialogs::import_images().and_then(|paths| paths.into_iter().next())
                }
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

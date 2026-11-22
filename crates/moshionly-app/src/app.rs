//! The [`eframe::App`] that owns the editor window.

/// The running editor: one window, one document.
#[derive(Default)]
pub struct MoshionlyApp {}

impl MoshionlyApp {
    /// Creates the app. Called once by `eframe` when the window is ready.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::theme::install(&cc.egui_ctx);
        Self::default()
    }
}

impl eframe::App for MoshionlyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.label("Moshionly");
        });
    }
}

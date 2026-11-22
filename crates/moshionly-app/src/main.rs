//! Moshionly desktop app entry point.

use moshionly_app::{
    APP_NAME,
    app::{MoshionlyApp, native_options},
};

fn main() -> eframe::Result {
    eframe::run_native(
        APP_NAME,
        native_options(),
        Box::new(|cc| Ok(Box::new(MoshionlyApp::new(cc)))),
    )
}

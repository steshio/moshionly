//! Moshionly desktop app entry point.

//!
//! `moshionly path/to/file.moshi` opens that project.

use moshionly_app::{
    APP_NAME,
    app::{MoshionlyApp, native_options, project_argument},
};

fn main() -> eframe::Result {
    let project = project_argument(std::env::args().skip(1));
    eframe::run_native(
        APP_NAME,
        native_options(),
        Box::new(|cc| Ok(Box::new(MoshionlyApp::new(cc, project)))),
    )
}

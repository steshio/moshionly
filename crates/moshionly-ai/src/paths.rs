//! Where Moshionly keeps its AI-related files.

use std::path::PathBuf;

/// The folder name under the platform's configuration directory.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const APP_DIR: &str = "Moshionly";
/// The folder name under the platform's configuration directory.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const APP_DIR: &str = "moshionly";

/// The app's configuration directory, for example
/// `~/Library/Application Support/Moshionly` on macOS or
/// `~/.config/moshionly` on Linux. `None` when the platform has no notion of
/// a home directory.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_DIR))
}

/// The directory agent CLIs run in when a request does not name one.
///
/// It is owned by the app and holds nothing of the user's, so an agent has
/// nothing to read there. It is also stable between runs, which matters
/// because agents look up a session to resume by the directory it was
/// started in.
pub fn default_working_dir() -> PathBuf {
    dirs::data_local_dir()
        .map(|dir| dir.join(APP_DIR))
        .unwrap_or_else(|| std::env::temp_dir().join("moshionly"))
        .join("agent-workspace")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_dir_is_inside_an_app_owned_folder() {
        let dir = default_working_dir();
        assert!(dir.ends_with("agent-workspace"));
        let parent = dir.parent().unwrap().file_name().unwrap().to_string_lossy();
        assert!(parent.eq_ignore_ascii_case("moshionly"));
    }

    #[test]
    fn config_dir_ends_with_the_app_name() {
        if let Some(dir) = config_dir() {
            let name = dir.file_name().unwrap().to_string_lossy();
            assert!(name.eq_ignore_ascii_case("moshionly"));
        }
    }
}

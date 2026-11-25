//! Native file dialogs.
//!
//! Thin helpers over the operating system's open and save panels, so the rest
//! of the app never spells out file filters. Each one blocks until the user
//! answers and returns `None` when they cancel.
//!
//! The editor asks through the [`Dialogs`] trait, so that tests can answer
//! in the user's place: [`NativeDialogs`] shows the real panels and
//! [`NoDialogs`] cancels every question.

use std::path::{Path, PathBuf};

use moshionly_export::Format;

/// File extension of Moshionly projects, without the dot.
pub const PROJECT_EXTENSION: &str = "moshi";

/// Image formats that can be placed on the canvas.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// Video formats offered for import. Anything `ffmpeg` reads works; these
/// are the common ones.
pub const VIDEO_EXTENSIONS: [&str; 5] = ["mp4", "mov", "m4v", "webm", "mkv"];

/// Sound formats offered for import.
pub const SOUND_EXTENSIONS: [&str; 8] = ["mp3", "wav", "m4a", "aac", "flac", "ogg", "aif", "aiff"];

/// The questions the editor asks with a file dialog.
pub trait Dialogs {
    /// Which project to open.
    fn open_project(&self) -> Option<PathBuf>;
    /// Where to save a project named `suggested_name`.
    fn save_project(&self, suggested_name: &str) -> Option<PathBuf>;
    /// Which images to import.
    fn import_images(&self) -> Option<Vec<PathBuf>>;
    /// Which video and sound files to import. Nothing is asked unless a
    /// dialog says otherwise.
    fn import_media(&self) -> Option<Vec<PathBuf>> {
        None
    }
    /// Where to write an export in `format`, named after `suggested_name`.
    /// For a PNG sequence this is a new folder. Nothing is asked unless a
    /// dialog says otherwise.
    fn export_to(&self, suggested_name: &str, format: Format) -> Option<PathBuf> {
        let _ = (suggested_name, format);
        None
    }
}

/// The operating system's own dialogs.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeDialogs;

impl Dialogs for NativeDialogs {
    fn open_project(&self) -> Option<PathBuf> {
        open_project()
    }

    fn save_project(&self, suggested_name: &str) -> Option<PathBuf> {
        save_project(suggested_name)
    }

    fn import_images(&self) -> Option<Vec<PathBuf>> {
        import_images()
    }

    fn import_media(&self) -> Option<Vec<PathBuf>> {
        import_media()
    }

    fn export_to(&self, suggested_name: &str, format: Format) -> Option<PathBuf> {
        export_to(suggested_name, format)
    }
}

/// Dialogs that are cancelled straight away: for tests and for running
/// without a window.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDialogs;

impl Dialogs for NoDialogs {
    fn open_project(&self) -> Option<PathBuf> {
        None
    }

    fn save_project(&self, _suggested_name: &str) -> Option<PathBuf> {
        None
    }

    fn import_images(&self) -> Option<Vec<PathBuf>> {
        None
    }
}

/// Whether `path` names an image the editor can place.
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        IMAGE_EXTENSIONS
            .iter()
            .any(|known| e.eq_ignore_ascii_case(known))
    })
}

/// Whether `path` names a video or sound file the editor offers to import.
pub fn is_media(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        VIDEO_EXTENSIONS
            .iter()
            .chain(&SOUND_EXTENSIONS)
            .any(|known| e.eq_ignore_ascii_case(known))
    })
}

/// Whether `path` names a Moshionly project.
pub fn is_project(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION))
}

/// Asks which project to open.
pub fn open_project() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open project")
        .add_filter("Moshionly project", &[PROJECT_EXTENSION])
        .pick_file()
}

/// Asks where to save a project. `suggested_name` is the project's name,
/// with or without the extension.
pub fn save_project(suggested_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Save project")
        .add_filter("Moshionly project", &[PROJECT_EXTENSION])
        .set_file_name(file_name(suggested_name, PROJECT_EXTENSION))
        .save_file()
}

/// Asks which images to import.
pub fn import_images() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new()
        .set_title("Import image")
        .add_filter("Images", &IMAGE_EXTENSIONS)
        .pick_files()
}

/// Asks which video and sound files to import.
pub fn import_media() -> Option<Vec<PathBuf>> {
    let all: Vec<&str> = VIDEO_EXTENSIONS
        .iter()
        .chain(&SOUND_EXTENSIONS)
        .copied()
        .collect();
    rfd::FileDialog::new()
        .set_title("Import video or sound")
        .add_filter("Video and sound", &all)
        .add_filter("Video", &VIDEO_EXTENSIONS)
        .add_filter("Sound", &SOUND_EXTENSIONS)
        .pick_files()
}

/// Asks where to write an export in `format`. `suggested_name` is usually the
/// project's or the artboard's name. A PNG sequence is written as a new
/// folder of that name.
pub fn export_to(suggested_name: &str, format: Format) -> Option<PathBuf> {
    let dialog = rfd::FileDialog::new().set_title(format!("Export {}", format.label()));
    match format.extension() {
        Some(extension) => dialog
            .add_filter(format.label(), &[extension])
            .set_file_name(file_name(suggested_name, extension))
            .save_file(),
        None => dialog
            .set_file_name(folder_name(suggested_name))
            .save_file(),
    }
}

/// A folder name made from a display name, as [`file_name`] without the
/// extension.
pub fn folder_name(name: &str) -> String {
    let named = file_name(name, "x");
    named.strip_suffix(".x").unwrap_or(&named).to_owned()
}

/// A file name made from a display name: characters a file system rejects
/// are replaced, and the extension is added unless it is already there.
pub fn file_name(name: &str, extension: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let stem = cleaned.trim_matches(|c| c == '.' || c == ' ');
    let stem = if stem.is_empty() { "Untitled" } else { stem };
    let already = Path::new(stem)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension));
    if already {
        stem.to_owned()
    } else {
        format!("{stem}.{extension}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_get_their_extension_once() {
        assert_eq!(
            file_name("Launch teaser", PROJECT_EXTENSION),
            "Launch teaser.moshi"
        );
        assert_eq!(file_name("teaser.moshi", PROJECT_EXTENSION), "teaser.moshi");
        assert_eq!(file_name("teaser.MP4", "mp4"), "teaser.MP4");
        assert_eq!(file_name("v1.2 final", "gif"), "v1.2 final.gif");
    }

    #[test]
    fn file_names_are_safe_for_the_file_system() {
        assert_eq!(file_name("a/b:c", "png"), "a-b-c.png");
        assert_eq!(file_name("  ", "png"), "Untitled.png");
        assert_eq!(file_name("...", "png"), "Untitled.png");
        assert_eq!(file_name("tab\there", "png"), "tab-here.png");
    }

    #[test]
    fn files_are_told_apart_by_their_extension() {
        assert!(is_image(Path::new("a/b/Photo.JPG")));
        assert!(is_image(Path::new("dot.webp")));
        assert!(!is_image(Path::new("notes.txt")));
        assert!(is_media(Path::new("Beach.MOV")));
        assert!(is_media(Path::new("a/song.mp3")));
        assert!(is_media(Path::new("voice.aiff")));
        assert!(!is_media(Path::new("dot.png")));
        assert!(!is_media(Path::new("mp4")));
        assert_eq!(NoDialogs.import_media(), None);
        assert!(is_project(Path::new("teaser.moshi")));
        assert!(!is_project(Path::new("teaser.moshi.bak")));
        assert!(!is_project(Path::new("moshi")));
        assert_eq!(NoDialogs.open_project(), None);
    }

    #[test]
    fn exports_are_named_after_the_scene() {
        assert_eq!(folder_name("Scene 1"), "Scene 1");
        assert_eq!(folder_name("a/b"), "a-b");
        assert_eq!(folder_name(" "), "Untitled");
        assert_eq!(NoDialogs.export_to("Scene", Format::Gif), None);
    }
}

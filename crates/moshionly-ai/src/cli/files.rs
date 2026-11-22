//! Files that providers hand to an agent CLI: configuration written into
//! the app-owned workspace, and images saved for the length of one turn.

use std::path::{Path, PathBuf};

use crate::chat::ImageData;
use crate::error::AiError;

/// Writes `contents` to `path` unless the file already holds exactly that.
///
/// The new file is put in place with a rename, so a second chat that starts
/// at the same moment never reads half a file.
pub(crate) fn write_if_changed(path: &Path, contents: &str) -> Result<(), AiError> {
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = Path::new(&temporary);
    std::fs::write(temporary, contents).map_err(|error| io_error(temporary, &error))?;
    std::fs::rename(temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(temporary);
        io_error(path, &error)
    })
}

/// Removes `path`. A file that is already gone is not an error.
pub(crate) fn remove_if_present(path: &Path) -> Result<(), AiError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, &error)),
    }
}

/// Creates `dir` and its parents.
pub(crate) fn create_dir(dir: &Path) -> Result<(), AiError> {
    std::fs::create_dir_all(dir).map_err(|error| io_error(dir, &error))
}

fn io_error(path: &Path, error: &std::io::Error) -> AiError {
    AiError::other(format!(
        "could not prepare the agent's files ({}): {error}",
        path.display()
    ))
}

/// Images saved to a private temporary directory for a CLI that takes them
/// as file paths. The directory is deleted when this is dropped.
#[derive(Debug)]
pub(crate) struct Attachments {
    paths: Vec<PathBuf>,
    _dir: Option<tempfile::TempDir>,
}

impl Attachments {
    /// Saves `images`, in order. Nothing is created when there are none.
    pub fn save(images: &[ImageData]) -> Result<Self, AiError> {
        if images.is_empty() {
            return Ok(Self {
                paths: Vec::new(),
                _dir: None,
            });
        }
        let failed =
            |error: std::io::Error| AiError::other(format!("could not save an image: {error}"));
        let dir = tempfile::Builder::new()
            .prefix("moshionly-images-")
            .tempdir()
            .map_err(failed)?;
        let mut paths = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            let path = dir.path().join(format!(
                "image-{}.{}",
                index + 1,
                extension_for(&image.media_type)
            ));
            std::fs::write(&path, &image.data).map_err(failed)?;
            paths.push(path);
        }
        Ok(Self {
            paths,
            _dir: Some(dir),
        })
    }

    /// Where the images were saved.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

/// The file extension for an image MIME type.
fn extension_for(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_file_is_replaced_and_no_temporary_file_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_if_changed(&path, "one").unwrap();
        write_if_changed(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn an_unchanged_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_if_changed(&path, "same").unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_if_changed(&path, "same").unwrap();
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn removing_a_missing_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone");
        remove_if_present(&path).unwrap();
        std::fs::write(&path, "x").unwrap();
        remove_if_present(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn images_are_saved_in_order_and_deleted_on_drop() {
        let images = [
            ImageData::png(vec![1, 2, 3]),
            ImageData {
                media_type: "image/jpeg".to_owned(),
                data: vec![9],
            },
        ];
        let saved = Attachments::save(&images).unwrap();
        let paths = saved.paths().to_vec();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("image-1.png"));
        assert!(paths[1].ends_with("image-2.jpg"));
        assert_eq!(std::fs::read(&paths[0]).unwrap(), [1, 2, 3]);
        drop(saved);
        assert!(!paths[0].exists());
        assert!(!paths[0].parent().unwrap().exists());
    }

    #[test]
    fn no_images_means_no_directory() {
        let saved = Attachments::save(&[]).unwrap();
        assert!(saved.paths().is_empty());
    }
}

//! Writing beside the destination and moving into place only at the end.
//!
//! An export writes into a hidden temporary file (or folder) in the same
//! folder as the destination, named `.<name>.<random>.part`. Only a finished
//! export is renamed to the destination, which is a single step on the same
//! disk, so whoever looks at the destination sees either the old file or
//! the whole new one. A failed or cancelled export drops its temporary file,
//! which deletes it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tempfile::{TempDir, TempPath};

use crate::error::ExportError;

/// Where an export is written while it runs.
pub(crate) struct Output {
    destination: PathBuf,
    temporary: Temporary,
}

enum Temporary {
    File(TempPath),
    Folder(TempDir),
}

impl Output {
    /// Prepares to write `destination`: a file, or a folder of files when
    /// `folder` is true.
    ///
    /// A file replaces whatever file is at the destination. A folder is
    /// only written where there is nothing yet, or an empty folder, so that
    /// no one's files are deleted.
    pub(crate) fn create(destination: &Path, folder: bool) -> Result<Self, ExportError> {
        let destination = absolute(destination);
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| ExportError::NoSuchFolder(destination.clone()))?;
        if !parent.is_dir() {
            return Err(ExportError::NoSuchFolder(parent.to_owned()));
        }
        let name = destination
            .file_name()
            .ok_or_else(|| ExportError::DestinationIsFolder(destination.clone()))?
            .to_string_lossy()
            .into_owned();
        let mut builder = tempfile::Builder::new();
        let prefix = format!(".{name}.");
        builder.prefix(&prefix).suffix(".part");
        let temporary = if folder {
            check_folder_is_free(&destination)?;
            let dir = builder
                .tempdir_in(parent)
                .map_err(|error| ExportError::io("created", parent, error))?;
            Temporary::Folder(dir)
        } else {
            if destination.is_dir() {
                return Err(ExportError::DestinationIsFolder(destination));
            }
            // Temporary files are private by default; the export becomes
            // an ordinary file, readable as the person's settings allow.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(fs::Permissions::from_mode(0o666));
            }
            let file = builder
                .tempfile_in(parent)
                .map_err(|error| ExportError::io("created", parent, error))?;
            Temporary::File(file.into_temp_path())
        };
        Ok(Self {
            destination,
            temporary,
        })
    }

    /// Where to write now: the temporary file, or the temporary folder to
    /// put files in.
    pub(crate) fn path(&self) -> &Path {
        match &self.temporary {
            Temporary::File(path) => path,
            Temporary::Folder(dir) => dir.path(),
        }
    }

    /// Where the export ends up.
    pub(crate) fn destination(&self) -> &Path {
        &self.destination
    }

    /// Moves the finished export into place and returns its size in bytes.
    pub(crate) fn commit(self) -> Result<u64, ExportError> {
        let destination = self.destination;
        match self.temporary {
            Temporary::File(path) => {
                let bytes = size_of(&path).map_err(|e| ExportError::io("read", &*path, e))?;
                path.persist(&destination).map_err(|error| {
                    ExportError::io("moved into place", &destination, error.error)
                })?;
                Ok(bytes)
            }
            Temporary::Folder(dir) => {
                let bytes =
                    size_of(dir.path()).map_err(|e| ExportError::io("read", dir.path(), e))?;
                // Someone may have made the folder while the export ran.
                check_folder_is_free(&destination)?;
                if destination.is_dir() {
                    fs::remove_dir(&destination)
                        .map_err(|error| ExportError::io("replaced", &destination, error))?;
                }
                fs::rename(dir.path(), &destination)
                    .map_err(|error| ExportError::io("moved into place", &destination, error))?;
                // Renamed, so there is nothing left for the guard to delete.
                let _ = dir.keep();
                Ok(bytes)
            }
        }
    }
}

/// A folder export may only go where nothing is, or into an empty folder.
fn check_folder_is_free(destination: &Path) -> Result<(), ExportError> {
    match fs::read_dir(destination).map(|mut entries| entries.next().is_none()) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ExportError::FolderNotEmpty(destination.to_owned())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        // A file, or something that cannot be read.
        Err(_) => Err(ExportError::FolderNotEmpty(destination.to_owned())),
    }
}

/// The path made absolute against the current folder, so its parent is
/// always known.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// The size of a file, or of every file in a folder.
fn size_of(path: &Path) -> io::Result<u64> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let mut bytes = 0;
    for entry in fs::read_dir(path)? {
        bytes += size_of(&entry?.path())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_file_appears_only_when_committed() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("clip.gif");
        let output = Output::create(&destination, false).unwrap();
        assert_eq!(output.destination(), destination);
        let name = output
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with(".clip.gif.") && name.ends_with(".part"),
            "{name}"
        );
        fs::write(output.path(), b"GIF89a").unwrap();
        assert!(!destination.exists());
        assert_eq!(output.commit().unwrap(), 6);
        assert_eq!(fs::read(&destination).unwrap(), b"GIF89a");
        assert_eq!(leftovers(dir.path()), ["clip.gif"]);
        // The file has the permissions of any other new file.
        let ordinary = dir.path().join("ordinary");
        fs::write(&ordinary, b"").unwrap();
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions(),
            fs::metadata(&ordinary).unwrap().permissions()
        );
        fs::remove_file(ordinary).unwrap();

        // A second export replaces the first.
        let output = Output::create(&destination, false).unwrap();
        fs::write(output.path(), b"GIF").unwrap();
        output.commit().unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"GIF");
    }

    #[test]
    fn an_abandoned_export_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let output = Output::create(&dir.path().join("clip.mp4"), false).unwrap();
        fs::write(output.path(), b"half").unwrap();
        drop(output);
        assert!(leftovers(dir.path()).is_empty());

        let output = Output::create(&dir.path().join("frames"), true).unwrap();
        fs::write(output.path().join("frame_0000.png"), b"png").unwrap();
        drop(output);
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn folders_never_overwrite_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("frames");
        let output = Output::create(&destination, true).unwrap();
        fs::write(output.path().join("frame_0000.png"), b"12345").unwrap();
        assert_eq!(output.commit().unwrap(), 5);
        assert_eq!(leftovers(&destination), ["frame_0000.png"]);

        // The folder has files now, so a second export is refused.
        let error = Output::create(&destination, true).err().unwrap();
        assert!(matches!(error, ExportError::FolderNotEmpty(_)), "{error}");

        // An empty folder is fine.
        let empty = dir.path().join("empty");
        fs::create_dir(&empty).unwrap();
        let output = Output::create(&empty, true).unwrap();
        fs::write(output.path().join("frame_0000.png"), b"1").unwrap();
        output.commit().unwrap();
        assert_eq!(leftovers(&empty), ["frame_0000.png"]);
    }

    #[test]
    fn destinations_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let error = Output::create(&dir.path().join("missing/clip.gif"), false)
            .err()
            .unwrap();
        assert!(matches!(error, ExportError::NoSuchFolder(_)), "{error}");
        let error = Output::create(dir.path(), false).err().unwrap();
        assert!(
            matches!(error, ExportError::DestinationIsFolder(_)),
            "{error}"
        );
        let file = dir.path().join("taken.png");
        fs::write(&file, b"x").unwrap();
        let error = Output::create(&file, true).err().unwrap();
        assert!(matches!(error, ExportError::FolderNotEmpty(_)), "{error}");
    }
}

//! Choosing where tools write files: safe names, and paths that never
//! overwrite what is there, even when two tool calls write at once.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The longest file name, without its extension, that tools write.
const MAX_STEM: usize = 80;

/// A file name made safe to write: letters, digits, spaces, `-`, `_` and
/// `.` are kept, anything else becomes `-`, and no name starts with a dot
/// or leaves its folder. `fallback` is used when nothing is left.
pub(crate) fn safe_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ' || c == '-');
    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// `stem.extension` (or `stem` alone) in `folder`, then the same with ` 2`,
/// ` 3` and so on added to the stem.
fn candidates<'a>(
    folder: &'a Path,
    stem: &str,
    extension: Option<&'a str>,
) -> impl Iterator<Item = PathBuf> + 'a {
    let stem: String = stem.chars().take(MAX_STEM).collect();
    (1..).map(move |n: usize| {
        let stem = if n == 1 {
            stem.clone()
        } else {
            format!("{stem} {n}")
        };
        folder.join(match extension {
            Some(extension) => format!("{stem}.{extension}"),
            None => stem,
        })
    })
}

/// Writes `bytes` to a new file at the first free name of
/// [`candidates`]. The file is created only if nothing is there, so a file
/// another call made a moment ago is never replaced.
pub(crate) fn write_new(
    folder: &Path,
    stem: &str,
    extension: Option<&str>,
    bytes: &[u8],
) -> Result<PathBuf, (PathBuf, std::io::Error)> {
    for path in candidates(folder, stem, extension) {
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match created {
            Ok(mut file) => {
                return match file.write_all(bytes) {
                    Ok(()) => Ok(path),
                    Err(error) => Err((path, error)),
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err((path, error)),
        }
    }
    unreachable!("the names never run out")
}

/// The paths that writers in this process hold while they write.
static HELD: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn held() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A path held for one writer, such as an export that takes a while to
/// write its file. No other reservation gets it until this one is dropped,
/// when the file is either there or was never made.
#[derive(Debug)]
pub(crate) struct Reserved(PathBuf);

impl Reserved {
    /// The path that is held.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        held().remove(&self.0);
    }
}

/// Holds the first name of [`candidates`] that is neither on disk nor held
/// by another writer.
pub(crate) fn reserve_free(folder: &Path, stem: &str, extension: Option<&str>) -> Reserved {
    let mut held = held();
    let path = candidates(folder, stem, extension)
        .find(|path| !path.exists() && !held.contains(path))
        .expect("the names never run out");
    held.insert(path.clone());
    Reserved(path)
}

/// Holds `path` itself, unless something is there or another writer holds
/// it.
pub(crate) fn reserve(path: PathBuf) -> Option<Reserved> {
    let mut held = held();
    if path.exists() || held.contains(&path) {
        return None;
    }
    held.insert(path.clone());
    Some(Reserved(path))
}

/// The name of a file without its extension, and its extension in lower
/// case. Only a short run of letters and digits after the last dot counts
/// as an extension, so "v1.2 final" keeps its name whole.
pub(crate) fn split_name(name: &str) -> (&str, Option<String>) {
    match name.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && (1..=5).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (stem, Some(extension.to_lowercase()))
        }
        _ => (name, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::TempDir;

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("Launch day", "x"), "Launch day");
        assert_eq!(safe_name("../../etc/passwd", "x"), "etc-passwd");
        assert_eq!(safe_name("a/b\\c:d", "x"), "a-b-c-d");
        assert_eq!(safe_name(".hidden", "x"), "hidden");
        assert_eq!(safe_name("///", "image"), "image");
        assert_eq!(safe_name("Café №1", "x"), "Café -1");
    }

    #[test]
    fn names_are_split_at_the_last_dot() {
        assert_eq!(split_name("photo.PNG"), ("photo", Some("png".to_owned())));
        assert_eq!(
            split_name("archive.tar.gz"),
            ("archive.tar", Some("gz".to_owned()))
        );
        assert_eq!(split_name("README"), ("README", None));
        assert_eq!(split_name(".png"), (".png", None));
        assert_eq!(split_name("v1.2 final"), ("v1.2 final", None));
    }

    #[test]
    fn free_paths_do_not_overwrite() {
        let dir = TempDir::new("files");
        let folder = dir.path();
        let first = reserve_free(folder, "Scene", Some("mp4"));
        assert_eq!(first.path(), folder.join("Scene.mp4"));
        std::fs::write(first.path(), b"x").unwrap();
        drop(first);
        assert_eq!(
            reserve_free(folder, "Scene", Some("mp4")).path(),
            folder.join("Scene 2.mp4")
        );
        assert_eq!(
            reserve_free(folder, "Scene", None).path(),
            folder.join("Scene")
        );
    }

    #[test]
    fn a_held_name_is_not_given_out_again_until_it_is_let_go() {
        let dir = TempDir::new("files-held");
        let folder = dir.path();
        let first = reserve_free(folder, "Scene", Some("mp4"));
        let second = reserve_free(folder, "Scene", Some("mp4"));
        assert_eq!(first.path(), folder.join("Scene.mp4"));
        assert_eq!(second.path(), folder.join("Scene 2.mp4"));
        assert!(reserve(folder.join("Scene.mp4")).is_none());
        drop(first);
        assert!(reserve(folder.join("Scene.mp4")).is_some());
    }

    #[test]
    fn new_files_never_replace_one_that_is_there() {
        let dir = TempDir::new("files-new");
        let folder = dir.path();
        std::fs::write(folder.join("hero.png"), b"first").unwrap();
        let path = write_new(folder, "hero", Some("png"), b"second").unwrap();
        assert_eq!(path, folder.join("hero 2.png"));
        assert_eq!(std::fs::read(folder.join("hero.png")).unwrap(), b"first");
        assert_eq!(std::fs::read(path).unwrap(), b"second");
    }
}

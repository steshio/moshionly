//! Choosing where tools write files: safe names, and paths that never
//! overwrite what is there.

use std::path::{Path, PathBuf};

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

/// `stem.extension` (or `stem` alone) in `folder`, with ` 2`, ` 3` and so
/// on added to the stem until the path is free.
pub(crate) fn free_path(folder: &Path, stem: &str, extension: Option<&str>) -> PathBuf {
    let stem: String = stem.chars().take(MAX_STEM).collect();
    let name = |n: usize| {
        let stem = if n == 1 {
            stem.clone()
        } else {
            format!("{stem} {n}")
        };
        match extension {
            Some(extension) => format!("{stem}.{extension}"),
            None => stem,
        }
    };
    (1..)
        .map(|n| folder.join(name(n)))
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join(name(1)))
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
        let folder = std::env::temp_dir().join(format!(
            "moshionly-agent-files-{}-{}",
            std::process::id(),
            moshionly_core::NodeId::generate()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let first = free_path(&folder, "Scene", Some("mp4"));
        assert_eq!(first, folder.join("Scene.mp4"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(
            free_path(&folder, "Scene", Some("mp4")),
            folder.join("Scene 2.mp4")
        );
        assert_eq!(free_path(&folder, "Scene", None), folder.join("Scene"));
        std::fs::remove_dir_all(&folder).unwrap();
    }
}

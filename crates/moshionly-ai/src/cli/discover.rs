//! Finding an agent CLI on the user's machine.
//!
//! A desktop app started from the Finder or a launcher does not inherit the
//! shell's `PATH`, so looking only there would miss most installations. The
//! search therefore also covers the directories that installers and package
//! managers commonly use.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::process::{CliCommand, run_to_string};

/// How long a CLI gets to answer `--version` or a status query.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Finds an executable by name.
///
/// `override_path` is a location chosen by the user in settings and wins when
/// it points at an executable. Otherwise the first of `names` found in
/// [`search_dirs`] is returned.
pub(crate) fn find_binary(names: &[&str], override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = override_path
        && is_executable(path)
    {
        return Some(path.to_path_buf());
    }
    find_in(names, &search_dirs())
}

/// Looks for the first of `names` in `dirs`, in directory order.
pub(crate) fn find_in(names: &[&str], dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        for name in names {
            for candidate in candidates(dir, name) {
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// The directories searched for agent CLIs: `PATH` first, then the usual
/// install locations. Duplicates are removed, order is kept.
pub(crate) fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    dirs.extend(common_install_dirs(dirs::home_dir().as_deref()));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| !dir.as_os_str().is_empty() && seen.insert(dir.clone()));
    dirs
}

/// Where installers put command line tools, most specific first.
fn common_install_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        for relative in [
            ".local/bin",
            ".claude/local",
            ".cursor/bin",
            ".npm-global/bin",
            ".bun/bin",
            ".volta/bin",
            ".yarn/bin",
            ".local/share/pnpm",
            "Library/pnpm",
            ".cargo/bin",
            "bin",
        ] {
            dirs.push(home.join(relative));
        }
        dirs.extend(node_version_manager_dirs(home));
    }
    for absolute in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/home/linuxbrew/.linuxbrew/bin",
        "/usr/bin",
        "/bin",
    ] {
        dirs.push(PathBuf::from(absolute));
    }
    dirs
}

/// The `bin` directories of Node versions installed through nvm or fnm,
/// newest first. Agents installed with `npm install -g` live there.
fn node_version_manager_dirs(home: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for (versions_dir, bin_suffix) in [
        (home.join(".nvm/versions/node"), "bin"),
        (
            home.join(".local/share/fnm/node-versions"),
            "installation/bin",
        ),
    ] {
        let Ok(entries) = std::fs::read_dir(&versions_dir) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        versions.sort_by_key(|path| std::cmp::Reverse(version_sort_key(path)));
        found.extend(versions.into_iter().map(|path| path.join(bin_suffix)));
    }
    found
}

/// Turns a directory name such as `v22.11.0` into numbers that sort by
/// version rather than by text.
fn version_sort_key(path: &Path) -> Vec<u64> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

#[cfg(not(windows))]
fn candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![dir.join(name)]
}

#[cfg(windows)]
fn candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    ["exe", "cmd", "bat"]
        .iter()
        .map(|extension| dir.join(format!("{name}.{extension}")))
        .collect()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The `PATH` to give an agent CLI.
///
/// Agents installed through npm are scripts that start with
/// `#!/usr/bin/env node`, so `node` has to be findable even when the app was
/// not started from a shell. The binary's own directory goes first, then the
/// inherited `PATH`, then the common install locations.
pub(crate) fn child_path(binary: &Path) -> OsString {
    let mut dirs = Vec::new();
    if let Some(parent) = binary.parent() {
        dirs.push(parent.to_path_buf());
    }
    dirs.extend(search_dirs());
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    std::env::join_paths(dirs).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// A command for `binary` with the search path an agent needs.
pub(crate) fn command_for(binary: &Path) -> CliCommand {
    let mut command = CliCommand::new(binary);
    command.env.push(("PATH".into(), child_path(binary)));
    command
}

/// Asks a CLI for its version with `--version`.
pub(crate) async fn read_version(binary: &Path) -> Option<String> {
    let command = command_for(binary).arg("--version");
    let (_, stdout, stderr) = run_to_string(command, PROBE_TIMEOUT).await.ok()?;
    parse_version(&stdout).or_else(|| parse_version(&stderr))
}

/// Picks the version number out of a `--version` line such as
/// `2.1.325 (Claude Code)` or `codex-cli 0.63.0`.
pub(crate) fn parse_version(output: &str) -> Option<String> {
    output
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|word| word.trim_start_matches('v'))
        .find(|word| word.contains('.') && word.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_owned)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_script(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\necho 'agent 1.4.2 (test)'\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn finds_the_first_match_in_directory_order() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write_script(second.path(), "agent", 0o755);
        let expected = write_script(first.path(), "agent", 0o755);
        let dirs = [first.path().to_path_buf(), second.path().to_path_buf()];
        assert_eq!(find_in(&["agent"], &dirs), Some(expected));
    }

    #[test]
    fn tries_alternative_names() {
        let dir = tempfile::tempdir().unwrap();
        let expected = write_script(dir.path(), "agent", 0o755);
        let dirs = [dir.path().to_path_buf()];
        assert_eq!(find_in(&["cursor-agent", "agent"], &dirs), Some(expected));
    }

    #[test]
    fn skips_files_that_are_not_executable_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        write_script(dir.path(), "agent", 0o644);
        std::fs::create_dir(dir.path().join("other")).unwrap();
        let dirs = [dir.path().to_path_buf()];
        assert_eq!(find_in(&["agent"], &dirs), None);
        assert_eq!(find_in(&["other"], &dirs), None);
        assert_eq!(find_in(&["missing"], &dirs), None);
    }

    #[test]
    fn an_override_wins_only_when_it_is_executable() {
        let dir = tempfile::tempdir().unwrap();
        let good = write_script(dir.path(), "custom-agent", 0o755);
        assert_eq!(find_binary(&["nope"], Some(&good)), Some(good));
        let bad = dir.path().join("missing");
        assert_eq!(find_binary(&["moshionly-no-such-agent"], Some(&bad)), None);
    }

    #[test]
    fn common_dirs_include_user_and_system_locations() {
        let dirs = common_install_dirs(Some(Path::new("/home/someone")));
        assert!(dirs.contains(&PathBuf::from("/home/someone/.local/bin")));
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
    }

    #[test]
    fn node_versions_are_listed_newest_first() {
        let home = tempfile::tempdir().unwrap();
        for version in ["v9.11.2", "v22.3.0", "v18.20.4"] {
            std::fs::create_dir_all(home.path().join(".nvm/versions/node").join(version)).unwrap();
        }
        let dirs = node_version_manager_dirs(home.path());
        let names: Vec<String> = dirs
            .iter()
            .map(|dir| {
                dir.parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["v22.3.0", "v18.20.4", "v9.11.2"]);
    }

    #[test]
    fn child_path_starts_with_the_binary_directory() {
        let path = child_path(Path::new("/opt/tools/bin/agent"));
        let first = std::env::split_paths(&path).next().unwrap();
        assert_eq!(first, PathBuf::from("/opt/tools/bin"));
    }

    #[test]
    fn parses_version_lines() {
        assert_eq!(
            parse_version("2.1.325 (Claude Code)").as_deref(),
            Some("2.1.325")
        );
        assert_eq!(
            parse_version("codex-cli 0.63.0\n").as_deref(),
            Some("0.63.0")
        );
        assert_eq!(
            parse_version("2026.08.11-e8db854").as_deref(),
            Some("2026.08.11-e8db854")
        );
        assert_eq!(parse_version("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(parse_version("no version here"), None);
    }

    #[tokio::test]
    async fn reads_the_version_from_a_real_executable() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(dir.path(), "agent", 0o755);
        assert_eq!(read_version(&script).await.as_deref(), Some("1.4.2"));
    }
}

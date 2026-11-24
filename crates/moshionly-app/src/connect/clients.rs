//! How to add Moshionly to the agents people use: the exact command or
//! configuration for each, shown in the "Connect an agent" dialog.

use std::path::Path;

use serde_json::json;

/// The name agents know the server by.
pub const SERVER_NAME: &str = "moshionly";

/// How one agent is told about Moshionly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientSetup {
    /// The agent, such as "Claude Code".
    pub name: &'static str,
    /// What to do with the snippet.
    pub how: &'static str,
    /// The command to run or the configuration to add.
    pub snippet: String,
}

/// The setups for the agents people use most, for the server at `url`.
pub fn client_setups(url: &str) -> Vec<ClientSetup> {
    let pretty = |value: serde_json::Value| {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
    };
    vec![
        ClientSetup {
            name: "Claude Code",
            how: "Run this in a terminal:",
            snippet: format!("claude mcp add --transport http {SERVER_NAME} {url}"),
        },
        ClientSetup {
            name: "Cursor",
            how: "Add this to ~/.cursor/mcp.json:",
            snippet: pretty(json!({ "mcpServers": { SERVER_NAME: { "url": url } } })),
        },
        ClientSetup {
            name: "VS Code",
            how: "Add this to .vscode/mcp.json in your project:",
            snippet: pretty(json!({ "servers": { SERVER_NAME: { "type": "http", "url": url } } })),
        },
        ClientSetup {
            name: "Other MCP clients",
            how: "Add a streamable HTTP server with this address:",
            snippet: url.to_owned(),
        },
    ]
}

/// The command that serves a file on standard input and output, for agents
/// that only start servers themselves. It works without the window, on the
/// file as saved, and writes each change back.
///
/// Started from `Moshionly.app`, the program is not on the `PATH` of a
/// terminal, so the command names it by its full path, ready to paste.
pub fn stdio_command() -> String {
    stdio_command_for(std::env::current_exe().ok().as_deref())
}

/// [`stdio_command`] when the running program is `exe`.
fn stdio_command_for(exe: Option<&Path>) -> String {
    let program = exe
        .filter(|exe| in_app_bundle(exe))
        .map(|exe| {
            let exe = std::path::absolute(exe).unwrap_or_else(|_| exe.to_owned());
            shell_word(&exe.to_string_lossy())
        })
        .unwrap_or_else(|| "moshionly".to_owned());
    format!("{program} mcp --save path/to/project.moshi")
}

/// Whether `exe` is the program of a macOS app bundle, that is
/// `Name.app/Contents/MacOS/program`.
fn in_app_bundle(exe: &Path) -> bool {
    let mut folders = exe.ancestors().skip(1);
    let mut next_name = || folders.next().and_then(Path::file_name);
    next_name().is_some_and(|name| name == "MacOS")
        && next_name().is_some_and(|name| name == "Contents")
        && next_name()
            .and_then(|name| Path::new(name).extension())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

/// `text` as one word for a terminal: as it is when every character is
/// safe there, otherwise in single quotes.
fn shell_word(text: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@%=".contains(c);
    if !text.is_empty() && text.chars().all(safe) {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setup_names_the_address() {
        let url = "http://127.0.0.1:47821/mcp";
        let setups = client_setups(url);
        assert_eq!(setups[0].name, "Claude Code");
        assert_eq!(
            setups[0].snippet,
            "claude mcp add --transport http moshionly http://127.0.0.1:47821/mcp"
        );
        for setup in &setups {
            assert!(setup.snippet.contains(url), "{}", setup.name);
            assert!(!setup.how.is_empty());
        }
        let cursor: serde_json::Value = serde_json::from_str(&setups[1].snippet).unwrap();
        assert_eq!(cursor["mcpServers"]["moshionly"]["url"], url);
        let vscode: serde_json::Value = serde_json::from_str(&setups[2].snippet).unwrap();
        assert_eq!(vscode["servers"]["moshionly"]["type"], "http");
        assert!(stdio_command().contains("moshionly mcp --save"));
    }

    #[test]
    fn inside_the_app_bundle_the_command_names_the_program_in_full() {
        assert_eq!(
            stdio_command_for(Some(Path::new(
                "/Applications/Moshionly.app/Contents/MacOS/moshionly"
            ))),
            "/Applications/Moshionly.app/Contents/MacOS/moshionly mcp --save path/to/project.moshi"
        );
        // A folder with a space in its name is quoted for the terminal.
        assert_eq!(
            stdio_command_for(Some(Path::new(
                "/Users/me/My Apps/Moshionly.app/Contents/MacOS/moshionly"
            ))),
            "'/Users/me/My Apps/Moshionly.app/Contents/MacOS/moshionly' mcp --save \
             path/to/project.moshi"
        );
    }

    #[test]
    fn outside_a_bundle_the_command_is_the_installed_name() {
        for exe in [
            None,
            Some(Path::new("/Users/me/moshionly/target/release/moshionly")),
            Some(Path::new("/usr/local/bin/moshionly")),
            Some(Path::new(
                "/Applications/Moshionly/Contents/MacOS/moshionly",
            )),
            Some(Path::new("/x/Moshionly.app/Contents/Resources/moshionly")),
        ] {
            assert_eq!(
                stdio_command_for(exe),
                "moshionly mcp --save path/to/project.moshi",
                "{exe:?}"
            );
        }
    }

    #[test]
    fn shell_words_are_quoted_only_when_needed() {
        assert_eq!(shell_word("/a/b-c_d.e"), "/a/b-c_d.e");
        assert_eq!(shell_word("/a b/c"), "'/a b/c'");
        assert_eq!(shell_word("/it's/here"), r"'/it'\''s/here'");
        assert_eq!(shell_word(""), "''");
    }
}

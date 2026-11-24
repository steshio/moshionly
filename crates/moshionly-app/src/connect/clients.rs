//! How to add Moshionly to the agents people use: the exact command or
//! configuration for each, shown in the "Connect an agent" dialog.

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
pub fn stdio_command() -> String {
    "moshionly mcp --save path/to/project.moshi".to_owned()
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
        assert!(stdio_command().starts_with("moshionly mcp"));
    }
}

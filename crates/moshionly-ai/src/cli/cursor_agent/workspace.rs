//! The files that scope a Cursor Agent run.
//!
//! Cursor Agent has no flag for "attach only this MCP server" and none for
//! "turn the built-in tools off". It takes both from files in the workspace
//! it runs in, so before every turn the provider writes them into the
//! app-owned working directory:
//!
//! - `.cursor/mcp.json` names the Moshionly MCP server and nothing else.
//! - `.cursor/cli.json` holds permissions: that server's tools may run
//!   without a prompt, and shell commands, file access and web fetches are
//!   denied. Deny rules win over anything the user allowed globally.
//! - `.cursor/rules/moshionly.mdc` carries the system prompt as a rule that
//!   always applies, because the CLI has no system prompt flag.
//!
//! Nothing outside that directory is written. The user's own configuration
//! under `~/.cursor` is never modified.

use std::path::Path;

use serde_json::{Value, json};

use crate::chat::{ChatRequest, McpEndpoint};
use crate::cli::files::{create_dir, remove_if_present, write_if_changed};
use crate::error::AiError;

/// What the agent may never do in a design chat: run commands, change files
/// or fetch from the web. `**` covers the workspace and `/**` everything
/// else, because relative patterns are scoped to the workspace.
const DENIED: &[&str] = &["Shell(*)", "Write(**)", "Write(/**)", "WebFetch(*)"];

/// Denies reading files, inside the workspace and outside it.
const DENIED_READS: &[&str] = &["Read(**)", "Read(/**)"];

/// The file name of the rule that carries the system prompt.
const RULE_FILE: &str = "moshionly.mdc";

/// Writes the configuration for one turn into `workspace`.
pub(super) fn write_workspace_files(
    workspace: &Path,
    request: &ChatRequest,
    allow_file_reads: bool,
) -> Result<(), AiError> {
    let config_dir = workspace.join(".cursor");
    let rules_dir = config_dir.join("rules");
    create_dir(&rules_dir)?;

    let mcp = request.mcp.as_ref();
    write_if_changed(&config_dir.join("mcp.json"), &pretty(&mcp_config(mcp)))?;
    write_if_changed(
        &config_dir.join("cli.json"),
        &pretty(&permissions(mcp, allow_file_reads)),
    )?;

    let rule_path = rules_dir.join(RULE_FILE);
    match request.system_prompt.as_deref().map(str::trim) {
        Some(prompt) if !prompt.is_empty() => write_if_changed(&rule_path, &rule_file(prompt))?,
        // A rule left over from an earlier turn must not outlive its prompt.
        _ => remove_if_present(&rule_path)?,
    }
    Ok(())
}

/// The project-level MCP configuration: only the Moshionly server.
pub(super) fn mcp_config(mcp: Option<&McpEndpoint>) -> Value {
    let mut servers = serde_json::Map::new();
    if let Some(mcp) = mcp {
        servers.insert(mcp.name.clone(), json!({ "url": mcp.url }));
    }
    json!({ "mcpServers": servers })
}

/// The project-level permissions.
pub(super) fn permissions(mcp: Option<&McpEndpoint>, allow_file_reads: bool) -> Value {
    let allow: Vec<String> = mcp
        .map(|mcp| format!("Mcp({}:*)", mcp.name))
        .into_iter()
        .collect();
    let mut deny: Vec<&str> = DENIED.to_vec();
    if !allow_file_reads {
        deny.extend(DENIED_READS);
    }
    json!({ "permissions": { "allow": allow, "deny": deny } })
}

/// A project rule that is included in every chat.
pub(super) fn rule_file(system_prompt: &str) -> String {
    format!(
        "---\ndescription: Instructions from Moshionly for this chat\nalwaysApply: true\n---\n\n{system_prompt}\n"
    )
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_mcp() -> ChatRequest {
        ChatRequest {
            mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
            system_prompt: Some("You edit motion designs.".to_owned()),
            ..ChatRequest::from_prompt("hi")
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn only_the_moshionly_server_is_configured() {
        let mcp = McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp");
        assert_eq!(
            mcp_config(Some(&mcp)),
            json!({ "mcpServers": { "moshionly": { "url": "http://127.0.0.1:47821/mcp" } } })
        );
        assert_eq!(mcp_config(None), json!({ "mcpServers": {} }));
    }

    #[test]
    fn our_tools_are_allowed_and_everything_else_is_denied() {
        let mcp = McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp");
        let value = permissions(Some(&mcp), false);
        assert_eq!(value["permissions"]["allow"], json!(["Mcp(moshionly:*)"]));
        let deny: Vec<&str> = value["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        for rule in [
            "Shell(*)",
            "Write(**)",
            "Write(/**)",
            "WebFetch(*)",
            "Read(**)",
            "Read(/**)",
        ] {
            assert!(deny.contains(&rule), "{rule} should be denied");
        }
    }

    #[test]
    fn without_a_server_nothing_is_allowed() {
        let value = permissions(None, false);
        assert_eq!(value["permissions"]["allow"], json!([]));
    }

    #[test]
    fn file_reads_can_be_let_through_but_writes_never() {
        let value = permissions(None, true);
        let deny = value["permissions"]["deny"].to_string();
        assert!(!deny.contains("Read("));
        assert!(deny.contains("Write(**)"));
        assert!(deny.contains("Shell(*)"));
    }

    #[test]
    fn all_three_files_are_written() {
        let dir = tempfile::tempdir().unwrap();
        write_workspace_files(dir.path(), &request_with_mcp(), false).unwrap();
        let mcp = read_json(&dir.path().join(".cursor/mcp.json"));
        assert_eq!(
            mcp["mcpServers"]["moshionly"]["url"],
            "http://127.0.0.1:47821/mcp"
        );
        let cli = read_json(&dir.path().join(".cursor/cli.json"));
        assert_eq!(cli["permissions"]["allow"][0], "Mcp(moshionly:*)");
        let rule = std::fs::read_to_string(dir.path().join(".cursor/rules/moshionly.mdc")).unwrap();
        assert!(rule.starts_with("---\n"));
        assert!(rule.contains("alwaysApply: true"));
        assert!(rule.ends_with("You edit motion designs.\n"));
    }

    #[test]
    fn a_rule_from_an_earlier_turn_is_removed_when_there_is_no_prompt() {
        let dir = tempfile::tempdir().unwrap();
        write_workspace_files(dir.path(), &request_with_mcp(), false).unwrap();
        let rule = dir.path().join(".cursor/rules/moshionly.mdc");
        assert!(rule.exists());

        let request = ChatRequest {
            system_prompt: Some("   ".to_owned()),
            ..request_with_mcp()
        };
        write_workspace_files(dir.path(), &request, false).unwrap();
        assert!(!rule.exists());
        // Removing it twice is not an error.
        write_workspace_files(dir.path(), &request, false).unwrap();
    }

    #[test]
    fn a_changed_endpoint_replaces_the_old_one_and_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        write_workspace_files(dir.path(), &request_with_mcp(), false).unwrap();
        let request = ChatRequest {
            mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:50000/mcp")),
            ..request_with_mcp()
        };
        write_workspace_files(dir.path(), &request, false).unwrap();
        let mcp = read_json(&dir.path().join(".cursor/mcp.json"));
        assert_eq!(
            mcp["mcpServers"]["moshionly"]["url"],
            "http://127.0.0.1:50000/mcp"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join(".cursor"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn unchanged_files_are_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        write_workspace_files(dir.path(), &request_with_mcp(), false).unwrap();
        let path = dir.path().join(".cursor/mcp.json");
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_workspace_files(dir.path(), &request_with_mcp(), false).unwrap();
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }
}

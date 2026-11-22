//! The Gemini CLI provider: chat through Google's `gemini` CLI and the
//! user's own Google account.
//!
//! UNVERIFIED: the Gemini CLI was not installed when this was written, so
//! the provider has never been run against it. Flags, settings and the event
//! stream follow Google's published documentation for headless mode, the
//! settings reference and the MCP guide. Expect to adjust details the first
//! time it meets the real CLI.
//!
//! The CLI is run once per turn in headless mode, in an app-owned workspace.
//! Before each turn the provider writes two files there and nowhere else:
//!
//! - `.gemini/settings.json`, project settings that name the Moshionly MCP
//!   server as trusted (so its tools run without a prompt nobody could
//!   answer) and exclude the CLI's own file, shell, web and memory tools.
//! - `GEMINI.md`, the context file, carrying the system prompt.
//!
//! The user's own `~/.gemini` settings stay as they are; servers configured
//! there are kept out of the run with `--allowed-mcp-server-names`. Moshionly
//! never reads the CLI's credentials: signing in is done by the user, in the
//! CLI.
//!
//! One limit comes from the CLI itself: the prompt is a command-line
//! argument, so other processes of the same user can see it while it runs.

mod stream;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use self::stream::GeminiStream;
use super::discover::{command_for, find_binary, read_version};
use super::files::{create_dir, remove_if_present, write_if_changed};
use super::process::{CliCommand, run_streaming};
use super::prompt::{cli_prompt, prepare_working_dir, validate_mcp_name};
use super::stopped;
use crate::chat::{ChatRequest, McpEndpoint};
use crate::error::{AiError, ErrorKind};
use crate::event::{EventSender, StopReason};
use crate::provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
    ToolTransport,
};

/// The provider id used in settings and on the command line.
pub const GEMINI_CLI_ID: &str = "gemini-cli";

/// The model entry that leaves the choice to the CLI's own configuration.
const DEFAULT_MODEL: &str = "default";

/// The built-in tools kept away from the model: everything that reads or
/// writes files, runs commands, reaches the web or changes the CLI's
/// memory. Names from both current and older releases are listed.
const EXCLUDED_TOOLS: &[&str] = &[
    "run_shell_command",
    "write_file",
    "replace",
    "read_file",
    "read_many_files",
    "list_directory",
    "glob",
    "grep_search",
    "search_file_content",
    "web_fetch",
    "google_web_search",
    "save_memory",
    "activate_skill",
];

/// A server name that matches nothing, for runs without an MCP endpoint.
const NO_MCP_SERVER: &str = "none";

/// Settings for the Gemini CLI provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeminiCliOptions {
    /// Where the `gemini` executable is, when the user chose one. Otherwise
    /// it is looked up on `PATH` and in the usual install locations.
    pub binary: Option<PathBuf>,
}

/// Chat through the user's Gemini CLI installation.
#[derive(Debug, Clone, Default)]
pub struct GeminiCli {
    options: GeminiCliOptions,
}

impl GeminiCli {
    /// A provider with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// A provider with the given options.
    pub fn with_options(options: GeminiCliOptions) -> Self {
        Self { options }
    }

    fn binary(&self) -> Result<PathBuf, AiError> {
        find_binary(&["gemini"], self.options.binary.as_deref()).ok_or_else(|| {
            AiError::NotInstalled {
                program: "gemini".to_owned(),
            }
        })
    }

    async fn run(
        &self,
        request: &ChatRequest,
        events: &EventSender,
        cancel: &CancellationToken,
    ) -> Result<StopReason, AiError> {
        let binary = self.binary()?;
        let working_dir = prepare_working_dir(request)?;
        let command = build_command(&binary, request, &working_dir)?;
        write_workspace_files(&working_dir, request)?;
        let mut stream = GeminiStream::new(request.mcp.as_ref().map(|mcp| mcp.name.as_str()));
        let exit = run_streaming(command, stopped(cancel, events), |line| {
            stream.push_line(line, events)
        })
        .await?;
        stream.finish(exit)
    }
}

#[async_trait]
impl ChatProvider for GeminiCli {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: GEMINI_CLI_ID.to_owned(),
            name: "Gemini CLI".to_owned(),
            kind: ProviderKind::Subscription,
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tools: ToolTransport::Mcp,
            // The CLI takes images only as file references that the agent
            // reads itself, and its file tools are excluded here.
            images: false,
            resume: true,
            thinking: false,
            live_model_list: false,
            needs_api_key: false,
            verified: false,
        }
    }

    /// The CLI has no command that lists models. These are its documented
    /// aliases, which follow the newest model of each tier; a full model
    /// name works too.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        Ok(vec![
            ModelInfo::new(DEFAULT_MODEL, "Default (as set in Gemini CLI)"),
            ModelInfo::new("pro", "Pro"),
            ModelInfo::new("flash", "Flash"),
            ModelInfo::new("flash-lite", "Flash-Lite"),
        ])
    }

    async fn check_ready(&self) -> Readiness {
        let binary = match self.binary() {
            Ok(binary) => binary,
            Err(_) => {
                return Readiness::not_ready(
                    ReadyState::NotInstalled,
                    "Gemini CLI is not installed. Install it from \
                     https://github.com/google-gemini/gemini-cli, then run `gemini` in a \
                     terminal and sign in.",
                );
            }
        };
        // The CLI has no command that reports its sign-in state, and looking
        // at its credential files is not something Moshionly does.
        match read_version(&binary).await {
            Some(version) => Readiness::ready(format!(
                "Gemini CLI {version} is installed. Whether it is signed in shows when you \
                 send a message."
            ))
            .with_version(version),
            None => Readiness::ready(
                "Gemini CLI is installed. Whether it is signed in shows when you send a message.",
            ),
        }
    }

    async fn send(&self, request: ChatRequest, events: EventSender, cancel: CancellationToken) {
        let outcome = self.run(&request, &events, &cancel).await;
        events.finish(outcome);
    }
}

/// The command line for one turn.
fn build_command(
    binary: &Path,
    request: &ChatRequest,
    working_dir: &Path,
) -> Result<CliCommand, AiError> {
    let prompt = cli_prompt(request)?;
    if prompt.trim().is_empty() {
        return Err(AiError::from_kind(
            ErrorKind::InvalidRequest,
            "the message has no text, and Gemini CLI cannot be sent images",
        ));
    }
    let server = match &request.mcp {
        Some(mcp) => {
            validate_mcp_name(&mcp.name)?;
            mcp.name.as_str()
        }
        None => NO_MCP_SERVER,
    };

    let mut command = command_for(binary).args([
        "--output-format",
        "stream-json",
        // Headless runs refuse a folder nobody has trusted yet; this one is
        // the app's own.
        "--skip-trust",
        // Anything that would need a confirmation is denied, not approved.
        "--approval-mode",
        "default",
        // Of all configured MCP servers, the user's included, only ours.
        "--allowed-mcp-server-names",
        server,
    ]);
    if let Some(model) = request.model.as_deref()
        && !model.is_empty()
        && model != DEFAULT_MODEL
    {
        command = command.args(["--model", model]);
    }
    if let Some(session_id) = request.session_id.as_deref() {
        command = command.args(["--resume", session_id]);
    }
    // Written as one argument so that a prompt starting with a dash is not
    // read as a flag. `--prompt` is also what selects headless mode.
    command = command.arg(format!("--prompt={prompt}"));
    command.cwd = Some(working_dir.to_path_buf());
    Ok(command)
}

/// Writes the project settings and the context file for one turn.
fn write_workspace_files(workspace: &Path, request: &ChatRequest) -> Result<(), AiError> {
    let config_dir = workspace.join(".gemini");
    create_dir(&config_dir)?;
    let mut settings =
        serde_json::to_string_pretty(&project_settings(request.mcp.as_ref())).unwrap_or_default();
    settings.push('\n');
    write_if_changed(&config_dir.join("settings.json"), &settings)?;

    let context_file = workspace.join("GEMINI.md");
    match request.system_prompt.as_deref().map(str::trim) {
        Some(prompt) if !prompt.is_empty() => {
            write_if_changed(&context_file, &format!("{prompt}\n"))
        }
        // A file left over from an earlier turn must not outlive its prompt.
        _ => remove_if_present(&context_file),
    }
}

/// The project-level settings: our server, trusted, and no built-in tools
/// that could touch the user's machine.
fn project_settings(mcp: Option<&McpEndpoint>) -> Value {
    let mut servers = serde_json::Map::new();
    if let Some(mcp) = mcp {
        servers.insert(
            mcp.name.clone(),
            json!({ "httpUrl": mcp.url, "trust": true }),
        );
    }
    let allowed = mcp.map_or(NO_MCP_SERVER, |mcp| mcp.name.as_str());
    json!({
        "mcpServers": servers,
        "mcp": { "allowed": [allowed] },
        "tools": { "exclude": EXCLUDED_TOOLS },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatMessage, ImageData};

    fn command(request: &ChatRequest) -> CliCommand {
        build_command(
            Path::new("/usr/local/bin/gemini"),
            request,
            Path::new("/tmp/workspace"),
        )
        .unwrap()
    }

    /// The value following `flag` on the command line.
    fn value_of(args: &[String], flag: &str) -> Option<String> {
        let index = args.iter().position(|arg| arg == flag)?;
        args.get(index + 1).cloned()
    }

    fn with_mcp() -> ChatRequest {
        ChatRequest {
            mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
            system_prompt: Some("You edit motion designs.".to_owned()),
            ..ChatRequest::from_prompt("hi")
        }
    }

    #[test]
    fn runs_headless_with_streaming_json_and_no_auto_approval() {
        let built = command(&ChatRequest::from_prompt("hi"));
        let args = built.args_lossy();
        assert_eq!(value_of(&args, "--output-format").unwrap(), "stream-json");
        assert!(args.contains(&"--skip-trust".to_owned()));
        assert_eq!(value_of(&args, "--approval-mode").unwrap(), "default");
        assert_eq!(built.cwd.as_deref(), Some(Path::new("/tmp/workspace")));
        for forbidden in ["--yolo", "-y", "yolo", "auto_edit"] {
            assert!(!args.contains(&forbidden.to_owned()), "{forbidden}");
        }
    }

    #[test]
    fn only_the_moshionly_server_is_allowed() {
        let args = command(&with_mcp()).args_lossy();
        assert_eq!(
            value_of(&args, "--allowed-mcp-server-names").unwrap(),
            "moshionly"
        );
        // Without an endpoint, no configured server may load at all.
        let args = command(&ChatRequest::from_prompt("hi")).args_lossy();
        assert_eq!(
            value_of(&args, "--allowed-mcp-server-names").unwrap(),
            "none"
        );
    }

    #[test]
    fn the_prompt_is_one_argument_even_when_it_starts_with_a_dash() {
        let args = command(&ChatRequest::from_prompt("--yolo please")).args_lossy();
        assert_eq!(args.last().unwrap(), "--prompt=--yolo please");
    }

    #[test]
    fn model_and_session_are_passed_when_present() {
        let request = ChatRequest {
            model: Some("flash".to_owned()),
            session_id: Some("abc123".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let args = command(&request).args_lossy();
        assert_eq!(value_of(&args, "--model").unwrap(), "flash");
        assert_eq!(value_of(&args, "--resume").unwrap(), "abc123");

        let request = ChatRequest {
            model: Some("default".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let args = command(&request).args_lossy();
        assert!(!args.contains(&"--model".to_owned()));
        assert!(!args.contains(&"--resume".to_owned()));
    }

    #[test]
    fn a_message_without_text_is_rejected() {
        let request = ChatRequest {
            messages: vec![ChatMessage::user("").with_image(ImageData::png(vec![1]))],
            ..ChatRequest::default()
        };
        let error = build_command(Path::new("gemini"), &request, Path::new("/tmp")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidRequest);
    }

    #[test]
    fn the_settings_trust_our_server_and_exclude_the_built_in_tools() {
        let mcp = McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp");
        let settings = project_settings(Some(&mcp));
        assert_eq!(
            settings["mcpServers"],
            json!({ "moshionly": { "httpUrl": "http://127.0.0.1:47821/mcp", "trust": true } })
        );
        assert_eq!(settings["mcp"]["allowed"], json!(["moshionly"]));
        let excluded = settings["tools"]["exclude"].to_string();
        for tool in ["run_shell_command", "write_file", "read_file", "web_fetch"] {
            assert!(excluded.contains(tool), "{tool} should be excluded");
        }

        let settings = project_settings(None);
        assert_eq!(settings["mcpServers"], json!({}));
        assert_eq!(settings["mcp"]["allowed"], json!(["none"]));
    }

    #[test]
    fn the_workspace_gets_settings_and_the_system_prompt() {
        let dir = tempfile::tempdir().unwrap();
        write_workspace_files(dir.path(), &with_mcp()).unwrap();
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["mcpServers"]["moshionly"]["trust"], true);
        let context = dir.path().join("GEMINI.md");
        assert_eq!(
            std::fs::read_to_string(&context).unwrap(),
            "You edit motion designs.\n"
        );

        // Without a prompt, the earlier context file goes.
        let request = ChatRequest {
            system_prompt: None,
            ..with_mcp()
        };
        write_workspace_files(dir.path(), &request).unwrap();
        assert!(!context.exists());
    }

    #[cfg(unix)]
    mod with_a_stand_in_cli {
        use super::*;
        use crate::cli::test_support::{collect_events, fake_agent, fixture_path};
        use crate::event::ChatEvent;

        fn provider(binary: PathBuf) -> GeminiCli {
            GeminiCli::with_options(GeminiCliOptions {
                binary: Some(binary),
            })
        }

        fn request(root: &Path) -> ChatRequest {
            ChatRequest {
                working_dir: Some(root.join("workspace")),
                ..with_mcp()
            }
        }

        #[tokio::test]
        async fn a_whole_turn_with_a_tool_call_comes_through_in_order() {
            let dir = tempfile::tempdir().unwrap();
            // Records what it finds in the workspace, then plays the
            // documented stream.
            let script = format!(
                "cat .gemini/settings.json > \"$0.settings\"; cat GEMINI.md > \"$0.context\"; cat {}",
                fixture_path("gemini/documented_tool_call.jsonl")
            );
            let binary = fake_agent(dir.path(), "gemini", &script);
            let events = collect_events(&provider(binary.clone()), request(dir.path())).await;

            assert!(matches!(events[0], ChatEvent::SessionStarted { .. }));
            assert!(
                matches!(&events[1], ChatEvent::ToolCallStarted { name, .. } if name == "echo")
            );
            assert!(matches!(
                events[2],
                ChatEvent::ToolCallFinished { ok: true, .. }
            ));
            assert_eq!(
                events.last(),
                Some(&ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                })
            );

            // The configuration was in place before the CLI started.
            let settings =
                std::fs::read_to_string(format!("{}.settings", binary.display())).unwrap();
            assert!(settings.contains("http://127.0.0.1:47821/mcp"));
            let context = std::fs::read_to_string(format!("{}.context", binary.display())).unwrap();
            assert_eq!(context, "You edit motion designs.\n");
        }

        #[tokio::test]
        async fn readiness_reports_the_version() {
            let dir = tempfile::tempdir().unwrap();
            let binary = fake_agent(dir.path(), "gemini", "echo 0.41.2");
            let readiness = provider(binary).check_ready().await;
            assert_eq!(readiness.state, ReadyState::Ready);
            assert_eq!(readiness.version.as_deref(), Some("0.41.2"));
        }

        #[tokio::test]
        async fn a_missing_sign_in_is_reported_as_not_signed_in() {
            let dir = tempfile::tempdir().unwrap();
            let binary = fake_agent(
                dir.path(),
                "gemini",
                "echo 'Please set an Auth method in your settings.json or specify one of the \
                 following environment variables before running: GEMINI_API_KEY' >&2; exit 41",
            );
            let events = collect_events(&provider(binary), request(dir.path())).await;
            assert!(matches!(
                events[0],
                ChatEvent::Error {
                    kind: ErrorKind::NotSignedIn,
                    ..
                }
            ));
        }
    }

    fn missing() -> GeminiCli {
        GeminiCli::with_options(GeminiCliOptions {
            binary: Some(PathBuf::from("/nonexistent/gemini")),
        })
    }

    #[tokio::test]
    async fn a_missing_binary_is_not_installed() {
        let readiness = missing().check_ready().await;
        assert_eq!(readiness.state, ReadyState::NotInstalled);
        assert!(readiness.message.contains("not installed"));
    }

    #[tokio::test]
    async fn sending_without_the_binary_fails_cleanly() {
        use crate::event::{ChatEvent, event_channel};

        let (events, mut receiver) = event_channel();
        missing()
            .send(
                ChatRequest::from_prompt("hi"),
                events,
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            receiver.recv().await,
            Some(ChatEvent::Error {
                message: "gemini is not installed".to_owned(),
                kind: ErrorKind::NotInstalled,
            })
        );
        assert_eq!(
            receiver.recv().await,
            Some(ChatEvent::Finished {
                stop_reason: StopReason::Error
            })
        );
        assert_eq!(receiver.recv().await, None);
    }
}

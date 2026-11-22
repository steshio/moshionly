//! The Cursor Agent provider: chat through the `cursor-agent` CLI and the
//! user's own Cursor subscription.
//!
//! UNVERIFIED beyond the signed-out path. No signed-in Cursor account was
//! available when this was written, so a full turn has never been run
//! against the real CLI. What is real: finding the binary, reading its
//! version and sign-in state, the flags (the CLI accepts them), and the
//! "not signed in" failure. The rest follows Cursor's documentation. The
//! ignored test in `tests/cursor_agent_live.rs` checks a whole turn once
//! someone is signed in.
//!
//! The CLI is run once per turn in print mode, in an app-owned workspace
//! that holds its configuration (see [`workspace`]): exactly one MCP server,
//! permissions that allow that server's tools and deny shell, file and web
//! access, and the system prompt as a rule. Moshionly never reads the CLI's
//! credentials: signing in is done by the user, in the CLI.
//!
//! Two limits come from the CLI itself:
//!
//! - MCP servers from the user's own `~/.cursor/mcp.json` are loaded as
//!   well; there is no strict mode. Their tools are not on the allow list
//!   written here, so they run only if the user's global permissions allow
//!   them.
//! - The prompt is a command-line argument, so other processes of the same
//!   user can see it while the CLI runs.

mod stream;
mod workspace;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use self::stream::CursorStream;
use self::workspace::write_workspace_files;
use super::discover::{PROBE_TIMEOUT, command_for, find_binary, read_version};
use super::process::{CliCommand, run_streaming, run_to_string};
use super::prompt::{cli_prompt, prepare_working_dir, validate_mcp_name};
use super::stopped;
use crate::chat::ChatRequest;
use crate::error::{AiError, ErrorKind};
use crate::event::{EventSender, StopReason};
use crate::provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
    ToolTransport,
};

/// The provider id used in settings and on the command line.
pub const CURSOR_AGENT_ID: &str = "cursor-agent";

/// The model entry that leaves the choice to the CLI's own configuration.
const DEFAULT_MODEL: &str = "default";

/// Settings for the Cursor Agent provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorAgentOptions {
    /// Where the `cursor-agent` executable is, when the user chose one.
    /// Otherwise it is looked up on `PATH` and in the usual install
    /// locations.
    pub binary: Option<PathBuf>,
    /// Let the agent read files. Off by default, so the agent can reach
    /// nothing but Moshionly's tools.
    ///
    /// Cursor Agent can keep the descriptions of MCP tools in files that the
    /// model reads on demand. If it cannot call Moshionly's tools with reads
    /// denied, turning this on is the thing to try; writing, shell commands
    /// and web fetches stay denied either way.
    pub allow_file_reads: bool,
}

/// Chat through the user's Cursor Agent installation.
#[derive(Debug, Clone, Default)]
pub struct CursorAgent {
    options: CursorAgentOptions,
}

impl CursorAgent {
    /// A provider with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// A provider with the given options.
    pub fn with_options(options: CursorAgentOptions) -> Self {
        Self { options }
    }

    fn binary(&self) -> Result<PathBuf, AiError> {
        // The installer also links the CLI as `agent`. That name is too
        // common to run whatever answers to it, so only the full name is
        // looked for.
        find_binary(&["cursor-agent"], self.options.binary.as_deref()).ok_or_else(|| {
            AiError::NotInstalled {
                program: "cursor-agent".to_owned(),
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
        if let Some(mcp) = &request.mcp {
            validate_mcp_name(&mcp.name)?;
        }
        let working_dir = prepare_working_dir(request)?;
        let command = build_command(&binary, request, &working_dir)?;
        write_workspace_files(&working_dir, request, self.options.allow_file_reads)?;
        let mut stream = CursorStream::new(request.mcp.as_ref().map(|mcp| mcp.name.as_str()));
        let exit = run_streaming(command, stopped(cancel, events), |line| {
            stream.push_line(line, events)
        })
        .await?;
        stream.finish(exit)
    }
}

#[async_trait]
impl ChatProvider for CursorAgent {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: CURSOR_AGENT_ID.to_owned(),
            name: "Cursor Agent".to_owned(),
            kind: ProviderKind::Subscription,
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tools: ToolTransport::Mcp,
            // The CLI takes images only as file paths that the agent reads
            // itself, and reading files is denied here.
            images: false,
            resume: true,
            thinking: true,
            live_model_list: true,
            needs_api_key: false,
            verified: false,
        }
    }

    /// Asks the CLI which models the signed-in account can use. When it
    /// cannot say (not installed, not signed in, or output this crate does
    /// not recognise) the list falls back to the two entries that always
    /// work.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let mut models = vec![ModelInfo::new(
            DEFAULT_MODEL,
            "Default (as set in Cursor Agent)",
        )];
        if let Ok(binary) = self.binary() {
            let command = command_for(&binary).arg("models");
            if let Ok((_, stdout, _)) = run_to_string(command, PROBE_TIMEOUT).await {
                models.extend(parse_models(&stdout));
            }
        }
        if models.len() == 1 {
            models.push(ModelInfo::new("auto", "Auto"));
        }
        Ok(models)
    }

    async fn check_ready(&self) -> Readiness {
        let binary = match self.binary() {
            Ok(binary) => binary,
            Err(_) => {
                return Readiness::not_ready(
                    ReadyState::NotInstalled,
                    "Cursor Agent is not installed. Install it from https://cursor.com/cli, \
                     then run `cursor-agent login` in a terminal.",
                );
            }
        };
        let version = read_version(&binary).await;
        let label = match &version {
            Some(version) => format!("Cursor Agent {version}"),
            None => "Cursor Agent".to_owned(),
        };
        let readiness = match signed_in(&binary).await {
            Some(true) => Readiness::ready(format!("{label} is installed and signed in.")),
            Some(false) => Readiness::not_ready(
                ReadyState::NotSignedIn,
                format!(
                    "{label} is installed but not signed in. Run `cursor-agent login` in a terminal."
                ),
            ),
            None => Readiness::ready(format!(
                "{label} is installed. Its sign-in state could not be checked."
            )),
        };
        match version {
            Some(version) => readiness.with_version(version),
            None => readiness,
        }
    }

    async fn send(&self, request: ChatRequest, events: EventSender, cancel: CancellationToken) {
        let outcome = self.run(&request, &events, &cancel).await;
        events.finish(outcome);
    }
}

/// Asks the CLI whether it is signed in. Only the yes-or-no answer is read;
/// the account details in the same output are ignored.
async fn signed_in(binary: &Path) -> Option<bool> {
    let command = command_for(binary).args(["status", "--format", "json"]);
    let (_, stdout, _) = run_to_string(command, PROBE_TIMEOUT).await.ok()?;
    parse_signed_in(&stdout)
}

fn parse_signed_in(status_output: &str) -> Option<bool> {
    let value: Value = serde_json::from_str(status_output.trim()).ok()?;
    value.get("isAuthenticated").and_then(Value::as_bool)
}

/// Reads the output of `cursor-agent models`: one `id - Name` per line,
/// between a heading and a tip. Anything that does not look like that is
/// skipped.
fn parse_models(output: &str) -> Vec<ModelInfo> {
    output
        .lines()
        .filter_map(|line| {
            let (id, name) = strip_ansi(line)
                .trim()
                .split_once(" - ")
                .map(|(id, name)| (id.trim().to_owned(), name.trim().to_owned()))?;
            let plausible_id = !id.is_empty()
                && id.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, '-' | '_' | '.' | '[' | ']' | '=' | ',')
                });
            if !plausible_id || id == DEFAULT_MODEL {
                return None;
            }
            // The CLI marks the model in use with a note in brackets.
            let name = match name.split_once("  (") {
                Some((name, _)) => name.trim().to_owned(),
                None => name
                    .trim_end_matches("(current)")
                    .trim_end_matches("(default)")
                    .trim()
                    .to_owned(),
            };
            Some(ModelInfo {
                name: if name.is_empty() { id.clone() } else { name },
                id,
            })
        })
        .collect()
}

/// Removes terminal colour codes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Skip a control sequence: ESC, '[', parameters, one final letter.
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
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
            "the message has no text, and Cursor Agent cannot be sent images",
        ));
    }

    let mut command = command_for(binary).args([
        "--print",
        "--output-format",
        "stream-json",
        "--stream-partial-output",
        // Use the workspace's own configuration without a trust prompt...
        "--trust",
        // ...and load its MCP server without an approval prompt.
        "--approve-mcps",
        // With the sandbox on, shell commands that fit in it run without
        // asking. Off, a command needs an allow rule, and the workspace
        // denies them all.
        "--sandbox",
        "disabled",
    ]);
    command = command.arg("--workspace").arg(working_dir);
    if let Some(model) = request.model.as_deref()
        && !model.is_empty()
        && model != DEFAULT_MODEL
    {
        command = command.args(["--model", model]);
    }
    if let Some(session_id) = request.session_id.as_deref() {
        command = command.args(["--resume", session_id]);
    }
    // After `--` nothing is read as an option, whatever the prompt starts
    // with.
    command = command.arg("--").arg(prompt);
    command.cwd = Some(working_dir.to_path_buf());
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatMessage, ImageData};

    fn command(request: &ChatRequest) -> CliCommand {
        build_command(
            Path::new("/usr/local/bin/cursor-agent"),
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

    #[test]
    fn runs_in_print_mode_with_streaming_json() {
        let args = command(&ChatRequest::from_prompt("hi")).args_lossy();
        assert!(args.contains(&"--print".to_owned()));
        assert_eq!(value_of(&args, "--output-format").unwrap(), "stream-json");
        assert!(args.contains(&"--stream-partial-output".to_owned()));
    }

    #[test]
    fn the_workspace_is_trusted_and_nothing_is_forced() {
        let built = command(&ChatRequest::from_prompt("hi"));
        let args = built.args_lossy();
        assert_eq!(value_of(&args, "--workspace").unwrap(), "/tmp/workspace");
        assert_eq!(built.cwd.as_deref(), Some(Path::new("/tmp/workspace")));
        assert!(args.contains(&"--trust".to_owned()));
        assert!(args.contains(&"--approve-mcps".to_owned()));
        assert_eq!(value_of(&args, "--sandbox").unwrap(), "disabled");
        for forbidden in ["--force", "-f", "--yolo"] {
            assert!(!args.contains(&forbidden.to_owned()), "{forbidden}");
        }
    }

    #[test]
    fn model_and_session_are_passed_when_present() {
        let request = ChatRequest {
            model: Some("gpt-5".to_owned()),
            session_id: Some("chat-1".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let args = command(&request).args_lossy();
        assert_eq!(value_of(&args, "--model").unwrap(), "gpt-5");
        assert_eq!(value_of(&args, "--resume").unwrap(), "chat-1");

        let request = ChatRequest {
            model: Some("default".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let args = command(&request).args_lossy();
        assert!(!args.contains(&"--model".to_owned()));
        assert!(!args.contains(&"--resume".to_owned()));
    }

    #[test]
    fn the_prompt_is_the_last_argument_and_cannot_be_read_as_an_option() {
        let args = command(&ChatRequest::from_prompt("--force everything")).args_lossy();
        let separator = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(separator, args.len() - 2);
        assert_eq!(args.last().unwrap(), "--force everything");
    }

    #[test]
    fn a_message_without_text_is_rejected() {
        let request = ChatRequest {
            messages: vec![ChatMessage::user("  ").with_image(ImageData::png(vec![1]))],
            ..ChatRequest::default()
        };
        let error =
            build_command(Path::new("cursor-agent"), &request, Path::new("/tmp")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidRequest);
    }

    #[test]
    fn sign_in_state_is_read_from_status_and_nothing_else() {
        // The real answer of a signed-out CLI.
        let signed_out = r#"{
            "status": "unauthenticated",
            "isAuthenticated": false,
            "hasAccessToken": false,
            "hasRefreshToken": false,
            "message": "Not logged in"
        }"#;
        assert_eq!(parse_signed_in(signed_out), Some(false));
        assert_eq!(
            parse_signed_in(r#"{"status":"authenticated","isAuthenticated":true}"#),
            Some(true)
        );
        assert_eq!(parse_signed_in("Not logged in"), None);
        assert_eq!(parse_signed_in("{}"), None);
    }

    #[test]
    fn model_lines_are_parsed_and_everything_else_is_skipped() {
        let output = "Available models\n\n\
            auto - Auto  (current)\n\
            gpt-5 - GPT-5\n\
            \u{1b}[1msonnet-4.5-thinking\u{1b}[0m - Claude 4.5 Sonnet Thinking (default)\n\
            \n\
            Tip: use --model <id> (or /model <id> in interactive mode) to switch.\n";
        assert_eq!(
            parse_models(output),
            vec![
                ModelInfo::new("auto", "Auto"),
                ModelInfo::new("gpt-5", "GPT-5"),
                ModelInfo::new("sonnet-4.5-thinking", "Claude 4.5 Sonnet Thinking"),
            ]
        );
    }

    #[test]
    fn the_signed_out_answer_of_the_models_command_yields_no_models() {
        // Real output of `cursor-agent models` without a sign-in.
        let output = "Error: Authentication required. Run 'agent login', pass \
                      --api-key/--auth-token, or set CURSOR_API_KEY/CURSOR_AUTH_TOKEN.";
        assert!(parse_models(output).is_empty());
    }

    #[cfg(unix)]
    mod with_a_stand_in_cli {
        use super::*;
        use crate::chat::McpEndpoint;
        use crate::cli::cursor_agent::stream::SIGN_IN_HINT;
        use crate::cli::test_support::{collect_events, fake_agent, fixture_path};
        use crate::event::{ChatEvent, event_channel};

        fn provider(binary: PathBuf) -> CursorAgent {
            CursorAgent::with_options(CursorAgentOptions {
                binary: Some(binary),
                ..CursorAgentOptions::default()
            })
        }

        fn request(root: &Path) -> ChatRequest {
            ChatRequest {
                mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
                system_prompt: Some("You edit motion designs.".to_owned()),
                working_dir: Some(root.join("workspace")),
                ..ChatRequest::from_prompt("Call the echo tool")
            }
        }

        #[tokio::test]
        async fn a_whole_turn_with_a_tool_call_comes_through_in_order() {
            let dir = tempfile::tempdir().unwrap();
            // Records its arguments and what it finds in the workspace, then
            // plays the documented stream.
            let script = format!(
                "printf '%s\\n' \"$@\" > \"$0.args\"; cat .cursor/mcp.json > \"$0.mcp\"; cat {}",
                fixture_path("cursor-agent/documented_tool_call.jsonl")
            );
            let binary = fake_agent(dir.path(), "cursor-agent", &script);
            let events = collect_events(&provider(binary.clone()), request(dir.path())).await;

            let kinds: Vec<&str> = events
                .iter()
                .map(|event| match event {
                    ChatEvent::SessionStarted { .. } => "session",
                    ChatEvent::TextDelta { .. } => "text",
                    ChatEvent::ThinkingDelta { .. } => "thinking",
                    ChatEvent::ToolCallStarted { .. } => "tool_started",
                    ChatEvent::ToolCallFinished { .. } => "tool_finished",
                    ChatEvent::Usage { .. } => "usage",
                    ChatEvent::Error { .. } => "error",
                    ChatEvent::Finished { .. } => "finished",
                })
                .collect();
            assert_eq!(
                kinds,
                [
                    "session",
                    "thinking",
                    "text",
                    "text",
                    "tool_started",
                    "tool_finished",
                    "text",
                    "usage",
                    "finished"
                ]
            );
            assert_eq!(
                events.last(),
                Some(&ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                })
            );

            // The configuration was in place before the CLI started.
            let mcp = std::fs::read_to_string(format!("{}.mcp", binary.display())).unwrap();
            assert!(mcp.contains("http://127.0.0.1:47821/mcp"));
            let workspace = dir.path().join("workspace");
            assert!(workspace.join(".cursor/cli.json").is_file());
            assert!(workspace.join(".cursor/rules/moshionly.mdc").is_file());
            let args = std::fs::read_to_string(format!("{}.args", binary.display())).unwrap();
            assert!(args.ends_with("--\nCall the echo tool\n"), "{args}");
        }

        #[tokio::test]
        async fn the_real_signed_out_failure_is_reported_as_not_signed_in() {
            let dir = tempfile::tempdir().unwrap();
            let script = format!(
                "cat {} >&2; exit 1",
                fixture_path("cursor-agent/not_signed_in.stderr.txt")
            );
            let binary = fake_agent(dir.path(), "cursor-agent", &script);
            let events = collect_events(&provider(binary), request(dir.path())).await;
            assert_eq!(
                events,
                vec![
                    ChatEvent::Error {
                        message: format!("not signed in: {SIGN_IN_HINT}"),
                        kind: ErrorKind::NotSignedIn,
                    },
                    ChatEvent::Finished {
                        stop_reason: StopReason::Error
                    },
                ]
            );
        }

        #[tokio::test]
        async fn readiness_reports_version_and_sign_in_state() {
            let dir = tempfile::tempdir().unwrap();
            let script = r#"case "$1" in
  --version) echo "2026.08.11-e8db854" ;;
  status) echo '{"status":"unauthenticated","isAuthenticated":false}' ;;
esac"#;
            let binary = fake_agent(dir.path(), "cursor-agent", script);
            let readiness = provider(binary).check_ready().await;
            assert_eq!(readiness.state, ReadyState::NotSignedIn);
            assert_eq!(readiness.version.as_deref(), Some("2026.08.11-e8db854"));
            assert!(readiness.message.contains("cursor-agent login"));
        }

        #[tokio::test]
        async fn models_come_from_the_cli_with_the_default_entry_first() {
            let dir = tempfile::tempdir().unwrap();
            let script = r#"if [ "$1" = models ]; then
  echo "Available models"; echo; echo "auto - Auto"; echo "gpt-5 - GPT-5"
fi"#;
            let binary = fake_agent(dir.path(), "cursor-agent", script);
            let models = provider(binary).list_models().await.unwrap();
            let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
            assert_eq!(ids, ["default", "auto", "gpt-5"]);
        }

        #[tokio::test]
        async fn cancelling_stops_the_cli_and_reports_cancelled() {
            let dir = tempfile::tempdir().unwrap();
            let script = "echo $$ > \"$0.pid\"; echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\"}'; sleep 300";
            let binary = fake_agent(dir.path(), "cursor-agent", script);
            let provider = provider(binary.clone());
            let cancel = CancellationToken::new();
            let (events, mut receiver) = event_channel();
            let task = tokio::spawn({
                let cancel = cancel.clone();
                let request = request(dir.path());
                async move { provider.send(request, events, cancel).await }
            });

            assert_eq!(
                receiver.recv().await,
                Some(ChatEvent::SessionStarted {
                    session_id: "s1".to_owned()
                })
            );
            cancel.cancel();
            assert_eq!(
                receiver.recv().await,
                Some(ChatEvent::Finished {
                    stop_reason: StopReason::Cancelled
                })
            );
            task.await.unwrap();

            let pid: i32 = std::fs::read_to_string(format!("{}.pid", binary.display()))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let alive = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok();
            assert!(!alive, "the CLI process survived cancellation");
        }
    }

    fn missing() -> CursorAgent {
        CursorAgent::with_options(CursorAgentOptions {
            binary: Some(PathBuf::from("/nonexistent/cursor-agent")),
            ..CursorAgentOptions::default()
        })
    }

    #[tokio::test]
    async fn a_missing_binary_is_not_installed() {
        let readiness = missing().check_ready().await;
        assert_eq!(readiness.state, ReadyState::NotInstalled);
        let models = missing().list_models().await.unwrap();
        assert_eq!(models.len(), 2);
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
                message: "cursor-agent is not installed".to_owned(),
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

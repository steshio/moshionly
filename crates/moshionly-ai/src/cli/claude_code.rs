//! The Claude Code provider: chat through the `claude` CLI and the user's
//! own subscription.
//!
//! The CLI is run once per turn in print mode. It is given exactly one MCP
//! server (Moshionly's), allowed to call that server's tools without asking,
//! and stripped of its own file, shell and web tools, so a chat in a design
//! app cannot touch the user's disk. Moshionly never reads the CLI's
//! credentials: signing in is done by the user, in the CLI.
//!
//! Verified against Claude Code 2.1.325.

mod stream;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use self::stream::ClaudeStream;
use super::discover::{PROBE_TIMEOUT, command_for, find_binary, read_version};
use super::process::{CliCommand, run_streaming, run_to_string};
use super::prompt::{cli_prompt, last_user_message, prepare_working_dir, validate_mcp_name};
use super::stopped;
use crate::chat::ChatRequest;
use crate::error::AiError;
use crate::event::{EventSender, StopReason};
use crate::provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
    ToolTransport,
};

/// The provider id used in settings and on the command line.
pub const CLAUDE_CODE_ID: &str = "claude-code";

/// The model entry that leaves the choice to the CLI's own configuration.
const DEFAULT_MODEL: &str = "default";

/// Settings for the Claude Code provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeOptions {
    /// Where the `claude` executable is, when the user chose one. Otherwise
    /// it is looked up on `PATH` and in the usual install locations.
    pub binary: Option<PathBuf>,
    /// Run without the user's own Claude Code settings: no personal
    /// instructions, hooks, plugins or skills, only Moshionly's prompt and
    /// tools. On by default.
    ///
    /// Turn it off for a setup that signs in through those settings (an
    /// `apiKeyHelper`, or environment variables for a cloud provider), at the
    /// price of the user's coding setup leaking into the design chat.
    pub isolate_settings: bool,
}

impl Default for ClaudeCodeOptions {
    fn default() -> Self {
        Self {
            binary: None,
            isolate_settings: true,
        }
    }
}

/// Chat through the user's Claude Code installation.
#[derive(Debug, Clone, Default)]
pub struct ClaudeCode {
    options: ClaudeCodeOptions,
}

impl ClaudeCode {
    /// A provider with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// A provider with the given options.
    pub fn with_options(options: ClaudeCodeOptions) -> Self {
        Self { options }
    }

    fn binary(&self) -> Result<PathBuf, AiError> {
        find_binary(&["claude"], self.options.binary.as_deref()).ok_or_else(|| {
            AiError::NotInstalled {
                program: "claude".to_owned(),
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
        let command = build_command(&binary, request, &self.options, &working_dir)?;
        let mut stream = ClaudeStream::new(request.mcp.as_ref().map(|mcp| mcp.name.as_str()));
        let exit = run_streaming(command, stopped(cancel, events), |line| {
            stream.push_line(line, events)
        })
        .await?;
        stream.finish(exit)
    }
}

#[async_trait]
impl ChatProvider for ClaudeCode {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: CLAUDE_CODE_ID.to_owned(),
            name: "Claude Code".to_owned(),
            kind: ProviderKind::Subscription,
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tools: ToolTransport::Mcp,
            images: true,
            resume: true,
            thinking: true,
            live_model_list: false,
            needs_api_key: false,
            verified: true,
        }
    }

    /// The CLI has no command that lists models. These are its documented
    /// aliases, which always point at the newest model of each family; a full
    /// model name works too.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        Ok(vec![
            ModelInfo::new(DEFAULT_MODEL, "Default (as set in Claude Code)"),
            ModelInfo::new("opus", "Opus"),
            ModelInfo::new("sonnet", "Sonnet"),
            ModelInfo::new("haiku", "Haiku"),
            ModelInfo::new("fable", "Fable"),
        ])
    }

    async fn check_ready(&self) -> Readiness {
        let binary = match self.binary() {
            Ok(binary) => binary,
            Err(_) => {
                return Readiness::not_ready(
                    ReadyState::NotInstalled,
                    "Claude Code is not installed. Install it from https://claude.com/claude-code, \
                     then run `claude` in a terminal and sign in.",
                );
            }
        };
        let version = read_version(&binary).await;
        let label = match &version {
            Some(version) => format!("Claude Code {version}"),
            None => "Claude Code".to_owned(),
        };
        let readiness = match signed_in(&binary).await {
            Some(true) => Readiness::ready(format!("{label} is installed and signed in.")),
            Some(false) => Readiness::not_ready(
                ReadyState::NotSignedIn,
                format!(
                    "{label} is installed but not signed in. Run `claude` in a terminal and sign in."
                ),
            ),
            // An older CLI without `auth status`: the first message will tell.
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
    let command = command_for(binary).args(["auth", "status"]);
    let (_, stdout, _) = run_to_string(command, PROBE_TIMEOUT).await.ok()?;
    parse_signed_in(&stdout)
}

fn parse_signed_in(auth_status_output: &str) -> Option<bool> {
    let value: Value = serde_json::from_str(auth_status_output.trim()).ok()?;
    value.get("loggedIn").and_then(Value::as_bool)
}

/// The command line for one turn.
fn build_command(
    binary: &Path,
    request: &ChatRequest,
    options: &ClaudeCodeOptions,
    working_dir: &Path,
) -> Result<CliCommand, AiError> {
    let mut command = command_for(binary).args([
        "--print",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        // Required by the CLI for stream-json output in print mode.
        "--verbose",
        "--include-partial-messages",
        // Skills and custom commands belong to the user's coding setup.
        "--disable-slash-commands",
        // No built-in tools: no file access, no shell, no web.
        "--tools",
        "",
        // Anything that is not allowed below is refused without a prompt
        // that nobody could answer.
        "--permission-mode",
        "dontAsk",
        // Only the servers named here, never the user's own MCP servers.
        "--strict-mcp-config",
    ]);
    if options.isolate_settings {
        command = command.args(["--setting-sources", ""]);
    }

    let mut servers = serde_json::Map::new();
    if let Some(mcp) = &request.mcp {
        validate_mcp_name(&mcp.name)?;
        servers.insert(mcp.name.clone(), json!({ "type": "http", "url": mcp.url }));
        // Every tool of our server, and nothing else, runs without asking.
        command = command.args(["--allowedTools".to_owned(), format!("mcp__{}", mcp.name)]);
    }
    command = command.args([
        "--mcp-config".to_owned(),
        json!({ "mcpServers": servers }).to_string(),
    ]);

    if let Some(system_prompt) = request.system_prompt.as_deref()
        && !system_prompt.trim().is_empty()
    {
        command = command.args(["--append-system-prompt", system_prompt]);
    }
    if let Some(model) = request.model.as_deref()
        && !model.is_empty()
        && model != DEFAULT_MODEL
    {
        command = command.args(["--model", model]);
    }
    if let Some(session_id) = request.session_id.as_deref() {
        command = command.args(["--resume", session_id]);
    }

    command.cwd = Some(working_dir.to_path_buf());
    command.stdin = Some(user_message_line(request)?);
    Ok(command)
}

/// The user's message as the one line of stream-json input the CLI reads.
/// This form, unlike a plain prompt argument, can carry images.
fn user_message_line(request: &ChatRequest) -> Result<String, AiError> {
    let (_, last) = last_user_message(request)?;
    let mut content = vec![json!({ "type": "text", "text": cli_prompt(request)? })];
    for image in &last.images {
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.media_type,
                "data": image.to_base64(),
            }
        }));
    }
    let mut line = json!({
        "type": "user",
        "message": { "role": "user", "content": content }
    })
    .to_string();
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatMessage, ImageData, McpEndpoint};

    fn command(request: &ChatRequest) -> CliCommand {
        build_command(
            Path::new("/usr/local/bin/claude"),
            request,
            &ClaudeCodeOptions::default(),
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
    fn runs_in_print_mode_with_streaming_json_both_ways() {
        let args = command(&ChatRequest::from_prompt("hi")).args_lossy();
        assert!(args.contains(&"--print".to_owned()));
        assert_eq!(value_of(&args, "--output-format").unwrap(), "stream-json");
        assert_eq!(value_of(&args, "--input-format").unwrap(), "stream-json");
        assert!(args.contains(&"--verbose".to_owned()));
        assert!(args.contains(&"--include-partial-messages".to_owned()));
    }

    #[test]
    fn built_in_tools_are_off_and_nothing_can_prompt() {
        let args = command(&ChatRequest::from_prompt("hi")).args_lossy();
        assert_eq!(value_of(&args, "--tools").unwrap(), "");
        assert_eq!(value_of(&args, "--permission-mode").unwrap(), "dontAsk");
        assert!(args.contains(&"--disable-slash-commands".to_owned()));
        assert!(!args.iter().any(|arg| arg.contains("skip-permissions")));
    }

    #[test]
    fn only_the_moshionly_server_is_attached_and_allowed() {
        let request = ChatRequest {
            mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
            ..ChatRequest::from_prompt("hi")
        };
        let args = command(&request).args_lossy();
        assert!(args.contains(&"--strict-mcp-config".to_owned()));
        let config: Value =
            serde_json::from_str(&value_of(&args, "--mcp-config").unwrap()).unwrap();
        assert_eq!(
            config,
            json!({ "mcpServers": { "moshionly": {
                "type": "http", "url": "http://127.0.0.1:47821/mcp" } } })
        );
        assert_eq!(value_of(&args, "--allowedTools").unwrap(), "mcp__moshionly");
    }

    #[test]
    fn without_an_endpoint_no_server_and_no_tool_is_allowed() {
        let args = command(&ChatRequest::from_prompt("hi")).args_lossy();
        assert!(args.contains(&"--strict-mcp-config".to_owned()));
        assert_eq!(
            value_of(&args, "--mcp-config").unwrap(),
            r#"{"mcpServers":{}}"#
        );
        assert!(!args.contains(&"--allowedTools".to_owned()));
    }

    #[test]
    fn an_unsafe_server_name_is_rejected() {
        let request = ChatRequest {
            mcp: Some(McpEndpoint::new("bad name", "http://127.0.0.1:1/mcp")),
            ..ChatRequest::from_prompt("hi")
        };
        let result = build_command(
            Path::new("claude"),
            &request,
            &ClaudeCodeOptions::default(),
            Path::new("/tmp"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn user_settings_are_left_out_unless_asked_for() {
        let request = ChatRequest::from_prompt("hi");
        let args = command(&request).args_lossy();
        assert_eq!(value_of(&args, "--setting-sources").unwrap(), "");

        let options = ClaudeCodeOptions {
            isolate_settings: false,
            ..ClaudeCodeOptions::default()
        };
        let args = build_command(Path::new("claude"), &request, &options, Path::new("/tmp"))
            .unwrap()
            .args_lossy();
        assert!(!args.contains(&"--setting-sources".to_owned()));
    }

    #[test]
    fn system_prompt_model_and_session_are_passed_when_present() {
        let request = ChatRequest {
            system_prompt: Some("You edit motion designs.".to_owned()),
            model: Some("sonnet".to_owned()),
            session_id: Some("0f0f".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let args = command(&request).args_lossy();
        assert_eq!(
            value_of(&args, "--append-system-prompt").unwrap(),
            "You edit motion designs."
        );
        assert_eq!(value_of(&args, "--model").unwrap(), "sonnet");
        assert_eq!(value_of(&args, "--resume").unwrap(), "0f0f");
    }

    #[test]
    fn the_default_model_and_empty_prompt_add_no_flags() {
        let request = ChatRequest {
            system_prompt: Some("  ".to_owned()),
            model: Some("default".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let args = command(&request).args_lossy();
        assert!(!args.contains(&"--append-system-prompt".to_owned()));
        assert!(!args.contains(&"--model".to_owned()));
        assert!(!args.contains(&"--resume".to_owned()));
    }

    #[test]
    fn the_prompt_goes_to_stdin_and_the_run_uses_the_working_dir() {
        let built = command(&ChatRequest::from_prompt("Fade the card in"));
        assert_eq!(built.cwd.as_deref(), Some(Path::new("/tmp/workspace")));
        let stdin = built.stdin.clone().unwrap();
        assert!(stdin.ends_with('\n'));
        let line: Value = serde_json::from_str(&stdin).unwrap();
        assert_eq!(line["type"], "user");
        assert_eq!(line["message"]["role"], "user");
        assert_eq!(line["message"]["content"][0]["text"], "Fade the card in");
        // The prompt never appears on the command line, where other
        // processes could read it.
        assert!(!built.args_lossy().iter().any(|arg| arg.contains("Fade")));
    }

    #[test]
    fn images_of_the_newest_message_are_attached() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage::user("old").with_image(ImageData::png(vec![9])),
                ChatMessage::assistant("ok"),
                ChatMessage::user("match this").with_image(ImageData::png(vec![1, 2, 3])),
            ],
            session_id: Some("s".to_owned()),
            ..ChatRequest::default()
        };
        let line: Value = serde_json::from_str(&user_message_line(&request).unwrap()).unwrap();
        let content = line["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["text"], "match this");
        assert_eq!(
            content[1],
            json!({ "type": "image", "source": {
                "type": "base64", "media_type": "image/png", "data": "AQID" } })
        );
    }

    #[test]
    fn sign_in_state_is_read_from_auth_status_and_nothing_else() {
        let signed_in =
            r#"{ "loggedIn": true, "authMethod": "claude.ai", "email": "x@example.com" }"#;
        assert_eq!(parse_signed_in(signed_in), Some(true));
        assert_eq!(parse_signed_in(r#"{"loggedIn": false}"#), Some(false));
        assert_eq!(parse_signed_in("error: unknown command 'auth'"), None);
        assert_eq!(parse_signed_in("{}"), None);
    }

    #[cfg(unix)]
    mod with_a_stand_in_cli {
        use super::*;
        use crate::cli::test_support::{collect_events, fake_agent, fixture_path};
        use crate::error::ErrorKind;
        use crate::event::{ChatEvent, event_channel};

        fn provider(binary: PathBuf) -> ClaudeCode {
            ClaudeCode::with_options(ClaudeCodeOptions {
                binary: Some(binary),
                ..ClaudeCodeOptions::default()
            })
        }

        fn request(workspace: &Path) -> ChatRequest {
            ChatRequest {
                mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
                working_dir: Some(workspace.join("workspace")),
                ..ChatRequest::from_prompt("Call the echo tool")
            }
        }

        #[tokio::test]
        async fn a_whole_turn_with_a_tool_call_comes_through_in_order() {
            let dir = tempfile::tempdir().unwrap();
            let script = format!(
                "cat > \"$0.stdin\"; pwd > \"$0.cwd\"; cat {}",
                fixture_path("claude-code/tool_call.jsonl")
            );
            let binary = fake_agent(dir.path(), "claude", &script);
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
                events[3],
                ChatEvent::TextDelta {
                    text: "done".to_owned()
                }
            );
            assert!(matches!(events[4], ChatEvent::Usage { .. }));
            assert_eq!(
                events[5],
                ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                }
            );
            assert_eq!(events.len(), 6);

            // The stand-in saw the prompt on stdin and ran in the workspace.
            let stdin = std::fs::read_to_string(format!("{}.stdin", binary.display())).unwrap();
            assert!(stdin.contains("Call the echo tool"));
            let cwd = std::fs::read_to_string(format!("{}.cwd", binary.display())).unwrap();
            assert!(cwd.trim().ends_with("workspace"));
        }

        #[tokio::test]
        async fn a_recorded_failure_ends_with_an_error_and_one_finished() {
            let dir = tempfile::tempdir().unwrap();
            let script = format!(
                "cat > /dev/null; cat {}; exit 1",
                fixture_path("claude-code/not_signed_in.jsonl")
            );
            let binary = fake_agent(dir.path(), "claude", &script);
            let events = collect_events(&provider(binary), request(dir.path())).await;
            let finished = events
                .iter()
                .filter(|event| matches!(event, ChatEvent::Finished { .. }))
                .count();
            assert_eq!(finished, 1);
            assert!(matches!(
                events[events.len() - 2],
                ChatEvent::Error {
                    kind: ErrorKind::NotSignedIn,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn a_cli_that_dies_early_is_explained_with_its_stderr() {
            let dir = tempfile::tempdir().unwrap();
            let binary = fake_agent(
                dir.path(),
                "claude",
                "echo \"error: unknown option '--input-format'\" >&2; exit 1",
            );
            let events = collect_events(&provider(binary), request(dir.path())).await;
            match &events[0] {
                ChatEvent::Error { message, kind } => {
                    assert_eq!(*kind, ErrorKind::Other);
                    assert!(message.contains("unknown option"), "{message}");
                }
                other => panic!("expected an error, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn cancelling_stops_the_cli_and_reports_cancelled() {
            let dir = tempfile::tempdir().unwrap();
            // Announces the session, then hangs like a slow model call.
            let script = "echo $$ > \"$0.pid\"; echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\",\"mcp_servers\":[]}'; sleep 300";
            let binary = fake_agent(dir.path(), "claude", script);
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
            let started = std::time::Instant::now();
            cancel.cancel();
            assert_eq!(
                receiver.recv().await,
                Some(ChatEvent::Finished {
                    stop_reason: StopReason::Cancelled
                })
            );
            task.await.unwrap();
            assert!(started.elapsed() < std::time::Duration::from_secs(10));

            let pid: i32 = std::fs::read_to_string(format!("{}.pid", binary.display()))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let alive = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok();
            assert!(!alive, "the CLI process survived cancellation");
        }

        #[tokio::test]
        async fn dropping_the_receiver_stops_the_cli() {
            let dir = tempfile::tempdir().unwrap();
            let binary = fake_agent(dir.path(), "claude", "sleep 300");
            let provider = provider(binary);
            let (events, receiver) = event_channel();
            drop(receiver);
            let started = std::time::Instant::now();
            provider
                .send(request(dir.path()), events, CancellationToken::new())
                .await;
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
        }
    }

    fn missing() -> ClaudeCode {
        ClaudeCode::with_options(ClaudeCodeOptions {
            binary: Some(PathBuf::from("/nonexistent/claude")),
            ..ClaudeCodeOptions::default()
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
        use crate::error::ErrorKind;
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
                message: "claude is not installed".to_owned(),
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

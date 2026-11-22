//! The Codex provider: chat through OpenAI's `codex` CLI and the user's own
//! ChatGPT plan.
//!
//! UNVERIFIED: the Codex CLI was not installed when this was written, so the
//! provider has never been run against it. Flags, configuration keys and the
//! event stream follow OpenAI's published documentation for `codex exec`
//! (non-interactive mode, the configuration reference and the MCP guide).
//! Expect to adjust details the first time it meets the real CLI.
//!
//! The CLI is run once per turn with `codex exec --json`. Everything that
//! scopes the run is passed on the command line, so no file of the user's is
//! touched: the Moshionly MCP server and its approval mode as `-c` overrides,
//! a read-only sandbox, and the command and web search tools switched off.
//! Moshionly never reads the CLI's credentials: signing in is done by the
//! user, in the CLI.

mod stream;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use self::stream::CodexStream;
use super::discover::{PROBE_TIMEOUT, command_for, find_binary, read_version};
use super::files::Attachments;
use super::process::{CliCommand, run_streaming, run_to_string};
use super::prompt::{cli_prompt, last_user_message, prepare_working_dir, validate_mcp_name};
use super::stopped;
use crate::chat::ChatRequest;
use crate::error::{AiError, ErrorKind};
use crate::event::{EventSender, StopReason};
use crate::provider::{
    Capabilities, ChatProvider, ModelInfo, ProviderInfo, ProviderKind, Readiness, ReadyState,
    ToolTransport,
};

/// The provider id used in settings and on the command line.
pub const CODEX_ID: &str = "codex";

/// The model entry that leaves the choice to the CLI's own configuration.
const DEFAULT_MODEL: &str = "default";

/// Settings for the Codex provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexOptions {
    /// Where the `codex` executable is, when the user chose one. Otherwise
    /// it is looked up on `PATH` and in the usual install locations.
    pub binary: Option<PathBuf>,
    /// Run without the user's own `config.toml`: none of their MCP servers,
    /// profiles or instructions, only Moshionly's prompt and tools. On by
    /// default. Signing in is not affected.
    ///
    /// Turn it off for a setup that depends on that file, such as a custom
    /// model provider, at the price of the user's coding setup leaking into
    /// the design chat.
    pub isolate_settings: bool,
}

impl Default for CodexOptions {
    fn default() -> Self {
        Self {
            binary: None,
            isolate_settings: true,
        }
    }
}

/// Chat through the user's Codex installation.
#[derive(Debug, Clone, Default)]
pub struct Codex {
    options: CodexOptions,
}

impl Codex {
    /// A provider with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// A provider with the given options.
    pub fn with_options(options: CodexOptions) -> Self {
        Self { options }
    }

    fn binary(&self) -> Result<PathBuf, AiError> {
        find_binary(&["codex"], self.options.binary.as_deref()).ok_or_else(|| {
            AiError::NotInstalled {
                program: "codex".to_owned(),
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
        let (_, newest) = last_user_message(request)?;
        // Kept until the end of the turn; the files go when it is dropped.
        let images = Attachments::save(&newest.images)?;
        let command = build_command(
            &binary,
            request,
            &self.options,
            &working_dir,
            images.paths(),
        )?;
        let mut stream = CodexStream::new();
        let exit = run_streaming(command, stopped(cancel, events), |line| {
            stream.push_line(line, events)
        })
        .await?;
        stream.finish(exit)
    }
}

#[async_trait]
impl ChatProvider for Codex {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: CODEX_ID.to_owned(),
            name: "Codex".to_owned(),
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
            verified: false,
        }
    }

    /// The CLI has no command that lists models. These are the ones its
    /// documentation recommended in November 2026; any other model id the
    /// account can use works too.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        Ok(vec![
            ModelInfo::new(DEFAULT_MODEL, "Default (as set in Codex)"),
            ModelInfo::new("gpt-6.2-sol", "GPT-6.2 Sol"),
            ModelInfo::new("gpt-6.1-sol", "GPT-6.1 Sol"),
            ModelInfo::new("gpt-6.1-terra", "GPT-6.1 Terra"),
            ModelInfo::new("gpt-6-luna", "GPT-6 Luna"),
        ])
    }

    async fn check_ready(&self) -> Readiness {
        let binary = match self.binary() {
            Ok(binary) => binary,
            Err(_) => {
                return Readiness::not_ready(
                    ReadyState::NotInstalled,
                    "Codex is not installed. Install it from https://developers.openai.com/codex/cli, \
                     then run `codex login` in a terminal.",
                );
            }
        };
        let version = read_version(&binary).await;
        let label = match &version {
            Some(version) => format!("Codex {version}"),
            None => "Codex".to_owned(),
        };
        let readiness = match signed_in(&binary).await {
            Some(true) => Readiness::ready(format!("{label} is installed and signed in.")),
            Some(false) => Readiness::not_ready(
                ReadyState::NotSignedIn,
                format!("{label} is installed but not signed in. Run `codex login` in a terminal."),
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

/// Asks the CLI whether it is signed in. `codex login status` answers with
/// its exit code, which is all that is looked at; its output can name the
/// account and is not read.
async fn signed_in(binary: &Path) -> Option<bool> {
    let command = command_for(binary).args(["login", "status"]);
    let (code, _, _) = run_to_string(command, PROBE_TIMEOUT).await.ok()?;
    code.map(|code| code == 0)
}

/// The command line for one turn.
fn build_command(
    binary: &Path,
    request: &ChatRequest,
    options: &CodexOptions,
    working_dir: &Path,
    images: &[PathBuf],
) -> Result<CliCommand, AiError> {
    let mut command = command_for(binary).args([
        "exec",
        "--json",
        // The workspace is an app-owned folder, not a repository.
        "--skip-git-repo-check",
        // Whatever the agent might still run can read but never write, and
        // has no network.
        "--sandbox",
        "read-only",
    ]);
    if options.isolate_settings {
        command = command.arg("--ignore-user-config");
    }
    command = command.arg("--cd").arg(working_dir);
    for setting in config_overrides(request)? {
        command = command.args(["-c".to_owned(), setting]);
    }
    if let Some(model) = request.model.as_deref()
        && !model.is_empty()
        && model != DEFAULT_MODEL
    {
        command = command.args(["--model", model]);
    }
    if let Some(session_id) = request.session_id.as_deref() {
        // The id is a bare argument here, so it must not look like a flag.
        if session_id.is_empty() || session_id.starts_with('-') {
            return Err(AiError::from_kind(
                ErrorKind::InvalidRequest,
                "the session id is not one that Codex produced",
            ));
        }
        command = command.args(["resume", session_id]);
    }
    // `-` makes the CLI read the prompt from stdin, which keeps it off the
    // command line where other processes could see it.
    command = command.arg("-");
    // Images go last: the flag takes several values, and nothing may follow
    // that could be mistaken for one.
    for image in images {
        command = command.arg("--image").arg(image);
    }
    command.cwd = Some(working_dir.to_path_buf());
    command.stdin = Some(cli_prompt(request)?);
    Ok(command)
}

/// The `-c key=value` settings for one turn. Values are TOML.
fn config_overrides(request: &ChatRequest) -> Result<Vec<String>, AiError> {
    let mut settings = vec![
        // Nobody is there to answer an approval prompt.
        r#"approval_policy="never""#.to_owned(),
        // No command execution and no web search in a design chat.
        "features.shell_tool=false".to_owned(),
        r#"web_search="disabled""#.to_owned(),
    ];
    if let Some(mcp) = &request.mcp {
        validate_mcp_name(&mcp.name)?;
        let server = format!("mcp_servers.{}", mcp.name);
        settings.push(format!("{server}.url={}", toml_string(&mcp.url)));
        // Our tools run without asking...
        settings.push(format!(r#"{server}.default_tools_approval_mode="approve""#));
        // ...and a turn without them is not worth starting.
        settings.push(format!("{server}.required=true"));
    }
    if let Some(system_prompt) = request.system_prompt.as_deref()
        && !system_prompt.trim().is_empty()
    {
        settings.push(format!(
            "developer_instructions={}",
            toml_string(system_prompt)
        ));
    }
    Ok(settings)
}

/// `text` as a TOML basic string, quoted and escaped.
fn toml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatMessage, ImageData, McpEndpoint};

    fn command(request: &ChatRequest) -> CliCommand {
        build_command(
            Path::new("/usr/local/bin/codex"),
            request,
            &CodexOptions::default(),
            Path::new("/tmp/workspace"),
            &[],
        )
        .unwrap()
    }

    /// The value following `flag` on the command line.
    fn value_of(args: &[String], flag: &str) -> Option<String> {
        let index = args.iter().position(|arg| arg == flag)?;
        args.get(index + 1).cloned()
    }

    /// Every `-c` setting on the command line.
    fn config_settings(args: &[String]) -> Vec<String> {
        args.windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| pair[1].clone())
            .collect()
    }

    #[test]
    fn runs_exec_with_json_events_in_a_read_only_sandbox() {
        let built = command(&ChatRequest::from_prompt("hi"));
        let args = built.args_lossy();
        assert_eq!(args[0], "exec");
        assert!(args.contains(&"--json".to_owned()));
        assert!(args.contains(&"--skip-git-repo-check".to_owned()));
        assert_eq!(value_of(&args, "--sandbox").unwrap(), "read-only");
        assert_eq!(value_of(&args, "--cd").unwrap(), "/tmp/workspace");
        assert_eq!(built.cwd.as_deref(), Some(Path::new("/tmp/workspace")));
        assert!(!args.iter().any(|arg| arg.contains("bypass")));
    }

    #[test]
    fn the_clis_own_tools_are_switched_off_and_nothing_can_prompt() {
        let settings = config_settings(&command(&ChatRequest::from_prompt("hi")).args_lossy());
        assert!(settings.contains(&r#"approval_policy="never""#.to_owned()));
        assert!(settings.contains(&"features.shell_tool=false".to_owned()));
        assert!(settings.contains(&r#"web_search="disabled""#.to_owned()));
        assert!(
            !settings
                .iter()
                .any(|setting| setting.contains("mcp_servers"))
        );
    }

    #[test]
    fn the_moshionly_server_is_attached_approved_and_required() {
        let request = ChatRequest {
            mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
            ..ChatRequest::from_prompt("hi")
        };
        let settings = config_settings(&command(&request).args_lossy());
        for expected in [
            r#"mcp_servers.moshionly.url="http://127.0.0.1:47821/mcp""#,
            r#"mcp_servers.moshionly.default_tools_approval_mode="approve""#,
            "mcp_servers.moshionly.required=true",
        ] {
            assert!(settings.contains(&expected.to_owned()), "{expected}");
        }
    }

    #[test]
    fn an_unsafe_server_name_is_rejected() {
        let request = ChatRequest {
            mcp: Some(McpEndpoint::new("a.b", "http://127.0.0.1:1/mcp")),
            ..ChatRequest::from_prompt("hi")
        };
        assert!(config_overrides(&request).is_err());
    }

    #[test]
    fn user_settings_are_left_out_unless_asked_for() {
        let request = ChatRequest::from_prompt("hi");
        assert!(
            command(&request)
                .args_lossy()
                .contains(&"--ignore-user-config".to_owned())
        );
        let options = CodexOptions {
            isolate_settings: false,
            ..CodexOptions::default()
        };
        let args = build_command(
            Path::new("codex"),
            &request,
            &options,
            Path::new("/tmp"),
            &[],
        )
        .unwrap()
        .args_lossy();
        assert!(!args.contains(&"--ignore-user-config".to_owned()));
    }

    #[test]
    fn the_system_prompt_is_passed_as_an_escaped_toml_string() {
        let request = ChatRequest {
            system_prompt: Some("Say \"hi\".\nUse C:\\fonts.".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let settings = config_settings(&command(&request).args_lossy());
        assert!(
            settings
                .contains(&r#"developer_instructions="Say \"hi\".\nUse C:\\fonts.""#.to_owned())
        );

        let request = ChatRequest {
            system_prompt: Some("  ".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let settings = config_settings(&command(&request).args_lossy());
        assert!(
            !settings
                .iter()
                .any(|s| s.starts_with("developer_instructions"))
        );
    }

    #[test]
    fn toml_strings_escape_control_characters() {
        assert_eq!(toml_string("plain"), r#""plain""#);
        assert_eq!(toml_string("tab\there"), r#""tab\there""#);
        assert_eq!(toml_string("bell\u{7}"), r#""bell\u0007""#);
        assert_eq!(toml_string("ünï ✓"), "\"ünï ✓\"");
    }

    #[test]
    fn the_prompt_goes_to_stdin() {
        let built = command(&ChatRequest::from_prompt("Fade the card in"));
        assert_eq!(built.stdin.as_deref(), Some("Fade the card in"));
        let args = built.args_lossy();
        assert_eq!(args.last().unwrap(), "-");
        assert!(!args.iter().any(|arg| arg.contains("Fade")));
    }

    #[test]
    fn a_model_is_passed_unless_it_is_the_default() {
        let request = ChatRequest {
            model: Some("gpt-6.1-sol".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        assert_eq!(
            value_of(&command(&request).args_lossy(), "--model").unwrap(),
            "gpt-6.1-sol"
        );
        let request = ChatRequest {
            model: Some("default".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        assert!(
            !command(&request)
                .args_lossy()
                .contains(&"--model".to_owned())
        );
    }

    #[test]
    fn resuming_uses_the_resume_subcommand_after_the_options() {
        let request = ChatRequest {
            session_id: Some("0199a213-81c0-7800-8aa1-bbab2a035a53".to_owned()),
            messages: vec![
                ChatMessage::user("first"),
                ChatMessage::assistant("ok"),
                ChatMessage::user("second"),
            ],
            ..ChatRequest::default()
        };
        let built = command(&request);
        let args = built.args_lossy();
        let resume = args.iter().position(|arg| arg == "resume").unwrap();
        assert_eq!(args[resume + 1], "0199a213-81c0-7800-8aa1-bbab2a035a53");
        assert_eq!(args[resume + 2], "-");
        assert!(args.iter().position(|arg| arg == "--sandbox").unwrap() < resume);
        // The CLI already holds the conversation.
        assert_eq!(built.stdin.as_deref(), Some("second"));
    }

    #[test]
    fn a_session_id_that_looks_like_a_flag_is_rejected() {
        let request = ChatRequest {
            session_id: Some("--last".to_owned()),
            ..ChatRequest::from_prompt("hi")
        };
        let error = build_command(
            Path::new("codex"),
            &request,
            &CodexOptions::default(),
            Path::new("/tmp"),
            &[],
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidRequest);
    }

    #[test]
    fn images_come_after_the_prompt_marker() {
        let images = [PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")];
        let args = build_command(
            Path::new("codex"),
            &ChatRequest::from_prompt("match this"),
            &CodexOptions::default(),
            Path::new("/tmp"),
            &images,
        )
        .unwrap()
        .args_lossy();
        let tail = &args[args.len() - 5..];
        assert_eq!(
            tail,
            ["-", "--image", "/tmp/a.png", "--image", "/tmp/b.png"]
        );
    }

    #[cfg(unix)]
    mod with_a_stand_in_cli {
        use super::*;
        use crate::cli::test_support::{collect_events, fake_agent, fixture_path};
        use crate::event::ChatEvent;

        fn provider(binary: PathBuf) -> Codex {
            Codex::with_options(CodexOptions {
                binary: Some(binary),
                ..CodexOptions::default()
            })
        }

        fn request(root: &Path) -> ChatRequest {
            ChatRequest {
                mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
                working_dir: Some(root.join("workspace")),
                messages: vec![
                    ChatMessage::user("Call the echo tool").with_image(ImageData::png(vec![1, 2])),
                ],
                ..ChatRequest::default()
            }
        }

        #[tokio::test]
        async fn a_whole_turn_with_a_tool_call_comes_through_in_order() {
            let dir = tempfile::tempdir().unwrap();
            // Records its stdin and the image it was given, then plays the
            // documented stream.
            let script = format!(
                "cat > \"$0.stdin\"; for last; do :; done; cp \"$last\" \"$0.image\"; cat {}",
                fixture_path("codex/documented_tool_call.jsonl")
            );
            let binary = fake_agent(dir.path(), "codex", &script);
            let events = collect_events(&provider(binary.clone()), request(dir.path())).await;

            assert!(matches!(events[0], ChatEvent::SessionStarted { .. }));
            assert!(
                matches!(&events[2], ChatEvent::ToolCallStarted { name, .. } if name == "echo")
            );
            assert!(matches!(
                events[3],
                ChatEvent::ToolCallFinished { ok: true, .. }
            ));
            assert_eq!(
                events.last(),
                Some(&ChatEvent::Finished {
                    stop_reason: StopReason::EndTurn
                })
            );

            let stdin = std::fs::read_to_string(format!("{}.stdin", binary.display())).unwrap();
            assert_eq!(stdin, "Call the echo tool");
            // The image existed while the CLI ran.
            let image = std::fs::read(format!("{}.image", binary.display())).unwrap();
            assert_eq!(image, [1, 2]);
        }

        #[tokio::test]
        async fn readiness_follows_the_exit_code_of_login_status() {
            let dir = tempfile::tempdir().unwrap();
            let script = r#"case "$1" in
  --version) echo "codex-cli 0.159.0" ;;
  login) echo "Not logged in"; exit 1 ;;
esac"#;
            let binary = fake_agent(dir.path(), "codex", script);
            let readiness = provider(binary).check_ready().await;
            assert_eq!(readiness.state, ReadyState::NotSignedIn);
            assert_eq!(readiness.version.as_deref(), Some("0.159.0"));

            let script = r#"case "$1" in
  --version) echo "codex-cli 0.159.0" ;;
  login) echo "Logged in using ChatGPT" ;;
esac"#;
            let binary = fake_agent(dir.path(), "codex-signed-in", script);
            let readiness = provider(binary).check_ready().await;
            assert_eq!(readiness.state, ReadyState::Ready);
        }

        #[tokio::test]
        async fn a_cli_that_dies_early_is_explained_with_its_stderr() {
            let dir = tempfile::tempdir().unwrap();
            let binary = fake_agent(
                dir.path(),
                "codex",
                "echo \"error: unexpected argument '--json' found\" >&2; exit 2",
            );
            let events = collect_events(&provider(binary), request(dir.path())).await;
            match &events[0] {
                ChatEvent::Error { message, kind } => {
                    assert_eq!(*kind, ErrorKind::Other);
                    assert!(message.contains("unexpected argument"), "{message}");
                }
                other => panic!("expected an error, got {other:?}"),
            }
        }
    }

    fn missing() -> Codex {
        Codex::with_options(CodexOptions {
            binary: Some(PathBuf::from("/nonexistent/codex")),
            ..CodexOptions::default()
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
                message: "codex is not installed".to_owned(),
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

//! Try a provider from the command line, without the app.
//!
//! ```text
//! cargo run -p moshionly-ai --example chat -- --list
//! cargo run -p moshionly-ai --example chat -- --provider claude-code "Reply with the single word ready"
//! cargo run -p moshionly-ai --example chat -- --provider ollama --echo-tool "Echo the word hello"
//! cargo run -p moshionly-ai --example chat -- --help
//! ```
//!
//! It uses the same settings file and the same keychain entries as the app,
//! so what works here works there. `--list` only checks readiness and costs
//! nothing. Sending a prompt to a subscription provider spends a little of
//! that subscription; sending one to an API provider is billed to the key.

#[path = "../tests/support/echo_mcp.rs"]
mod echo_mcp;

use std::io::{BufRead as _, IsTerminal as _};
use std::process::ExitCode;
use std::sync::Arc;

use async_trait::async_trait;
use echo_mcp::{ECHO_DESCRIPTION, ECHO_TOOL, EchoMcpServer, echo_input_schema, echo_reply};
use moshionly_ai::{
    AiSettings, ApiKey, CancellationToken, ChatEvent, ChatProvider, ChatRequest, KeyStore,
    KeychainStore, McpEndpoint, MemoryKeyStore, ProviderKind, ProviderRegistry, ProviderStatus,
    StopReason, ToolExecutor, ToolOutput, ToolSpec, ToolTransport, choose_provider, event_channel,
};
use serde_json::Value;

const USAGE: &str = "\
Try a Moshionly chat provider from the command line.

Usage:
  chat --list [--json]
  chat [--provider <id>] [options] <prompt>
  chat --models [--provider <id>]
  chat --set-key <id>          (the key is read from standard input)
  chat --forget-key <id>

Options:
  --provider <id>   The provider to use. Without it: the default from the
                    settings, or else the first provider that is ready.
  --model <id>      The model to use, as listed by --models.
  --system <text>   A system prompt.
  --session <id>    Continue an earlier session, for providers that keep one.
  --mcp <url>       Attach this MCP server under the name `moshionly`
                    (subscription providers).
  --echo-tool       Offer the model one harmless tool, `echo`, to check that
                    tool calls work. API providers call it in-process;
                    subscription providers reach it through a local MCP
                    server started for the run.
  --base-url <url>  Use this server address for the run (API providers).
  --key-env <VAR>   Take the API key from this environment variable for the
                    run, instead of the keychain. Needs --provider.
  --json            Print each event, or the --list rows, as JSON.
  --help            Show this text.
";

/// What the command line asked for.
#[derive(Default)]
struct Args {
    list: bool,
    models: bool,
    json: bool,
    echo_tool: bool,
    provider: Option<String>,
    model: Option<String>,
    system: Option<String>,
    session: Option<String>,
    mcp: Option<String>,
    base_url: Option<String>,
    key_env: Option<String>,
    set_key: Option<String>,
    forget_key: Option<String>,
    prompt: Vec<String>,
}

/// Reads the command line. `Ok(None)` means help was asked for.
fn parse_args(mut raw: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut args = Args::default();
    while let Some(arg) = raw.next() {
        let mut value = |name: &str| raw.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--list" => args.list = true,
            "--models" => args.models = true,
            "--json" => args.json = true,
            "--echo-tool" => args.echo_tool = true,
            "--provider" => args.provider = Some(value("--provider")?),
            "--model" => args.model = Some(value("--model")?),
            "--system" => args.system = Some(value("--system")?),
            "--session" => args.session = Some(value("--session")?),
            "--mcp" => args.mcp = Some(value("--mcp")?),
            "--base-url" => args.base_url = Some(value("--base-url")?),
            "--key-env" => args.key_env = Some(value("--key-env")?),
            "--set-key" => args.set_key = Some(value("--set-key")?),
            "--forget-key" => args.forget_key = Some(value("--forget-key")?),
            // Everything after `--` is the prompt, whatever it looks like.
            "--" => args.prompt.extend(raw.by_ref()),
            option if option.starts_with("--") => {
                return Err(format!("unknown option {option}; try --help"));
            }
            _ => args.prompt.push(arg),
        }
    }
    Ok(Some(args))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

async fn run(args: Args) -> Result<ExitCode, String> {
    let registry = registry(&args)?;

    if let Some(id) = &args.set_key {
        let key = read_key_from_stdin()?;
        registry
            .set_key(id, key)
            .await
            .map_err(|error| error.to_string())?;
        eprintln!("Stored the key for {id} in the keychain.");
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(id) = &args.forget_key {
        registry
            .delete_key(id)
            .await
            .map_err(|error| error.to_string())?;
        eprintln!("Removed the key for {id} from the keychain.");
        return Ok(ExitCode::SUCCESS);
    }
    if args.list {
        print_statuses(&registry.statuses().await, args.json);
        return Ok(ExitCode::SUCCESS);
    }

    let id = match &args.provider {
        Some(id) => id.clone(),
        None => {
            let statuses = registry.statuses().await;
            let chosen = choose_provider(registry.settings(), &statuses).ok_or(
                "no provider is ready; run with --list to see why, or name one with --provider",
            )?;
            eprintln!("Using {} ({}).", chosen.info.name, chosen.info.id);
            chosen.info.id.clone()
        }
    };
    let provider = registry
        .provider(&id)
        .await
        .map_err(|error| error.to_string())?;

    if args.models {
        let models = provider
            .list_models()
            .await
            .map_err(|error| error.to_string())?;
        for (index, model) in models.iter().enumerate() {
            let default = if index == 0 { "  (default)" } else { "" };
            println!("{:<40} {}{default}", model.id, model.name);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let prompt = args.prompt.join(" ");
    if prompt.trim().is_empty() {
        return Err("give a prompt to send, or --list; try --help".to_owned());
    }
    chat(provider, &args, prompt).await
}

/// The registry for this run: the app's settings and keychain, with the
/// command line's overrides on top.
fn registry(args: &Args) -> Result<ProviderRegistry, String> {
    let mut settings = AiSettings::load().unwrap_or_else(|error| {
        eprintln!("warning: {error}; using default settings");
        AiSettings::default()
    });
    if let Some(base_url) = &args.base_url {
        let id = args
            .provider
            .as_deref()
            .ok_or("--base-url needs --provider")?;
        settings.provider_mut(id).base_url = Some(base_url.clone());
    }
    let keys: Arc<dyn KeyStore> = match &args.key_env {
        Some(variable) => {
            let id = args
                .provider
                .as_deref()
                .ok_or("--key-env needs --provider")?;
            let key = std::env::var(variable)
                .ok()
                .map(ApiKey::new)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| format!("the environment variable {variable} is not set"))?;
            Arc::new(MemoryKeyStore::new().with_key(id, key))
        }
        None => Arc::new(KeychainStore::new()),
    };
    Ok(ProviderRegistry::new(settings, keys))
}

/// Reads a key from standard input, so that it never appears in the shell
/// history or in the process list.
fn read_key_from_stdin() -> Result<ApiKey, String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprintln!("Paste the key and press Enter. It will be visible while you do.");
    }
    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .map_err(|error| format!("could not read the key: {error}"))?;
    let key = ApiKey::new(line);
    if key.is_empty() {
        return Err("no key was given".to_owned());
    }
    Ok(key)
}

/// Sends one prompt and prints the events as they arrive.
async fn chat(
    provider: Box<dyn ChatProvider>,
    args: &Args,
    prompt: String,
) -> Result<ExitCode, String> {
    let mut request = ChatRequest {
        system_prompt: args.system.clone(),
        model: args.model.clone(),
        session_id: args.session.clone(),
        mcp: args
            .mcp
            .as_ref()
            .map(|url| McpEndpoint::new("moshionly", url)),
        ..ChatRequest::from_prompt(prompt)
    };

    // Kept until the turn is over; the server stops when it is dropped.
    let mut echo_server = None;
    if args.echo_tool {
        request.tools = Some(Arc::new(EchoTool));
        let needs_server = provider.capabilities().tools == ToolTransport::Mcp;
        if needs_server && request.mcp.is_none() {
            let server = EchoMcpServer::start()
                .await
                .map_err(|error| format!("could not start the echo server: {error}"))?;
            request.mcp = Some(McpEndpoint::new("moshionly", server.url()));
            echo_server = Some(server);
        }
    }

    let cancel = CancellationToken::new();
    let (events, mut receiver) = event_channel();
    let turn = tokio::spawn({
        let cancel = cancel.clone();
        async move { provider.send(request, events, cancel).await }
    });

    let mut answer = String::new();
    let mut stop_reason = None;
    let mut interrupted = false;
    loop {
        tokio::select! {
            event = receiver.recv() => {
                let Some(event) = event else { break };
                match &event {
                    ChatEvent::TextDelta { text } => answer.push_str(text),
                    ChatEvent::Finished { stop_reason: reason } => stop_reason = Some(*reason),
                    _ => {}
                }
                print_event(&event, args.json);
            }
            // Ctrl-C stops the turn through the provider, which also stops
            // an agent CLI and everything it started.
            _ = tokio::signal::ctrl_c(), if !interrupted => {
                interrupted = true;
                eprintln!("Stopping...");
                cancel.cancel();
            }
        }
    }
    let _ = turn.await;
    drop(echo_server);

    if !args.json && !answer.trim().is_empty() {
        println!("\n{}", answer.trim_end());
    }
    Ok(match stop_reason {
        Some(StopReason::Error) | None => ExitCode::FAILURE,
        Some(_) => ExitCode::SUCCESS,
    })
}

/// One line per event.
fn print_event(event: &ChatEvent, json: bool) {
    if json {
        match serde_json::to_string(event) {
            Ok(line) => println!("{line}"),
            Err(error) => eprintln!("could not print an event: {error}"),
        }
        return;
    }
    match event {
        ChatEvent::SessionStarted { session_id } => println!("session   {session_id}"),
        ChatEvent::TextDelta { text } => println!("text      {text:?}"),
        ChatEvent::ThinkingDelta { text } => println!("thinking  {text:?}"),
        ChatEvent::ToolCallStarted { id, name, input } => {
            println!("tool      {name} {input}  [{id}]");
        }
        ChatEvent::ToolCallFinished { id, ok, summary } => {
            let outcome = if *ok { "ok" } else { "failed" };
            println!("result    {outcome}: {summary}  [{id}]");
        }
        ChatEvent::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
        } => {
            let cost = cost_usd.map_or_else(String::new, |cost| format!(", ${cost:.4}"));
            println!("usage     {input_tokens} in, {output_tokens} out{cost}");
        }
        ChatEvent::Error { message, kind } => {
            println!("error     [{}] {message}", label(kind));
        }
        ChatEvent::Finished { stop_reason } => println!("finished  {}", label(stop_reason)),
    }
}

/// The providers and whether each can be used, as a table or as JSON.
fn print_statuses(statuses: &[ProviderStatus], json: bool) {
    if json {
        match serde_json::to_string_pretty(statuses) {
            Ok(text) => println!("{text}"),
            Err(error) => eprintln!("could not print the list: {error}"),
        }
        return;
    }
    for status in statuses {
        let kind = match status.info.kind {
            ProviderKind::Subscription => "subscription",
            ProviderKind::Api => "api",
        };
        let mut notes = String::new();
        if !status.enabled {
            notes.push_str(" [disabled]");
        }
        if !status.capabilities.verified {
            notes.push_str(" [unverified]");
        }
        println!(
            "{:<13} {:<13} {:<14} {}{notes}",
            status.info.id,
            kind,
            label(&status.readiness.state),
            status.readiness.message,
        );
    }
    if statuses.iter().any(|status| !status.capabilities.verified) {
        println!(
            "\n[unverified] has not been run against the real CLI or service yet; \
             see the crate README."
        );
    }
}

/// The name a value serialises to, with spaces: `not_signed_in` becomes
/// `not signed in`.
fn label(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name.replace('_', " "),
        _ => "?".to_owned(),
    }
}

/// The `echo` tool, for API providers. Subscription providers reach the
/// same tool through [`EchoMcpServer`].
struct EchoTool;

#[async_trait]
impl ToolExecutor for EchoTool {
    fn tools(&self) -> Vec<ToolSpec> {
        vec![ToolSpec::new(
            ECHO_TOOL,
            ECHO_DESCRIPTION,
            echo_input_schema(),
        )]
    }

    async fn call(&self, name: &str, input: Value) -> ToolOutput {
        if name == ECHO_TOOL {
            ToolOutput::text(echo_reply(&input))
        } else {
            ToolOutput::error(format!("unknown tool: {name}"))
        }
    }
}

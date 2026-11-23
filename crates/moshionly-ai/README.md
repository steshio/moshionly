# moshionly-ai

One chat interface over many AI providers, so a Moshionly user can work with whichever AI they already pay for. This crate has no user interface and does not depend on the other Moshionly crates.

There are two kinds of provider behind the `ChatProvider` trait:

- **Subscription providers** run an agent CLI that the user has installed and signed in to (Claude Code, Cursor Agent, Codex, Gemini CLI). Moshionly starts the CLI non-interactively, attaches the Moshionly MCP server and reads its output. It never reads, copies or stores the CLI's credentials; it only runs the binary.
- **API providers** call a model API with the user's key (Anthropic, or anything that speaks the OpenAI Chat Completions format, including models on the user's own machine). The tool loop runs in this crate, against a `ToolExecutor` that the app supplies.

One turn is one call to `ChatProvider::send`. The answer arrives as a stream of `ChatEvent`s that always ends with exactly one `Finished`.

## Using it from the app

```rust
use moshionly_ai::{
    AiSettings, CancellationToken, ChatEvent, ChatRequest, McpEndpoint, ProviderRegistry,
    choose_provider, event_channel,
};

// The user's choices, and their keys in the OS keychain.
let registry = ProviderRegistry::with_keychain(AiSettings::load().unwrap_or_default());

// For a settings screen: every provider and whether it can be used now.
let statuses = registry.statuses().await;

// For the chat: the user's default, or else the first provider that is ready.
let Some(chosen) = choose_provider(registry.settings(), &statuses) else {
    return Ok(()); // nothing is ready; send the user to the settings screen
};
let provider = registry.provider(&chosen.info.id).await?;

let request = ChatRequest {
    system_prompt: Some("You edit motion designs.".to_owned()),
    // Subscription providers reach the tools through the app's MCP server...
    mcp: Some(McpEndpoint::new("moshionly", "http://127.0.0.1:47821/mcp")),
    // ...and API providers call them in-process.
    tools: Some(tools),
    ..ChatRequest::from_prompt("Fade the title in over half a second")
};

let (events, mut receiver) = event_channel();
let cancel = CancellationToken::new(); // cancel() stops the turn
tokio::spawn(async move { provider.send(request, events, cancel).await });

while let Some(event) = receiver.recv().await {
    match event {
        ChatEvent::TextDelta { text } => print!("{text}"),
        ChatEvent::Error { message, .. } => eprintln!("{message}"),
        ChatEvent::Finished { .. } => break,
        _ => {}
    }
}
```

The same code is compiled as the example in the crate documentation, so it stays correct.

What to know when wiring it in:

- Fill in both `mcp` and `tools` on every request. Each provider uses the one it needs (`Capabilities::tools` says which).
- Leave `ChatRequest::model` as `None` to get the model the user chose in settings, or the provider's default when they chose none.
- Providers with `Capabilities::resume` keep the conversation themselves. Keep the id from `ChatEvent::SessionStarted` and pass it as `ChatRequest::session_id` on the next turn; only the newest user message is then sent. For the others, send the whole conversation every time.
- Cancelling the token, or dropping the receiver, stops the turn promptly. An agent CLI is terminated together with everything it started; an HTTP stream is closed.
- Agent CLIs run in `default_working_dir()`, an empty folder owned by the app, unless `ChatRequest::working_dir` names another. Keep it stable: agents find a session to resume by the folder it started in.

## Providers

| Id | Provider | Kind | What the user needs | Checked against the real thing? |
|---|---|---|---|---|
| `claude-code` | Claude Code | subscription | the `claude` CLI, signed in | Yes, CLI 2.1.325 |
| `cursor-agent` | Cursor Agent | subscription | the `cursor-agent` CLI, signed in | Partly: everything up to a signed-in turn |
| `codex` | Codex | subscription | the `codex` CLI, signed in | No, written from documentation |
| `gemini-cli` | Gemini CLI | subscription | the `gemini` CLI, signed in | No, written from documentation |
| `anthropic` | Anthropic | API | an API key | No, a local stand-in server only |
| `openai` | OpenAI | API | an API key | No, a local stand-in server only |
| `openrouter` | OpenRouter | API | an API key | No, a local stand-in server only |
| `gemini` | Google Gemini | API | an API key | No, a local stand-in server only |
| `groq` | Groq | API | an API key | No, a local stand-in server only |
| `mistral` | Mistral | API | an API key | No, a local stand-in server only |
| `xai` | xAI | API | an API key | No, a local stand-in server only |
| `ollama` | Ollama | API | Ollama running on this machine | No, a local stand-in server only |
| `lm-studio` | LM Studio | API | LM Studio's local server running | No, a local stand-in server only |
| `custom` | Custom | API | a server address, and a key if the server wants one | No, a local stand-in server only |

`Capabilities::verified` carries the last column into the app, so a settings screen can label providers that nobody has run for real yet. See [What has been verified](#what-has-been-verified) for the details.

### Claude Code (`claude-code`)

Runs `claude --print` once per turn with stream-JSON in both directions and partial messages. The run is scoped on the command line alone:

- `--strict-mcp-config` with an inline `--mcp-config` attaches the Moshionly MCP server and none of the user's own.
- `--allowedTools mcp__<server>` lets that server's tools run without asking; `--permission-mode dontAsk` refuses everything else instead of showing a prompt nobody could answer.
- `--tools ""` switches off the CLI's own file, shell and web tools, so a chat in a design app cannot touch the user's disk.
- `--setting-sources ""` leaves out the user's coding setup (instructions, hooks, plugins, skills). Set `isolate_settings` to `false` for a setup that signs in through those settings.

The system prompt goes in `--append-system-prompt`, the model in `--model`, a session to continue in `--resume`. The message is written to the CLI's standard input, which keeps it off the process list and lets it carry images. Readiness comes from `claude --version` and `claude auth status`, of which only the signed-in flag is read. The model list is the CLI's documented aliases (`opus`, `sonnet`, `haiku`, `fable`); a full model name works too.

### Cursor Agent (`cursor-agent`)

Runs `cursor-agent --print --output-format stream-json --stream-partial-output`. The CLI has no flags for "only this MCP server" or "no built-in tools", so before each turn the provider writes three files into the app-owned workspace, and nothing anywhere else:

- `.cursor/mcp.json` names the Moshionly MCP server.
- `.cursor/cli.json` allows that server's tools and denies shell commands, file writes, web fetches and (unless `allow_file_reads` is set) file reads.
- `.cursor/rules/moshionly.mdc` carries the system prompt, because the CLI has no flag for one.

`--trust` and `--approve-mcps` accept that workspace without a prompt, and `--sandbox disabled` makes shell commands need an allow rule, which the workspace never gives. Readiness comes from `cursor-agent status --format json`, the model list from `cursor-agent models`.

Limits that come from the CLI: MCP servers in the user's own `~/.cursor/mcp.json` are loaded too (their tools are not on the allow list written here); the prompt is a command-line argument, visible to other processes of the same user while it runs; images cannot be sent.

### Codex (`codex`) — unverified

Runs `codex exec --json` with a read-only sandbox. Everything that scopes the run is a `-c` override on the command line: the Moshionly MCP server and its approval mode, and the shell and web search tools switched off. `--ignore-user-config` leaves the user's own `config.toml` out unless `isolate_settings` is `false`. The prompt goes to standard input; images are saved to temporary files and passed with `--image`.

### Gemini CLI (`gemini-cli`) — unverified

Runs `gemini --output-format stream-json` in headless mode. Before each turn the provider writes `.gemini/settings.json` (the Moshionly MCP server as trusted, the CLI's file, shell, web and memory tools excluded) and `GEMINI.md` (the system prompt) into the app-owned workspace. `--allowed-mcp-server-names` keeps servers from the user's own settings out of the run. The prompt is a command-line argument; images cannot be sent.

### Anthropic (`anthropic`)

Calls the Messages API with streaming (`POST /v1/messages`, API version `2026-10-21`). The default model is `claude-opus-5-5`; the model list comes from `GET /v1/models`. Requests use adaptive thinking with summaries, cache the system prompt and the tools, and allow up to 64,000 output tokens. Tools the model asks for in one step run at the same time and are answered in a single message, images included. A turn stops after 30 rounds of tool calls.

Bad keys, rate limits and overload are reported as their own error kinds. Failures that pass on their own are retried twice with backoff, but only while nothing of the response has reached the user, so text is never shown twice.

### OpenAI-compatible (`openai`, `openrouter`, `gemini`, `groq`, `mistral`, `xai`, `ollama`, `lm-studio`, `custom`)

One provider for every service that speaks Chat Completions (`POST <address>/chat/completions` with streaming and function tools, `GET <address>/models`). Which service it talks to is a preset: an address, whether a key is needed, whether its models take images, and a few model names to prefer. A preset is only data.

Where a model cannot be sent images, an image in a message or a tool result is replaced by a short note saying so. Where it can, an image from a tool follows the tool result as a user message, because the format has no place for images inside a tool result.

`custom` has no address of its own; the user enters one, up to and including the version segment (`https://host/v1`).

## Settings and keys

`AiSettings` is what the user chose. It is stored as `ai-settings.json` in the app's configuration directory (`~/Library/Application Support/Moshionly` on macOS, `~/.config/moshionly` on Linux). Only what differs from the defaults is written:

```json
{
  "default_provider": "claude-code",
  "providers": {
    "claude-code": { "model": "sonnet" },
    "cursor-agent": { "enabled": false },
    "anthropic": { "effort": "high" },
    "ollama": { "base_url": "http://192.168.1.20:11434/v1", "model": "llama3.2", "images": true }
  }
}
```

| Field | Read by | Meaning |
|---|---|---|
| `enabled` | all | Offer the provider in the chat. On unless turned off. |
| `model` | all | The model to use when a request names none. |
| `base_url` | API providers | The server address to use in place of the usual one. Anthropic's is given without `/v1`, the others with it. |
| `binary` | subscription providers | Where the CLI is, when it should not be looked up. |
| `isolate_settings` | `claude-code`, `codex` | Run without the user's own configuration for the agent. On by default. |
| `allow_file_reads` | `cursor-agent` | Let the agent read files. Off by default. |
| `effort` | `anthropic` | `low`, `medium`, `high`, `xhigh` or `max`. |
| `max_tokens` | `anthropic` | The most the model may write in one request. |
| `max_tool_rounds` | API providers | How many rounds of tool calls one turn may take. |
| `images` | OpenAI-compatible | Whether the model in use can be sent images, when the preset's answer is wrong for it. |
| `stream_usage` | OpenAI-compatible | Ask for token counts in the stream. On by default. |

A file with fields this version does not know still loads. A file that is not valid JSON is reported and left untouched, so the user can repair it.

**API keys are never in that file.** They live in the operating system's keychain (Keychain on macOS, Credential Manager on Windows, Secret Service on Linux) under the service name `Moshionly AI`, one entry per provider id. The rules the code keeps:

- The only way to a key is the `KeyStore` trait. `KeychainStore` is the real one; `MemoryKeyStore` replaces it in tests, which therefore never touch the user's keychain.
- `ApiKey` cannot be serialised and prints as `ApiKey([redacted])`. Error messages from outside pass through a scrubber that masks anything shaped like a key.
- A server address that carries a credential (`https://user:password@host` or `...?key=...`) is refused when settings are saved and is never used to build a provider.
- A settings screen can ask whether a key is stored (`ProviderRegistry::has_key`, `ProviderStatus::key_stored`) but is never handed the key back.

On macOS the system asks for permission when an app reads a keychain entry that a differently signed build created. Development builds are re-signed on every build, so expect that prompt after rebuilding; a signed release does not show it.

## Trying a provider without the app

```bash
cargo run -p moshionly-ai --example chat -- --list
cargo run -p moshionly-ai --example chat -- --provider claude-code "Reply with the single word ready"
cargo run -p moshionly-ai --example chat -- --provider ollama --echo-tool "Echo the word hello"
cargo run -p moshionly-ai --example chat -- --help
```

`--list` shows every provider and whether it can be used; it only checks, and costs nothing. A prompt prints one line per event and then the whole answer. The example uses the same settings file and keychain entries as the app.

- `--echo-tool` gives the model one harmless tool, so a tool call can be checked end to end. API providers call it in-process; for subscription providers the example starts a small local MCP server and attaches it.
- `--set-key <id>` stores an API key in the keychain, reading it from standard input so that it stays out of the shell history: `pbpaste | cargo run -p moshionly-ai --example chat -- --set-key anthropic`. `--forget-key <id>` removes it.
- `--key-env <VAR>` takes the key from an environment variable for one run and stores nothing, which is the way to go on a system without a keychain.
- `--model`, `--system`, `--session`, `--mcp <url>` and `--base-url <url>` set the matching parts of the request; `--json` prints events as JSON lines.

Sending a prompt to a subscription provider spends a little of that subscription. Sending one to an API provider is billed to the key.

## What has been verified

Stated plainly, because "it compiles and the tests pass" is not the same as "it works with the real service".

**Run against the real thing**

- **Claude Code**, CLI 2.1.325: readiness, a text reply, a resumed session, an unknown model, a signed-out run, and one turn that attached a local MCP server, read an attached image, called the server's tool and answered. The parser's fixtures in `tests/fixtures/claude-code` are recordings of that CLI with identifiers removed.
- **Cursor Agent**, CLI 2026.08.11, signed out: finding the binary, reading its version and sign-in state, the model list fallback, the command-line flags (the CLI accepts them), and the "not signed in" failure. **A whole turn has never been run**, because no signed-in Cursor account was available. Its stream fixture is built from Cursor's published format, not recorded.
- **The OS keychain**, on macOS: storing, replacing, reading and removing an entry.

**Run against a stand-in only**

- **Anthropic** and the **OpenAI-compatible** provider have only met a local server that plays the documented formats: text streaming, tool round trips with parallel calls and images, errors, retries and cancellation. No API key was available, so no request has reached a real service. The preset addresses and model names are from each service's documentation as of November 2026.

**Written from documentation, never run**

- **Codex** and **Gemini CLI** were not installed. Flags, configuration keys and event formats follow the public documentation, and the fixtures in `tests/fixtures/codex` and `tests/fixtures/gemini` are constructed from it. Expect to adjust details the first time either meets the real CLI. Both fail cleanly with "not installed" when the binary is missing.

To move a provider up this list, run its ignored test or the example against the real thing, replace any constructed fixture with a recording (remove account identifiers first), and set `verified: true` in its `capabilities()`.

## Adding a provider

**A service that speaks the OpenAI format** needs one entry and no code: add an `OpenAiPreset` in `src/api/openai/presets.rs` and list it in `OPENAI_PRESETS`. The registry, the settings and the example pick it up from there.

**A new agent CLI** is a new file in `src/cli/`:

1. Write `src/cli/<name>.rs` with an options struct, the provider struct and `impl ChatProvider`, and a `stream.rs` beside it that turns the CLI's output lines into `ChatEvent`s. The existing providers show the shape; `claude_code.rs` is the one to copy.
2. Use the shared pieces rather than starting processes yourself: `discover::find_binary` and `command_for` (a desktop app does not inherit the shell's `PATH`), `process::run_streaming` (runs the CLI in its own process group and stops the whole group on cancel), `prompt::cli_prompt` and `prepare_working_dir`.
3. Scope the run. Attach the Moshionly MCP server and nothing else, let its tools run without a prompt, and switch off the CLI's own file, shell and web tools. Prefer command-line flags; if the CLI only reads files, write them into the working directory and never into the user's home.
4. Never read the CLI's credentials. `check_ready` may ask the CLI whether it is signed in and must read only the yes or no.
5. Record a real run as a fixture under `tests/fixtures/<name>/`, with account identifiers removed, and test the parser against it. `cli::test_support::fake_agent` makes a stand-in binary for testing a whole turn, a failure and a cancellation.
6. Register it: add the module to `src/cli/mod.rs`, re-export the types from `src/lib.rs`, and add the id to `SUBSCRIPTION_IDS` and a branch to `build` in `src/registry.rs`. Options the user should be able to change become optional fields on `ProviderSettings`.
7. Set `verified: false` until someone has run it against the real CLI, and say so at the top of the file.

**A new model API that is not OpenAI-compatible** is a new file in `src/api/`. Use `http::client`, `http::send`, `http::next_chunk` and `http::with_retries` for requests, `http::sse::SseParser` for the stream, and `api::run_tools` and `UsageTotals` for the tool loop; test it against `http::test_support::MockServer`. Register it in `src/registry.rs` next to `anthropic`.

Whatever the provider, the trait's promises are the same:

- `send` ends the stream with exactly one `Finished`, whatever happened. Do the work in a function that returns `Result<StopReason, AiError>` and pass the result to `EventSender::finish`.
- A cancelled token or a dropped receiver stops the work promptly and leaves no process behind.
- `check_ready` never sends a prompt and never costs anything.
- Every error message built from outside text goes through `AiError::from_kind` or `redact::scrub`.

## Tests

```bash
cargo test -p moshionly-ai
```

runs everything that needs no account: the parsers against their fixtures, whole turns against stand-in CLIs and a stand-in HTTP server, cancellation, settings, the registry and the key store in memory.

Three tests are ignored by default because they touch the real world:

```bash
# Runs the real claude CLI; spends a little of the subscription.
cargo test -p moshionly-ai --test claude_code_live -- --ignored --nocapture
# Runs the real cursor-agent CLI; the tool-call test needs it signed in.
cargo test -p moshionly-ai --test cursor_agent_live -- --ignored --nocapture
# Stores and removes a made-up value in the real OS keychain.
cargo test -p moshionly-ai --test keychain_live -- --ignored --nocapture
```

## Layout

| Path | What is there |
|---|---|
| `src/chat.rs`, `event.rs`, `tool.rs`, `error.rs` | The request, the event stream, the tool bridge and the error type |
| `src/provider.rs` | The `ChatProvider` trait, capabilities and readiness |
| `src/cli/` | Subscription providers, binary discovery and the process runner |
| `src/api/` | API providers and the shared tool loop |
| `src/http/` | The HTTP client, retries and the server-sent events parser |
| `src/settings.rs`, `keys.rs`, `registry.rs` | Settings, the key store and the provider registry |
| `src/redact.rs` | `ApiKey` and the scrubber |
| `examples/chat.rs` | The command-line smoke tool |
| `tests/` | Live tests (ignored by default), fixtures and the echo MCP server |

## Known gaps

- Cursor Agent has not completed a turn against the real CLI; Codex and Gemini CLI have never been run; the API providers have not met a real service. See above.
- There is one `custom` server. A user with two self-hosted endpoints has to switch its address.
- Only Claude Code states what a turn cost. Every other provider reports token counts without a cost.
- Cursor Agent and Gemini CLI cannot be sent images, and both take the prompt as a command-line argument.
- Windows is untested. The process runner stops a whole process group on Unix; on Windows it stops the CLI itself.

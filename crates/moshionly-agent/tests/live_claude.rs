//! A live check with the real `claude` command-line tool.
//!
//! It serves a small document over HTTP MCP, asks Claude Code in one short
//! non-interactive turn to add a fade-in to a named layer, and checks that
//! the document changed and that one undo step restores it.
//!
//! It is ignored by default because it needs the CLI, a signed-in account
//! and the network, and spends the account's usage. Run it by hand:
//!
//! ```text
//! cargo test -p moshionly-agent --test live_claude -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use moshionly_agent::mcp::McpServer;
use moshionly_agent::mcp::http::{HttpConfig, HttpServer};
use moshionly_agent::{
    Frame, FrameRequest, FrameSource, Host, HostError, Origin, Session, SessionHandle, Tools,
};
use moshionly_core::build::PLACEHOLDER_IMAGE;
use moshionly_core::io::{self, Asset};
use moshionly_core::{NodeId, NodeType, Rect};
use serde_json::{Value, json};

const DEMO: &str = r##"{
  "format": "moshionly", "version": 1, "name": "Live check",
  "nodes": [
    {"id":"scene","parent":null,"index":"a0","type":"artboard","name":"Scene","width":1080,"height":1080,"fill":"#F2F3F5"},
    {"id":"card","parent":"scene","index":"a0","type":"rect","name":"Card","x":240,"y":340,"width":600,"height":400,"fill":"#FFFFFF","corner_radius":32},
    {"id":"title","parent":"scene","index":"a1","type":"text","name":"Title","x":300,"y":480,"content":"Hello","font_size":72,"fill":"#111111"}
  ]
}"##;

/// A renderer stand-in: every picture is one transparent pixel.
struct Placeholder;

impl FrameSource for Placeholder {
    fn render(&self, request: &FrameRequest<'_>) -> Result<Frame, HostError> {
        let Ok(Asset::Data { bytes, .. }) = io::resolve_asset(PLACEHOLDER_IMAGE, None) else {
            return Err(HostError::Failed("no placeholder image".into()));
        };
        let artboard = request
            .document
            .world_bounds(request.artboard)
            .unwrap_or_default();
        Ok(Frame {
            png: bytes,
            width: 1,
            height: 1,
            region: request
                .crop
                .unwrap_or(Rect::new(0.0, 0.0, artboard.width, artboard.height)),
            layer_box: request
                .layer
                .and_then(|layer| request.document.world_bounds(layer)),
        })
    }
}

/// The `claude` program, when it is installed.
fn find_claude() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|folder| folder.join("claude"))
        .find(|candidate| candidate.is_file())
}

/// Whether the document has a fade-in, or an opacity animation that starts
/// from 0, on the title.
fn title_fades_in(session: &SessionHandle) -> bool {
    session
        .document()
        .animations_targeting(&NodeId::from("title"))
        .iter()
        .any(|animation| match animation.node_type() {
            NodeType::FadeIn => true,
            NodeType::Opacity => animation.to_value()["from"] == json!(0),
            _ => false,
        })
}

#[test]
#[ignore = "runs the real claude CLI and spends the account's usage; run by hand with --ignored"]
fn claude_adds_a_fade_in_through_mcp() {
    let Some(claude) = find_claude() else {
        eprintln!("skipped: there is no claude command on the PATH");
        return;
    };
    let original = io::parse(DEMO).unwrap();
    let session = SessionHandle::new(Session::new(original.clone()));
    let tools = Tools::new(
        session.clone(),
        Host::new().with_frames(Arc::new(Placeholder)),
    );
    let server = HttpServer::start(
        McpServer::new(tools),
        HttpConfig {
            port: 0,
            ..HttpConfig::default()
        },
    )
    .unwrap();
    let workdir = std::env::temp_dir().join(format!("moshionly-live-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).unwrap();

    let config = json!({ "mcpServers": { "moshionly": { "type": "http", "url": server.url() } } });
    let prompt = "The open Moshionly file has a text layer named \"Title\". Add a fade-in to it \
                  that starts at 300 ms, in one change, and keep everything else as it is. \
                  Answer in one short sentence.";
    let mut child = Command::new(&claude)
        .args(["-p", prompt, "--output-format", "json"])
        .args(["--model", "sonnet"])
        .args(["--strict-mcp-config", "--mcp-config", &config.to_string()])
        .args(["--allowedTools", "mcp__moshionly", "--tools", ""])
        .args(["--permission-mode", "dontAsk", "--disable-slash-commands"])
        .args(["--setting-sources", "", "--no-session-persistence"])
        .current_dir(&workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(300);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("claude did not finish within five minutes");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let output = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&workdir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: Value = serde_json::from_str(stdout.trim()).unwrap_or(json!({ "raw": stdout }));
    eprintln!("claude exit status: {}", output.status);
    eprintln!("claude said: {}", result["result"]);
    eprintln!(
        "turns: {}, cost reported: {} USD",
        result["num_turns"], result["total_cost_usd"]
    );
    if !output.stderr.is_empty() {
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    let steps = session.read(|session| session.undo_steps().to_vec());
    eprintln!(
        "undo steps: {:?}",
        steps
            .iter()
            .map(|step| format!("{}: {}", step.origin, step.summary))
            .collect::<Vec<_>>()
    );
    assert!(title_fades_in(&session), "the title has no fade-in");
    assert!(steps.iter().all(|step| step.origin == Origin::Agent));
    assert_eq!(steps.len(), 1, "the change should be one undo step");
    session.undo().unwrap();
    assert_eq!(*session.document(), original, "one undo restores the file");
}

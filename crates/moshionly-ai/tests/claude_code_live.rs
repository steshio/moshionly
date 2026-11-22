//! Runs the real `claude` CLI. Ignored by default, because it needs Claude
//! Code installed and signed in and it spends a little of the subscription:
//!
//! ```text
//! cargo test -p moshionly-ai --test claude_code_live -- --ignored --nocapture
//! ```
//!
//! One short prompt checks the whole path at once: the CLI attaches a local
//! MCP server, sees an image, calls the server's tool and answers.

#[path = "support/echo_mcp.rs"]
mod echo_mcp;

use echo_mcp::EchoMcpServer;
use moshionly_ai::{
    CancellationToken, ChatEvent, ChatMessage, ChatProvider, ChatRequest, ClaudeCode, ImageData,
    McpEndpoint, ReadyState, StopReason, event_channel,
};

/// A 32 by 32 pixel image filled with pure red.
const RED_PNG: &[u8] = include_bytes!("fixtures/red.png");

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs the real claude CLI and spends subscription usage"]
async fn claude_sees_an_image_and_calls_the_attached_tool() {
    let provider = ClaudeCode::new();
    let readiness = provider.check_ready().await;
    println!("readiness: {:?} {}", readiness.state, readiness.message);
    assert_eq!(
        readiness.state,
        ReadyState::Ready,
        "Claude Code must be installed and signed in for this test"
    );

    let server = EchoMcpServer::start().await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let request = ChatRequest {
        messages: vec![
            ChatMessage::user(
                "Call the echo tool with the main colour of the attached image as its text \
                 (one lowercase word), then reply with only that word.",
            )
            .with_image(ImageData::png(RED_PNG)),
        ],
        system_prompt: Some("You are a test assistant inside a design app.".to_owned()),
        mcp: Some(McpEndpoint::new("moshionly", server.url())),
        working_dir: Some(workspace.path().to_path_buf()),
        ..ChatRequest::default()
    };

    let (events, mut receiver) = event_channel();
    provider
        .send(request, events, CancellationToken::new())
        .await;
    let mut collected = Vec::new();
    while let Some(event) = receiver.recv().await {
        println!("{event:?}");
        collected.push(event);
    }

    assert!(matches!(collected[0], ChatEvent::SessionStarted { .. }));
    let started = collected
        .iter()
        .find_map(|event| match event {
            ChatEvent::ToolCallStarted { name, input, .. } => Some((name.clone(), input.clone())),
            _ => None,
        })
        .expect("the model should call the tool");
    assert_eq!(started.0, "echo", "the MCP prefix should be removed");
    assert!(
        started.1["text"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("red"),
        "the model should have seen a red image, got {}",
        started.1
    );
    assert!(collected.iter().any(|event| matches!(
        event,
        ChatEvent::ToolCallFinished { ok: true, summary, .. } if summary.contains("echo: ")
    )));
    let answer: String = collected
        .iter()
        .filter_map(|event| match event {
            ChatEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        answer.to_lowercase().contains("red"),
        "answer was {answer:?}"
    );
    assert!(collected.iter().any(|event| matches!(
        event,
        ChatEvent::Usage { output_tokens, .. } if *output_tokens > 0
    )));
    assert_eq!(
        collected.last(),
        Some(&ChatEvent::Finished {
            stop_reason: StopReason::EndTurn
        })
    );

    // The server's own record: the CLI listed the tools and called ours.
    assert!(server.methods().iter().any(|method| method == "tools/list"));
    assert_eq!(server.calls().len(), 1);
}

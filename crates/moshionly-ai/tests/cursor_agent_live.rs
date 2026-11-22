//! Runs the real `cursor-agent` CLI. Ignored by default.
//!
//! ```text
//! cargo test -p moshionly-ai --test cursor_agent_live -- --ignored --nocapture
//! ```
//!
//! `the_installed_cli_reports_its_state` costs nothing and passes whether or
//! not anyone is signed in. `cursor_agent_calls_the_attached_tool` needs a
//! signed-in CLI and spends a little of the subscription; it has NOT been
//! run yet, because no Cursor account was available when the provider was
//! written. It is the test to run first when one is.

#[path = "support/echo_mcp.rs"]
mod echo_mcp;

use echo_mcp::EchoMcpServer;
use moshionly_ai::{
    CancellationToken, ChatEvent, ChatProvider, ChatRequest, CursorAgent, ErrorKind, McpEndpoint,
    ReadyState, StopReason, event_channel,
};

async fn run(provider: &CursorAgent, request: ChatRequest) -> Vec<ChatEvent> {
    let (events, mut receiver) = event_channel();
    provider
        .send(request, events, CancellationToken::new())
        .await;
    let mut collected = Vec::new();
    while let Some(event) = receiver.recv().await {
        println!("{event:?}");
        collected.push(event);
    }
    collected
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the cursor-agent CLI to be installed"]
async fn the_installed_cli_reports_its_state() {
    let provider = CursorAgent::new();
    let readiness = provider.check_ready().await;
    println!("readiness: {:?} {}", readiness.state, readiness.message);
    assert_ne!(readiness.state, ReadyState::NotInstalled);
    assert!(readiness.version.is_some(), "the version should be read");

    let models = provider.list_models().await.unwrap();
    println!("models: {models:?}");
    assert_eq!(models[0].id, "default");
    assert!(models.len() >= 2);

    if readiness.state == ReadyState::NotSignedIn {
        // Nobody is signed in, so a turn costs nothing and must say why it
        // failed.
        let workspace = tempfile::tempdir().unwrap();
        let request = ChatRequest {
            working_dir: Some(workspace.path().to_path_buf()),
            ..ChatRequest::from_prompt("Reply with the single word ready")
        };
        let events = run(&provider, request).await;
        assert!(matches!(
            events[0],
            ChatEvent::Error {
                kind: ErrorKind::NotSignedIn,
                ..
            }
        ));
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Finished {
                stop_reason: StopReason::Error
            })
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs the real cursor-agent CLI and spends subscription usage"]
async fn cursor_agent_calls_the_attached_tool() {
    let provider = CursorAgent::new();
    let readiness = provider.check_ready().await;
    println!("readiness: {:?} {}", readiness.state, readiness.message);
    assert_eq!(
        readiness.state,
        ReadyState::Ready,
        "Cursor Agent must be installed and signed in for this test"
    );

    let server = EchoMcpServer::start().await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let request = ChatRequest {
        system_prompt: Some("You are a test assistant inside a design app.".to_owned()),
        mcp: Some(McpEndpoint::new("moshionly", server.url())),
        working_dir: Some(workspace.path().to_path_buf()),
        ..ChatRequest::from_prompt(
            "Call the echo tool with the text ping, then reply with only the word done.",
        )
    };
    let collected = run(&provider, request).await;

    assert!(matches!(collected[0], ChatEvent::SessionStarted { .. }));
    let started = collected
        .iter()
        .find_map(|event| match event {
            ChatEvent::ToolCallStarted { name, input, .. } => Some((name.clone(), input.clone())),
            _ => None,
        })
        .expect("the model should call the tool");
    assert_eq!(started.0, "echo", "the server prefix should be removed");
    assert_eq!(started.1["text"], "ping");
    assert!(collected.iter().any(|event| matches!(
        event,
        ChatEvent::ToolCallFinished { ok: true, summary, .. } if summary.contains("echo: ping")
    )));
    let answer: String = collected
        .iter()
        .filter_map(|event| match event {
            ChatEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        answer.to_lowercase().contains("done"),
        "answer was {answer:?}"
    );
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

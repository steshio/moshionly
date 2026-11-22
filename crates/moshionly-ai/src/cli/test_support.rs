//! Stand-ins for agent CLIs: small shell scripts that behave like the real
//! thing for one scenario, so providers can be tested from start to finish
//! without the CLI or a subscription.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use crate::chat::ChatRequest;
use crate::event::{ChatEvent, event_channel};
use crate::provider::ChatProvider;

/// Writes an executable shell script named `name` into `dir`.
pub(crate) fn fake_agent(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The path of a recorded output file under `tests/fixtures`.
pub(crate) fn fixture_path(relative: &str) -> String {
    format!("{}/tests/fixtures/{relative}", env!("CARGO_MANIFEST_DIR"))
}

/// Runs one turn and returns every event it produced.
pub(crate) async fn collect_events(
    provider: &dyn ChatProvider,
    request: ChatRequest,
) -> Vec<ChatEvent> {
    let (events, mut receiver) = event_channel();
    provider
        .send(request, events, CancellationToken::new())
        .await;
    let mut collected = Vec::new();
    while let Some(event) = receiver.recv().await {
        collected.push(event);
    }
    collected
}

//! The app's connections end to end, without a window: an agent editing
//! the open document over MCP on HTTP, a design posted by the Figma plugin
//! and added at once, and an export started from the Export dialog.
//!
//! The server listens on a free port chosen by the system, never on 47821,
//! which may be taken where the tests run.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use moshionly_agent::import::MAX_WAITING;
use moshionly_agent::{Origin, SessionHandle};
use moshionly_app::connect::ServerStatus;
use moshionly_app::export::RunOutcome;
use moshionly_app::settings::Settings;
use moshionly_app::shell::{FileAction, MAX_IMPORTS_PER_WINDOW, Shell};
use moshionly_core::{NodeId, NodeType};
use moshionly_export::Format;
use moshionly_render::{AssetLoader, FontLibrary, Renderer};
use serde_json::{Value, json};

/// The plugin's hero design, converted by `plugins/figma/tools/convert-fixtures.js`.
const ENVELOPE: &str = include_str!("fixtures/figma-hero.json");

fn demo_shell() -> Shell {
    let demo = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moshionly-core/examples/demo.moshi"
    );
    let session = SessionHandle::open(demo).expect("the demo opens");
    let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
    Shell::headless(Settings::default(), session, renderer)
}

/// A shell serving agents on a free port, and that port's address.
fn serving_shell() -> (Shell, SocketAddr) {
    let mut shell = demo_shell();
    shell.serve_agents(0);
    let port = shell.agent_server().port().expect("the server runs");
    (shell, SocketAddr::from(([127, 0, 0, 1], port)))
}

/// What came back from the server.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|error| panic!("{error}: {}", self.body))
    }
}

/// Sends one HTTP/1.1 request and reads the whole answer.
fn send(address: SocketAddr, path: &str, headers: &[(&str, &str)], body: &str) -> Reply {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut request = format!("POST {path} HTTP/1.1\r\nConnection: close\r\nHost: {address}\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let raw = String::from_utf8(raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

const JSON: (&str, &str) = ("Content-Type", "application/json");
const ACCEPT: (&str, &str) = ("Accept", "application/json, text/event-stream");

#[test]
fn an_agent_edits_the_open_document_over_http_and_the_person_can_undo_it() {
    let (mut shell, address) = serving_shell();

    let reply = send(
        address,
        "/mcp",
        &[JSON, ACCEPT],
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" } } })
        .to_string(),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    let session = reply
        .header("mcp-session-id")
        .expect("a session")
        .to_owned();
    let with_session = [
        JSON,
        ACCEPT,
        ("Mcp-Session-Id", session.as_str()),
        ("MCP-Protocol-Version", "2025-06-18"),
    ];

    let reply = send(
        address,
        "/mcp",
        &with_session,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "apply_edits",
            "arguments": { "operations": [
                { "op": "add", "parent": "scene",
                  "node": { "type": "rect", "name": "From an agent", "x": 40, "y": 40,
                            "width": 200, "height": 120, "fill": "#FF5500" } }
            ] } } })
        .to_string(),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    let result = &reply.json()["result"];
    assert_ne!(result["isError"], true, "{result}");

    // The editor sees the agent's layer in the same session.
    shell.editor_mut().sync();
    let added = shell
        .editor()
        .document()
        .nodes()
        .find(|node| node.name() == "From an agent")
        .map(|node| node.id.clone())
        .expect("the agent's layer is in the editor's document");
    assert_eq!(
        shell
            .editor()
            .session()
            .read(|s| s.undo_steps().last().map(|step| step.origin)),
        Some(Origin::Agent)
    );
    shell.editor_mut().undo().unwrap();
    assert!(!shell.editor().document().contains(&added));
}

#[test]
fn a_design_posted_from_figma_is_added_at_once_as_one_step() {
    let (mut shell, address) = serving_shell();
    let before = shell.editor().document().artboards().len();

    let reply = send(address, "/import", &[JSON, ("Origin", "null")], ENVELOPE);
    assert_eq!(reply.status, 202, "{}", reply.body);
    assert_eq!(reply.json()["status"], "pending");

    // Nothing asks first: the design is added as soon as it arrives.
    let deadline = Instant::now() + Duration::from_secs(10);
    while shell.editor().document().artboards().len() == before {
        shell.receive_imports(None);
        assert!(Instant::now() < deadline, "the design never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    let document = shell.editor().document();
    // The frame and each loose layer beside it become artboards.
    assert_eq!(document.artboards().len(), before + 3);
    let hero = document
        .artboards()
        .iter()
        .find(|id| {
            document
                .get(id)
                .and_then(|node| node.layer()?.source_id.clone())
                .is_some_and(|source| source.starts_with("figma:"))
        })
        .cloned()
        .expect("the design came in as an artboard");
    assert!(
        document
            .nodes()
            .any(|node| node.parent.as_ref() == Some(&hero) && node.node_type() == NodeType::Text)
    );
    assert!(
        shell.editor().selection.ids().contains(&hero),
        "what arrived is selected"
    );
    assert_eq!(
        shell
            .editor()
            .session()
            .read(|s| s.undo_steps().last().map(|step| step.origin)),
        Some(Origin::Import)
    );

    // One undo takes the whole design away.
    shell.editor_mut().undo().unwrap();
    assert_eq!(shell.editor().document().artboards().len(), before);
    assert!(!shell.editor().document().contains(&hero));
}

/// Nothing asks before a design is added, so a page posting designs in a
/// loop is slowed down: a few are added a minute, the next wait, and the
/// server refuses more once a few are waiting.
#[test]
fn designs_posted_in_a_loop_are_added_a_few_a_minute() {
    assert!(MAX_IMPORTS_PER_WINDOW >= MAX_WAITING && MAX_IMPORTS_PER_WINDOW < 2 * MAX_WAITING);
    let (mut shell, address) = serving_shell();
    let post = || send(address, "/import", &[JSON, ("Origin", "null")], ENVELOPE);
    for _ in 0..2 {
        for _ in 0..MAX_WAITING {
            assert_eq!(post().status, 202);
        }
        shell.receive_imports(None);
    }
    // The minute's allowance is used up, so these stay waiting.
    let waiting = 2 * MAX_WAITING - MAX_IMPORTS_PER_WINDOW;
    for _ in waiting..MAX_WAITING {
        assert_eq!(post().status, 202);
    }
    let refused = post();
    assert_eq!(refused.status, 429, "{}", refused.body);
    assert!(refused.body.contains("still adding"), "{}", refused.body);
}

#[test]
fn a_design_sent_again_replaces_the_first_and_keeps_its_animations() {
    let mut shell = demo_shell();
    let pending = moshionly_agent::import::read_envelope(ENVELOPE.as_bytes()).unwrap();
    assert!(shell.import_design(&pending));
    let title = shell
        .editor()
        .document()
        .nodes()
        .find(|node| {
            node.node_type() == NodeType::Text
                && node.layer().is_some_and(|l| l.source_id.is_some())
        })
        .map(|node| node.id.clone())
        .unwrap();
    let output = shell.tools().call(
        "apply_edits",
        json!({ "operations": [
            { "op": "add", "node": { "type": "fade_in", "target": title.as_str(), "start": 0, "end": 400 } }
        ] }),
    );
    assert!(!output.is_error, "{}", output.joined_text());
    shell.editor_mut().sync();
    let fade = shell
        .editor()
        .document()
        .nodes()
        .find(|node| node.node_type() == NodeType::FadeIn && node.target() == Some(&title))
        .map(|node| node.id.clone())
        .expect("the fade was added");

    let artboards = shell.editor().document().artboards().len();
    assert!(shell.import_design(&pending));
    let document = shell.editor().document();
    assert_eq!(document.artboards().len(), artboards, "no copy was added");
    let fade = document.get(&fade).expect("the animation stays");
    let target = fade.target().expect("it moves a layer");
    assert!(document.contains(target), "on the new version of the layer");
}

#[test]
fn turning_agents_off_stops_the_server_and_on_starts_it() {
    let (mut shell, _) = serving_shell();
    shell.allow_agents(false);
    assert_eq!(shell.agent_server().status(), &ServerStatus::Off);
    assert!(!shell.settings().agents.serve);
    shell.allow_agents(true);
    assert!(matches!(
        shell.agent_server().status(),
        ServerStatus::Running { .. }
    ));
}

#[test]
fn the_export_dialog_writes_its_file_off_the_ui_thread_and_remembers_the_choices() {
    let mut shell = demo_shell();
    shell.open_export_dialog();
    assert!(shell.is_export_dialog_open());
    let setup = shell.export_setup_mut().unwrap();
    assert_eq!(setup.artboard, NodeId::from("scene"));
    setup.choices.set_format(Format::Gif);
    setup.choices.scale = 0.5;
    setup.choices.fps = 12.0;

    let dir = std::env::temp_dir().join(format!(
        "moshionly-app-export-dialog-{}-{}",
        std::process::id(),
        NodeId::generate()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    shell.start_export(dir.join("demo.gif"), None);
    let deadline = Instant::now() + Duration::from_secs(60);
    let outcome = loop {
        if let Some(outcome) = shell.poll_export() {
            break outcome;
        }
        assert!(Instant::now() < deadline, "the export never finished");
        std::thread::sleep(Duration::from_millis(10));
    };
    let RunOutcome::Done(exported) = outcome else {
        panic!("expected a file, got {outcome:?}");
    };
    assert_eq!(exported.format, Format::Gif);
    assert_eq!(exported.width, 540);
    assert!(
        std::fs::read(&exported.path)
            .unwrap()
            .starts_with(b"GIF89a")
    );
    assert_eq!(shell.settings().export.format(), Format::Gif);
    assert_eq!(shell.settings().export.scale, 0.5);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What is in `dir`, by name.
fn files_in(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect()
}

/// Starts a large MP4 export from the Export dialog into a new folder,
/// and returns that folder once the export is writing there.
fn start_long_export(shell: &mut Shell, label: &str) -> std::path::PathBuf {
    shell.open_export_dialog();
    let setup = shell.export_setup_mut().unwrap();
    setup.choices.set_format(Format::Mp4);
    setup.choices.scale = 2.0;
    let dir = std::env::temp_dir().join(format!(
        "moshionly-app-export-{label}-{}-{}",
        std::process::id(),
        NodeId::generate()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    shell.start_export(dir.join("demo.mp4"), None);
    // The unfinished file appears beside the destination once it starts.
    let deadline = Instant::now() + Duration::from_secs(30);
    while files_in(&dir).is_empty() {
        assert!(Instant::now() < deadline, "the export never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shell.poll_export(), None, "still running");
    dir
}

#[test]
fn quitting_during_an_export_stops_it_and_leaves_nothing_behind() {
    let mut shell = demo_shell();
    let dir = start_long_export(&mut shell, "quit");
    shell.prepare_to_quit();
    assert!(!shell.is_export_dialog_open());
    assert!(
        files_in(&dir).is_empty(),
        "left behind: {:?}",
        files_in(&dir)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quitting_while_the_chat_exports_stops_it_and_leaves_nothing_behind() {
    // A copy of the demo in a folder of its own: the chat's exports go to
    // an `exports` folder beside the project.
    let project = std::env::temp_dir().join(format!(
        "moshionly-app-chat-export-{}-{}",
        std::process::id(),
        NodeId::generate()
    ));
    std::fs::create_dir_all(&project).unwrap();
    let file = project.join("demo.moshi");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moshionly-core/examples/demo.moshi"
        ),
        &file,
    )
    .unwrap();
    let session = SessionHandle::open(&file).expect("the copy opens");
    let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
    let mut shell = Shell::headless(Settings::default(), session, renderer);

    // The chat calls the export tool on a thread of its own.
    let tools = shell.tools().clone();
    let chat =
        std::thread::spawn(move || tools.call("export", json!({ "format": "mp4", "scale": 2 })));
    let exports = project.join("exports");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !exports.is_dir() || files_in(&exports).is_empty() {
        assert!(Instant::now() < deadline, "the export never started");
        std::thread::sleep(Duration::from_millis(5));
    }

    shell.prepare_to_quit();
    assert!(
        files_in(&exports).is_empty(),
        "left behind: {:?}",
        files_in(&exports)
    );
    let answer = chat.join().unwrap();
    assert!(answer.is_error, "the export was stopped, not finished");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn asking_for_export_again_while_one_runs_keeps_it() {
    let mut shell = demo_shell();
    let dir = start_long_export(&mut shell, "again");
    shell.open_export_dialog();
    assert!(shell.is_exporting(), "the running export is still shown");
    shell.cancel_export();
    let deadline = Instant::now() + Duration::from_secs(10);
    let outcome = loop {
        if let Some(outcome) = shell.poll_export() {
            break outcome;
        }
        assert!(Instant::now() < deadline, "the export never stopped");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(outcome, RunOutcome::Cancelled);
    assert!(
        files_in(&dir).is_empty(),
        "left behind: {:?}",
        files_in(&dir)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn another_document_closes_the_export_dialog_and_cancels_its_export() {
    let mut shell = demo_shell();
    shell.open_export_dialog();
    shell.request(FileAction::New);
    assert!(
        !shell.is_export_dialog_open(),
        "it was about the old document"
    );
    shell.open_export_dialog();
    let first = shell.editor().document().artboards().first().cloned();
    assert_eq!(
        shell.export_setup().map(|setup| &setup.artboard),
        first.as_ref()
    );
    assert_ne!(first, Some(NodeId::from("scene")));

    let mut shell = demo_shell();
    let dir = start_long_export(&mut shell, "replaced");
    shell.request(FileAction::New);
    assert!(!shell.is_export_dialog_open());
    assert!(
        files_in(&dir).is_empty(),
        "left behind: {:?}",
        files_in(&dir)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

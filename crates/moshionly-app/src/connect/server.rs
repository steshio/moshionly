//! The MCP server inside the app: agents outside the window (Claude Code,
//! Cursor and the like) work on the open document through it, and the
//! Figma plugin sends designs to it.

use std::io;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use moshionly_agent::Tools;
use moshionly_agent::import::{ImportSink, PendingImport};
use moshionly_agent::mcp::McpServer;
use moshionly_agent::mcp::http::{HttpConfig, HttpServer};
use moshionly_export::CancelToken;

pub use moshionly_agent::mcp::http::DEFAULT_PORT;

/// How long a restart waits for the port the old server held. Its listener
/// closes a few milliseconds after the stop; this only covers that.
const PORT_RELEASE: Duration = Duration::from_millis(500);
/// How often a restart tries the port again while it waits.
const PORT_RETRY: Duration = Duration::from_millis(5);

/// Whether agents can reach the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    /// Turned off in the settings.
    Off,
    /// Listening: agents connect at `url`.
    Running {
        /// The MCP endpoint, such as `http://127.0.0.1:47821/mcp`.
        url: String,
    },
    /// The server could not start, for example because another program
    /// holds the port. Everything else in the app works.
    Unavailable {
        /// The port that was asked for.
        port: u16,
        /// Why, in a sentence for the person.
        reason: String,
    },
}

impl ServerStatus {
    /// A few words for the status item's tooltip.
    pub fn headline(&self) -> String {
        match self {
            Self::Off => "Agents cannot connect: turned off".to_owned(),
            Self::Running { .. } => "Agents can connect".to_owned(),
            Self::Unavailable { port, .. } => format!("Agents cannot connect: port {port} is busy"),
        }
    }
}

/// The app's MCP server, or why there is none.
///
/// Stopping it never makes the window wait: what agents started through it
/// is cancelled, and the requests still running finish on a thread of
/// their own.
pub struct AgentServer {
    status: ServerStatus,
    server: Option<HttpServer>,
    inbox: Option<Receiver<PendingImport>>,
    /// The window to wake when a design arrives.
    window: Arc<Mutex<Option<egui::Context>>>,
    /// Cancelled when the server stops, so the exports and screenshots
    /// agents started through it end at once.
    cancel: CancelToken,
    /// The threads letting stopped servers finish their requests.
    stopping: Vec<JoinHandle<()>>,
}

impl Default for AgentServer {
    fn default() -> Self {
        Self::off()
    }
}

impl AgentServer {
    /// No server: agents cannot connect.
    pub fn off() -> Self {
        Self {
            status: ServerStatus::Off,
            server: None,
            inbox: None,
            window: Arc::new(Mutex::new(None)),
            cancel: CancelToken::new(),
            stopping: Vec::new(),
        }
    }

    /// Serves `tools` on `127.0.0.1:port`, and takes designs from the Figma
    /// plugin at `/import`. Never fails: when the port is taken the status
    /// says so and the app carries on without the server.
    ///
    /// `cancel` is cancelled when the server stops. Build `tools` with a
    /// host that watches it (see
    /// [`cancellable_app_host`](crate::host::cancellable_app_host)), so that
    /// an export an agent is running ends with the server instead of
    /// keeping it busy.
    pub fn start(tools: Tools, port: u16, cancel: CancelToken) -> Self {
        Self::listen(tools, port, cancel, Duration::ZERO)
    }

    /// Stops this server and starts one serving `tools` on `port`, as
    /// [`start`](Self::start) does. When the old server held that port, the
    /// new one waits a moment for it to be let go.
    pub fn restart(&mut self, tools: Tools, port: u16, cancel: CancelToken) {
        let held = self.port();
        self.stop();
        let patience = if port != 0 && held == Some(port) {
            PORT_RELEASE
        } else {
            Duration::ZERO
        };
        let mut next = Self::listen(tools, port, cancel, patience);
        self.stopping.retain(|thread| !thread.is_finished());
        next.stopping = std::mem::take(&mut self.stopping);
        *self = next;
    }

    /// Starts serving, trying the port again for up to `patience` while it
    /// is in use.
    fn listen(tools: Tools, port: u16, cancel: CancelToken, patience: Duration) -> Self {
        let mut server = Self::off();
        server.cancel = cancel;
        let (sink, inbox) = ImportSink::new();
        let window = Arc::clone(&server.window);
        let sink = sink.with_wake(move || {
            let window = window.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(ctx) = window.as_ref() {
                ctx.request_repaint();
            }
        });
        let config = HttpConfig {
            port,
            ..HttpConfig::default()
        };
        let deadline = Instant::now() + patience;
        let started = loop {
            match HttpServer::start_with_imports(
                McpServer::new(tools.clone()),
                config.clone(),
                sink.clone(),
            ) {
                Err(error)
                    if error.kind() == io::ErrorKind::AddrInUse && Instant::now() < deadline =>
                {
                    std::thread::sleep(PORT_RETRY);
                }
                started => break started,
            }
        };
        match started {
            Ok(http) => {
                server.status = ServerStatus::Running { url: http.url() };
                server.server = Some(http);
                server.inbox = Some(inbox);
            }
            Err(error) => {
                server.status = ServerStatus::Unavailable {
                    port,
                    reason: unavailable_reason(port, &error),
                };
            }
        }
        server
    }

    /// Whether agents can reach the app.
    pub fn status(&self) -> &ServerStatus {
        &self.status
    }

    /// The MCP endpoint while the server runs.
    pub fn url(&self) -> Option<&str> {
        match &self.status {
            ServerStatus::Running { url } => Some(url),
            _ => None,
        }
    }

    /// The port the server listens on while it runs.
    pub fn port(&self) -> Option<u16> {
        self.server.as_ref().map(|server| server.address().port())
    }

    /// Tells the server which window to wake when a design arrives. Cheap;
    /// call it every frame.
    pub fn attach(&self, ctx: &egui::Context) {
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if window.is_none() {
            *window = Some(ctx.clone());
        }
    }

    /// The next design sent from Figma, if one is waiting.
    pub fn take_import(&self) -> Option<PendingImport> {
        self.inbox.as_ref()?.try_recv().ok()
    }

    /// Stops the server at once: agents cannot reach it any more, and what
    /// they started through it is cancelled. The requests still running
    /// finish on a thread of their own; the window does not wait for them.
    pub fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(server) = self.server.take() {
            let spawned = std::thread::Builder::new()
                .name("moshionly-mcp-stop".into())
                .spawn(move || server.stop());
            // Without a thread the server was stopped where it was dropped,
            // inside the failed spawn, which waited for its requests.
            if let Ok(thread) = spawned {
                self.stopping.push(thread);
            }
        }
        self.inbox = None;
        self.status = ServerStatus::Off;
    }

    /// Stops the server and waits up to `limit` for the requests that were
    /// running to end, so that cancelled exports can clean up before the
    /// app quits. Returns whether they all ended in time.
    pub fn shut_down(&mut self, limit: Duration) -> bool {
        self.stop();
        let deadline = Instant::now() + limit;
        while self.stopping.iter().any(|thread| !thread.is_finished()) {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(PORT_RETRY);
        }
        for thread in self.stopping.drain(..) {
            let _ = thread.join();
        }
        true
    }
}

impl Drop for AgentServer {
    /// Stops the server without waiting for it.
    fn drop(&mut self) {
        self.stop();
    }
}

/// Why the server could not listen on `port`, for the person.
fn unavailable_reason(port: u16, error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AddrInUse {
        format!(
            "Another program is using port {port}, perhaps another Moshionly window. Agents and \
             the Figma plugin reach that one instead; everything else here works."
        )
    } else {
        format!("The server for agents could not start on port {port}: {error}.")
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::mpsc;

    use moshionly_agent::{
        ExportFormat, ExportProgress, ExportRequest, Exported, Exporter, Host, HostError,
        SessionHandle, ToolsConfig,
    };
    use moshionly_core::Document;
    use serde_json::{Value, json};

    use super::*;

    fn tools() -> Tools {
        Tools::new(
            SessionHandle::from_document(Document::new("Test")),
            Host::new(),
        )
    }

    /// An export that runs until it is cancelled, or for three seconds.
    struct EndlessExporter {
        cancel: CancelToken,
        started: Mutex<mpsc::Sender<()>>,
        ended: Mutex<mpsc::Sender<bool>>,
    }

    impl Exporter for EndlessExporter {
        fn formats(&self) -> Vec<ExportFormat> {
            vec![ExportFormat::Png]
        }

        fn export(
            &self,
            _: &ExportRequest<'_>,
            _: &mut dyn FnMut(ExportProgress),
        ) -> Result<Exported, HostError> {
            let _ = self.started.lock().unwrap().send(());
            let until = Instant::now() + Duration::from_secs(3);
            while !self.cancel.is_cancelled() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = self.ended.lock().unwrap().send(self.cancel.is_cancelled());
            Err(HostError::Failed("stopped".into()))
        }
    }

    /// Sends one request and returns the answer's session header and body.
    fn post(address: SocketAddr, session: Option<&str>, body: &Value) -> (Option<String>, String) {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let body = body.to_string();
        let session = session
            .map(|id| format!("Mcp-Session-Id: {id}\r\nMCP-Protocol-Version: 2025-06-18\r\n"))
            .unwrap_or_default();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nConnection: close\r\nHost: {address}\r\n\
             Content-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\n{session}\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut raw = String::new();
        let _ = stream.read_to_string(&mut raw);
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let session = head.lines().find_map(|line| {
            let (name, value) = line.split_once(": ")?;
            name.eq_ignore_ascii_case("mcp-session-id")
                .then(|| value.to_owned())
        });
        (session, body.to_owned())
    }

    #[test]
    fn the_server_says_where_agents_connect() {
        let server = AgentServer::start(tools(), 0, CancelToken::new());
        let url = server.url().expect("running").to_owned();
        let port = server.port().unwrap();
        assert_eq!(url, format!("http://127.0.0.1:{port}/mcp"));
        assert_eq!(server.status().headline(), "Agents can connect");
        assert!(server.take_import().is_none());
    }

    #[test]
    fn a_taken_port_is_reported_and_the_app_carries_on() {
        let first = AgentServer::start(tools(), 0, CancelToken::new());
        let port = first.port().unwrap();
        let second = AgentServer::start(tools(), port, CancelToken::new());
        match second.status() {
            ServerStatus::Unavailable {
                port: asked,
                reason,
            } => {
                assert_eq!(*asked, port);
                assert!(reason.contains(&port.to_string()), "{reason}");
            }
            other => panic!("expected unavailable, got {other:?}"),
        }
        assert_eq!(second.url(), None);
        assert!(second.status().headline().contains("busy"));
    }

    #[test]
    fn a_stopped_server_is_off() {
        let mut server = AgentServer::start(tools(), 0, CancelToken::new());
        server.stop();
        assert_eq!(server.status(), &ServerStatus::Off);
        assert_eq!(server.url(), None);
        assert_eq!(server.port(), None);
    }

    #[test]
    fn stopping_cancels_an_agents_export_without_waiting_for_it() {
        let cancel = CancelToken::new();
        let (started, has_started) = mpsc::channel();
        let (ended, has_ended) = mpsc::channel();
        let exporter = Arc::new(EndlessExporter {
            cancel: cancel.clone(),
            started: Mutex::new(started),
            ended: Mutex::new(ended),
        });
        let folder =
            std::env::temp_dir().join(format!("moshionly-app-server-stop-{}", std::process::id()));
        let tools = Tools::with_config(
            SessionHandle::from_document(crate::shell::new_document()),
            Host::new().with_exporter(exporter),
            ToolsConfig {
                export_dir: Some(folder.clone()),
                ..ToolsConfig::default()
            },
        );
        let mut server = AgentServer::start(tools, 0, cancel);
        let address = SocketAddr::from(([127, 0, 0, 1], server.port().unwrap()));

        let agent = std::thread::spawn(move || {
            let (session, _) = post(
                address,
                None,
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                    "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "test", "version": "1" } } }),
            );
            post(
                address,
                session.as_deref(),
                &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
                    "name": "export", "arguments": { "format": "png" } } }),
            )
            .1
        });
        has_started
            .recv_timeout(Duration::from_secs(5))
            .expect("the agent's export started");

        let asked = Instant::now();
        server.stop();
        assert!(
            asked.elapsed() < Duration::from_millis(200),
            "stopping waited {:?} for the export",
            asked.elapsed()
        );
        assert_eq!(server.status(), &ServerStatus::Off);
        let cancelled = has_ended
            .recv_timeout(Duration::from_secs(1))
            .expect("the export ended soon after");
        assert!(cancelled, "the export was cancelled, not left to run");
        assert!(server.shut_down(Duration::from_secs(2)));
        let answer = agent.join().unwrap();
        assert!(answer.contains("stopped"), "{answer}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_restart_takes_back_the_port_the_server_had() {
        let mut server = AgentServer::start(tools(), 0, CancelToken::new());
        let port = server.port().unwrap();
        for _ in 0..3 {
            server.restart(tools(), port, CancelToken::new());
            assert_eq!(
                server.status(),
                &ServerStatus::Running {
                    url: format!("http://127.0.0.1:{port}/mcp")
                }
            );
        }
        assert!(server.shut_down(Duration::from_secs(2)));
    }

    #[test]
    fn a_restart_after_the_port_was_busy_starts_at_once() {
        let first = AgentServer::start(tools(), 0, CancelToken::new());
        let port = first.port().unwrap();
        let mut second = AgentServer::start(tools(), port, CancelToken::new());
        let asked = Instant::now();
        second.restart(tools(), port, CancelToken::new());
        assert!(matches!(second.status(), ServerStatus::Unavailable { .. }));
        assert!(
            asked.elapsed() < PORT_RELEASE,
            "only a held port is waited for"
        );
    }
}

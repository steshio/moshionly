//! The MCP server inside the app: agents outside the window (Claude Code,
//! Cursor and the like) work on the open document through it, and the
//! Figma plugin sends designs to it.

use std::io;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError};

use moshionly_agent::Tools;
use moshionly_agent::import::{ImportSink, PendingImport};
use moshionly_agent::mcp::McpServer;
use moshionly_agent::mcp::http::{HttpConfig, HttpServer};

pub use moshionly_agent::mcp::http::DEFAULT_PORT;

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
pub struct AgentServer {
    status: ServerStatus,
    server: Option<HttpServer>,
    inbox: Option<Receiver<PendingImport>>,
    /// The window to wake when a design arrives.
    window: Arc<Mutex<Option<egui::Context>>>,
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
        }
    }

    /// Serves `tools` on `127.0.0.1:port`, and takes designs from the Figma
    /// plugin at `/import`. Never fails: when the port is taken the status
    /// says so and the app carries on without the server.
    pub fn start(tools: Tools, port: u16) -> Self {
        let mut server = Self::off();
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
        match HttpServer::start_with_imports(McpServer::new(tools), config, sink) {
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

    /// Stops the server.
    pub fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.stop();
        }
        self.inbox = None;
        self.status = ServerStatus::Off;
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
    use moshionly_agent::{Host, SessionHandle};
    use moshionly_core::Document;

    use super::*;

    fn tools() -> Tools {
        Tools::new(
            SessionHandle::from_document(Document::new("Test")),
            Host::new(),
        )
    }

    #[test]
    fn the_server_says_where_agents_connect() {
        let server = AgentServer::start(tools(), 0);
        let url = server.url().expect("running").to_owned();
        let port = server.port().unwrap();
        assert_eq!(url, format!("http://127.0.0.1:{port}/mcp"));
        assert_eq!(server.status().headline(), "Agents can connect");
        assert!(server.take_import().is_none());
    }

    #[test]
    fn a_taken_port_is_reported_and_the_app_carries_on() {
        let first = AgentServer::start(tools(), 0);
        let port = first.port().unwrap();
        let second = AgentServer::start(tools(), port);
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
        let mut server = AgentServer::start(tools(), 0);
        server.stop();
        assert_eq!(server.status(), &ServerStatus::Off);
        assert_eq!(server.url(), None);
        assert_eq!(server.port(), None);
    }
}

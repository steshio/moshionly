//! The shell's connections: the MCP server agents reach the open document
//! through, the "Connect an agent" dialog, and designs sent from Figma.

use std::time::{Duration, Instant};

use egui::{Context, Frame, Margin, RichText, ScrollArea, Ui, UserAttentionType, ViewportCommand};
use moshionly_agent::Tools;
use moshionly_agent::import::{EnvelopeError, FORMAT, ImportTarget, PendingImport, read_envelope};
use moshionly_export::CancelToken;

use super::{QUIT_PATIENCE, Shell};
use crate::{
    connect::{
        AgentServer, ServerStatus,
        clients::{client_setups, stdio_command},
        import::{contents, origin},
    },
    host::cancellable_app_host,
    theme::{Theme, corner, icons},
    widgets::{ButtonKind, IconButton, ModalFrame, TextButton, Toast, modal_footer},
};

/// At most this many designs sent from Figma are added in one
/// [`IMPORT_WINDOW`], and no more once they came to
/// [`MAX_IMPORT_BYTES_PER_WINDOW`]. A person sends a few a minute at most.
/// Nothing asks before a design is added, so this slows down a page that
/// posts designs in a loop; turning off "Let agents on this computer
/// connect" stops it. Designs over the limit wait, and the server refuses
/// more once a few are waiting.
pub const MAX_IMPORTS_PER_WINDOW: usize = 6;

/// How many bytes of designs are added in one [`IMPORT_WINDOW`]. The first
/// design of a window is always added, however large, so one design at the
/// server's size limit never gets stuck.
pub const MAX_IMPORT_BYTES_PER_WINDOW: usize = 32 * 1024 * 1024;

/// See [`MAX_IMPORTS_PER_WINDOW`].
pub const IMPORT_WINDOW: Duration = Duration::from_secs(60);

/// How the status item in the top bar looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentLight {
    /// Turned off: no light.
    Off,
    /// Agents can connect: a green light.
    Running,
    /// The server could not start: an amber light.
    Unavailable,
}

impl AgentLight {
    /// The light for a server status.
    pub fn of(status: &ServerStatus) -> Self {
        match status {
            ServerStatus::Off => Self::Off,
            ServerStatus::Running { .. } => Self::Running,
            ServerStatus::Unavailable { .. } => Self::Unavailable,
        }
    }
}

impl Shell {
    /// Serves the open document to agents over MCP on `127.0.0.1:port`,
    /// unless the settings turn that off. The app calls it once with
    /// [`DEFAULT_PORT`](crate::connect::DEFAULT_PORT); tests pass 0 for a
    /// free port. A port that is taken is reported in the status, and the
    /// window works without the server.
    pub fn serve_agents(&mut self, port: u16) {
        self.agent_port = Some(port);
        self.restart_agent_server();
    }

    /// Turns the MCP server on or off, and remembers the choice.
    pub fn allow_agents(&mut self, allowed: bool) {
        self.settings.agents.serve = allowed;
        self.restart_agent_server();
    }

    /// Starts the server afresh, or stops it when agents are not allowed.
    /// Neither waits for what agents are running: that is cancelled.
    fn restart_agent_server(&mut self) {
        match self.agent_port.filter(|_| self.settings.agents.serve) {
            Some(port) => {
                // The server's own tools, so that stopping it cancels what
                // agents started and leaves the chat's work alone.
                let cancel = CancelToken::new();
                let host = cancellable_app_host(self.editor.renderer(), &cancel);
                let tools = Tools::new(self.tools.session().clone(), host);
                self.server.restart(tools, port, cancel);
            }
            None => self.server.stop(),
        }
    }

    /// Stops the server before the app quits, and gives what agents were
    /// running a moment to clean up after being cancelled.
    pub(super) fn shut_down_agent_server(&mut self) {
        self.server.shut_down(QUIT_PATIENCE);
    }

    /// The MCP server and whether agents can connect.
    pub fn agent_server(&self) -> &AgentServer {
        &self.server
    }

    /// The agent tools over the open document, with the app's renderer,
    /// fonts and exporter: what the chat uses. Their exports stop when the
    /// app quits.
    pub fn tools(&self) -> &Tools {
        &self.tools
    }

    /// Opens the "Connect an agent" dialog.
    pub fn open_connect_dialog(&mut self) {
        self.connect_open = true;
    }

    /// Whether the "Connect an agent" dialog is open.
    pub fn is_connect_dialog_open(&self) -> bool {
        self.connect_open
    }

    /// Adds the designs sent from Figma since the last frame, within
    /// [`MAX_IMPORTS_PER_WINDOW`] and [`MAX_IMPORT_BYTES_PER_WINDOW`] a
    /// minute, and with a window brings it
    /// forward, or has its Dock icon ask for attention when it is minimized
    /// or hidden. [`show`](Shell::show) calls this every frame, and the app
    /// also while the window is covered or minimized.
    pub fn receive_imports(&mut self, ctx: Option<&Context>) {
        let now = Instant::now();
        while self
            .recent_imports
            .front()
            .is_some_and(|(added, _)| now.duration_since(*added) >= IMPORT_WINDOW)
        {
            self.recent_imports.pop_front();
        }
        let mut arrived = false;
        while !self.import_limit_reached() {
            let Some(pending) = self.server.take_import() else {
                break;
            };
            self.recent_imports.push_back((now, pending.bytes));
            self.import_design(&pending);
            arrived = true;
        }
        let Some(ctx) = ctx else {
            return;
        };
        if arrived {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                UserAttentionType::Informational,
            ));
        }
        // Over the limit, look again once the oldest design is a minute old.
        if let Some((oldest, _)) = self
            .recent_imports
            .front()
            .filter(|_| self.import_limit_reached())
        {
            ctx.request_repaint_after(IMPORT_WINDOW.saturating_sub(now.duration_since(*oldest)));
        }
    }

    /// Whether the designs added in the last minute used up the allowance:
    /// [`MAX_IMPORTS_PER_WINDOW`] of them, or [`MAX_IMPORT_BYTES_PER_WINDOW`].
    fn import_limit_reached(&self) -> bool {
        let bytes: usize = self.recent_imports.iter().map(|(_, bytes)| bytes).sum();
        self.recent_imports.len() >= MAX_IMPORTS_PER_WINDOW || bytes >= MAX_IMPORT_BYTES_PER_WINDOW
    }

    /// Adds a design sent from Figma as one undo step, without asking, and
    /// selects what arrived. Layers that came over before are put in place
    /// of their earlier versions and keep their animations; the rest become
    /// new artboards. Returns whether it was added.
    pub fn import_design(&mut self, pending: &PendingImport) -> bool {
        self.finish_work_in_progress();
        match pending.apply(self.editor.session(), &ImportTarget::ReplaceMatching) {
            Ok(outcome) => {
                if self.editor.sync() {
                    self.after_document_change();
                }
                let added = outcome.roots.len().saturating_sub(outcome.replaced);
                let mut message = if outcome.replaced == 0 {
                    format!("Added {} from {}", contents(pending), origin(pending))
                } else {
                    let mut text = format!(
                        "Updated {} from {} and kept their animations",
                        plural(outcome.replaced, "layer"),
                        origin(pending)
                    );
                    if added > 0 {
                        text.push_str(&format!(", and added {} new", plural(added, "layer")));
                    }
                    text
                };
                if outcome.dropped_animations > 0 {
                    message.push_str(&format!(
                        "; {} no longer had a layer to move",
                        plural(outcome.dropped_animations, "animation")
                    ));
                }
                message.push_str(". Undo takes it back.");
                self.editor.selection.set(outcome.roots);
                self.toasts.push(Toast::success(message));
                true
            }
            Err(error) => {
                self.toasts.push(Toast::error(format!(
                    "The design from {} could not be added: {error}",
                    origin(pending)
                )));
                false
            }
        }
    }

    /// Adds a design pasted from the plugin's Copy button, as a sent one is.
    /// Returns false when `text` is not an import envelope at all.
    pub(super) fn paste_envelope(&mut self, text: &str) -> bool {
        if !text.contains(FORMAT) {
            return false;
        }
        match read_envelope(text.as_bytes()) {
            Ok(pending) => {
                self.import_design(&pending);
                true
            }
            // Layers that merely mention the word are pasted as layers.
            Err(EnvelopeError::NotJson(_) | EnvelopeError::NotAnEnvelope(_)) => false,
            Err(error) => {
                self.toasts.push(Toast::error(format!(
                    "The pasted design cannot be added: {error}"
                )));
                true
            }
        }
    }

    /// The "Connect an agent" dialog, while it is open.
    pub(super) fn connect_dialog(&mut self, ctx: &Context) {
        if !self.connect_open {
            return;
        }
        let theme = Theme::of_ctx(ctx);
        let c = &theme.color;
        let status = self.server.status().clone();
        let mut allowed = self.settings.agents.serve;
        let mut retry = false;
        let mut done = false;
        let mut copied = None;
        let shown = ModalFrame::new("Connect an agent")
            .width(520.0)
            .show(ctx, |ui| {
                status_line(ui, &status);
                let explanation = match &status {
                    ServerStatus::Running { url } => format!(
                        "Agents on this computer work on the open project at {url}. Their \
                         changes appear here as they make them, and you can undo them."
                    ),
                    ServerStatus::Unavailable { reason, .. } => reason.clone(),
                    ServerStatus::Off => {
                        "Agents and the Figma plugin cannot reach this window.".to_owned()
                    }
                };
                ui.label(
                    RichText::new(explanation)
                        .font(theme.body())
                        .color(c.text_secondary),
                );
                ui.checkbox(
                    &mut allowed,
                    RichText::new("Let agents on this computer connect")
                        .font(theme.body())
                        .color(c.text),
                );
                if let ServerStatus::Running { url } = &status {
                    ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = theme.space.sm;
                        for setup in client_setups(url) {
                            ui.add_space(theme.space.xs);
                            ui.label(
                                RichText::new(setup.name)
                                    .font(theme.body_strong())
                                    .color(c.text),
                            );
                            ui.label(
                                RichText::new(setup.how)
                                    .font(theme.small())
                                    .color(c.text_secondary),
                            );
                            if code_block(ui, &setup.snippet) {
                                copied = Some(setup.snippet.clone());
                            }
                        }
                        ui.add_space(theme.space.xs);
                        ui.label(
                            RichText::new(format!(
                                "Agents that start servers themselves can run “{}” on a saved \
                                 file, without this window.",
                                stdio_command()
                            ))
                            .font(theme.small())
                            .color(c.text_secondary),
                        );
                    });
                }
                modal_footer(ui, |ui| {
                    if ui.add(TextButton::primary("Done")).clicked() {
                        done = true;
                    }
                    if matches!(status, ServerStatus::Unavailable { .. })
                        && ui
                            .add(TextButton::new("Try again").kind(ButtonKind::Quiet))
                            .clicked()
                    {
                        retry = true;
                    }
                });
            });
        if let Some(text) = copied {
            ctx.copy_text(text);
            self.toasts.push(Toast::success("Copied"));
        }
        if allowed != self.settings.agents.serve {
            self.allow_agents(allowed);
        } else if retry {
            self.restart_agent_server();
        }
        if done || shown.close_requested {
            self.connect_open = false;
        }
    }
}

/// The server's status as a coloured dot and a few words.
fn status_line(ui: &mut Ui, status: &ServerStatus) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let colour = match AgentLight::of(status) {
        AgentLight::Off => c.text_tertiary,
        AgentLight::Running => c.success,
        AgentLight::Unavailable => c.warning,
    };
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.0, colour);
        ui.label(
            RichText::new(status.headline())
                .font(theme.body_strong())
                .color(c.text),
        );
    });
}

/// A command or configuration in a box, with a copy button. Returns whether
/// the button was clicked.
fn code_block(ui: &mut Ui, text: &str) -> bool {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut copy = false;
    Frame::new()
        .fill(c.well)
        .corner_radius(corner(theme.radius.sm))
        .inner_margin(Margin::same(theme.space.md as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let copy_width = theme.size.control + theme.space.xs;
                let width = ui.available_width() - copy_width;
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        ui.add(
                            egui::Label::new(RichText::new(text).font(theme.mono()).color(c.text))
                                .wrap(),
                        );
                    },
                );
                copy = ui
                    .add(IconButton::new(icons::COPY).tooltip("Copy"))
                    .clicked();
            });
        });
    copy
}

/// `1 layer`, `2 layers`.
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use moshionly_render::{AssetLoader, FontLibrary, Renderer};

    use super::*;
    use crate::settings::Settings;

    const ENVELOPE: &str = include_str!("../../tests/fixtures/figma-hero.json");

    fn shell() -> Shell {
        let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
        Shell::headless(
            Settings::default(),
            super::super::untitled_session(),
            renderer,
        )
    }

    #[test]
    fn a_pasted_envelope_is_added_without_asking() {
        let mut shell = shell();
        let before = shell.editor().document().artboards().len();
        assert!(shell.paste_envelope(ENVELOPE));
        let after = shell.editor().document().artboards().len();
        assert!(after > before, "the design came in as artboards");
        // Layers that only mention the format are pasted as layers.
        assert!(!shell.paste_envelope(r#"[{"name":"moshionly-import"}]"#));
        assert!(!shell.paste_envelope("plain text"));
        // An envelope this app cannot read says so, and adds nothing.
        assert!(shell.paste_envelope(r#"{"format":"moshionly-import","version":99}"#));
        assert_eq!(shell.editor().document().artboards().len(), after);
    }

    #[test]
    fn the_minute_allowance_counts_designs_and_bytes() {
        let mut shell = shell();
        let now = std::time::Instant::now();
        assert!(!shell.import_limit_reached());
        shell
            .recent_imports
            .push_back((now, MAX_IMPORT_BYTES_PER_WINDOW));
        assert!(
            shell.import_limit_reached(),
            "one large design uses up the minute's bytes"
        );
        shell.recent_imports.clear();
        for _ in 0..MAX_IMPORTS_PER_WINDOW {
            assert!(!shell.import_limit_reached());
            shell.recent_imports.push_back((now, 1));
        }
        assert!(shell.import_limit_reached());
    }

    #[test]
    fn the_same_design_again_updates_what_came_before() {
        let mut shell = shell();
        let pending = read_envelope(ENVELOPE.as_bytes()).unwrap();
        assert!(shell.import_design(&pending));
        let once = shell.editor().document().artboards().len();
        assert!(shell.import_design(&pending));
        assert_eq!(
            shell.editor().document().artboards().len(),
            once,
            "the second send replaced the first instead of adding a copy"
        );
        assert_eq!(AgentLight::of(&ServerStatus::Off), AgentLight::Off);
    }
}

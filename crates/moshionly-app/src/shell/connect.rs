//! The shell's connections: the MCP server agents reach the open document
//! through, the "Connect an agent" dialog, and designs sent from Figma
//! waiting for the person to accept them.

use egui::{Context, Frame, Margin, RichText, ScrollArea, Ui, UserAttentionType, ViewportCommand};
use moshionly_agent::Tools;
use moshionly_agent::import::{EnvelopeError, FORMAT, PendingImport, read_envelope};
use moshionly_export::CancelToken;

use super::{QUIT_PATIENCE, Shell};
use crate::{
    connect::{
        AgentServer, ServerStatus,
        clients::{client_setups, stdio_command},
        import::{ImportChoice, contents, import_target, is_possible, origin, suggested_choice},
    },
    host::cancellable_app_host,
    theme::{Theme, corner, icons},
    widgets::{ButtonKind, IconButton, ModalFrame, TextButton, Toast, modal_footer},
};

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

    /// Takes the next design sent from Figma, when none is being shown.
    /// With a window, it is brought forward. [`show`](Shell::show) calls
    /// this every frame.
    pub fn receive_imports(&mut self, ctx: Option<&Context>) {
        if !self.imports.is_empty() {
            return;
        }
        let Some(pending) = self.server.take_import() else {
            return;
        };
        self.offer_import(pending);
        if let Some(ctx) = ctx {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                UserAttentionType::Informational,
            ));
        }
    }

    /// Asks the person where a design goes, after any already waiting.
    pub fn offer_import(&mut self, pending: PendingImport) {
        if self.imports.is_empty() {
            self.import_choice = suggested_choice(&pending, self.editor.document());
        }
        self.imports.push_back(pending);
    }

    /// The design the import dialog is asking about.
    pub fn pending_import(&self) -> Option<&PendingImport> {
        self.imports.front()
    }

    /// Where the design asked about would go.
    pub fn import_choice(&self) -> ImportChoice {
        self.import_choice
    }

    /// Chooses where the design asked about goes.
    pub fn set_import_choice(&mut self, choice: ImportChoice) {
        self.import_choice = choice;
    }

    /// Adds the design asked about where chosen, as one undo step, and
    /// selects what arrived. Returns whether it was added.
    pub fn accept_import(&mut self) -> bool {
        let Some(pending) = self.imports.pop_front() else {
            return false;
        };
        self.finish_work_in_progress();
        let artboard = self.editor.artboard().cloned();
        let added = match import_target(self.import_choice, artboard.as_ref()) {
            None => {
                self.toasts
                    .push(Toast::error("There is no artboard to put the design into"));
                false
            }
            Some(target) => match pending.apply(self.editor.session(), &target) {
                Ok(outcome) => {
                    if self.editor.sync() {
                        self.after_document_change();
                    }
                    let mut message =
                        format!("Added {} from {}", contents(&pending), origin(&pending));
                    if outcome.replaced > 0 {
                        message = format!(
                            "Replaced {} from {} and kept their animations",
                            plural(outcome.replaced, "layer"),
                            origin(&pending)
                        );
                    }
                    if outcome.dropped_animations > 0 {
                        message.push_str(&format!(
                            "; {} no longer had a layer to move",
                            plural(outcome.dropped_animations, "animation")
                        ));
                    }
                    self.editor.selection.set(outcome.roots);
                    self.toasts.push(Toast::success(message));
                    true
                }
                Err(error) => {
                    self.toasts.push(Toast::error(format!(
                        "The design could not be added: {error}"
                    )));
                    false
                }
            },
        };
        self.next_import();
        added
    }

    /// Throws the design asked about away.
    pub fn reject_import(&mut self) {
        if self.imports.pop_front().is_some() {
            self.toasts
                .push(Toast::info("The design from Figma was not added"));
        }
        self.next_import();
    }

    /// Prepares the dialog for the next design waiting, if any.
    fn next_import(&mut self) {
        if let Some(next) = self.imports.front() {
            self.import_choice = suggested_choice(next, self.editor.document());
        }
    }

    /// Opens the import dialog for a design pasted from the plugin's Copy
    /// button. Returns false when `text` is not an import envelope at all.
    pub(super) fn paste_envelope(&mut self, text: &str) -> bool {
        if !text.contains(FORMAT) {
            return false;
        }
        match read_envelope(text.as_bytes()) {
            Ok(pending) => {
                self.offer_import(pending);
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

    /// The dialog asking where a design sent from Figma goes, while one is
    /// waiting.
    pub(super) fn import_dialog(&mut self, ctx: &Context) {
        let Some(pending) = self.imports.front() else {
            return;
        };
        let theme = Theme::of_ctx(ctx);
        let c = &theme.color;
        let document = std::sync::Arc::clone(self.editor.document());
        let artboard = self.editor.artboard().cloned();
        let waiting = self.imports.len() - 1;
        let mut choice = self.import_choice;
        let mut answer = None;
        let shown = ModalFrame::new("Design from Figma")
            .width(460.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("{} from {}", contents(pending), origin(pending)))
                        .font(theme.body_strong())
                        .color(c.text),
                );
                ui.label(
                    RichText::new("Nothing changes in your project until you accept it.")
                        .font(theme.body())
                        .color(c.text_secondary),
                );
                if !pending.notes.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "What is shown differently ({})",
                            pending.notes.len()
                        ))
                        .font(theme.body_medium())
                        .color(c.text),
                    );
                    ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = theme.space.xs;
                        for note in &pending.notes {
                            let text = match &note.layer_name {
                                Some(name) => format!("{name}: {}", note.message),
                                None => note.message.clone(),
                            };
                            ui.label(
                                RichText::new(text)
                                    .font(theme.small())
                                    .color(c.text_secondary),
                            );
                        }
                    });
                }
                ui.label(
                    RichText::new("Where should it go?")
                        .font(theme.body_medium())
                        .color(c.text),
                );
                for option in ImportChoice::ALL {
                    let possible = is_possible(option, pending, &document, artboard.as_ref());
                    ui.add_enabled_ui(possible, |ui| {
                        if ui
                            .radio(
                                choice == option,
                                RichText::new(option.label()).font(theme.body()),
                            )
                            .clicked()
                        {
                            choice = option;
                        }
                    });
                }
                if waiting > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} more waiting after this one",
                            plural(waiting, "design")
                        ))
                        .font(theme.small())
                        .color(c.text_tertiary),
                    );
                }
                modal_footer(ui, |ui| {
                    let ready = is_possible(choice, pending, &document, artboard.as_ref());
                    if ui
                        .add_enabled(ready, TextButton::primary("Accept"))
                        .clicked()
                    {
                        answer = Some(true);
                    }
                    if ui.add(TextButton::new("Reject")).clicked() {
                        answer = Some(false);
                    }
                });
            });
        self.import_choice = choice;
        match answer {
            Some(true) => {
                self.accept_import();
            }
            Some(false) => self.reject_import(),
            None if shown.close_requested => self.reject_import(),
            None => {}
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
    fn a_pasted_envelope_asks_where_it_goes() {
        let mut shell = shell();
        assert!(shell.paste_envelope(ENVELOPE));
        assert!(shell.pending_import().is_some());
        // Layers that only mention the format are pasted as layers.
        assert!(!shell.paste_envelope(r#"[{"name":"moshionly-import"}]"#));
        assert!(!shell.paste_envelope("plain text"));
        // An envelope this app cannot read says so, and is not pasted.
        assert!(shell.paste_envelope(r#"{"format":"moshionly-import","version":99}"#));
        assert_eq!(shell.imports.len(), 1);
        shell.reject_import();
        assert!(shell.pending_import().is_none());
    }

    #[test]
    fn designs_wait_their_turn() {
        let mut shell = shell();
        let pending = read_envelope(ENVELOPE.as_bytes()).unwrap();
        shell.offer_import(pending.clone());
        shell.offer_import(pending);
        shell.set_import_choice(ImportChoice::IntoArtboard);
        assert!(shell.accept_import());
        assert_eq!(shell.imports.len(), 1);
        // The next one starts on its own suggestion.
        assert_eq!(shell.import_choice(), ImportChoice::NewArtboards);
        assert_eq!(AgentLight::of(&ServerStatus::Off), AgentLight::Off);
    }
}

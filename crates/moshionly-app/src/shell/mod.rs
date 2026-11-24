//! The shell: the window's layout and everything in it.
//!
//! [`Shell`] owns the editor window: the [`Editor`] with the open document,
//! the pictures of its artboards, the panel sizes, the theme choice, which
//! inspector tab is showing, toasts. [`Shell::show`] draws the whole window
//! once per frame and returns the commands it could not carry out itself.

mod canvas_panel;
mod chat_panel;
mod connect;
mod edit_commands;
mod export_dialog;
mod files;
mod inspector;
mod layers_panel;
pub mod layout;
mod parts;
mod tool_bar;
mod top_bar;

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use egui::{Context, CursorIcon, Pos2, Rect, Ui, ViewportCommand, pos2};
use moshionly_agent::import::PendingImport;
use moshionly_agent::{Session, SessionHandle, Tools};
use moshionly_core::{Document, NodeId, build};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};

use self::{
    canvas_panel::CanvasDrag,
    parts::{Edge, edge_line},
    top_bar::TopBar,
};
pub use self::{
    connect::AgentLight,
    files::{Answer, FileAction, carry_on},
    inspector::{InspectorState, InspectorTab},
    layout::{CANVAS_MIN, PanelLimits, PanelSizes, ShellLayout},
    tool_bar::Tool,
};
use crate::{
    canvas::{CanvasRenderer, Quality},
    commands::{CommandId, CommandRegistry, Palette},
    connect::{AgentServer, import::ImportChoice},
    dialogs::{Dialogs, NoDialogs, is_image, is_project},
    editor::{
        Editor,
        actions::{ActionError, targets_of, with_emptied_folders},
        pen::Pen,
        playback::{Playback, step_frames},
        text::TextEditing,
        timeline::{is_folder, timeline_groups},
        tree::TreeState,
    },
    key::Key,
    settings::Settings,
    theme::{self, Theme, ThemeMode},
    timeline::{BarCommand, Timeline, TimelineEvent, TimelineLayout, TimelineState},
    viewport::Camera,
    widgets::{Grow, Toast, Toasts, splitter},
};

/// The editor window.
pub struct Shell {
    settings: Settings,
    commands: CommandRegistry,
    inspector: InspectorState,
    toasts: Toasts,
    palette: Palette,
    /// The playhead tied to the clock while playing.
    playback: Playback,
    /// The window's clock, in seconds, as of this frame.
    now: f64,
    /// How sharp the canvas is drawn while playing.
    quality: Quality,
    looping: bool,
    snapping: bool,
    timeline: TimelineState,
    camera: Camera,
    /// The canvas rectangle of the last frame; `None` before the first one.
    canvas: Option<Rect>,
    /// The camera has been fitted to the document since it was opened.
    fitted: bool,
    /// Set when the top bar is also the window's title area: the room to
    /// leave at its left edge for the window's buttons.
    title_inset: Option<f32>,
    /// The open document and the editor's state around it.
    editor: Editor,
    /// The pictures of the artboards.
    pictures: CanvasRenderer,
    /// Which rows of the layer list are open.
    tree: TreeState,
    /// What is typed into the layer search.
    layer_query: String,
    /// The selection as of the last frame, to notice changes.
    shown_selection: Vec<NodeId>,
    /// The selected actions as of the last frame, to notice changes.
    shown_actions: Vec<NodeId>,
    /// Scroll the layer list to this row on the next frame.
    reveal_row: Option<NodeId>,
    /// The layer row being renamed, with the name typed so far.
    renaming: Option<(NodeId, String)>,
    /// The layers whose rows are being dragged in the list.
    list_drag: Option<Vec<NodeId>>,
    /// The timeline groups whose rows are folded away.
    collapsed_groups: HashSet<NodeId>,
    /// A drag on the timeline holds an undo step open.
    timeline_edit: bool,
    /// Where the parts of the timeline were drawn in the last frame.
    timeline_layout: Option<TimelineLayout>,
    /// The drag in progress on the canvas.
    drag: Option<CanvasDrag>,
    /// Snapping guides to draw while dragging.
    guides: Vec<[Pos2; 2]>,
    /// The world position under the pointer, while it is over the canvas.
    pointer: Option<Pos2>,
    /// A path being drawn with the pen.
    pen: Option<Pen>,
    /// Text being edited on the canvas.
    text_edit: Option<TextEditing>,
    /// How files are chosen.
    dialogs: Box<dyn Dialogs>,
    /// The last layers copied in this window, for a paste chosen from a
    /// menu (a paste by keyboard brings the system clipboard's text).
    clipboard: Option<String>,
    /// An action waiting for an answer about unsaved changes.
    pending: Option<FileAction>,
    /// The window may close: the person said so.
    closing: bool,
    /// The AI chat in the chat panel, once the app set one.
    chat: Option<Box<crate::chat::Chat>>,
    /// The agent tools over the open document, for the MCP server and the
    /// chat.
    tools: Tools,
    /// The MCP server agents connect to.
    server: AgentServer,
    /// The port to serve agents on, once the app asked for a server.
    agent_port: Option<u16>,
    /// The "Connect an agent" dialog is open.
    connect_open: bool,
    /// Designs sent from Figma, waiting for the person: the first is being
    /// asked about.
    imports: VecDeque<PendingImport>,
    /// Where the design asked about would go.
    import_choice: ImportChoice,
    /// The Export dialog, while it is open.
    export: Option<export_dialog::ExportDialog>,
}

/// What [`Shell::show`] hands back to the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellOutput {
    /// Commands the user asked for that the shell does not carry out itself:
    /// exporting and the like.
    pub commands: Vec<CommandId>,
    /// The person agreed to close the window: close it.
    pub close_window: bool,
}

/// How long quitting waits for work in the background to clean up after
/// it was cancelled.
const QUIT_PATIENCE: Duration = Duration::from_secs(2);

/// A new document: one empty 1080 by 1080 artboard named "Scene 1".
pub fn new_document() -> Document {
    let mut document = Document::new("Untitled");
    if let Ok((_, edits)) = build::add_artboard(&document, "Scene 1", 1080.0, 1080.0) {
        // A fresh document always takes a first artboard.
        let _ = document.apply(&edits);
    }
    document
}

/// A session on a new document with nothing in it worth saving yet: it
/// counts as unsaved only once something changes.
pub fn untitled_session() -> SessionHandle {
    let mut session = Session::new(new_document());
    session.mark_clean();
    SessionHandle::new(session)
}

impl Shell {
    /// A shell on `session`, drawing with the bundled and installed fonts
    /// on worker threads.
    pub fn new(settings: Settings, session: SessionHandle) -> Self {
        let renderer = Renderer::new(FontLibrary::with_system_fonts(), AssetLoader::default());
        let pictures = CanvasRenderer::new(renderer.clone());
        Self::build(settings, session, renderer, pictures)
    }

    /// A shell on `session` that draws with `renderer` and renders the
    /// canvas on the calling thread, so that every frame shows finished
    /// pictures. For tests and screenshots: the bundled fonts look the same
    /// on every machine.
    pub fn headless(settings: Settings, session: SessionHandle, renderer: Renderer) -> Self {
        let pictures = CanvasRenderer::inline(renderer.clone());
        Self::build(settings, session, renderer, pictures)
    }

    /// A shell on a new, untitled document.
    pub fn untitled(settings: Settings) -> Self {
        Self::new(settings, untitled_session())
    }

    fn build(
        settings: Settings,
        session: SessionHandle,
        renderer: Renderer,
        pictures: CanvasRenderer,
    ) -> Self {
        let tools = Tools::new(session.clone(), crate::host::app_host(&renderer));
        Self {
            settings,
            commands: CommandRegistry::new(),
            inspector: InspectorState::default(),
            toasts: Toasts::default(),
            palette: Palette::default(),
            playback: Playback::new(),
            now: 0.0,
            quality: Quality::new(),
            looping: true,
            snapping: true,
            timeline: TimelineState::default(),
            camera: Camera::default(),
            canvas: None,
            fitted: false,
            title_inset: None,
            editor: Editor::new(session, renderer),
            pictures,
            tree: TreeState::default(),
            layer_query: String::new(),
            shown_selection: Vec::new(),
            shown_actions: Vec::new(),
            reveal_row: None,
            renaming: None,
            list_drag: None,
            collapsed_groups: HashSet::new(),
            timeline_edit: false,
            timeline_layout: None,
            drag: None,
            guides: Vec::new(),
            pointer: None,
            pen: None,
            text_edit: None,
            dialogs: Box::new(NoDialogs),
            clipboard: None,
            pending: None,
            closing: false,
            chat: None,
            tools,
            server: AgentServer::off(),
            agent_port: None,
            connect_open: false,
            imports: VecDeque::new(),
            import_choice: ImportChoice::NewArtboards,
            export: None,
        }
    }

    /// Stops what runs in the background before the app quits: a running
    /// export, and the work agents asked for, are cancelled and get a
    /// moment to clean up, so no half-written file is left behind. The app
    /// calls this as it exits.
    pub fn prepare_to_quit(&mut self) {
        // Closing the dialog cancels its export and waits for it to stop.
        self.export = None;
        self.shut_down_agent_server();
    }

    /// Tells the shell that its top bar doubles as the window's title bar,
    /// as on macOS with a full-size content view. The bar then leaves `inset`
    /// points free at its left edge for the window's own buttons, and its
    /// empty space drags the window.
    pub fn use_top_bar_as_title_bar(&mut self, inset: f32) {
        self.title_inset = Some(inset);
    }

    /// Says how files are chosen: the app gives the operating system's
    /// dialogs; until then every dialog is cancelled.
    pub fn set_dialogs(&mut self, dialogs: Box<dyn Dialogs>) {
        self.dialogs = dialogs;
    }

    /// Whether text is being edited on the canvas.
    pub fn is_editing_text(&self) -> bool {
        self.text_edit.is_some()
    }

    /// The name of the open project: the document's name, or its file's.
    pub fn project_name(&self) -> String {
        let name = self.editor.document().name.trim();
        if !name.is_empty() {
            return name.to_owned();
        }
        self.editor
            .session()
            .path()
            .and_then(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Untitled".to_owned())
    }

    /// The editor: the open document and the state around it.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// The editor, to change the selection or the document.
    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    /// The pictures of the artboards.
    pub fn pictures(&self) -> &CanvasRenderer {
        &self.pictures
    }

    /// The active tool.
    pub fn tool(&self) -> Tool {
        self.editor.tool
    }

    /// The canvas camera.
    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// The canvas camera.
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// The canvas rectangle of the last frame.
    pub fn canvas_rect(&self) -> Option<Rect> {
        self.canvas
    }

    /// Where the parts of the timeline were drawn in the last frame.
    pub fn timeline_layout(&self) -> Option<TimelineLayout> {
        self.timeline_layout
    }

    /// Whether the animation is playing.
    pub fn is_playing(&self) -> bool {
        self.playback.is_playing()
    }

    /// The settings as they are now, for saving.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The sizes of the panels. Changing them moves the panels.
    pub fn panels_mut(&mut self) -> &mut PanelSizes {
        &mut self.settings.panels
    }

    /// The tab the inspector shows.
    pub fn inspector_tab(&self) -> InspectorTab {
        self.inspector.tab
    }

    /// The inspector's state, for choosing the tab.
    pub fn inspector_mut(&mut self) -> &mut InspectorState {
        &mut self.inspector
    }

    /// Which commands are enabled.
    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        &mut self.commands
    }

    /// The command palette, for opening it from outside.
    pub fn palette_mut(&mut self) -> &mut Palette {
        &mut self.palette
    }

    /// Shows a short message over the canvas.
    pub fn toast(&mut self, toast: Toast) {
        self.toasts.push(toast);
    }

    /// Draws the whole window. Call once per frame with the window's root
    /// `Ui`; the shell fills the window regardless of that `Ui`'s cursor.
    pub fn show(&mut self, ui: &mut Ui) -> ShellOutput {
        let ctx = ui.ctx().clone();
        let theme = Theme::of(ui);
        let c = &theme.color;
        let window = ctx.content_rect();
        let layout = ShellLayout::compute(window, &self.settings.panels, &theme.size);
        let mut requested: Vec<CommandId> = Vec::new();
        // Escape closes an open menu or the palette, and ends a path or text
        // being edited, before it means anything else.
        let overlay_open = egui::Popup::is_any_open(&ctx)
            || self.palette.is_open()
            || self.pending.is_some()
            || self.text_edit.is_some()
            || self.pen.is_some()
            || self.renaming.is_some()
            || self.list_drag.is_some()
            || self.connect_open
            || !self.imports.is_empty()
            || self.export.is_some();

        self.now = ctx.input(|i| i.time);
        self.editor.attach(&ctx);
        self.server.attach(&ctx);
        self.receive_imports(Some(&ctx));
        if self.editor.sync() {
            self.after_document_change();
        }
        if self.editor.take_project_change() {
            // Relative image paths now lead somewhere else.
            self.pictures.clear();
        }
        self.pictures.poll(&ctx);
        if self.playback.is_playing() {
            let times = self.pictures.times();
            self.quality.observe_renders(times.finished(), times.last());
        }
        let animate = self.inspector.tab == InspectorTab::Animate;
        if self.editor.is_animating() && !animate {
            // The Design tab works on layers and does not show actions:
            // leaving the Animate tab lets go of the selected ones.
            self.editor.set_actions([]);
        }
        self.editor.set_animating(animate);
        self.follow_selection();
        self.dropped_files(&ctx);
        self.pen_keys(&ctx);

        // The canvas first, so the panels' edges are drawn over it.
        requested.extend(self.canvas_panel(ui, layout.canvas));

        // In full screen the window's buttons are hidden, so the inset goes.
        let fullscreen = ctx.input(|i| i.viewport().fullscreen == Some(true));
        let project_name = self.project_name();
        let recent = self.settings.recent.clone();
        let agents_status = self.server.status().headline();
        let bar = TopBar {
            project_name: &project_name,
            recent: &recent,
            unsaved: self.editor.has_unsaved_changes(),
            zoom: self.camera.zoom,
            playing: self.playback.is_playing(),
            chat_open: self.settings.panels.chat_open,
            agents: AgentLight::of(self.server.status()),
            agents_status: &agents_status,
            theme_mode: self.settings.theme,
            commands: &self.commands,
            leading_inset: self.title_inset.filter(|_| !fullscreen).unwrap_or(0.0),
            drags_window: self.title_inset.is_some(),
        };
        let top = parts::panel(ui, "top-bar", layout.top_bar, c.surface, |ui| {
            top_bar::top_bar(ui, layout.top_bar, &bar)
        });
        edge_line(ui, layout.top_bar, Edge::Bottom);
        requested.extend(top.commands);
        if let Some(path) = top.open_recent {
            self.request(FileAction::Open(Some(path)));
        }
        if top.drag_window {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if top.toggle_maximized {
            let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        let reveal = self.reveal_row.take();
        let layers = parts::panel(ui, "layers", layout.left, c.surface, |ui| {
            layers_panel::layers_panel(
                ui,
                &mut layers_panel::LayersPanel {
                    editor: &mut self.editor,
                    tree: &mut self.tree,
                    query: &mut self.layer_query,
                    registry: &self.commands,
                    reveal: reveal.as_ref(),
                    renaming: &mut self.renaming,
                    dragging: &mut self.list_drag,
                },
            )
        });
        edge_line(ui, layout.left, Edge::Right);
        requested.extend(layers.commands);
        if let Some(message) = layers.error {
            self.toasts.push(Toast::error(message));
        }

        let canvas_gesture = self.canvas_gesture();
        let inspected = parts::panel(ui, "inspector", layout.right, c.surface, |ui| {
            inspector::inspector(ui, &mut self.inspector, &mut self.editor, canvas_gesture)
        });
        edge_line(ui, layout.right, Edge::Left);
        if let Some(message) = inspected.error {
            self.toasts.push(Toast::error(message));
        }
        if inspected.replace_image {
            self.replace_image_dialog();
        }

        self.advance_playback(&ctx);
        self.timeline_panel(ui, &layout);

        if self.chat_panel(ui, layout.chat) {
            requested.push(CommandId::ToggleChat);
        }

        self.splitters(ui, window, &layout);

        // Toasts stack upwards from just above the tool bar.
        let toast_anchor = tool_bar::tool_bar_rect(layout.canvas, theme).center_top()
            - egui::vec2(0.0, theme.space.lg);
        self.toasts.show(&ctx, toast_anchor);

        requested.extend(self.palette.show(&ctx, &self.commands));
        self.connect_dialog(&ctx);
        self.export_dialog(&ctx);
        self.import_dialog(&ctx);
        self.unsaved_prompt(&ctx);
        if !self.palette.is_open() {
            self.nudge(&ctx);
        }
        requested.extend(
            self.commands
                .triggered(&ctx)
                .into_iter()
                .filter(|id| !(overlay_open && *id == CommandId::Deselect)),
        );
        let mut output = ShellOutput::default();
        for id in requested {
            if !self.run(&ctx, id) {
                output.commands.push(id);
            }
        }
        self.update_commands();
        output.close_window = self.closing;
        output
    }

    /// Whether the canvas holds an undo step open: a drag of the selection,
    /// a layer being drawn, or text being added.
    fn canvas_gesture(&self) -> bool {
        matches!(
            self.drag,
            Some(CanvasDrag::Selection(_) | CanvasDrag::Create(_) | CanvasDrag::Animate(_))
        ) || self
            .text_edit
            .as_ref()
            .is_some_and(|editing| editing.created)
    }

    /// Images dropped on the window are placed where they were dropped.
    fn dropped_files(&mut self, ctx: &Context) {
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_owned())
                .collect()
        });
        let images: Vec<&std::path::Path> = dropped
            .iter()
            .map(std::path::PathBuf::as_path)
            .filter(|path| is_image(path))
            .collect();
        if !images.is_empty() {
            let at = self.pointer;
            self.import_images(&images, at);
        }
        // A project dropped on the window is opened.
        if let Some(project) = dropped.iter().find(|path| is_project(path)) {
            self.request(FileAction::Open(Some(project.clone())));
        }
    }

    /// Asks for images and places them on the current artboard.
    fn import_image_dialog(&mut self) {
        self.editor.tool = Tool::Select;
        if let Some(paths) = self.dialogs.import_images() {
            let paths: Vec<&std::path::Path> =
                paths.iter().map(std::path::PathBuf::as_path).collect();
            self.import_images(&paths, None);
        }
    }

    /// Asks for an image to show in the selected image layers instead.
    fn replace_image_dialog(&mut self) {
        let document = std::sync::Arc::clone(self.editor.document());
        let images: Vec<NodeId> = self
            .editor
            .selection
            .ids()
            .iter()
            .filter(|id| {
                document
                    .get(id)
                    .is_some_and(|node| node.node_type() == moshionly_core::NodeType::Image)
            })
            .cloned()
            .collect();
        let Some(path) = self
            .dialogs
            .import_images()
            .and_then(|paths| paths.into_iter().next())
        else {
            return;
        };
        if let Err(problem) = self.editor.replace_image(&images, &path) {
            self.toasts.push(Toast::error(problem));
        }
    }

    /// Places image files as image layers, and says which could not be used.
    fn import_images(&mut self, paths: &[&std::path::Path], at: Option<Pos2>) {
        let (_, problems) = self.editor.import_images(paths, at);
        for problem in problems {
            self.toasts.push(Toast::error(problem));
        }
    }

    /// Enter and Escape finish a path drawn with the pen.
    fn pen_keys(&mut self, ctx: &Context) {
        if self.pen.is_none() || ctx.text_edit_focused() {
            return;
        }
        let finish = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
        });
        if finish {
            self.finish_pen(false);
        }
    }

    /// The arrow keys move the selection by a pixel, or ten with Shift,
    /// unless a text field has the keyboard.
    fn nudge(&mut self, ctx: &Context) {
        if ctx.text_edit_focused() || self.editor.selection.is_empty() || self.drag.is_some() {
            return;
        }
        let mut delta = egui::Vec2::ZERO;
        ctx.input_mut(|input| {
            for (key, direction) in [
                (egui::Key::ArrowLeft, egui::vec2(-1.0, 0.0)),
                (egui::Key::ArrowRight, egui::vec2(1.0, 0.0)),
                (egui::Key::ArrowUp, egui::vec2(0.0, -1.0)),
                (egui::Key::ArrowDown, egui::vec2(0.0, 1.0)),
            ] {
                // Shift first: a plain pattern would take shifted presses too.
                let shifted = input.count_and_consume_key(egui::Modifiers::SHIFT, key);
                let plain = input.count_and_consume_key(egui::Modifiers::NONE, key);
                delta += direction * (plain as f32 + 10.0 * shifted as f32);
            }
        });
        if delta != egui::Vec2::ZERO
            && let Err(error) = self.editor.nudge(delta)
        {
            self.toasts.push(Toast::error(error.problem.message));
        }
    }

    /// Forgets view state about nodes that are gone.
    fn after_document_change(&mut self) {
        let document = std::sync::Arc::clone(self.editor.document());
        self.pictures.retain(&document);
        self.tree.retain(&document);
        self.collapsed_groups.retain(|id| document.contains(id));
    }

    /// When the selection changes, its rows come into view in the layer
    /// list and its actions in the timeline, and its artboard becomes the
    /// current one.
    fn follow_selection(&mut self) {
        // Newly selected actions come into view on the timeline.
        if self.editor.actions() != self.shown_actions.as_slice() {
            self.shown_actions = self.editor.actions().to_vec();
            if let Some(first) = self.shown_actions.first() {
                self.timeline.reveal(Key::new(first.as_str()));
            }
        }
        let selection = self.editor.selection.ids().to_vec();
        if selection == self.shown_selection {
            return;
        }
        let document = std::sync::Arc::clone(self.editor.document());
        for id in &selection {
            self.tree.reveal(&document, id);
        }
        // Actions stay selected only while the layers they animate are; a
        // folder while a layer its actions animate is.
        let kept: Vec<NodeId> = self
            .editor
            .actions()
            .iter()
            .filter(|action| {
                targets_of(&document, std::slice::from_ref(action))
                    .iter()
                    .any(|target| selection.contains(target))
            })
            .cloned()
            .collect();
        if kept.len() != self.editor.actions().len() {
            self.editor.set_actions(kept);
        }
        self.reveal_row = selection.first().cloned();
        if let Some(first) = selection.first() {
            self.timeline.reveal(Key::new(first.as_str()));
        }
        self.editor.follow_selection();
        self.shown_selection = selection;
    }

    /// The timeline at the bottom: the current artboard's actions.
    fn timeline_panel(&mut self, ui: &mut Ui, layout: &ShellLayout) {
        let c = &Theme::of(ui).color;
        let document = std::sync::Arc::clone(self.editor.document());
        let artboard = self.editor.artboard().cloned();
        let groups = artboard
            .as_ref()
            .map(|id| timeline_groups(&document, id, &self.collapsed_groups))
            .unwrap_or_default();
        let duration = artboard
            .as_ref()
            .and_then(|id| moshionly_core::eval::duration_of(&document, id))
            .unwrap_or(0.0);
        // A selected folder's row is tinted, or else the selected layer's.
        let selected_folder = self
            .editor
            .actions()
            .first()
            .map(|id| Key::new(id.as_str()))
            .filter(|key| is_folder(&document, key));
        let selected_layer = selected_folder.or_else(|| {
            self.editor
                .selection
                .single()
                .map(|id| Key::new(id.as_str()))
        });
        let selected_bars: Vec<Key> = self
            .editor
            .actions()
            .iter()
            .map(|id| Key::new(id.as_str()))
            .collect();
        let timeline = parts::panel(ui, "timeline", layout.timeline, c.surface, |ui| {
            Timeline::new(&mut self.timeline, &groups)
                .selection(&selected_bars)
                .selected_group(selected_layer.as_ref())
                .playhead(self.editor.playhead())
                .duration(duration)
                .playing(self.playback.is_playing())
                .follow_playhead(self.playback.is_playing())
                .looping(self.looping)
                .snapping(self.snapping)
                .show(ui, layout.timeline)
        });
        edge_line(ui, layout.timeline, Edge::Top);
        self.timeline_layout = Some(timeline.layout);
        for event in timeline.events {
            self.timeline_event(event);
        }
    }

    /// Applies what the user did on the timeline.
    fn timeline_event(&mut self, event: TimelineEvent) {
        let node = |key: &Key| NodeId::from(key.as_str());
        match event {
            TimelineEvent::PlayheadMoved { time } => {
                // The timeline is about time: show that moment.
                self.editor.set_playhead(time);
                self.inspector.tab = InspectorTab::Animate;
            }
            TimelineEvent::PlayToggled { playing } => self.set_playing(playing),
            TimelineEvent::LoopToggled { looping } => self.looping = looping,
            TimelineEvent::SnappingToggled { snapping } => self.snapping = snapping,
            TimelineEvent::JumpedToStart => self.editor.set_playhead(0.0),
            TimelineEvent::JumpedToEnd => self.jump_to_end(),
            TimelineEvent::BarActivated { .. } => self.inspector.tab = InspectorTab::Animate,
            TimelineEvent::SelectionChanged { bars } => {
                // Selecting actions selects the layers they animate, and
                // shows their settings.
                self.editor.set_actions(bars.iter().map(node));
                self.editor.select_targets();
                if !bars.is_empty() {
                    self.inspector.tab = InspectorTab::Animate;
                }
            }
            TimelineEvent::GroupClicked { key } => {
                let document = std::sync::Arc::clone(self.editor.document());
                if is_folder(&document, &key) {
                    // The folder itself, so that Delete, Duplicate and
                    // Ungroup work on it whole, with the actions in it.
                    self.editor.select_folder(&node(&key));
                    self.editor.select_targets();
                    self.inspector.tab = InspectorTab::Animate;
                } else {
                    self.editor.selection.select(node(&key));
                }
            }
            TimelineEvent::GroupCollapsed { key, collapsed } => {
                if collapsed {
                    self.collapsed_groups.insert(node(&key));
                } else {
                    self.collapsed_groups.remove(&node(&key));
                }
            }
            TimelineEvent::GroupHidden { key, hidden } => {
                let summary = if hidden { "Hide layer" } else { "Show layer" };
                self.apply_or_toast(
                    summary,
                    &[moshionly_core::Edit::set(node(&key), "hidden", hidden)],
                );
            }
            TimelineEvent::GroupLocked { key, locked } => {
                let summary = if locked { "Lock layer" } else { "Unlock layer" };
                self.apply_or_toast(
                    summary,
                    &[moshionly_core::Edit::set(node(&key), "locked", locked)],
                );
            }
            TimelineEvent::BarMoved { key, start, end } => {
                self.timeline_drag("Move action", |editor| {
                    editor.retime_actions("Move action", &[(node(&key), start, end)])
                });
            }
            TimelineEvent::BarResized {
                key, start, end, ..
            } => {
                self.timeline_drag("Change action length", |editor| {
                    editor.retime_actions("Change action length", &[(node(&key), start, end)])
                });
            }
            TimelineEvent::DurationChanged { duration } => {
                let Some(artboard) = self.editor.artboard().cloned() else {
                    return;
                };
                self.timeline_drag("Change scene length", |editor| {
                    Ok(editor.set_scene_length(&artboard, duration)?)
                });
            }
            TimelineEvent::EditFinished => self.end_timeline_drag(),
            TimelineEvent::Command { command } => self.action_command(command),
        }
    }

    /// One step of a drag on the timeline. The first step opens an undo
    /// step that the end of the drag closes.
    fn timeline_drag(
        &mut self,
        summary: &str,
        step: impl FnOnce(&mut Editor) -> Result<bool, ActionError>,
    ) {
        if !self.timeline_edit || !self.editor.in_gesture() {
            self.end_canvas_drag();
            self.editor.begin_gesture(summary);
            self.timeline_edit = true;
        }
        if let Err(error) = step(&mut self.editor) {
            self.toasts.push(Toast::error(error.to_string()));
        }
    }

    /// Closes the undo step of a drag on the timeline, before something
    /// else changes the document.
    fn end_timeline_drag(&mut self) {
        if std::mem::take(&mut self.timeline_edit) && self.editor.in_gesture() {
            self.editor.commit_gesture();
        }
    }

    /// Carries out a command on the selected actions.
    fn action_command(&mut self, command: BarCommand) {
        let result = match command {
            BarCommand::Duplicate => self.editor.duplicate_actions().map(drop),
            BarCommand::Delete => self.editor.delete_actions().map(drop),
            BarCommand::Group => self.editor.group_actions().map(drop),
            BarCommand::Ungroup => self.editor.ungroup_actions().map(drop),
            BarCommand::Stagger => {
                let delay = self.inspector.stagger();
                self.editor.stagger_actions(delay).map(drop)
            }
        };
        if let Err(error) = result {
            self.toasts.push(Toast::error(error.to_string()));
        }
    }

    /// Moves the playhead to the end of the current scene.
    fn jump_to_end(&mut self) {
        let end = self
            .editor
            .artboard()
            .and_then(|id| moshionly_core::eval::duration_of(self.editor.document(), id))
            .unwrap_or(0.0);
        self.playback.pause();
        self.editor.set_playhead(end);
    }

    /// Applies edits as one undo step, or says in a toast why not.
    pub(super) fn apply_or_toast(&mut self, summary: &str, edits: &[moshionly_core::Edit]) -> bool {
        match self.editor.apply(summary, edits) {
            Ok(changed) => changed,
            Err(error) => {
                self.toasts.push(Toast::error(error.problem.to_string()));
                false
            }
        }
    }

    /// Enables the commands that make sense now, and names undo and redo
    /// after their steps.
    fn update_commands(&mut self) {
        let undo = self.editor.undo_label();
        let redo = self.editor.redo_label();
        self.commands.set_enabled(CommandId::Undo, undo.is_some());
        self.commands.set_enabled(CommandId::Redo, redo.is_some());
        self.commands.set_label(CommandId::Undo, undo);
        self.commands.set_label(CommandId::Redo, redo);
        let selected = !self.editor.selection.is_empty();
        for id in [CommandId::Delete, CommandId::Deselect] {
            self.commands.set_enabled(id, selected);
        }
        edit_commands::enable_edit_commands(&mut self.commands, &self.editor);
        // Selected actions can be deleted, copied and grouped on their own.
        if self.on_actions() {
            for id in [
                CommandId::Delete,
                CommandId::Duplicate,
                CommandId::Group,
                CommandId::Ungroup,
            ] {
                self.commands.set_enabled(id, true);
            }
        }
        if !self.editor.actions().is_empty() {
            self.commands.set_enabled(CommandId::Deselect, true);
        }
        let has_artboards = !self.editor.document().artboards().is_empty();
        self.commands
            .set_enabled(CommandId::SelectAll, has_artboards);
        self.commands
            .set_enabled(CommandId::ZoomToFit, has_artboards);
    }

    /// The draggable edges between the panels.
    fn splitters(&mut self, ui: &mut Ui, window: Rect, layout: &ShellLayout) {
        let size = &Theme::of(ui).size;
        let limits = ShellLayout::limits(window, &self.settings.panels, size);
        let defaults = PanelSizes::default();
        let panels = &mut self.settings.panels;

        // Each splitter works on the size the panel has on screen, which can
        // be smaller than the remembered one in a small window. The
        // remembered size only changes when the user drags.
        let drag = |ui: &mut Ui,
                    salt: &str,
                    edge: Rect,
                    grow: Grow,
                    shown: f32,
                    stored: &mut f32,
                    range: std::ops::RangeInclusive<f32>,
                    default: f32| {
            let mut value = shown;
            if splitter(ui, salt, edge, grow, &mut value, range, default).changed() {
                *stored = value;
            }
        };

        let left = layout.left;
        drag(
            ui,
            "split-left",
            Rect::from_min_max(left.right_top(), left.right_bottom()),
            Grow::Right,
            left.width(),
            &mut panels.left,
            limits.left,
            defaults.left,
        );
        let right = layout.right;
        drag(
            ui,
            "split-right",
            Rect::from_min_max(right.left_top(), right.left_bottom()),
            Grow::Left,
            right.width(),
            &mut panels.right,
            limits.right,
            defaults.right,
        );
        let timeline = layout.timeline;
        drag(
            ui,
            "split-timeline",
            Rect::from_min_max(timeline.left_top(), timeline.right_top()),
            Grow::Up,
            timeline.height(),
            &mut panels.timeline,
            limits.timeline,
            defaults.timeline,
        );
        if let Some(chat) = layout.chat {
            drag(
                ui,
                "split-chat",
                Rect::from_min_max(chat.left_top(), chat.left_bottom()),
                Grow::Left,
                chat.width(),
                &mut panels.chat,
                limits.chat,
                defaults.chat,
            );
        }
    }

    /// Whether Delete, Duplicate, Group and Ungroup work on the selected
    /// actions rather than the selected layers: when actions are selected
    /// and the Animate tab, which shows them, is showing.
    fn on_actions(&self) -> bool {
        self.inspector.tab == InspectorTab::Animate && !self.editor.actions().is_empty()
    }

    /// Carries out a command if it is one the shell owns. Returns false for
    /// the others.
    fn run(&mut self, ctx: &Context, id: CommandId) -> bool {
        // A drag on the canvas is one undo step only while nothing else
        // touches the document: a command that does ends the drag first.
        if ends_a_canvas_drag(id) {
            self.end_canvas_drag();
            self.end_timeline_drag();
        }
        match id {
            CommandId::ToggleChat => {
                self.settings.panels.chat_open = !self.settings.panels.chat_open;
            }
            CommandId::ThemeSystem => self.set_theme(ctx, ThemeMode::System),
            CommandId::ThemeLight => self.set_theme(ctx, ThemeMode::Light),
            CommandId::ThemeDark => self.set_theme(ctx, ThemeMode::Dark),
            CommandId::CommandPalette => self.palette.toggle(),
            CommandId::Undo => self.undo_or_redo(true),
            CommandId::Redo => self.undo_or_redo(false),
            CommandId::Delete if self.on_actions() => {
                self.action_command(BarCommand::Delete);
            }
            CommandId::Duplicate if self.on_actions() => {
                self.action_command(BarCommand::Duplicate);
            }
            CommandId::Group if self.on_actions() => {
                self.action_command(BarCommand::Group);
            }
            CommandId::Ungroup if self.on_actions() => {
                self.action_command(BarCommand::Ungroup);
            }
            CommandId::Delete => self.delete_selection(),
            CommandId::SelectAll => {
                let document = std::sync::Arc::clone(self.editor.document());
                let all = crate::editor::selection::select_all(
                    &document,
                    self.editor.selection.ids(),
                    self.editor.artboard(),
                );
                self.editor.selection.set(all);
            }
            CommandId::Deselect => {
                self.editor.selection.clear();
                self.editor.set_actions([]);
            }
            CommandId::PlayPause => self.set_playing(!self.playback.is_playing()),
            CommandId::JumpToStart => self.editor.set_playhead(0.0),
            CommandId::JumpToEnd => self.jump_to_end(),
            CommandId::PreviousFrame => self.step(-1),
            CommandId::NextFrame => self.step(1),
            CommandId::BackTenFrames => self.step(-10),
            CommandId::AheadTenFrames => self.step(10),
            CommandId::SwitchMode => {
                self.inspector.tab = match self.inspector.tab {
                    InspectorTab::Design => InspectorTab::Animate,
                    InspectorTab::Animate => InspectorTab::Design,
                };
            }
            CommandId::ToggleLoop => self.looping = !self.looping,
            CommandId::ZoomIn => self.camera.zoom_in(),
            CommandId::ZoomOut => self.camera.zoom_out(),
            CommandId::ZoomTo100 => self.camera.set_zoom(1.0),
            CommandId::ZoomToFit => {
                if let (Some(canvas), Some(bounds)) = (self.canvas, self.editor.content_bounds()) {
                    self.fit(canvas, bounds);
                }
            }
            CommandId::AskAi => self.settings.panels.chat_open = true,
            CommandId::ConnectAgent => self.connect_open = true,
            CommandId::Export => self.open_export_dialog(),
            CommandId::ImportImage => self.import_image_dialog(),
            other if self.edit_command(ctx, other) => {}
            CommandId::NewProject => self.request(FileAction::New),
            CommandId::OpenProject => self.request(FileAction::Open(None)),
            CommandId::Save => {
                self.save();
            }
            CommandId::SaveAs => {
                self.save_as();
            }
            other => match Tool::from_command(other) {
                Some(tool) => self.choose_tool(tool),
                None => return false,
            },
        }
        true
    }

    /// Switches to `tool`, finishing a path or text in progress first. The
    /// image tool asks for an image straight away.
    fn choose_tool(&mut self, tool: Tool) {
        if self.pen.is_some() && tool != Tool::Pen {
            self.finish_pen(false);
        }
        if self.text_edit.is_some() {
            self.finish_text_edit();
        }
        if tool == Tool::Image {
            self.import_image_dialog();
        } else {
            self.editor.tool = tool;
        }
    }

    /// Undoes or redoes one step, and says what when it was not obvious.
    fn undo_or_redo(&mut self, undo: bool) {
        let result = if undo {
            self.editor.undo()
        } else {
            self.editor.redo()
        };
        if let Err(error) = result {
            self.toasts.push(Toast::error(format!(
                "That step could not be {}: {}",
                if undo { "undone" } else { "redone" },
                error.problem
            )));
        }
    }

    /// Removes the selected layers, with their actions and the timeline
    /// folders that leaves empty.
    pub(super) fn delete_selection(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if ids.is_empty() {
            return;
        }
        let document = std::sync::Arc::clone(self.editor.document());
        let edits = match with_emptied_folders(&document, build::remove(&document, &ids)) {
            Ok(edits) => edits,
            Err(error) => {
                self.toasts.push(Toast::error(error.problem.to_string()));
                return;
            }
        };
        let summary = if ids.len() == 1 {
            "Delete layer"
        } else {
            "Delete layers"
        };
        if self.apply_or_toast(summary, &edits) {
            self.editor.selection.clear();
        }
    }

    /// Starts or stops playback. Playing shows the animation, so the
    /// Animate tab comes forward. Play at the end of the scene starts over.
    fn set_playing(&mut self, playing: bool) {
        if playing {
            let from = self
                .playback
                .play(self.now, self.editor.playhead(), self.scene_length());
            self.editor.set_playhead(from);
            self.inspector.tab = InspectorTab::Animate;
        } else {
            self.playback.pause();
        }
    }

    /// The length of the current scene, in milliseconds.
    fn scene_length(&self) -> f64 {
        self.editor
            .artboard()
            .and_then(|id| moshionly_core::eval::duration_of(self.editor.document(), id))
            .unwrap_or(0.0)
    }

    /// Pauses and moves the playhead `frames` frames on, or back for a
    /// negative count.
    fn step(&mut self, frames: i32) {
        self.playback.pause();
        let time = step_frames(self.editor.playhead(), frames, self.scene_length());
        self.editor.set_playhead(time);
        self.inspector.tab = InspectorTab::Animate;
    }

    /// Moves the playhead on with the clock while playing. At the end of
    /// the scene it starts over when looping and stops otherwise.
    ///
    /// Only the Animate tab shows the animation on the canvas; the Design
    /// tab shows the design at rest. Leaving the Animate tab (with M or the
    /// tab itself) therefore pauses playback where it is, rather than run
    /// the clock over a canvas that does not move.
    fn advance_playback(&mut self, ctx: &Context) {
        if !self.playback.is_playing() {
            return;
        }
        if self.inspector.tab != InspectorTab::Animate {
            self.playback.pause();
            return;
        }
        let tick = self.playback.tick(
            self.now,
            self.editor.playhead(),
            self.scene_length(),
            self.looping,
        );
        self.editor.set_playhead(tick.time);
        if tick.playing {
            ctx.request_repaint();
        }
    }

    /// Zooms and pans so `world` fills the part of `canvas` that is free of
    /// the tool bar, with room for the artboard names above.
    fn fit(&mut self, canvas: Rect, world: Rect) {
        let theme = Theme::light();
        let margin = theme.space.xxl * 2.0;
        let bottom = theme.size.tool_bar + theme.size.tool_bar_inset + theme.space.xxl;
        let area = Rect::from_min_max(
            pos2(canvas.left() + margin, canvas.top() + margin),
            pos2(canvas.right() - margin, canvas.bottom() - bottom),
        );
        self.camera.fit(canvas, area, world);
    }

    fn set_theme(&mut self, ctx: &Context, mode: ThemeMode) {
        self.settings.theme = mode;
        theme::set_mode(ctx, mode);
    }
}

/// Whether a command, run while a drag on the canvas is going on, ends
/// that drag first: the commands that change the document, switch the
/// tool, or open a dialog (during which the button may be let go unseen).
fn ends_a_canvas_drag(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Undo
            | CommandId::Redo
            | CommandId::Cut
            | CommandId::Paste
            | CommandId::Duplicate
            | CommandId::Delete
            | CommandId::Group
            | CommandId::Ungroup
            | CommandId::BringToFront
            | CommandId::BringForward
            | CommandId::SendBackward
            | CommandId::SendToBack
            | CommandId::NewProject
            | CommandId::OpenProject
            | CommandId::Save
            | CommandId::SaveAs
            | CommandId::ImportImage
            | CommandId::Export
    ) || Tool::from_command(id).is_some()
}

/// The pointer shape a tool shows over the canvas.
fn tool_cursor(tool: Tool) -> CursorIcon {
    match tool {
        Tool::Select => CursorIcon::Default,
        Tool::Text => CursorIcon::Text,
        Tool::Artboard | Tool::Rectangle | Tool::Ellipse | Tool::Star | Tool::Pen | Tool::Image => {
            CursorIcon::Crosshair
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_shell() -> Shell {
        let demo = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moshionly-core/examples/demo.moshi"
        );
        let session = SessionHandle::open(demo).expect("the demo opens");
        let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
        Shell::headless(Settings::default(), session, renderer)
    }

    fn span(shell: &Shell, id: &str) -> (f64, f64) {
        let document = shell.editor().document();
        document
            .get(&NodeId::from(id))
            .unwrap()
            .data
            .span()
            .unwrap()
    }

    #[test]
    fn a_drag_of_several_bars_is_one_undo_step() {
        let mut shell = demo_shell();
        // A drag reports every moved bar each frame, then its end.
        for delta in [100.0, 250.0, 300.0] {
            for (key, start, end) in [("cardfade", 0.0, 500.0), ("cardslide", 0.0, 700.0)] {
                shell.timeline_event(TimelineEvent::BarMoved {
                    key: Key::new(key),
                    start: start + delta,
                    end: end + delta,
                });
            }
        }
        shell.timeline_event(TimelineEvent::EditFinished);
        assert_eq!(span(&shell, "cardfade"), (300.0, 800.0));
        assert_eq!(span(&shell, "cardslide"), (300.0, 1000.0));
        assert!(!shell.editor().in_gesture());
        assert_eq!(
            shell.editor().undo_label().as_deref(),
            Some("Undo Move action")
        );
        shell.editor_mut().undo().unwrap();
        assert_eq!(span(&shell, "cardfade"), (0.0, 500.0));
        assert_eq!(span(&shell, "cardslide"), (0.0, 700.0));
        assert_eq!(shell.editor().undo_label(), None);
    }

    #[test]
    fn resizing_a_bar_and_the_scene_are_steps_of_their_own() {
        let mut shell = demo_shell();
        for end in [600.0, 900.0] {
            shell.timeline_event(TimelineEvent::BarResized {
                key: Key::new("cardfade"),
                edge: crate::timeline::Edge::End,
                start: 0.0,
                end,
            });
        }
        shell.timeline_event(TimelineEvent::EditFinished);
        for duration in [5000.0, 6500.0] {
            shell.timeline_event(TimelineEvent::DurationChanged { duration });
        }
        shell.timeline_event(TimelineEvent::EditFinished);
        let artboard = NodeId::from("scene");
        assert_eq!(
            moshionly_core::eval::duration_of(shell.editor().document(), &artboard),
            Some(6500.0)
        );
        assert_eq!(span(&shell, "cardfade"), (0.0, 900.0));
        shell.editor_mut().undo().unwrap();
        assert_eq!(
            moshionly_core::eval::duration_of(shell.editor().document(), &artboard),
            Some(4000.0)
        );
        assert_eq!(span(&shell, "cardfade"), (0.0, 900.0), "a step of its own");
        shell.editor_mut().undo().unwrap();
        assert_eq!(span(&shell, "cardfade"), (0.0, 500.0));
    }

    #[test]
    fn selecting_bars_selects_their_layers_and_shows_the_animate_tab() {
        let mut shell = demo_shell();
        shell.timeline_event(TimelineEvent::SelectionChanged {
            bars: vec![Key::new("dotin"), Key::new("count")],
        });
        assert_eq!(
            shell.editor().selection.ids(),
            [NodeId::from("dot"), NodeId::from("raised")]
        );
        assert_eq!(shell.inspector_tab(), InspectorTab::Animate);
        // A layer chosen elsewhere lets go of actions that are not its own.
        shell.editor_mut().selection.select(NodeId::from("dot"));
        shell.follow_selection();
        assert_eq!(shell.editor().actions(), [NodeId::from("dotin")]);
        // Delete then removes the action, not the layer.
        shell.action_command(BarCommand::Delete);
        assert!(!shell.editor().document().contains(&NodeId::from("dotin")));
        assert!(shell.editor().document().contains(&NodeId::from("dot")));
    }

    /// The demo with the card's fade and slide in a folder. Returns the
    /// folder.
    fn card_folder(shell: &mut Shell) -> NodeId {
        let editor = shell.editor_mut();
        editor.set_actions([NodeId::from("cardfade"), NodeId::from("cardslide")]);
        let folder = editor.group_actions().unwrap();
        editor.set_actions([]);
        folder
    }

    #[test]
    fn a_folder_clicked_on_the_timeline_is_deleted_whole() {
        let mut shell = demo_shell();
        let folder = card_folder(&mut shell);
        shell.timeline_event(TimelineEvent::GroupClicked {
            key: Key::new(folder.as_str()),
        });
        assert_eq!(shell.inspector_tab(), InspectorTab::Animate);
        assert_eq!(shell.editor().selection.ids(), [NodeId::from("card")]);
        // The folder stays selected with its layer.
        shell.follow_selection();
        assert_eq!(shell.editor().actions().len(), 3);
        assert_eq!(shell.editor().actions()[0], folder);
        assert!(shell.run(&Context::default(), CommandId::Delete));
        let document = shell.editor().document();
        for gone in [folder.as_str(), "cardfade", "cardslide"] {
            assert!(!document.contains(&NodeId::from(gone)), "{gone}");
        }
        assert!(document.contains(&NodeId::from("card")), "not the layer");
        assert_eq!(
            shell.editor().undo_label().as_deref(),
            Some("Undo Delete folder")
        );
    }

    #[test]
    fn deleting_a_layer_takes_the_folders_it_empties() {
        let mut shell = demo_shell();
        let folder = card_folder(&mut shell);
        shell.editor_mut().selection.select(NodeId::from("card"));
        shell.delete_selection();
        let document = shell.editor().document();
        assert!(!document.contains(&NodeId::from("card")));
        assert!(!document.contains(&folder));
        shell.editor_mut().undo().unwrap();
        assert!(shell.editor().document().contains(&folder));
    }

    #[test]
    fn a_new_document_has_nothing_to_save() {
        let session = untitled_session();
        assert!(!session.read(|s| s.has_unsaved_changes()));
        assert_eq!(session.path(), None);
    }

    #[test]
    fn a_new_document_has_one_square_artboard() {
        let document = new_document();
        assert_eq!(document.name, "Untitled");
        let artboards = document.artboards();
        assert_eq!(artboards.len(), 1);
        let artboard = document.get(&artboards[0]).unwrap();
        let layer = artboard.layer().unwrap();
        assert_eq!((layer.width, layer.height), (1080.0, 1080.0));
        assert_eq!(artboard.name(), "Scene 1");
    }
}

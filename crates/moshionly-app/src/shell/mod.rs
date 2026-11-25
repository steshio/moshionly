//! The shell: the window's layout and everything in it.
//!
//! [`Shell`] owns the editor window: the [`Editor`] with the open document,
//! the pictures of its artboards, the panel sizes, the theme choice, which
//! inspector tab is showing, toasts. [`Shell::show`] draws the whole window
//! once per frame and returns the commands it could not carry out itself.

mod canvas_panel;
mod chat_panel;
mod component_commands;
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
use std::time::{Duration, Instant};

use egui::{Context, CursorIcon, Pos2, Rect, Ui, ViewportCommand, pos2};
use moshionly_agent::{Session, SessionHandle, Tools};
use moshionly_core::{Document, NodeId, build};
use moshionly_export::media::{MediaTools, SoundDecoder};
use moshionly_render::{FontLibrary, Renderer};

use self::{
    canvas_panel::CanvasDrag,
    parts::{Edge, edge_line},
    top_bar::TopBar,
};
pub use self::{
    connect::{AgentLight, IMPORT_WINDOW, MAX_IMPORT_BYTES_PER_WINDOW, MAX_IMPORTS_PER_WINDOW},
    files::{Answer, FileAction, carry_on},
    inspector::{InspectorState, InspectorTab},
    layers_panel::LayersTab,
    layout::{CANVAS_MIN, PanelLimits, PanelSizes, ShellLayout},
    tool_bar::Tool,
};
use crate::host::HostWork;
use crate::{
    audio::{AudioOut, Soundtrack},
    canvas::{CanvasRenderer, Quality},
    commands::{CommandId, CommandRegistry, Palette},
    connect::AgentServer,
    dialogs::{Dialogs, NoDialogs, is_image, is_media, is_project},
    editor::{
        Editor,
        actions::{ActionError, targets_of, with_emptied_folders},
        clips::{ClipDrag, clip_key, clip_of},
        pen::Pen,
        playback::{Playback, step_frames},
        text::TextEditing,
        timeline::{SOUND_GROUP, is_folder, timeline_groups},
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
    /// The scene's sound, played in step with the playhead.
    soundtrack: Soundtrack,
    /// Where playback put the playhead in the last frame, to notice it
    /// being moved by something else while playing.
    shown_playhead: f64,
    /// The installed `ffmpeg` video and sound are read with; `None`
    /// without it, and then media cannot be imported.
    media: Option<MediaTools>,
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
    /// What the layers panel lists.
    layers_tab: LayersTab,
    /// The main component whose row is being dragged to the canvas.
    component_drag: Option<NodeId>,
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
    /// The agent tools over the open document, for the chat. The MCP
    /// server has tools of its own, whose work stops with the server.
    tools: Tools,
    /// The exports and screenshots `tools` do, stopped when the app quits.
    own_work: HostWork,
    /// The MCP server agents connect to.
    server: AgentServer,
    /// The port to serve agents on, once the app asked for a server.
    agent_port: Option<u16>,
    /// The "Connect an agent" dialog is open.
    connect_open: bool,
    /// When the designs sent from Figma in the last minute were added, and
    /// how many bytes each was.
    recent_imports: VecDeque<(Instant, usize)>,
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
    ///
    /// Video and sound are read with the installed `ffmpeg`, when there is
    /// one; sound plays through [`set_audio`](Self::set_audio)'s output.
    pub fn new(settings: Settings, session: SessionHandle) -> Self {
        let media = MediaTools::locate();
        let renderer = Renderer::new(
            FontLibrary::with_system_fonts(),
            crate::media::asset_loader(None, media.as_ref()),
        );
        let pictures = CanvasRenderer::new(renderer.clone());
        let mut shell = Self::build(settings, session, renderer, pictures);
        shell.set_media(media);
        shell
    }

    /// A shell on `session` that draws with `renderer` and renders the
    /// canvas on the calling thread, so that every frame shows finished
    /// pictures. For tests and screenshots: the bundled fonts look the same
    /// on every machine. It reads no video or sound files until
    /// [`set_media`](Self::set_media) says how, and plays no sound.
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
        let own_work = HostWork::new();
        let tools = Tools::new(
            session.clone(),
            crate::host::stoppable_app_host(&renderer, &own_work),
        );
        let mut commands = CommandRegistry::new();
        // Shown while media cannot be imported.
        commands.set_disabled_reason(
            CommandId::ImportMedia,
            Some(crate::media::FFMPEG_NEEDED.to_owned()),
        );
        Self {
            settings,
            commands,
            inspector: InspectorState::default(),
            toasts: Toasts::default(),
            palette: Palette::default(),
            playback: Playback::new(),
            soundtrack: Soundtrack::default(),
            shown_playhead: 0.0,
            media: None,
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
            layers_tab: LayersTab::default(),
            component_drag: None,
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
            own_work,
            server: AgentServer::off(),
            agent_port: None,
            connect_open: false,
            recent_imports: VecDeque::new(),
            export: None,
        }
    }

    /// Stops what runs in the background before the app quits: a running
    /// export, and the work agents asked for, are cancelled and get a
    /// moment to clean up, so no half-written file is left behind. The app
    /// calls this as it exits.
    pub fn prepare_to_quit(&mut self) {
        self.pause();
        // The chat's exports stop while the rest shuts down.
        self.own_work.cancel();
        // Closing the dialog cancels its export and waits for it to stop.
        self.export = None;
        self.shut_down_agent_server();
        self.own_work.wait(QUIT_PATIENCE);
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

    /// Reads video and sound with `tools` from now on, or with nothing:
    /// without them media cannot be imported and the scene plays without
    /// sound. [`new`](Self::new) finds the installed `ffmpeg` itself;
    /// [`headless`](Self::headless) starts without. Video frames come from
    /// the renderer's own provider, which this does not change.
    pub fn set_media(&mut self, tools: Option<MediaTools>) {
        let decoder = tools
            .as_ref()
            .map(|tools| std::sync::Arc::new(SoundDecoder::new(tools.clone())));
        self.soundtrack.set_decoder(decoder);
        self.media = tools;
    }

    /// The installed `ffmpeg` video and sound are read with, if any.
    pub fn media_tools(&self) -> Option<&MediaTools> {
        self.media.as_ref()
    }

    /// Says where sound goes: the app gives the computer's speakers;
    /// until then nothing is heard.
    pub fn set_audio(&mut self, out: Box<dyn AudioOut>) {
        let playhead = self.editor.playhead();
        self.soundtrack.set_output(out, playhead);
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

    /// What the layers panel lists.
    pub fn layers_tab(&self) -> LayersTab {
        self.layers_tab
    }

    /// Makes the layers panel list the layers or the components.
    pub fn set_layers_tab(&mut self, tab: LayersTab) {
        self.layers_tab = tab;
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
            || self.component_drag.is_some()
            || self.connect_open
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
                    tab: &mut self.layers_tab,
                    component_drag: &mut self.component_drag,
                },
            )
        });
        edge_line(ui, layout.left, Edge::Right);
        requested.extend(layers.commands);
        if let Some(message) = layers.error {
            self.toasts.push(Toast::error(message));
        }
        if let Some((main, at)) = layers.place {
            self.end_canvas_drag();
            self.end_timeline_drag();
            match at {
                None => self.place_instance(&main, None),
                // Let go over the canvas: placed there. Anywhere else, the
                // drag was taken back.
                Some(screen) => {
                    if let Some(canvas) = self.canvas.filter(|canvas| canvas.contains(screen)) {
                        let world = self.camera.screen_to_world(canvas, screen);
                        self.place_instance(&main, Some(world));
                    }
                }
            }
        }
        if let Some(main) = layers.show {
            self.show_main(main);
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
        if let Some(ids) = inspected.replace_media {
            self.replace_media_dialog(&ids);
        }
        requested.extend(inspected.commands);

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

    /// Images, videos and sound dropped on the window are placed where
    /// they were dropped.
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
        let media: Vec<&std::path::Path> = dropped
            .iter()
            .map(std::path::PathBuf::as_path)
            .filter(|path| is_media(path))
            .collect();
        if !media.is_empty() {
            let at = self.pointer;
            self.import_media(&media, at);
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

    /// Asks for video and sound files and adds them to the current
    /// artboard. Without `ffmpeg`, says how to get it.
    fn import_media_dialog(&mut self) {
        self.editor.tool = Tool::Select;
        if self.media.is_none() {
            self.toasts.push(Toast::info(crate::media::FFMPEG_NEEDED));
            return;
        }
        if let Some(paths) = self.dialogs.import_media() {
            let paths: Vec<&std::path::Path> =
                paths.iter().map(std::path::PathBuf::as_path).collect();
            self.import_media(&paths, None);
        }
    }

    /// Adds video and sound files, as video layers centred on `at` (or on
    /// the current artboard) and audio items on its timeline, and says
    /// what could not be used and when the scene grew to fit them. Sound
    /// alone shows the Animate tab, where its settings are.
    fn import_media(&mut self, paths: &[&std::path::Path], at: Option<Pos2>) {
        let Some(tools) = self.media.clone() else {
            self.toasts.push(Toast::info(crate::media::FFMPEG_NEEDED));
            return;
        };
        let probe = std::sync::Arc::new(crate::media::FfmpegProbe::with_tools(tools));
        let made = self.editor.import_media(paths, at, probe);
        for problem in &made.problems {
            self.toasts.push(Toast::error(problem.clone()));
        }
        if let Some(length) = made.scene_length {
            self.toasts.push(Toast::info(format!(
                "The scene now lasts {} s, to fit what was added.",
                crate::widgets::numeric::format_value(length / 1000.0, 2)
            )));
        }
        if let Some(name) = made.unknown_length.first() {
            self.toasts.push(Toast::info(format!(
                "{name} does not say how long it is, so it plays for {} s. Drag its end on the \
                 timeline to change that.",
                crate::widgets::numeric::format_value(
                    crate::editor::media::UNKNOWN_LENGTH / 1000.0,
                    2
                )
            )));
        }
        if made.videos.is_empty() && !made.sounds.is_empty() {
            self.inspector.tab = InspectorTab::Animate;
        }
    }

    /// Asks for a file for the video layers or sounds `ids` to play
    /// instead.
    fn replace_media_dialog(&mut self, ids: &[NodeId]) {
        let Some(tools) = self.media.clone() else {
            self.toasts.push(Toast::info(crate::media::FFMPEG_NEEDED));
            return;
        };
        let Some(path) = self
            .dialogs
            .import_media()
            .and_then(|paths| paths.into_iter().next())
        else {
            return;
        };
        let probe = std::sync::Arc::new(crate::media::FfmpegProbe::with_tools(tools));
        if let Err(problem) = self.editor.replace_media(ids, &path, probe) {
            self.toasts.push(Toast::error(problem));
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
        self.collapsed_groups
            .retain(|id| document.contains(id) || id.as_str() == SOUND_GROUP);
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
        // folder while a layer its actions animate is. Sound, which
        // animates no layer, while no layer is selected.
        let kept: Vec<NodeId> = self
            .editor
            .actions()
            .iter()
            .filter(|action| {
                let targets = targets_of(&document, std::slice::from_ref(action));
                if targets.is_empty() {
                    selection.is_empty()
                } else {
                    targets.iter().any(|target| selection.contains(target))
                }
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
        // A selected folder's row is tinted, or the sound's when only sound
        // is selected, or else the selected layer's.
        let only_sound = !self.editor.actions().is_empty()
            && self.editor.actions().iter().all(|id| {
                document.get(id).is_some_and(|node| {
                    node.node_type() == moshionly_core::NodeType::Audio
                        && node.parent.as_ref() == artboard.as_ref()
                })
            });
        let selected_folder = self
            .editor
            .actions()
            .first()
            .map(|id| Key::new(id.as_str()))
            .filter(|key| is_folder(&document, key))
            .or_else(|| only_sound.then(|| Key::new(SOUND_GROUP)));
        let selected_layer = selected_folder.or_else(|| {
            self.editor
                .selection
                .single()
                .map(|id| Key::new(id.as_str()))
        });
        // The clips of selected videos show as selected too.
        let selected_bars: Vec<Key> = self
            .editor
            .actions()
            .iter()
            .map(|id| Key::new(id.as_str()))
            .chain(self.editor.selection.ids().iter().filter_map(|id| {
                let node = document.get(id)?;
                (node.node_type() == moshionly_core::NodeType::Video)
                    .then(|| clip_key(id, node.node_type()))
            }))
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
            TimelineEvent::SelectionChanged { bars } => self.select_bars(&bars),
            TimelineEvent::GroupClicked { key } => {
                let document = std::sync::Arc::clone(self.editor.document());
                if key.as_str() == SOUND_GROUP {
                    // All the scene's sound, which belongs to no layer.
                    let sound: Vec<NodeId> = self
                        .editor
                        .artboard()
                        .map(|artboard| {
                            document
                                .children(artboard, moshionly_core::Family::Animation)
                                .iter()
                                .filter(|id| {
                                    document.get(id).is_some_and(|node| {
                                        node.node_type() == moshionly_core::NodeType::Audio
                                    })
                                })
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    self.editor.selection.clear();
                    self.editor.set_actions(sound);
                    self.inspector.tab = InspectorTab::Animate;
                } else if is_folder(&document, &key) {
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
                match clip_of(self.editor.document(), &key) {
                    Some(clip) => self.timeline_drag("Move clip", |editor| {
                        Ok(editor.drag_clip("Move clip", &clip, ClipDrag::Move, start, end)?)
                    }),
                    None => self.timeline_drag("Move action", |editor| {
                        editor.retime_actions("Move action", &[(node(&key), start, end)])
                    }),
                }
            }
            TimelineEvent::BarResized {
                key,
                edge,
                start,
                end,
            } => match clip_of(self.editor.document(), &key) {
                Some(clip) => self.timeline_drag("Trim clip", |editor| {
                    Ok(editor.drag_clip("Trim clip", &clip, ClipDrag::Trim(edge), start, end)?)
                }),
                None => self.timeline_drag("Change action length", |editor| {
                    editor.retime_actions("Change action length", &[(node(&key), start, end)])
                }),
            },
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

    /// Selects what the bars `bars` stand for: actions and sound, and the
    /// layers they animate; a video's clip selects the video. Sound alone
    /// selects no layer. Bars show their settings in the Animate tab.
    fn select_bars(&mut self, bars: &[Key]) {
        let document = std::sync::Arc::clone(self.editor.document());
        let mut videos = Vec::new();
        let mut actions = Vec::new();
        for key in bars {
            match clip_of(&document, key) {
                Some(id) if id.as_str() != key.as_str() => videos.push(id),
                _ => actions.push(NodeId::from(key.as_str())),
            }
        }
        self.editor.set_actions(actions);
        let mut layers = targets_of(&document, self.editor.actions());
        for video in videos {
            if !layers.contains(&video) {
                layers.push(video);
            }
        }
        if !layers.is_empty() {
            self.editor.selection.set(layers);
        } else if !self.editor.actions().is_empty() {
            self.editor.selection.clear();
        }
        if !bars.is_empty() {
            self.inspector.tab = InspectorTab::Animate;
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
        self.pause();
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
        component_commands::enable_component_commands(&mut self.commands, &self.editor);
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
        // Media is read with ffmpeg; without it the command says so.
        self.commands
            .set_enabled(CommandId::ImportMedia, self.media.is_some());
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
            CommandId::ImportMedia => self.import_media_dialog(),
            other if self.component_command(other) => {}
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
    /// folders that leaves empty. Instances of a main component that goes
    /// are detached into plain groups, and a toast says so.
    pub(super) fn delete_selection(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if ids.is_empty() {
            return;
        }
        let document = std::sync::Arc::clone(self.editor.document());
        let detached = crate::editor::components::detach_message(&document, &ids);
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
            if let Some(message) = detached {
                self.toasts.push(Toast::info(message));
            }
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
            self.shown_playhead = self.editor.playhead();
            self.inspector.tab = InspectorTab::Animate;
            self.soundtrack.play(from);
        } else {
            self.pause();
        }
    }

    /// Stops playback, and its sound, where it is.
    pub(super) fn pause(&mut self) {
        self.playback.pause();
        self.soundtrack.stop();
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
        self.pause();
        let time = step_frames(self.editor.playhead(), frames, self.scene_length());
        self.editor.set_playhead(time);
        self.inspector.tab = InspectorTab::Animate;
    }

    /// Moves the playhead on with the clock while playing. At the end of
    /// the scene it starts over when looping and stops otherwise.
    ///
    /// While sound is heard, the playhead follows the sound rather than
    /// the window's clock, so the two stay in step; the sound jumps with
    /// the playhead when something else moves it (a click on the ruler) or
    /// the loop starts over.
    ///
    /// Only the Animate tab shows the animation on the canvas; the Design
    /// tab shows the design at rest. Leaving the Animate tab (with M or the
    /// tab itself) therefore pauses playback where it is, rather than run
    /// the clock over a canvas that does not move.
    fn advance_playback(&mut self, ctx: &Context) {
        self.follow_sound();
        if !self.playback.is_playing() {
            return;
        }
        if self.inspector.tab != InspectorTab::Animate {
            self.pause();
            return;
        }
        let playhead = self.editor.playhead();
        let moved = playhead != self.shown_playhead;
        if moved {
            self.soundtrack.seek(playhead);
        }
        // What is heard is where the scene is, unless the playhead was just
        // moved and the sound has yet to follow.
        let from = match self.soundtrack.position() {
            Some(heard) if !moved => heard,
            _ => playhead,
        };
        let tick = self
            .playback
            .tick(self.now, from, self.scene_length(), self.looping);
        if !tick.playing {
            self.soundtrack.stop();
        } else if tick.time < from {
            // Started over.
            self.soundtrack.seek(tick.time);
        }
        self.editor.set_playhead(tick.time);
        self.shown_playhead = self.editor.playhead();
        if tick.playing {
            ctx.request_repaint();
        }
    }

    /// Keeps the sound of the current scene mixed while the Animate tab
    /// shows it, so that play starts with sound, and says which sound
    /// files could not be read.
    fn follow_sound(&mut self) {
        let animate = self.inspector.tab == InspectorTab::Animate;
        if (animate || self.playback.is_playing())
            && let Some(artboard) = self.editor.artboard().cloned()
        {
            let document = std::sync::Arc::clone(self.editor.document());
            let project = self.editor.project_file().map(ToOwned::to_owned);
            self.soundtrack
                .follow(&document, &artboard, project.as_deref());
        }
        for warning in self.soundtrack.poll(self.editor.playhead()) {
            self.toasts
                .push(Toast::warning(format!("{}.", files::sentence(&warning))));
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
            | CommandId::CreateComponent
            | CommandId::InsertInstance
            | CommandId::GoToMainComponent
            | CommandId::DetachInstance
            | CommandId::NewProject
            | CommandId::OpenProject
            | CommandId::Save
            | CommandId::SaveAs
            | CommandId::ImportImage
            | CommandId::ImportMedia
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
    use moshionly_render::AssetLoader;

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
    fn the_sound_plays_jumps_and_stops_with_playback() {
        use crate::audio::{AudioCall, SilentAudio};
        let mut shell = demo_shell();
        let out = SilentAudio::recording();
        shell.set_audio(Box::new(out.clone()));
        let ctx = Context::default();
        shell.now = 10.0;
        shell.set_playing(true);
        shell.now = 10.1;
        shell.advance_playback(&ctx);
        assert!((shell.editor().playhead() - 100.0).abs() < 1e-6);
        // A click on the ruler while playing: the sound jumps there.
        shell.timeline_event(TimelineEvent::PlayheadMoved { time: 2000.0 });
        shell.now = 10.2;
        shell.advance_playback(&ctx);
        // Past the end of the 4 second scene, the loop starts over.
        shell.now = 12.5;
        shell.advance_playback(&ctx);
        assert!((shell.editor().playhead() - 300.0).abs() < 1e-6);
        shell.set_playing(false);
        // Paused, the playhead moves without sound.
        shell.timeline_event(TimelineEvent::PlayheadMoved { time: 1000.0 });
        shell.advance_playback(&ctx);
        // Leaving the Animate tab stops playback and its sound.
        shell.set_playing(true);
        shell.inspector.tab = InspectorTab::Design;
        shell.advance_playback(&ctx);
        assert!(!shell.is_playing());
        shell.prepare_to_quit();
        let calls = out.calls();
        assert_eq!(calls.len(), 7, "{calls:?}");
        assert_eq!(
            calls[..3],
            [
                AudioCall::Play(0.0),
                // The demo has no sound: its track is silence.
                AudioCall::Track(None),
                AudioCall::Seek(2000.0),
            ]
        );
        assert!(
            matches!(calls[3], AudioCall::Seek(at) if (at - 300.0).abs() < 1e-6),
            "{calls:?}"
        );
        assert_eq!(
            calls[4..],
            [AudioCall::Stop, AudioCall::Play(1000.0), AudioCall::Stop]
        );
    }

    /// A shell on a scene with a video and a song.
    fn media_shell() -> Shell {
        let document = moshionly_core::io::parse(
            r##"{"format":"moshionly","version":3,"name":"Media","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","duration":5000},
            {"id":"film","parent":"ab","index":"a0","type":"video","source":"/no/film.mp4",
             "start":1000,"end":3000,"trim_start":500},
            {"id":"song","parent":"ab","index":"a0","type":"audio","source":"/no/song.mp3",
             "start":0,"end":2000}
            ]}"##,
        )
        .unwrap();
        let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
        Shell::headless(
            Settings::default(),
            SessionHandle::from_document(document),
            renderer,
        )
    }

    fn clip(shell: &Shell, id: &str) -> moshionly_core::model::ClipTiming {
        *shell
            .editor()
            .document()
            .get(&NodeId::from(id))
            .unwrap()
            .data
            .clip()
            .unwrap()
    }

    #[test]
    fn dragging_a_clip_moves_or_trims_it_in_one_step() {
        use crate::timeline::Edge;
        let mut shell = media_shell();
        for start in [1100.0, 1200.0] {
            shell.timeline_event(TimelineEvent::BarMoved {
                key: Key::new("film/clip"),
                start,
                end: start + 2000.0,
            });
        }
        shell.timeline_event(TimelineEvent::EditFinished);
        let film = clip(&shell, "film");
        assert_eq!(
            (film.start, film.end, film.trim_start),
            (1200.0, 3200.0, 500.0)
        );
        assert_eq!(
            shell.editor().undo_label().as_deref(),
            Some("Undo Move clip")
        );
        // The left edge trims: the file's moments stay where they were.
        shell.timeline_event(TimelineEvent::BarResized {
            key: Key::new("film/clip"),
            edge: Edge::Start,
            start: 1500.0,
            end: 3200.0,
        });
        shell.timeline_event(TimelineEvent::EditFinished);
        let film = clip(&shell, "film");
        assert_eq!(
            (film.start, film.end, film.trim_start),
            (1500.0, 3200.0, 800.0)
        );
        // A sound's right edge changes its end.
        shell.timeline_event(TimelineEvent::BarResized {
            key: Key::new("song"),
            edge: Edge::End,
            start: 0.0,
            end: 2600.0,
        });
        shell.timeline_event(TimelineEvent::EditFinished);
        assert_eq!(clip(&shell, "song").end, 2600.0);
        assert_eq!(
            shell.editor().undo_label().as_deref(),
            Some("Undo Trim clip")
        );
        for _ in 0..3 {
            shell.editor_mut().undo().unwrap();
        }
        let film = clip(&shell, "film");
        assert_eq!(
            (film.start, film.end, film.trim_start),
            (1000.0, 3000.0, 500.0)
        );
        assert_eq!(clip(&shell, "song").end, 2000.0);
    }

    #[test]
    fn sound_is_selected_on_its_own_and_a_clip_selects_its_video() {
        let mut shell = media_shell();
        shell.editor_mut().selection.select(NodeId::from("film"));
        shell.follow_selection();
        shell.timeline_event(TimelineEvent::SelectionChanged {
            bars: vec![Key::new("song")],
        });
        assert!(shell.editor().selection.is_empty());
        assert_eq!(shell.editor().actions(), [NodeId::from("song")]);
        assert_eq!(shell.inspector_tab(), InspectorTab::Animate);
        // Sound stays selected while no layer is.
        shell.follow_selection();
        assert_eq!(shell.editor().actions(), [NodeId::from("song")]);
        // A video's clip selects the video.
        shell.timeline_event(TimelineEvent::SelectionChanged {
            bars: vec![Key::new("film/clip")],
        });
        assert_eq!(shell.editor().selection.ids(), [NodeId::from("film")]);
        assert!(shell.editor().actions().is_empty());
        // The Sound row selects all the sound.
        shell.timeline_event(TimelineEvent::GroupClicked {
            key: Key::new(SOUND_GROUP),
        });
        assert_eq!(shell.editor().actions(), [NodeId::from("song")]);
        assert!(shell.editor().selection.is_empty());
        // Delete then takes the sound away.
        assert!(shell.run(&Context::default(), CommandId::Delete));
        assert!(!shell.editor().document().contains(&NodeId::from("song")));
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

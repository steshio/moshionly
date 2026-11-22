//! The shell: the window's layout and everything in it.
//!
//! [`Shell`] owns the state of the editor window that is not the document:
//! panel sizes, the theme choice, which inspector tab is showing, toasts.
//! [`Shell::show`] draws the whole window once per frame and returns the
//! commands it could not carry out itself.
//!
//! The panels show placeholder content from [`sample`] until the shell is
//! bound to the document engine.

mod chat_panel;
mod inspector;
mod layers_panel;
pub mod layout;
mod parts;
pub mod sample;
mod sample_art;
mod tool_bar;
mod top_bar;

use egui::{Context, CursorIcon, Rect, Ui, ViewportCommand, pos2};

pub use self::{
    inspector::{InspectorState, InspectorTab},
    layout::{CANVAS_MIN, PanelLimits, PanelSizes, ShellLayout},
    tool_bar::Tool,
};
use self::{
    parts::{Edge, edge_line},
    sample::Sample,
    top_bar::TopBar,
};
use crate::{
    commands::{CommandId, CommandRegistry},
    key::Key,
    settings::Settings,
    theme::{self, Theme, ThemeMode},
    timeline::{Timeline, TimelineEvent, TimelineState},
    viewport::{Camera, Viewport, ViewportEvent},
    widgets::{Grow, Toast, Toasts, splitter},
};

/// The editor window.
pub struct Shell {
    settings: Settings,
    commands: CommandRegistry,
    inspector: InspectorState,
    toasts: Toasts,
    tool: Tool,
    playing: bool,
    looping: bool,
    snapping: bool,
    /// The current time, in milliseconds.
    playhead: f64,
    timeline: TimelineState,
    camera: Camera,
    /// The canvas rectangle of the last frame; `None` before the first one.
    canvas: Option<Rect>,
    /// The sample layer under the pointer, found on the last frame.
    hover: Option<Key>,
    /// Set when the top bar is also the window's title area: the room to
    /// leave at its left edge for the window's buttons.
    title_inset: Option<f32>,
    sample: Sample,
}

/// What [`Shell::show`] hands back to the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellOutput {
    /// Commands the user asked for that the shell does not carry out itself:
    /// opening, saving, exporting and the like.
    pub commands: Vec<CommandId>,
}

impl Shell {
    /// A shell with the given settings and the sample scene.
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            commands: CommandRegistry::new(),
            inspector: InspectorState::default(),
            toasts: Toasts::default(),
            tool: Tool::default(),
            playing: false,
            looping: true,
            snapping: true,
            playhead: 1200.0,
            timeline: TimelineState::default(),
            camera: Camera::default(),
            canvas: None,
            hover: None,
            title_inset: None,
            sample: Sample::default(),
        }
    }

    /// Tells the shell that its top bar doubles as the window's title bar,
    /// as on macOS with a full-size content view. The bar then leaves `inset`
    /// points free at its left edge for the window's own buttons, and its
    /// empty space drags the window.
    pub fn use_top_bar_as_title_bar(&mut self, inset: f32) {
        self.title_inset = Some(inset);
    }

    /// The active tool.
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// The canvas camera.
    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// The settings as they are now, for saving.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The sizes of the panels. Changing them moves the panels.
    pub fn panels_mut(&mut self) -> &mut PanelSizes {
        &mut self.settings.panels
    }

    /// The inspector's state, for choosing the tab.
    pub fn inspector_mut(&mut self) -> &mut InspectorState {
        &mut self.inspector
    }

    /// Which commands are enabled.
    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        &mut self.commands
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

        // The canvas first, so the panels' edges are drawn over it.
        self.sample.load_textures(&ctx);
        if self.canvas.is_none() {
            // First frame: start with the first artboard filling the canvas.
            let first = self.sample.artboards.first().map(|a| a.rect);
            self.fit(layout.canvas, first.unwrap_or(Rect::NOTHING));
        }
        self.canvas = Some(layout.canvas);
        let tool = self.tool;
        let artboards = self.sample.viewport_artboards();
        let overlays = self.sample.overlays(self.hover.as_ref());
        let (viewport, picked) = parts::panel(ui, "canvas", layout.canvas, c.canvas, |ui| {
            let viewport = Viewport::new(&mut self.camera)
                .artboards(&artboards)
                .overlays(&overlays)
                .cursor(tool_cursor(tool))
                .show(ui, layout.canvas);
            (viewport, tool_bar::tool_bar(ui, layout.canvas, tool))
        });
        requested.extend(picked);

        // TODO: route canvas gestures to the document engine. Until then the
        // select tool picks sample layers by their boxes; drags and handles
        // are reported by the viewport but change nothing.
        let selecting = tool == Tool::Select;
        self.hover = viewport
            .pointer
            .filter(|_| selecting && viewport.handle.is_none())
            .and_then(|world| self.sample.layer_at(world));
        for event in viewport.events {
            match event {
                ViewportEvent::ArtboardNameClicked { key, .. } => self.sample.select(Some(key)),
                ViewportEvent::Clicked { world, .. } if selecting => {
                    self.sample.select(self.sample.layer_at(world));
                }
                _ => {}
            }
        }

        // In full screen the window's buttons are hidden, so the inset goes.
        let fullscreen = ctx.input(|i| i.viewport().fullscreen == Some(true));
        let bar = TopBar {
            project_name: &self.sample.project_name,
            zoom: self.camera.zoom,
            playing: self.playing,
            chat_open: self.settings.panels.chat_open,
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
        if top.drag_window {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if top.toggle_maximized {
            let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        parts::panel(ui, "layers", layout.left, c.surface, |ui| {
            requested.extend(layers_panel::layers_panel(
                ui,
                &mut self.sample,
                &self.commands,
            ));
        });
        edge_line(ui, layout.left, Edge::Right);

        parts::panel(ui, "inspector", layout.right, c.surface, |ui| {
            inspector::inspector(ui, &mut self.inspector, &mut self.sample);
        });
        edge_line(ui, layout.right, Edge::Left);

        self.advance_playback(&ctx);
        let groups = self.sample.timeline_groups();
        let timeline = parts::panel(ui, "timeline", layout.timeline, c.surface, |ui| {
            let out = Timeline::new(&mut self.timeline, &groups)
                .selection(&self.sample.selected_actions)
                .selected_group(self.sample.selected_layer.as_ref())
                .playhead(self.playhead)
                .duration(self.sample.duration)
                .playing(self.playing)
                .looping(self.looping)
                .snapping(self.snapping)
                .show(ui, layout.timeline);
            // The tag floats in the corner of the tracks.
            let corner = out.layout.tracks.right_top() + egui::vec2(-64.0, 8.0);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_size(corner, egui::vec2(56.0, 16.0))),
                parts::sample_tag,
            );
            out
        });
        edge_line(ui, layout.timeline, Edge::Top);
        for event in &timeline.events {
            match event {
                TimelineEvent::PlayheadMoved { time } => self.playhead = *time,
                TimelineEvent::PlayToggled { playing } => self.playing = *playing,
                TimelineEvent::LoopToggled { looping } => self.looping = *looping,
                TimelineEvent::SnappingToggled { snapping } => self.snapping = *snapping,
                TimelineEvent::JumpedToStart => self.playhead = 0.0,
                TimelineEvent::BarActivated { .. } => self.inspector.tab = InspectorTab::Animate,
                other => self.sample.apply(other),
            }
        }

        if let Some(chat) = layout.chat {
            let out = parts::panel(ui, "chat", chat, c.surface, |ui| {
                chat_panel::chat_panel(ui, &mut self.sample.chat_draft)
            });
            edge_line(ui, chat, Edge::Left);
            if out.close {
                requested.push(CommandId::ToggleChat);
            }
            if out.sent.is_some() {
                self.toasts
                    .push(Toast::info("Chat is not connected to a provider yet"));
            }
        }

        self.splitters(ui, window, &layout);

        // Toasts stack upwards from just above the tool bar.
        let toast_anchor = tool_bar::tool_bar_rect(layout.canvas, theme).center_top()
            - egui::vec2(0.0, theme.space.lg);
        self.toasts.show(&ctx, toast_anchor);

        requested.extend(self.commands.triggered(&ctx));
        let mut output = ShellOutput::default();
        for id in requested {
            if !self.run(&ctx, id) {
                output.commands.push(id);
            }
        }
        output
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

    /// Carries out a command if it is one the shell owns. Returns false for
    /// the others.
    fn run(&mut self, ctx: &Context, id: CommandId) -> bool {
        match id {
            CommandId::ToggleChat => {
                self.settings.panels.chat_open = !self.settings.panels.chat_open;
            }
            CommandId::ThemeSystem => self.set_theme(ctx, ThemeMode::System),
            CommandId::ThemeLight => self.set_theme(ctx, ThemeMode::Light),
            CommandId::ThemeDark => self.set_theme(ctx, ThemeMode::Dark),
            CommandId::PlayPause => self.playing = !self.playing,
            CommandId::JumpToStart => self.playhead = 0.0,
            CommandId::ToggleLoop => self.looping = !self.looping,
            CommandId::ZoomIn => self.camera.zoom_in(),
            CommandId::ZoomOut => self.camera.zoom_out(),
            CommandId::ZoomTo100 => self.camera.set_zoom(1.0),
            CommandId::ZoomToFit => {
                if let Some(canvas) = self.canvas {
                    self.fit(canvas, self.sample.bounds());
                }
            }
            CommandId::AskAi => self.settings.panels.chat_open = true,
            other => match Tool::from_command(other) {
                Some(tool) => self.tool = tool,
                None => return false,
            },
        }
        true
    }

    /// Moves the playhead on while playing. At the end of the scene it
    /// wraps around when looping and stops otherwise.
    fn advance_playback(&mut self, ctx: &Context) {
        if !self.playing {
            return;
        }
        let elapsed = f64::from(ctx.input(|i| i.stable_dt)) * 1000.0;
        let (time, playing) = played(self.playhead, elapsed, self.sample.duration, self.looping);
        self.playhead = time;
        self.playing = playing;
        if playing {
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

/// The time after playing on from `time` for `elapsed` milliseconds in a
/// scene `duration` long, and whether playback goes on. Past the end, a loop
/// wraps around; otherwise playback stops at the end.
fn played(time: f64, elapsed: f64, duration: f64, looping: bool) -> (f64, bool) {
    if duration <= 0.0 {
        return (0.0, false);
    }
    // Pressing play at the end starts over.
    let from = if time >= duration { 0.0 } else { time };
    let next = from + elapsed.max(0.0);
    if next < duration {
        (next, true)
    } else if looping {
        (next % duration, true)
    } else {
        (duration, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_runs_to_the_end_and_stops_or_loops() {
        assert_eq!(played(1000.0, 16.0, 4000.0, false), (1016.0, true));
        assert_eq!(played(3990.0, 16.0, 4000.0, false), (4000.0, false));
        assert_eq!(played(3990.0, 16.0, 4000.0, true), (6.0, true));
        // Play pressed at the end starts from the beginning.
        assert_eq!(played(4000.0, 16.0, 4000.0, false), (16.0, true));
        // A scene with no length cannot play.
        assert_eq!(played(0.0, 16.0, 0.0, true), (0.0, false));
        // A long stall wraps as many times as needed.
        assert_eq!(played(0.0, 9000.0, 4000.0, true), (1000.0, true));
    }
}

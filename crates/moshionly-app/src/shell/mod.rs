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

use egui::{Align2, Context, Rect, Ui, pos2};

pub use self::{
    inspector::{InspectorState, InspectorTab},
    layout::{CANVAS_MIN, PanelLimits, PanelSizes, ShellLayout},
};
use self::{
    parts::{Edge, edge_line},
    sample::Sample,
};
use crate::{
    commands::{CommandId, CommandRegistry},
    settings::Settings,
    theme::{self, Theme, ThemeMode},
    widgets::{Grow, Toast, Toasts, paint, splitter},
};

/// The editor window.
pub struct Shell {
    settings: Settings,
    commands: CommandRegistry,
    inspector: InspectorState,
    toasts: Toasts,
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
            sample: Sample::default(),
        }
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
        parts::panel(ui, "canvas", layout.canvas, c.canvas, |_| {});

        parts::panel(ui, "top-bar", layout.top_bar, c.surface, |ui| {
            let name = Rect::from_min_max(
                pos2(layout.top_bar.left() + theme.space.xl, layout.top_bar.top()),
                layout.top_bar.max,
            );
            paint::text_in(
                ui,
                name,
                Align2::LEFT_CENTER,
                &self.sample.project_name,
                theme.title(),
                c.text,
            );
        });
        edge_line(ui, layout.top_bar, Edge::Bottom);

        parts::panel(ui, "layers", layout.left, c.surface, |ui| {
            requested.extend(layers_panel::layers_panel(ui, &mut self.sample));
        });
        edge_line(ui, layout.left, Edge::Right);

        parts::panel(ui, "inspector", layout.right, c.surface, |ui| {
            inspector::inspector(ui, &mut self.inspector, &mut self.sample);
        });
        edge_line(ui, layout.right, Edge::Left);

        parts::panel(ui, "timeline", layout.timeline, c.surface, |_| {});
        edge_line(ui, layout.timeline, Edge::Top);

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

        let toast_anchor = layout.canvas.center_bottom() - egui::vec2(0.0, theme.space.xl);
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
            _ => return false,
        }
        true
    }

    fn set_theme(&mut self, ctx: &Context, mode: ThemeMode) {
        self.settings.theme = mode;
        theme::set_mode(ctx, mode);
    }
}

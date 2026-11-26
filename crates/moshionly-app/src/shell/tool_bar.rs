//! The tool bar that floats over the bottom of the canvas.

use egui::{Align, Align2, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2};

use super::parts::command_item;
use crate::{
    commands::{CommandId, CommandRegistry},
    theme::{Theme, icons},
    widgets::{ToolButton, context_menu, paint, tooltip},
};

/// What a click or drag on the canvas does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Select, move and resize layers.
    #[default]
    Select,
    /// Draw an artboard.
    Artboard,
    /// Draw a rectangle.
    Rectangle,
    /// Draw an ellipse.
    Ellipse,
    /// Draw a star.
    Star,
    /// Draw a path.
    Pen,
    /// Place text.
    Text,
    /// Place an image.
    Image,
}

impl Tool {
    /// Every tool, in tool bar order.
    pub const ALL: [Self; 8] = [
        Self::Select,
        Self::Artboard,
        Self::Rectangle,
        Self::Ellipse,
        Self::Star,
        Self::Pen,
        Self::Text,
        Self::Image,
    ];

    /// The command that switches to this tool. It carries the tool's name,
    /// icon and shortcut.
    pub fn command(self) -> CommandId {
        match self {
            Self::Select => CommandId::ToolSelect,
            Self::Artboard => CommandId::ToolArtboard,
            Self::Rectangle => CommandId::ToolRectangle,
            Self::Ellipse => CommandId::ToolEllipse,
            Self::Star => CommandId::ToolStar,
            Self::Pen => CommandId::ToolPen,
            Self::Text => CommandId::ToolText,
            Self::Image => CommandId::ToolImage,
        }
    }

    /// The tool a command switches to, if it is a tool command.
    pub fn from_command(id: CommandId) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.command() == id)
    }
}

/// The rectangle the tool bar occupies over `canvas`: centred, a little above
/// the bottom edge.
pub fn tool_bar_rect(canvas: Rect, theme: &Theme) -> Rect {
    let size = &theme.size;
    let gap = theme.space.xxs;
    let pad = (size.tool_bar - size.tool_button) / 2.0;
    let tools = Tool::ALL.len() as f32;
    let width =
        pad + tools * size.tool_button + (tools - 1.0) * gap + DIVIDER_WIDTH + AI_WIDTH + pad;
    Rect::from_center_size(
        pos2(
            canvas.center().x.round(),
            canvas.bottom() - size.tool_bar_inset - size.tool_bar / 2.0,
        ),
        Vec2::new(width, size.tool_bar),
    )
}

/// Room for the hairline between the tools and the AI entry.
const DIVIDER_WIDTH: f32 = 13.0;
/// Width of the AI entry.
const AI_WIDTH: f32 = 82.0;

/// Draws the tool bar over `canvas` and returns the command of the button
/// that was clicked, if any: a tool command, [`CommandId::AskAi`], or an
/// import from the image tool's menu (a right click), which also offers
/// video and sound.
pub fn tool_bar(
    ui: &mut Ui,
    canvas: Rect,
    active: Tool,
    commands: &CommandRegistry,
) -> Option<CommandId> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let size = &theme.size;
    let rect = tool_bar_rect(canvas, theme);
    let mut clicked = None;

    let mut bar = ui.new_child(
        UiBuilder::new()
            .id_salt("tool-bar")
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    bar.set_clip_rect(canvas.intersect(ui.clip_rect()));
    // Swallow clicks between the buttons so they do not reach the canvas.
    bar.interact(
        rect,
        bar.scope_id().with("background"),
        Sense::click_and_drag(),
    );

    paint::shadows(bar.painter(), rect, theme.radius.lg, &theme.shadow.float);
    paint::panel(
        bar.painter(),
        rect,
        theme.radius.lg,
        c.raised,
        Stroke::new(1.0, c.border),
    );

    bar.spacing_mut().item_spacing.x = theme.space.xxs;
    bar.add_space((size.tool_bar - size.tool_button) / 2.0);
    for tool in Tool::ALL {
        let command = tool.command();
        let shortcut = command.shortcut_label().unwrap_or_default();
        let icon = command.icon().unwrap_or(icons::CURSOR);
        let mut button = ToolButton::new(icon, command.name()).selected(tool == active);
        if !shortcut.is_empty() {
            button = button.shortcut(&shortcut);
        }
        let response = bar.add(button);
        if response.clicked() {
            clicked = Some(command);
        }
        if tool == Tool::Image {
            context_menu(&response, |ui| {
                for import in [CommandId::ImportImage, CommandId::ImportMedia] {
                    if command_item(ui, import, commands, None) {
                        clicked = Some(import);
                    }
                }
            });
        }
    }

    let (divider, _) = bar.allocate_exact_size(
        Vec2::new(DIVIDER_WIDTH - theme.space.xxs * 2.0, size.tool_button),
        Sense::hover(),
    );
    bar.painter().vline(
        divider.center().x.floor() + 0.5,
        divider.y_range().shrink(6.0),
        theme.hairline(),
    );

    if ai_entry(&mut bar) {
        clicked = Some(CommandId::AskAi);
    }
    clicked
}

/// The AI entry at the end of the tool bar: a soft accent pill.
fn ai_entry(ui: &mut Ui) -> bool {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(AI_WIDTH, theme.size.tool_button), Sense::click());
    let name = CommandId::AskAi.name();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));

    let fill = if response.is_pointer_button_down_on() {
        c.accent_soft.gamma_multiply(0.8)
    } else if response.hovered() {
        c.accent_soft
    } else {
        c.accent_soft.gamma_multiply(0.6)
    };
    ui.painter().rect_filled(rect, theme.radius.sm + 1.0, fill);
    let label = paint::line(ui, name, theme.body_medium(), c.accent_text);
    let icon_box = theme.size.icon;
    let content = icon_box + theme.space.xs + label.size().x;
    let left = rect.center().x - content / 2.0;
    icons::paint(
        ui.painter(),
        pos2(left + icon_box / 2.0, rect.center().y),
        icons::SPARKLE,
        icon_box,
        c.accent_text,
    );
    let text_rect =
        Rect::from_min_max(pos2(left + icon_box + theme.space.xs, rect.top()), rect.max);
    paint::galley_in(ui.painter(), text_rect, Align2::LEFT_CENTER, label);

    let clicked = response.clicked();
    tooltip(response, "Ask an AI agent to change this file", None);
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_command_with_an_icon_and_a_shortcut() {
        for tool in Tool::ALL {
            let command = tool.command();
            assert_eq!(Tool::from_command(command), Some(tool));
            assert!(command.icon().is_some(), "{tool:?}");
            assert!(command.shortcut().is_some(), "{tool:?}");
        }
        assert_eq!(Tool::from_command(CommandId::Undo), None);
        assert_eq!(Tool::from_command(CommandId::AskAi), None);
    }

    #[test]
    fn the_bar_sits_centred_above_the_bottom_of_the_canvas() {
        let theme = Theme::light();
        let canvas = Rect::from_min_max(pos2(248.0, 44.0), pos2(1176.0, 652.0));
        let bar = tool_bar_rect(canvas, theme);
        assert_eq!(bar.center().x, canvas.center().x);
        assert_eq!(bar.bottom(), canvas.bottom() - theme.size.tool_bar_inset);
        assert_eq!(bar.height(), theme.size.tool_bar);
        assert!(bar.width() < canvas.width());
    }
}

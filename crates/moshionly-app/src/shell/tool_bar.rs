//! The tool bar that floats over the bottom of the canvas.

use egui::{
    Align, Align2, Color32, Layout, Popup, PopupCloseBehavior, Rect, Sense, Stroke, Ui, UiBuilder,
    Vec2, WidgetInfo, pos2,
};

use crate::{
    commands::CommandId,
    theme::{Theme, icons},
    widgets::{MenuItem, ToolButton, paint, popup_frame, tooltip},
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
    /// Every tool. Order here is the order commands are matched in, not the
    /// tool bar: the primitive shapes share one button.
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

    /// Primitive shapes that share the shape button, rectangle first. The
    /// rectangle is the shape shown until another one is chosen.
    pub const SHAPES: [Self; 3] = [Self::Rectangle, Self::Ellipse, Self::Star];

    /// Whether this tool draws a primitive shape.
    pub fn is_shape(self) -> bool {
        Self::SHAPES.contains(&self)
    }

    /// The shape the toolbar button shows. While a shape tool is active it
    /// shows that shape; otherwise it keeps showing `remembered`.
    pub fn shown_shape(active: Self, remembered: Self) -> Self {
        if active.is_shape() {
            active
        } else if remembered.is_shape() {
            remembered
        } else {
            Self::Rectangle
        }
    }

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

    /// Buttons on the bar besides the shared shape button.
    pub const BAR: [Self; 5] = [
        Self::Select,
        Self::Artboard,
        Self::Pen,
        Self::Text,
        Self::Image,
    ];
}

/// The rectangle the tool bar occupies over `canvas`: centred, a little above
/// the bottom edge.
pub fn tool_bar_rect(canvas: Rect, theme: &Theme) -> Rect {
    let size = &theme.size;
    let gap = theme.space.xxs;
    let pad = (size.tool_bar - size.tool_button) / 2.0;
    // The primitive shapes share one button, which is a caret wider than
    // the others.
    let tools = (Tool::BAR.len() + 1) as f32;
    let width = pad
        + tools * size.tool_button
        + SHAPE_CARET
        + (tools - 1.0) * gap
        + DIVIDER_WIDTH
        + AI_WIDTH
        + pad;
    Rect::from_center_size(
        pos2(
            canvas.center().x.round(),
            canvas.bottom() - size.tool_bar_inset - size.tool_bar / 2.0,
        ),
        Vec2::new(width, size.tool_bar),
    )
}

/// Tools drawn before the shape button, then the ones drawn after it.
const BEFORE_SHAPES: [Tool; 2] = [Tool::Select, Tool::Artboard];
const AFTER_SHAPES: [Tool; 3] = [Tool::Pen, Tool::Text, Tool::Image];

/// Room for the hairline between the tools and the AI entry.
const DIVIDER_WIDTH: f32 = 13.0;
/// Width of the AI entry.
const AI_WIDTH: f32 = 82.0;
/// Extra width of the shape button, where its caret sits.
const SHAPE_CARET: f32 = 16.0;

/// Draws the tool bar over `canvas` and returns the command of the button
/// that was clicked, if any: a tool command or [`CommandId::AskAi`].
///
/// `shape` is the primitive last chosen from the shape button. The button
/// shows that shape's icon, unless `active` is itself a shape.
pub fn tool_bar(ui: &mut Ui, canvas: Rect, active: Tool, shape: Tool) -> Option<CommandId> {
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
    for tool in BEFORE_SHAPES {
        if tool_button(&mut bar, tool, active) {
            clicked = Some(tool.command());
        }
    }
    if let Some(command) = shape_button(
        &mut bar,
        Tool::shown_shape(active, shape),
        active.is_shape(),
    ) {
        clicked = Some(command);
    }
    for tool in AFTER_SHAPES {
        if tool_button(&mut bar, tool, active) {
            clicked = Some(tool.command());
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

/// One tool button. Returns whether it was clicked.
fn tool_button(ui: &mut Ui, tool: Tool, active: Tool) -> bool {
    let command = tool.command();
    let shortcut = command.shortcut_label().unwrap_or_default();
    let icon = command.icon().unwrap_or(icons::CURSOR);
    let mut button = ToolButton::new(icon, command.name()).selected(tool == active);
    if !shortcut.is_empty() {
        button = button.shortcut(&shortcut);
    }
    ui.add(button).clicked()
}

/// The shape button: the icon of `shape` activates that tool, and the caret
/// opens the list of primitive shapes. `selected` is whether a shape tool
/// is the active tool, so the button shows the accent tile.
fn shape_button(ui: &mut Ui, shape: Tool, selected: bool) -> Option<CommandId> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let height = theme.size.tool_button;
    let width = height + SHAPE_CARET;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let icon_rect = Rect::from_min_size(rect.min, Vec2::splat(height));
    let caret_rect = Rect::from_min_max(pos2(icon_rect.right(), rect.top()), rect.max);

    let icon = ui.interact(icon_rect, ui.scope_id().with("shape"), Sense::click());
    let caret = ui.interact(caret_rect, ui.scope_id().with("shape-menu"), Sense::click());
    let command = shape.command();
    let name = command.name();
    icon.widget_info(|| WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, name));
    caret.widget_info(|| WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Shapes"));

    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&caret));
    let hovered = icon.hovered() || caret.hovered() || open;
    let pressed = icon.is_pointer_button_down_on() || caret.is_pointer_button_down_on();
    if ui.is_rect_visible(rect) {
        let (fill, ink) = if selected {
            (c.accent, c.text_on_accent)
        } else if pressed {
            (c.well_pressed, c.text)
        } else if hovered {
            (c.well_hover, c.text)
        } else {
            (Color32::TRANSPARENT, c.text)
        };
        ui.painter().rect_filled(rect, theme.radius.sm + 1.0, fill);
        let icon_glyph = command.icon().unwrap_or(icons::RECTANGLE);
        icons::paint(
            ui.painter(),
            icon_rect.center(),
            icon_glyph,
            theme.size.icon_large,
            ink,
        );
        let caret_ink = if selected {
            c.text_on_accent
        } else {
            c.text_secondary
        };
        icons::paint(
            ui.painter(),
            caret_rect.center(),
            icons::CARET_DOWN,
            10.0,
            caret_ink,
        );
    }

    let mut picked = None;
    Popup::menu(&caret)
        .align(egui::RectAlign {
            parent: Align2::LEFT_TOP,
            child: Align2::LEFT_BOTTOM,
        })
        .gap(theme.space.xs)
        .width(theme.size.menu_width * 0.85)
        .close_behavior(PopupCloseBehavior::CloseOnClick)
        .frame(popup_frame(ui))
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
            for tool in Tool::SHAPES {
                let command = tool.command();
                let shortcut = command.shortcut_label().unwrap_or_default();
                let mut item = MenuItem::new(command.name())
                    .icon(command.icon().unwrap_or(icons::RECTANGLE))
                    .checked(tool == shape);
                if !shortcut.is_empty() {
                    item = item.shortcut(&shortcut);
                }
                if ui.add(item).clicked() {
                    picked = Some(command);
                }
            }
        });

    let shortcut = command.shortcut_label().unwrap_or_default();
    let shortcut = (!shortcut.is_empty()).then_some(shortcut.as_str());
    let icon = tooltip(icon, name, shortcut);
    tooltip(caret, "Shapes", None);
    // A choice from the menu wins over a click on the icon in the same frame.
    picked.or(icon.clicked().then_some(command))
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
    fn the_shape_button_remembers_the_last_shape() {
        assert_eq!(Tool::SHAPES[0], Tool::Rectangle);
        assert!(Tool::SHAPES.iter().all(|tool| tool.is_shape()));
        for tool in Tool::BAR {
            assert!(!tool.is_shape(), "{tool:?} has its own button");
        }
        assert_eq!(
            Tool::shown_shape(Tool::Select, Tool::Rectangle),
            Tool::Rectangle
        );
        // Choosing a shape, from the menu or its key, becomes what the
        // button shows.
        assert_eq!(
            Tool::shown_shape(Tool::Ellipse, Tool::Rectangle),
            Tool::Ellipse
        );
        // Leaving the shape tool keeps that icon on the button.
        assert_eq!(Tool::shown_shape(Tool::Select, Tool::Star), Tool::Star);
        assert_eq!(Tool::shown_shape(Tool::Pen, Tool::Star), Tool::Star);
        // A remembered value that is not a shape falls back to the rectangle.
        assert_eq!(Tool::shown_shape(Tool::Text, Tool::Select), Tool::Rectangle);
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

//! Small pieces shared by the parts of the shell.

use egui::{Align, Align2, Color32, Layout, Rect, Sense, Ui, UiBuilder, Vec2, pos2};

use crate::{
    commands::{CommandId, CommandRegistry},
    theme::{Theme, Tone},
    widgets::{MenuItem, paint, tooltip},
};

/// Fills `rect` and runs `add` in a child that is laid out top to bottom,
/// clipped to the rectangle, with no gaps between items.
pub fn panel<R>(
    ui: &mut Ui,
    id_salt: &str,
    rect: Rect,
    fill: Color32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.painter().rect_filled(rect, 0.0, fill);
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing.y = 0.0;
    add(&mut child)
}

/// Which side of a rectangle a hairline is drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// The first row of the rectangle.
    Top,
    /// The last row.
    Bottom,
    /// The first column.
    Left,
    /// The last column.
    Right,
}

/// Paints a one-point border line just inside one edge of `rect`.
///
/// The line is a filled strip rather than a stroke, so it lands on whole
/// pixels at every scale.
pub fn edge_line(ui: &Ui, rect: Rect, edge: Edge) {
    let color = Theme::of(ui).color.border;
    let strip = match edge {
        Edge::Top => Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + 1.0)),
        Edge::Bottom => Rect::from_min_max(pos2(rect.left(), rect.bottom() - 1.0), rect.max),
        Edge::Left => Rect::from_min_max(rect.min, pos2(rect.left() + 1.0, rect.bottom())),
        Edge::Right => Rect::from_min_max(pos2(rect.right() - 1.0, rect.top()), rect.max),
    };
    ui.painter().rect_filled(strip, 0.0, color);
}

/// A small "Sample" tag for the header of a panel that shows placeholder
/// content instead of the document.
pub fn sample_tag(ui: &mut Ui) {
    let theme = Theme::of(ui);
    let tone = theme.color.tone(Tone::Amber);
    let galley = paint::line(ui, "Sample", theme.caption(), tone.text);
    let size = Vec2::new(galley.size().x + theme.space.sm * 2.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, theme.radius.xs, tone.soft);
    paint::galley_in(ui.painter(), rect, Align2::CENTER_CENTER, galley);
    tooltip(
        response,
        "Placeholder content: this panel is not connected to a document yet",
        None,
    );
}

/// One menu line for a command, with its icon and shortcut, greyed out when
/// the command is disabled. `checked` makes it a checkable item. Returns true
/// when it was clicked.
pub fn command_item(
    ui: &mut Ui,
    id: CommandId,
    commands: &CommandRegistry,
    checked: Option<bool>,
) -> bool {
    let shortcut = id.shortcut_label().unwrap_or_default();
    let mut item = MenuItem::new(commands.label(id));
    if let Some(icon) = id.icon() {
        item = item.icon(icon);
    }
    if let Some(checked) = checked {
        item = item.checked(checked);
    }
    if !shortcut.is_empty() {
        item = item.shortcut(&shortcut);
    }
    ui.add_enabled(commands.is_enabled(id), item).clicked()
}

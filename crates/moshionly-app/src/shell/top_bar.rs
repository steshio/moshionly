//! The bar across the top of the window.
//!
//! Left to right: the app mark, the project name (which opens the main menu),
//! undo and redo; then, from the right edge inwards, the chat toggle, Export,
//! play, and the zoom menu. When the bar doubles as the window's title area,
//! its empty space drags the window.

use egui::{
    Align, Align2, Color32, FontId, Layout, Popup, PopupCloseBehavior, Rect, Response, Sense, Ui,
    UiBuilder, Vec2, pos2,
};

use super::parts::command_item;
use crate::{
    commands::{Category, CommandId, CommandRegistry},
    theme::{Theme, ThemeMode, brand, icons},
    widgets::{IconButton, MenuItem, TextButton, paint, popup_frame},
};

/// What the top bar shows.
pub struct TopBar<'a> {
    /// The project's name.
    pub project_name: &'a str,
    /// Whether the project has changes that are not saved.
    pub unsaved: bool,
    /// The canvas zoom, where 1 is 100%.
    pub zoom: f32,
    /// Whether the animation is playing.
    pub playing: bool,
    /// Whether the chat panel is open.
    pub chat_open: bool,
    /// The theme choice, for the check marks in the menu.
    pub theme_mode: ThemeMode,
    /// Which commands are enabled.
    pub commands: &'a CommandRegistry,
    /// Room to leave at the left edge for the window's own buttons.
    pub leading_inset: f32,
    /// Whether the bar's empty space drags the window.
    pub drags_window: bool,
}

/// What happened in the top bar this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopBarOutput {
    /// Commands chosen from buttons and menus.
    pub commands: Vec<CommandId>,
    /// The user began dragging the bar's empty space.
    pub drag_window: bool,
    /// The user double-clicked the bar's empty space.
    pub toggle_maximized: bool,
}

/// Draws the top bar in `rect`.
pub fn top_bar(ui: &mut Ui, rect: Rect, bar: &TopBar<'_>) -> TopBarOutput {
    let theme = Theme::of(ui);
    let mut out = TopBarOutput::default();

    // The drag area goes first so every button sits on top of it.
    if bar.drags_window {
        let background = ui.interact(
            rect,
            ui.scope_id().with("window-drag"),
            Sense::click_and_drag(),
        );
        out.drag_window = background.drag_started();
        out.toggle_maximized = background.double_clicked();
    }

    let run = |id: CommandId, clicked: bool, out: &mut TopBarOutput| {
        if clicked {
            out.commands.push(id);
        }
    };

    // Left group.
    let mut left = ui.new_child(
        UiBuilder::new()
            .id_salt("left")
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    left.spacing_mut().item_spacing.x = theme.space.xxs;
    left.add_space(bar.leading_inset.max(theme.space.lg));
    let (mark, _) = left.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    brand::paint_mark(left.painter(), mark, theme);
    left.add_space(theme.space.sm);

    let file = caret_button(&mut left, bar.project_name, theme.title(), "Main menu");
    if bar.unsaved {
        let (dot, response) = left.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        left.painter()
            .circle_filled(dot.center(), 3.0, theme.color.text_secondary);
        crate::widgets::tooltip(response, "Unsaved changes", None);
    }
    Popup::menu(&file)
        .frame(popup_frame(&left))
        .gap(theme.space.xs)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            if let Some(id) = main_menu(ui, bar) {
                out.commands.push(id);
                ui.close();
            }
        });

    left.add_space(theme.space.md);
    for id in [CommandId::Undo, CommandId::Redo] {
        let clicked = command_button(&mut left, id, bar.commands).clicked();
        run(id, clicked, &mut out);
    }

    // Right group, from the edge inwards.
    let mut right = ui.new_child(
        UiBuilder::new()
            .id_salt("right")
            .max_rect(rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    right.spacing_mut().item_spacing.x = theme.space.md;
    right.add_space(theme.space.lg);

    let chat = right.add(
        IconButton::new(icons::CHAT_TEARDROP_TEXT)
            .selected(bar.chat_open)
            .tooltip(if bar.chat_open {
                "Hide chat"
            } else {
                "Show chat"
            })
            .shortcut(&CommandId::ToggleChat.shortcut_label().unwrap_or_default()),
    );
    run(CommandId::ToggleChat, chat.clicked(), &mut out);

    let export = right.add_enabled(
        bar.commands.is_enabled(CommandId::Export),
        TextButton::primary("Export")
            .icon(icons::EXPORT)
            .tooltip("Export this scene")
            .shortcut(&CommandId::Export.shortcut_label().unwrap_or_default()),
    );
    run(CommandId::Export, export.clicked(), &mut out);

    let play = right.add(
        IconButton::new(if bar.playing {
            icons::PAUSE
        } else {
            icons::PLAY
        })
        .tooltip(if bar.playing { "Pause" } else { "Play" })
        .shortcut(&CommandId::PlayPause.shortcut_label().unwrap_or_default()),
    );
    run(CommandId::PlayPause, play.clicked(), &mut out);

    let zoom = caret_button(&mut right, &zoom_text(bar.zoom), theme.body(), "Zoom");
    Popup::menu(&zoom)
        .frame(popup_frame(&right))
        .gap(theme.space.xs)
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
            ui.set_min_width(theme.size.menu_width - theme.space.xs * 2.0);
            for id in [
                CommandId::ZoomIn,
                CommandId::ZoomOut,
                CommandId::ZoomToFit,
                CommandId::ZoomTo100,
            ] {
                if command_item(ui, id, bar.commands, None) {
                    out.commands.push(id);
                }
            }
        });

    out
}

/// The zoom as the percentage shown on the zoom button.
pub fn zoom_text(zoom: f32) -> String {
    let percent = zoom * 100.0;
    if !percent.is_finite() {
        "100%".to_owned()
    } else if percent < 10.0 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.0}%")
    }
}

/// An icon button for a command, with its name and shortcut as the tooltip,
/// disabled when the command is.
fn command_button(ui: &mut Ui, id: CommandId, commands: &CommandRegistry) -> Response {
    let shortcut = id.shortcut_label().unwrap_or_default();
    ui.add_enabled(
        commands.is_enabled(id),
        IconButton::new(id.icon().unwrap_or(icons::QUESTION))
            .tooltip(commands.label(id))
            .shortcut(&shortcut),
    )
}

/// A quiet button made of a text and a small caret, used to open a menu.
fn caret_button(ui: &mut Ui, text: &str, font: FontId, name: &str) -> Response {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let label = paint::elided(ui, text, font, c.text, 240.0);
    let caret = 14.0;
    let pad = theme.space.md;
    let width = pad + label.size().x + theme.space.xxs + caret + pad - 4.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme.size.control + 4.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{name}: {text}"))
    });

    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));
    let fill = if open || response.is_pointer_button_down_on() {
        c.well_pressed
    } else if response.hovered() {
        c.well_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme.radius.sm, fill);
    let text_rect = Rect::from_min_max(pos2(rect.left() + pad, rect.top()), rect.max);
    let label_width = label.size().x;
    paint::galley_in(ui.painter(), text_rect, Align2::LEFT_CENTER, label);
    icons::paint(
        ui.painter(),
        pos2(
            text_rect.left() + label_width + theme.space.xxs + caret / 2.0,
            rect.center().y + 0.5,
        ),
        icons::CARET_DOWN,
        10.0,
        c.text_secondary,
    );
    response
}

/// Width of the category column of the main menu.
const CATEGORY_WIDTH: f32 = 128.0;
/// Width of the command column of the main menu.
const COMMAND_WIDTH: f32 = 248.0;

/// The main menu: categories on the left, the commands of the category under
/// the pointer on the right. Returns the command that was chosen.
fn main_menu(ui: &mut Ui, bar: &TopBar<'_>) -> Option<CommandId> {
    let theme = Theme::of(ui);
    let memory = ui.scope_id().with("category");
    let mut active: Category = ui.data(|d| d.get_temp(memory)).unwrap_or(Category::File);
    let mut chosen = None;

    // Tall enough for the longest category, so the menu does not change
    // size as the pointer moves down the categories.
    let row = theme.size.list_row + 1.0;
    let longest = Category::ALL
        .iter()
        .map(|category| {
            CommandId::ALL
                .iter()
                .filter(|id| id.category() == *category)
                .count()
        })
        .max()
        .unwrap_or(0);
    let height = longest as f32 * row;

    ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
    let columns = ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(CATEGORY_WIDTH);
            ui.set_min_height(height);
            for category in Category::ALL {
                let item = ui.add(
                    MenuItem::new(category.label())
                        .icon(category_icon(category))
                        .submenu(true)
                        .highlighted(category == active),
                );
                if item.hovered() || item.clicked() {
                    active = category;
                }
            }
        });
        ui.add_space(theme.space.xs * 2.0 + 1.0);
        ui.vertical(|ui| {
            ui.set_width(COMMAND_WIDTH);
            for id in CommandId::ALL
                .iter()
                .copied()
                .filter(|id| id.category() == active)
            {
                let checked = match id {
                    CommandId::ThemeSystem => Some(bar.theme_mode == ThemeMode::System),
                    CommandId::ThemeLight => Some(bar.theme_mode == ThemeMode::Light),
                    CommandId::ThemeDark => Some(bar.theme_mode == ThemeMode::Dark),
                    _ => None,
                };
                if command_item(ui, id, bar.commands, checked) {
                    chosen = Some(id);
                }
            }
        });
    });
    let rect = columns.response.rect;
    ui.painter().vline(
        rect.left() + CATEGORY_WIDTH + theme.space.xs + 0.5,
        rect.y_range(),
        theme.hairline(),
    );

    ui.data_mut(|d| d.insert_temp(memory, active));
    chosen
}

fn category_icon(category: Category) -> &'static str {
    match category {
        Category::File => icons::FILE,
        Category::Edit => icons::PEN_NIB,
        Category::View => icons::EYE,
        Category::Playback => icons::PLAY,
        Category::Tools => icons::CURSOR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_reads_as_a_percentage() {
        assert_eq!(zoom_text(1.0), "100%");
        assert_eq!(zoom_text(0.5), "50%");
        assert_eq!(zoom_text(0.874), "87%");
        assert_eq!(zoom_text(0.051), "5.1%");
        assert_eq!(zoom_text(32.0), "3200%");
        assert_eq!(zoom_text(f32::NAN), "100%");
    }
}

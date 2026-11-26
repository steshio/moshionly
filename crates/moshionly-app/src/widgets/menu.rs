//! Menus: popup frames, menu items, dropdowns and context menus.

use egui::{
    Align2, Color32, Frame, Margin, Popup, PopupCloseBehavior, Rect, Response, Sense, Stroke, Ui,
    Vec2, Widget, WidgetInfo,
};

use super::paint;
use crate::theme::{Theme, corner, icons};

/// The frame of everything that pops up: menus, dropdown lists, pickers.
pub fn popup_frame(ui: &Ui) -> Frame {
    let theme = Theme::of(ui);
    Frame::new()
        .fill(theme.color.raised)
        .stroke(Stroke::new(1.0, theme.color.border))
        .corner_radius(corner(theme.radius.md))
        .shadow(theme.shadow.popup)
        .inner_margin(Margin::same(theme.space.xs as i8))
}

/// One line of a menu.
///
/// ```ignore
/// if ui.add(MenuItem::new("Duplicate").icon(icons::COPY).shortcut("⌘D")).clicked() { /* ... */ }
/// ```
#[must_use = "add it to a Ui"]
pub struct MenuItem<'a> {
    label: &'a str,
    accessible_name: Option<&'a str>,
    icon: Option<&'a str>,
    shortcut: Option<&'a str>,
    checked: Option<bool>,
    danger: bool,
    highlighted: bool,
    submenu: bool,
}

impl<'a> MenuItem<'a> {
    /// An item labelled `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            accessible_name: None,
            icon: None,
            shortcut: None,
            checked: None,
            danger: false,
            highlighted: false,
            submenu: false,
        }
    }

    /// An icon in the leading slot.
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// What screen readers call the item when its label alone says too
    /// little, such as "Add glow" for "Glow" in a menu of effects to add.
    pub fn accessible_name(mut self, name: &'a str) -> Self {
        self.accessible_name = Some(name);
        self
    }

    /// A shortcut hint at the right edge.
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Makes the item checkable and sets its state. A check mark takes the
    /// leading slot when checked.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Colours the item as destructive.
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// Draws the item as if hovered, for keyboard navigation of a list.
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    /// Shows an arrow at the right edge: the item leads to a further list.
    pub fn submenu(mut self, submenu: bool) -> Self {
        self.submenu = submenu;
        self
    }
}

impl Widget for MenuItem<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let enabled = ui.is_enabled();
        let height = theme.size.list_row;
        let slot = theme.size.icon + theme.space.md;

        let label = paint::line(ui, self.label, theme.body(), Color32::PLACEHOLDER);
        let shortcut = self
            .shortcut
            .map(|s| paint::line(ui, s, theme.small(), Color32::PLACEHOLDER));
        let natural = theme.space.sm
            + slot
            + label.size().x
            + shortcut
                .as_ref()
                .map_or(0.0, |g| g.size().x + theme.space.xxl)
            + theme.space.lg;
        let width = ui.available_width().max(natural);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
        let name = self.accessible_name.unwrap_or(self.label);
        response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, name));

        if ui.is_rect_visible(rect) {
            let active = enabled && (response.hovered() || self.highlighted);
            if active {
                ui.painter()
                    .rect_filled(rect, theme.radius.sm, c.well_hover);
            }
            let ink = if !enabled {
                c.text_tertiary
            } else if self.danger {
                c.danger
            } else {
                c.text
            };
            let slot_center = egui::pos2(
                rect.left() + theme.space.sm + theme.size.icon / 2.0,
                rect.center().y,
            );
            if self.checked == Some(true) {
                icons::paint(ui.painter(), slot_center, icons::CHECK, 14.0, ink);
            } else if let Some(icon) = self.icon {
                let icon_ink = if self.danger || !enabled {
                    ink
                } else {
                    c.text_secondary
                };
                icons::paint(ui.painter(), slot_center, icon, theme.size.icon, icon_ink);
            }
            let text_rect = Rect::from_min_max(
                egui::pos2(rect.left() + theme.space.sm + slot, rect.top()),
                egui::pos2(rect.right() - theme.space.md, rect.bottom()),
            );
            let pos = Align2::LEFT_CENTER
                .align_size_within_rect(label.size(), text_rect)
                .min;
            ui.painter().galley(paint::snap(pos), label, ink);
            if self.submenu {
                icons::paint(
                    ui.painter(),
                    egui::pos2(text_rect.right() - 5.0, rect.center().y),
                    icons::CARET_RIGHT,
                    10.0,
                    c.text_secondary,
                );
            } else if let Some(shortcut) = shortcut {
                let pos = Align2::RIGHT_CENTER
                    .align_size_within_rect(shortcut.size(), text_rect)
                    .min;
                ui.painter()
                    .galley(paint::snap(pos), shortcut, c.text_secondary);
            }
        }
        response
    }
}

/// A hairline between groups of menu items.
pub fn menu_separator(ui: &mut Ui) {
    let theme = Theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.space.xs * 2.0 + 1.0),
        Sense::hover(),
    );
    ui.painter().hline(
        rect.x_range().shrink(theme.space.xs),
        rect.center().y,
        theme.hairline(),
    );
}

/// A small quiet heading above a group of menu items.
pub fn menu_heading(ui: &mut Ui, text: &str) {
    let theme = Theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.control),
        Sense::hover(),
    );
    paint::text_in(
        ui,
        rect.shrink2(Vec2::new(theme.space.md, 0.0)),
        Align2::LEFT_CENTER,
        text,
        theme.small_medium(),
        theme.color.text_secondary,
    );
}

/// Shows a context menu for `response` on secondary click. Fill it with
/// [`MenuItem`]s; clicking an item closes the menu.
pub fn context_menu(response: &Response, add_items: impl FnOnce(&mut Ui)) {
    let ctx = response.ctx.clone();
    let frame = {
        let theme = Theme::of_ctx(&ctx);
        Frame::new()
            .fill(theme.color.raised)
            .stroke(Stroke::new(1.0, theme.color.border))
            .corner_radius(corner(theme.radius.md))
            .shadow(theme.shadow.popup)
            .inner_margin(Margin::same(theme.space.xs as i8))
    };
    Popup::context_menu(response).frame(frame).show(|ui| {
        menu_body(ui, add_items);
    });
}

/// Sets up the spacing shared by all menus and runs `add_items`.
pub(crate) fn menu_body(ui: &mut Ui, add_items: impl FnOnce(&mut Ui)) {
    let theme = Theme::of(ui);
    ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
    ui.set_min_width(theme.size.menu_width - theme.space.xs * 2.0);
    add_items(ui);
}

/// The tallest a dropdown's list grows before it scrolls, in points.
const MAX_LIST_HEIGHT: f32 = 360.0;

/// A dropdown: a field showing the current choice that opens a list of all
/// choices.
///
/// ```ignore
/// Dropdown::new(&mut easing, "Easing", &[(Easing::Smooth, "Smooth"), (Easing::Bounce, "Bounce")])
///     .show(ui);
/// ```
#[must_use = "call show()"]
pub struct Dropdown<'a, T> {
    value: &'a mut T,
    name: &'a str,
    options: &'a [(T, &'a str)],
    icon: Option<&'a str>,
    width: Option<f32>,
    quiet: bool,
    mixed: bool,
}

impl<'a, T: PartialEq + Copy> Dropdown<'a, T> {
    /// A dropdown choosing `value` among `options`. `name` is what screen
    /// readers announce.
    pub fn new(value: &'a mut T, name: &'a str, options: &'a [(T, &'a str)]) -> Self {
        Self {
            value,
            name,
            options,
            icon: None,
            width: None,
            quiet: false,
            mixed: false,
        }
    }

    /// Says that the layers being edited differ: the field shows "Mixed",
    /// no option is checked, and picking any option sets it for all.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// A leading icon inside the field.
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A fixed width instead of the available width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Draws the field without a fill until hovered, for use in bars.
    pub fn quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Draws the dropdown. `changed()` on the response is true when the user
    /// picked a different option.
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let current = if self.mixed {
            "Mixed"
        } else {
            self.options
                .iter()
                .find(|(option, _)| *option == *self.value)
                .map_or("", |(_, label)| *label)
        };

        let width = self.width.unwrap_or_else(|| ui.available_width());
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, theme.size.control), Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(
                egui::Role::ComboBox,
                ui.is_enabled(),
                format!("{}: {current}", self.name),
            )
        });
        let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));

        if ui.is_rect_visible(rect) {
            let fill = if open || response.is_pointer_button_down_on() {
                c.well_pressed
            } else if response.hovered() {
                c.well_hover
            } else if self.quiet {
                Color32::TRANSPARENT
            } else {
                c.well
            };
            ui.painter().rect_filled(rect, theme.radius.sm, fill);
            let mut left = rect.left() + theme.space.md;
            if let Some(icon) = self.icon {
                icons::paint(
                    ui.painter(),
                    egui::pos2(left + 6.0, rect.center().y),
                    icon,
                    theme.size.icon - 2.0,
                    c.text_secondary,
                );
                left += theme.size.icon + theme.space.xxs;
            }
            let caret = egui::pos2(rect.right() - theme.space.md - 4.0, rect.center().y);
            icons::paint(
                ui.painter(),
                caret,
                icons::CARET_DOWN,
                10.0,
                c.text_secondary,
            );
            let text_rect = Rect::from_min_max(
                egui::pos2(left, rect.top()),
                egui::pos2(caret.x - 8.0, rect.bottom()),
            );
            paint::text_in(
                ui,
                text_rect,
                Align2::LEFT_CENTER,
                current,
                theme.body(),
                if self.mixed { c.text_secondary } else { c.text },
            );
        }

        let mut picked = None;
        Popup::menu(&response)
            .frame(popup_frame(ui))
            .gap(theme.space.xs)
            .width(rect.width().max(theme.size.menu_width * 0.7))
            .close_behavior(PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
                // Long lists, such as the installed fonts, scroll.
                egui::ScrollArea::vertical()
                    .max_height(MAX_LIST_HEIGHT)
                    .show(ui, |ui| {
                        for (option, label) in self.options {
                            let checked = !self.mixed && *option == *self.value;
                            let item = MenuItem::new(label).checked(checked);
                            if ui.add(item).clicked() {
                                picked = Some(*option);
                            }
                        }
                    });
            });
        if let Some(option) = picked
            && (option != *self.value || self.mixed)
        {
            *self.value = option;
            response.mark_changed();
        }
        response
    }
}

//! Buttons: icon buttons and toggles, tool buttons and text buttons.

use egui::{
    Align2, Color32, CursorIcon, Response, Role, Sense, Stroke, Ui, Vec2, Widget, WidgetInfo,
};

use super::{paint, tooltip::tooltip};
use crate::theme::{Theme, icons};

/// A small square button that shows one icon.
///
/// Quiet at rest, a soft well under the pointer. When `selected`, the icon
/// takes the accent colour on a pale accent tile.
///
/// ```ignore
/// if ui.add(IconButton::new(icons::PLUS).tooltip("Add action")).clicked() { /* ... */ }
/// ```
#[must_use = "add it to a Ui"]
pub struct IconButton<'a> {
    icon: &'a str,
    selected: bool,
    dimmed: bool,
    size: Option<f32>,
    icon_size: Option<f32>,
    tooltip: Option<&'a str>,
    shortcut: Option<&'a str>,
}

impl<'a> IconButton<'a> {
    /// A button showing `icon` (a constant from [`crate::theme::icons`]).
    pub fn new(icon: &'a str) -> Self {
        Self {
            icon,
            selected: false,
            dimmed: false,
            size: None,
            icon_size: None,
            tooltip: None,
            shortcut: None,
        }
    }

    /// Shows the button in its "on" state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Draws the icon in the secondary text colour until hovered.
    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    /// Overrides the side of the button (the control height by default).
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Overrides the size of the icon.
    pub fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = Some(size);
        self
    }

    /// Text shown when the pointer rests on the button.
    pub fn tooltip(mut self, text: &'a str) -> Self {
        self.tooltip = Some(text);
        self
    }

    /// A shortcut shown next to the tooltip text, for example `"⌘Z"`.
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let side = self.size.unwrap_or(theme.size.control);
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
        let enabled = ui.is_enabled();
        let name = self.tooltip.unwrap_or("Button");
        response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name));

        if ui.is_rect_visible(rect) {
            let (fill, ink) = if !enabled {
                (Color32::TRANSPARENT, c.text_tertiary)
            } else if self.selected {
                (c.accent_soft, c.accent_text)
            } else if response.is_pointer_button_down_on() {
                (c.well_pressed, c.text)
            } else if response.hovered() {
                (c.well_hover, c.text)
            } else if self.dimmed {
                (Color32::TRANSPARENT, c.text_secondary)
            } else {
                (Color32::TRANSPARENT, c.text)
            };
            ui.painter().rect_filled(rect, theme.radius.sm, fill);
            icons::paint(
                ui.painter(),
                rect.center(),
                self.icon,
                self.icon_size.unwrap_or(theme.size.icon),
                ink,
            );
        }

        match self.tooltip {
            Some(text) => tooltip(response, text, self.shortcut),
            None => response,
        }
    }
}

/// An icon button that flips a `bool` and shows a different icon for each
/// state, for example an eye for "visible" and a struck eye for "hidden".
///
/// Returns the response; `changed()` is true on the frame the value flipped.
pub fn icon_toggle(
    ui: &mut Ui,
    value: &mut bool,
    icon_on: &str,
    icon_off: &str,
    tooltip_text: &str,
) -> Response {
    let icon = if *value { icon_on } else { icon_off };
    let mut response = ui.add(IconButton::new(icon).dimmed(!*value).tooltip(tooltip_text));
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
}

/// A button on the tool bar: a large icon on a tile that turns solid accent
/// when its tool is active, with a tooltip that names the tool and its key.
#[must_use = "add it to a Ui"]
pub struct ToolButton<'a> {
    icon: &'a str,
    name: &'a str,
    shortcut: Option<&'a str>,
    selected: bool,
}

impl<'a> ToolButton<'a> {
    /// A tool button showing `icon`, named `name` in its tooltip.
    pub fn new(icon: &'a str, name: &'a str) -> Self {
        Self {
            icon,
            name,
            shortcut: None,
            selected: false,
        }
    }

    /// The key that selects the tool, shown in the tooltip.
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Whether this is the active tool.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl Widget for ToolButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::splat(theme.size.tool_button), Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(Role::Button, ui.is_enabled(), self.selected, self.name)
        });

        if ui.is_rect_visible(rect) {
            let (fill, ink) = if self.selected {
                (c.accent, c.text_on_accent)
            } else if response.is_pointer_button_down_on() {
                (c.well_pressed, c.text)
            } else if response.hovered() {
                (c.well_hover, c.text)
            } else {
                (Color32::TRANSPARENT, c.text)
            };
            ui.painter().rect_filled(rect, theme.radius.sm + 1.0, fill);
            icons::paint(
                ui.painter(),
                rect.center(),
                self.icon,
                theme.size.icon_large,
                ink,
            );
        }

        tooltip(response, self.name, self.shortcut)
    }
}

/// How much visual weight a [`TextButton`] carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one main action of a view: solid accent.
    Primary,
    /// An ordinary action: a soft well.
    #[default]
    Secondary,
    /// A low-key action: no fill until hovered.
    Quiet,
    /// A destructive action: solid red.
    Danger,
}

/// A button with a text label and an optional leading icon.
#[must_use = "add it to a Ui"]
pub struct TextButton<'a> {
    label: &'a str,
    icon: Option<&'a str>,
    kind: ButtonKind,
    fill_width: bool,
    tooltip: Option<&'a str>,
    shortcut: Option<&'a str>,
}

impl<'a> TextButton<'a> {
    /// A secondary button labelled `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            kind: ButtonKind::Secondary,
            fill_width: false,
            tooltip: None,
            shortcut: None,
        }
    }

    /// A primary button labelled `label`.
    pub fn primary(label: &'a str) -> Self {
        Self::new(label).kind(ButtonKind::Primary)
    }

    /// Sets the visual weight.
    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    /// Adds an icon before the label.
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Stretches the button to the available width.
    pub fn fill_width(mut self, fill: bool) -> Self {
        self.fill_width = fill;
        self
    }

    /// Text shown when the pointer rests on the button.
    pub fn tooltip(mut self, text: &'a str) -> Self {
        self.tooltip = Some(text);
        self
    }

    /// A shortcut shown next to the tooltip text.
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }
}

impl Widget for TextButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let enabled = ui.is_enabled();

        let font = theme.body_medium();
        let label = paint::line(ui, self.label, font, Color32::PLACEHOLDER);
        let icon_width = if self.icon.is_some() {
            theme.size.icon + theme.space.xs
        } else {
            0.0
        };
        let padding = theme.space.lg - 2.0;
        let natural = label.size().x + icon_width + padding * 2.0;
        let width = if self.fill_width {
            ui.available_width().max(natural)
        } else {
            natural
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, theme.size.control), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, self.label));

        if ui.is_rect_visible(rect) {
            let down = response.is_pointer_button_down_on();
            let hover = response.hovered();
            let (fill, ink, outline) = match self.kind {
                _ if !enabled => (c.well, c.text_tertiary, Stroke::NONE),
                ButtonKind::Primary => (
                    pick(down, hover, c.accent_pressed, c.accent_hover, c.accent),
                    c.text_on_accent,
                    Stroke::NONE,
                ),
                ButtonKind::Danger => (
                    pick(
                        down,
                        hover,
                        c.danger.gamma_multiply(0.8),
                        c.danger.gamma_multiply(0.9),
                        c.danger,
                    ),
                    c.text_on_accent,
                    Stroke::NONE,
                ),
                ButtonKind::Secondary => (
                    pick(down, hover, c.well_pressed, c.well_hover, c.surface),
                    c.text,
                    Stroke::new(1.0, c.border_strong),
                ),
                ButtonKind::Quiet => (
                    pick(
                        down,
                        hover,
                        c.well_pressed,
                        c.well_hover,
                        Color32::TRANSPARENT,
                    ),
                    c.text,
                    Stroke::NONE,
                ),
            };
            paint::panel(ui.painter(), rect, theme.radius.sm, fill, outline);

            let content_width = label.size().x + icon_width;
            let mut x = rect.center().x - content_width / 2.0;
            if let Some(icon) = self.icon {
                icons::paint(
                    ui.painter(),
                    egui::pos2(x + theme.size.icon / 2.0, rect.center().y),
                    icon,
                    theme.size.icon,
                    ink,
                );
                x += icon_width;
            }
            let text_rect = egui::Rect::from_min_max(
                egui::pos2(x, rect.top()),
                egui::pos2(rect.right(), rect.bottom()),
            );
            let pos = Align2::LEFT_CENTER
                .align_size_within_rect(label.size(), text_rect)
                .min;
            ui.painter().galley(paint::snap(pos), label, ink);
        }

        let response = if enabled {
            response.on_hover_cursor(CursorIcon::Default)
        } else {
            response
        };
        match self.tooltip {
            Some(text) => tooltip(response, text, self.shortcut),
            None => response,
        }
    }
}

fn pick<T>(down: bool, hover: bool, pressed: T, hovered: T, rest: T) -> T {
    if down {
        pressed
    } else if hover {
        hovered
    } else {
        rest
    }
}

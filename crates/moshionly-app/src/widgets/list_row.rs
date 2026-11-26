//! List rows: the indented, expandable, selectable lines of the layers list.

use egui::{Align, Align2, Color32, Layout, Rect, Response, Sense, Ui, UiBuilder, Vec2};

use super::paint;
use crate::theme::{Theme, icons};

/// One row of a tree list.
///
/// A row has an indent, an optional expand arrow, an icon and a label. It
/// shows hover and selected states and can carry small trailing controls
/// (visibility, lock) that appear on hover.
#[must_use = "call show()"]
pub struct ListRow<'a> {
    label: &'a str,
    icon: Option<&'a str>,
    depth: usize,
    expanded: Option<bool>,
    selected: bool,
    parent_selected: bool,
    dimmed: bool,
    quiet: bool,
    strong: bool,
    tint: Option<Color32>,
    height: Option<f32>,
    id_salt: Option<egui::IdSalt>,
}

/// What a [`ListRow`] reports back.
pub struct ListRowResponse {
    /// The whole row. `clicked()`, `double_clicked()` and `secondary_clicked()`
    /// are the row's gestures; drag is sensed too, for reordering.
    pub response: Response,
    /// True on the frame the expand arrow was clicked.
    pub toggled: bool,
    /// Whether the pointer is over the row, including its trailing controls.
    pub hovered: bool,
}

impl<'a> ListRow<'a> {
    /// A row labelled `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            depth: 0,
            expanded: None,
            selected: false,
            parent_selected: false,
            dimmed: false,
            quiet: false,
            strong: false,
            tint: None,
            height: None,
            id_salt: None,
        }
    }

    /// The icon before the label.
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Nesting level, 0 for top-level rows.
    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// Shows an expand arrow in the given state. Leave unset for leaf rows.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Shows the row as selected.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Gives the row a faint tint because an ancestor is selected.
    pub fn parent_selected(mut self, parent_selected: bool) -> Self {
        self.parent_selected = parent_selected;
        self
    }

    /// Draws label and icon faded, for hidden layers.
    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    /// Draws the label in the secondary text colour, for rows that are a
    /// detail of the row above them.
    pub fn quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// The row's height, when it has to line up with something other than
    /// a list (the list row height by default).
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Draws the label semibold, for top-level containers.
    pub fn strong(mut self, strong: bool) -> Self {
        self.strong = strong;
        self
    }

    /// Draws the icon and label in this colour, for rows that stand for a
    /// special kind of thing (main components and their instances). Hidden
    /// rows stay faded.
    pub fn tint(mut self, tint: Option<Color32>) -> Self {
        self.tint = tint;
        self
    }

    /// A stable id for the row. Needed when labels can repeat.
    pub fn id_salt(mut self, id: impl egui::AsIdSalt) -> Self {
        self.id_salt = Some(egui::IdSalt::new(id));
        self
    }

    /// Draws the row. `trailing` adds controls at the right edge, laid out
    /// right to left; it is told whether the row is hovered so it can hide
    /// controls that only matter on hover.
    pub fn show(self, ui: &mut Ui, trailing: impl FnOnce(&mut Ui, bool)) -> ListRowResponse {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let inset = theme.space.md;
        let (outer, _) = ui.allocate_exact_size(
            Vec2::new(
                ui.available_width(),
                self.height.unwrap_or(theme.size.list_row),
            ),
            Sense::hover(),
        );
        let rect = Rect::from_min_max(
            egui::pos2(outer.left() + inset, outer.top()),
            egui::pos2(outer.right() - inset, outer.bottom()),
        );
        let salt = self
            .id_salt
            .unwrap_or_else(|| egui::IdSalt::new(self.label));
        let id = ui.make_persistent_id(("list-row", salt));
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let (label, selected) = (self.label, self.selected);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::ListItem, true, selected, label)
        });
        let hovered = ui.rect_contains_pointer(rect);

        let fill = if self.selected {
            c.accent_soft
        } else if hovered {
            c.well
        } else if self.parent_selected {
            c.accent_soft.gamma_multiply(0.45)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, theme.radius.sm, fill);

        let mut x = rect.left() + theme.space.xs + self.depth as f32 * theme.size.list_indent;
        let arrow_width = 16.0;
        let mut toggled = false;
        if let Some(expanded) = self.expanded {
            let arrow_rect = Rect::from_min_size(
                egui::pos2(x, rect.top()),
                Vec2::new(arrow_width, rect.height()),
            );
            let arrow = ui.interact(arrow_rect, id.with("arrow"), Sense::click());
            let ink = if arrow.hovered() {
                c.text
            } else {
                c.text_tertiary
            };
            paint::disclosure(ui.painter(), arrow_rect.center(), expanded, ink);
            toggled = arrow.clicked();
        }
        x += arrow_width;

        let (ink, icon_ink) = if self.dimmed {
            (c.text_tertiary, c.text_tertiary)
        } else if let Some(tint) = self.tint {
            (tint, tint)
        } else if self.selected {
            (c.text, c.accent_text)
        } else if self.quiet {
            (c.text_secondary, c.text_secondary)
        } else {
            (c.text, c.text_secondary)
        };
        if let Some(icon) = self.icon {
            icons::paint(
                ui.painter(),
                egui::pos2(x + theme.size.icon / 2.0, rect.center().y),
                icon,
                theme.size.icon,
                icon_ink,
            );
            x += theme.size.icon + theme.space.sm;
        }

        // Trailing controls, right to left.
        let trailing_rect = Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(rect.right() - theme.space.xxs, rect.bottom()),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(trailing_rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 0.0;
        trailing(&mut child, hovered);
        let label_right = if child.min_rect().width() > 0.0 {
            child.min_rect().left() - theme.space.xs
        } else {
            rect.right() - theme.space.md
        };

        let label_rect = Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(label_right.max(x), rect.bottom()),
        );
        let font = if self.strong {
            theme.body_strong()
        } else {
            theme.body()
        };
        paint::text_in(ui, label_rect, Align2::LEFT_CENTER, self.label, font, ink);

        ListRowResponse {
            response,
            toggled,
            hovered,
        }
    }
}

/// A trailing toggle for a [`ListRow`]: a small icon that is always visible
/// while `value` is true (the layer is hidden or locked) and otherwise only
/// while the row is hovered.
///
/// Returns true when the value was flipped this frame.
pub fn row_toggle(
    ui: &mut Ui,
    value: &mut bool,
    icon_on: &str,
    icon_off: &str,
    row_hovered: bool,
) -> bool {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let side = theme.size.control;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    if *value || row_hovered {
        let ink = if response.hovered() {
            c.text
        } else if *value {
            c.text_secondary
        } else {
            c.text_tertiary
        };
        let icon = if *value { icon_on } else { icon_off };
        icons::paint(
            ui.painter(),
            rect.center(),
            icon,
            theme.size.icon - 2.0,
            ink,
        );
    }
    if response.clicked() {
        *value = !*value;
        true
    } else {
        false
    }
}

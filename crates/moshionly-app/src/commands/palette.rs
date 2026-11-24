//! The command palette: find and run any command by typing part of its name.

use egui::{Align2, Context, Frame, Id, Key, Margin, Modal, Modifiers, ScrollArea, Stroke, Vec2};

use super::{CommandId, CommandRegistry};
use crate::{
    theme::{Theme, corner, icons},
    widgets::{MenuItem, TextField, divider, empty_state, menu_heading},
};

/// How far below the top of the window the palette sits.
const TOP_OFFSET: f32 = 96.0;
/// Most rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 9.5;

/// The command palette's state: whether it is open, what is typed and which
/// result is highlighted.
#[derive(Clone, Debug, Default)]
pub struct Palette {
    open: bool,
    query: String,
    highlighted: usize,
    /// Give the search field keyboard focus on the next frame.
    focus: bool,
    /// Scroll the highlighted row into view on the next frame.
    reveal: bool,
}

impl Palette {
    /// Opens the palette with an empty search.
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            focus: true,
            ..Self::default()
        };
    }

    /// Closes the palette.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Opens the palette if it is closed and closes it if it is open.
    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Whether the palette is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// What is typed into the search field.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Moves the highlight by `step` rows through `count` results, wrapping
    /// round at both ends.
    fn step(&mut self, step: isize, count: usize) {
        self.highlighted = wrapped(self.highlighted, step, count);
        self.reveal = true;
    }

    /// Draws the palette if it is open and handles its keys: arrows move the
    /// highlight, Enter runs the highlighted command, Escape or a click
    /// outside closes. Returns the command to run; the palette closes itself
    /// when one is chosen.
    pub fn show(&mut self, ctx: &Context, registry: &CommandRegistry) -> Option<CommandId> {
        if !self.open {
            return None;
        }
        let theme = Theme::of_ctx(ctx);
        let c = &theme.color;
        let results = registry.search(&self.query);
        self.highlighted = self.highlighted.min(results.len().saturating_sub(1));

        // Take the navigation keys before the search field sees them.
        let (down, up, enter) = ctx.input_mut(|i| {
            (
                i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown),
                i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if down != up {
            self.step(down as isize - up as isize, results.len());
        }
        let mut chosen = enter
            .then(|| results.get(self.highlighted).copied())
            .flatten()
            .filter(|id| registry.is_enabled(*id));

        let id = Id::unique("moshionly-command-palette");
        let field_id = id.with("search");
        let frame = Frame::new()
            .fill(c.raised)
            .stroke(Stroke::new(1.0, c.border))
            .corner_radius(corner(theme.radius.lg))
            .shadow(theme.shadow.modal)
            .inner_margin(Margin::ZERO);
        let modal = Modal::new(id)
            .backdrop_color(c.scrim.gamma_multiply(0.35))
            .frame(frame)
            .area(Modal::default_area(id).anchor(Align2::CENTER_TOP, Vec2::new(0.0, TOP_OFFSET)))
            .show(ctx, |ui| {
                ui.set_width(theme.size.palette_width);
                ui.spacing_mut().item_spacing.y = 0.0;

                // Search field.
                Frame::new()
                    .inner_margin(Margin::same(theme.space.lg as i8))
                    .show(ui, |ui| {
                        let before = self.query.clone();
                        TextField::new(&mut self.query)
                            .hint("Search commands")
                            .icon(icons::MAGNIFYING_GLASS)
                            .id(field_id)
                            .show(ui);
                        if self.query != before {
                            self.highlighted = 0;
                            self.reveal = true;
                        }
                    });
                if std::mem::take(&mut self.focus) {
                    ui.memory_mut(|m| m.request_focus(field_id));
                }
                divider(ui);

                // Results.
                if results.is_empty() {
                    empty_state(
                        ui,
                        icons::MAGNIFYING_GLASS,
                        "No commands found",
                        "Nothing matches what you typed.",
                        None,
                    );
                    return;
                }
                let grouped = self.query.trim().is_empty();
                let row = theme.size.list_row + 1.0;
                ScrollArea::vertical()
                    .max_height(row * VISIBLE_ROWS)
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin::same(theme.space.xs as i8))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
                                let mut category = None;
                                for (index, command) in results.iter().copied().enumerate() {
                                    // With nothing typed the list reads as menus.
                                    if grouped && category != Some(command.category()) {
                                        category = Some(command.category());
                                        menu_heading(ui, command.category().label());
                                    }
                                    let shortcut = command.shortcut_label().unwrap_or_default();
                                    let mut item = MenuItem::new(registry.label(command))
                                        .highlighted(index == self.highlighted);
                                    if let Some(icon) = command.icon() {
                                        item = item.icon(icon);
                                    }
                                    if !shortcut.is_empty() {
                                        item = item.shortcut(&shortcut);
                                    }
                                    let response =
                                        ui.add_enabled(registry.is_enabled(command), item);
                                    if response.clicked() {
                                        chosen = Some(command);
                                    }
                                    if index == self.highlighted && self.reveal {
                                        response.scroll_to_me(None);
                                    }
                                }
                            });
                    });
                self.reveal = false;
            });

        if chosen.is_some() || modal.should_close() {
            self.close();
        }
        chosen
    }
}

/// The index `step` rows from `index` in a list of `count`, wrapping round.
fn wrapped(index: usize, step: isize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (index as isize + step).rem_euclid(count as isize) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_highlight_wraps_round_the_list() {
        assert_eq!(wrapped(0, 1, 5), 1);
        assert_eq!(wrapped(4, 1, 5), 0);
        assert_eq!(wrapped(0, -1, 5), 4);
        assert_eq!(wrapped(2, -7, 5), 0);
        assert_eq!(wrapped(3, 1, 0), 0);
    }

    #[test]
    fn opening_starts_a_fresh_search() {
        let mut palette = Palette::default();
        assert!(!palette.is_open());
        palette.toggle();
        assert!(palette.is_open());
        palette.query.push_str("zoom");
        palette.step(2, 4);
        assert_eq!(palette.highlighted, 2);
        palette.toggle();
        assert!(!palette.is_open());
        palette.open();
        assert_eq!(palette.query(), "");
        assert_eq!(palette.highlighted, 0);
    }
}

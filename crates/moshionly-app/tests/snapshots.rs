//! Headless screenshots of the editor's interface.
//!
//! Each test renders a piece of the interface with the real theme, without
//! opening a window, in light and dark at 1x and 2x, and writes PNG files (see
//! [`common::output_dir`]). Look at them when you change the theme or a
//! widget. The tests only assert that rendering worked; they do not compare
//! pixels, because graphics adapters differ.

mod common;

use common::{Variant, assert_rendered, can_render, snapshot};
use egui::vec2;
use moshionly_app::{
    theme::{Theme, icons},
    widgets::*,
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Design,
    Animate,
}

fn controls_gallery(ui: &mut egui::Ui) {
    let theme = Theme::of(ui);
    let mut mode = Mode::Design;
    let mut open = true;
    let mut closed = false;
    let mut name = String::from("Hero card");
    let mut query = String::new();
    let mut visible = true;

    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(theme.space.md);
    padded(ui, |ui| {
        segmented(
            ui,
            &mut mode,
            &[(Mode::Design, "Design"), (Mode::Animate, "Animate")],
        );
        ui.add_space(theme.space.md);
    });

    section(
        ui,
        "Buttons",
        &mut open,
        |ui| {
            ui.add(IconButton::new(icons::PLUS).tooltip("Add"));
            ui.add(IconButton::new(icons::SLIDERS_HORIZONTAL).dimmed(true));
        },
        |ui| {
            control_row(ui, |ui| {
                ui.add(TextButton::primary("Export").icon(icons::EXPORT));
                ui.add(TextButton::new("Secondary"));
                ui.add(TextButton::new("Quiet").kind(ButtonKind::Quiet));
            });
            control_row(ui, |ui| {
                ui.add(IconButton::new(icons::ARROW_U_UP_LEFT));
                ui.add(IconButton::new(icons::MAGNET).selected(true));
                icon_toggle(
                    ui,
                    &mut visible,
                    icons::EYE,
                    icons::EYE_SLASH,
                    "Show or hide",
                );
                ui.add(ToolButton::new(icons::CURSOR, "Select").selected(true));
                ui.add(ToolButton::new(icons::RECTANGLE, "Rectangle"));
            });
        },
    );
    section(
        ui,
        "Text",
        &mut open,
        |_| {},
        |ui| {
            property_row(ui, "Name", |ui| {
                TextField::new(&mut name).show(ui);
            });
            control_row(ui, |ui| {
                search_field(ui, &mut query);
            });
        },
    );
    section(ui, "Collapsed", &mut closed, |_| {}, |_| {});
    divider(ui);

    ListRow::new("Scene 1")
        .icon(icons::FRAME_CORNERS)
        .expanded(true)
        .strong(true)
        .show(ui, |_, _| {});
    ListRow::new("Card")
        .icon(icons::RECTANGLE)
        .depth(1)
        .selected(true)
        .show(ui, |ui, hovered| {
            let mut locked = true;
            let mut hidden = false;
            row_toggle(ui, &mut hidden, icons::EYE_SLASH, icons::EYE, hovered);
            row_toggle(
                ui,
                &mut locked,
                icons::LOCK_SIMPLE,
                icons::LOCK_SIMPLE_OPEN,
                hovered,
            );
        });
    ListRow::new("Title with a long name that will not fit")
        .icon(icons::TEXT_T)
        .depth(1)
        .show(ui, |_, _| {});
    ListRow::new("Hidden photo")
        .icon(icons::IMAGE)
        .depth(1)
        .dimmed(true)
        .show(ui, |_, _| {});
    divider(ui);
    empty_state(
        ui,
        icons::LIGHTNING,
        "No actions yet",
        "Select a layer and add an action to make it move.",
        Some("Add action"),
    );
}

#[test]
fn controls() {
    if !can_render() {
        return;
    }
    let size = vec2(280.0, 760.0);
    for variant in Variant::ALL {
        let (path, image) = snapshot("controls", size, variant, controls_gallery);
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

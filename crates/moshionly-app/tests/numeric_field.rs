//! Drives a numeric field the way a user would, without a window.

use egui::{Key, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use moshionly_app::{theme, widgets::NumericField};

fn harness(start: f64) -> Harness<'static, f64> {
    let mut harness = Harness::builder()
        .with_size(vec2(200.0, 60.0))
        .with_context_setup(theme::install)
        .build_ui_state(
            |ui, value: &mut f64| {
                NumericField::new(value, "Width")
                    .label("W")
                    .silent_unit("px")
                    .range(0.0..=1000.0)
                    .width(120.0)
                    .show(ui);
            },
            start,
        );
    harness.run();
    harness
}

#[test]
fn typing_a_calculation_sets_the_value() {
    let mut harness = harness(40.0);
    harness.get_by_label("Width").click();
    harness.run();
    harness.get_by_label("Width").type_text("100+20");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(*harness.state(), 120.0);
}

#[test]
fn percent_is_relative_to_the_current_value() {
    let mut harness = harness(300.0);
    harness.get_by_label("Width").click();
    harness.run();
    harness.get_by_label("Width").type_text("50%");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(*harness.state(), 150.0);
}

#[test]
fn escape_cancels_and_nonsense_is_ignored() {
    let mut harness = harness(40.0);
    harness.get_by_label("Width").click();
    harness.run();
    harness.get_by_label("Width").type_text("999");
    harness.run();
    harness.key_press(Key::Escape);
    harness.run();
    assert_eq!(*harness.state(), 40.0);

    harness.get_by_label("Width").click();
    harness.run();
    harness.get_by_label("Width").type_text("12 apples");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(*harness.state(), 40.0);
}

#[test]
fn typed_values_are_clamped_to_the_range() {
    let mut harness = harness(40.0);
    harness.get_by_label("Width").click();
    harness.run();
    harness.get_by_label("Width").type_text("5000");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(*harness.state(), 1000.0);
}

#[test]
fn arrow_keys_step_while_typing() {
    let mut harness = harness(40.0);
    harness.get_by_label("Width").click();
    harness.run();
    harness.key_press(Key::ArrowUp);
    harness.run();
    assert_eq!(*harness.state(), 41.0);
    harness.key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowDown);
    harness.run();
    assert_eq!(*harness.state(), 31.0);
}

#[test]
fn dragging_scrubs_the_value() {
    let mut harness = harness(40.0);
    let center = harness.get_by_label("Width").rect().center();
    harness.drag_at(center);
    harness.run();
    harness.hover_at(pos2(center.x + 25.0, center.y));
    harness.run();
    assert_eq!(*harness.state(), 65.0);
    harness.drop_at(pos2(center.x + 25.0, center.y));
    harness.run();
    assert_eq!(*harness.state(), 65.0);
}

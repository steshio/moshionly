//! Drives the command palette the way a user would, without a window.

use egui::{Key, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use moshionly_app::{
    commands::{CommandId, CommandRegistry, Palette},
    theme,
};

struct State {
    palette: Palette,
    registry: CommandRegistry,
    chosen: Vec<CommandId>,
}

fn harness() -> Harness<'static, State> {
    let mut palette = Palette::default();
    palette.open();
    let mut harness = Harness::builder()
        .with_size(vec2(800.0, 600.0))
        .with_context_setup(theme::install)
        .build_ui_state(
            |ui, state: &mut State| {
                state
                    .chosen
                    .extend(state.palette.show(ui.ctx(), &state.registry));
            },
            State {
                palette,
                registry: CommandRegistry::new(),
                chosen: Vec::new(),
            },
        );
    harness.run();
    harness
}

#[test]
fn typing_and_enter_runs_the_best_match() {
    let mut harness = harness();
    harness
        .get_by_label("Search commands")
        .type_text("theme dark");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    let state = harness.state();
    assert_eq!(state.chosen, [CommandId::ThemeDark]);
    assert!(!state.palette.is_open());
}

#[test]
fn the_field_has_focus_as_soon_as_the_palette_opens() {
    let mut harness = harness();
    // No click: the keys go straight to the search field.
    harness.event(egui::Event::Text("export".to_owned()));
    harness.run();
    assert_eq!(harness.state().palette.query(), "export");
}

#[test]
fn arrows_move_through_the_results() {
    let mut harness = harness();
    harness.get_by_label("Search commands").type_text("zoom");
    harness.run();
    // Zoom in, Zoom out, Zoom to fit, Zoom to 100%: two down is "to fit".
    harness.key_press(Key::ArrowDown);
    harness.key_press(Key::ArrowDown);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().chosen, [CommandId::ZoomToFit]);
}

#[test]
fn up_from_the_first_result_wraps_to_the_last() {
    let mut harness = harness();
    harness.get_by_label("Search commands").type_text("zoom");
    harness.run();
    harness.key_press(Key::ArrowUp);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().chosen, [CommandId::ZoomTo100]);
}

#[test]
fn a_disabled_command_does_not_run() {
    let mut harness = harness();
    harness
        .state_mut()
        .registry
        .set_enabled(CommandId::Undo, false);
    harness.get_by_label("Search commands").type_text("undo");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    let state = harness.state();
    assert!(state.chosen.is_empty());
    assert!(state.palette.is_open());
}

#[test]
fn escape_closes_without_running_anything() {
    let mut harness = harness();
    harness.get_by_label("Search commands").type_text("save");
    harness.run();
    harness.key_press(Key::Escape);
    harness.run();
    let state = harness.state();
    assert!(state.chosen.is_empty());
    assert!(!state.palette.is_open());
}

#[test]
fn clicking_a_result_runs_it() {
    let mut harness = harness();
    harness.get_by_label("Search commands").type_text("loop");
    harness.run();
    harness.get_by_label("Loop playback").click();
    harness.run();
    assert_eq!(harness.state().chosen, [CommandId::ToggleLoop]);
    assert!(!harness.state().palette.is_open());
}

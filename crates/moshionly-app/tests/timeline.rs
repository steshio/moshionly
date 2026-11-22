//! Drives the timeline the way a user would, without a window.

use egui::{Event, Pos2, Vec2, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use moshionly_app::{
    key::Key,
    theme::{self, Tone},
    timeline::{Bar, Group, Timeline, TimelineEvent, TimelineLayout, TimelineState},
};

const SIZE: Vec2 = vec2(900.0, 240.0);

struct State {
    view: TimelineState,
    groups: Vec<Group>,
    selection: Vec<Key>,
    playhead: f64,
    duration: f64,
    events: Vec<TimelineEvent>,
    layout: Option<TimelineLayout>,
}

impl State {
    fn layout(&self) -> TimelineLayout {
        self.layout.expect("the timeline has been shown")
    }

    /// The middle of the bar on the row at `row`.
    fn bar_center(&self, row: usize, start: f64, end: f64) -> Pos2 {
        self.layout().bar_rect(row, start, end).center()
    }
}

fn bar(key: &str, start: f64, end: f64) -> Bar {
    Bar {
        key: Key::new(key),
        label: key.to_owned(),
        icon: None,
        tone: Tone::Accent,
        start,
        end,
    }
}

fn group(key: &str, bars: Vec<Bar>) -> Group {
    Group {
        key: Key::new(key),
        label: key.to_owned(),
        icon: None,
        collapsed: false,
        hidden: false,
        locked: false,
        bars,
    }
}

/// Rows: 0 Card, 1 fade, 2 move, 3 Badge, 4 grow. Four seconds at 200
/// points per second.
fn harness() -> Harness<'static, State> {
    let state = State {
        view: TimelineState {
            points_per_second: 200.0,
            fit_requested: false,
            ..TimelineState::default()
        },
        groups: vec![
            group(
                "Card",
                vec![bar("fade", 0.0, 600.0), bar("move", 200.0, 1100.0)],
            ),
            group("Badge", vec![bar("grow", 900.0, 1500.0)]),
        ],
        selection: Vec::new(),
        playhead: 0.0,
        duration: 4000.0,
        events: Vec::new(),
        layout: None,
    };
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_context_setup(theme::install)
        .build_ui_state(
            |ui, state: &mut State| {
                let rect = ui.ctx().content_rect();
                let output = Timeline::new(&mut state.view, &state.groups)
                    .selection(&state.selection)
                    .playhead(state.playhead)
                    .duration(state.duration)
                    .show(ui, rect);
                // Apply what a real app would, so gestures can build on
                // each other.
                for event in &output.events {
                    match event {
                        TimelineEvent::PlayheadMoved { time } => state.playhead = *time,
                        TimelineEvent::SelectionChanged { bars } => {
                            state.selection.clone_from(bars);
                        }
                        _ => {}
                    }
                }
                state.events.extend(output.events);
                state.layout = Some(output.layout);
            },
            state,
        );
    harness.run();
    harness
}

fn click(harness: &mut Harness<'static, State>, at: Pos2) {
    harness.hover_at(at);
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run();
}

fn keys(names: &[&str]) -> Vec<Key> {
    names.iter().map(Key::new).collect()
}

#[test]
fn clicking_the_ruler_moves_the_playhead() {
    let mut harness = harness();
    let layout = harness.state().layout();
    let at = pos2(layout.scale.x(1500.0), layout.ruler.center().y);
    click(&mut harness, at);
    assert_eq!(harness.state().playhead, 1500.0);
    assert!(
        harness
            .state()
            .events
            .contains(&TimelineEvent::PlayheadMoved { time: 1500.0 })
    );
}

#[test]
fn dragging_in_the_ruler_scrubs_and_stops_at_the_end() {
    let mut harness = harness();
    let layout = harness.state().layout();
    let y = layout.ruler.center().y;
    harness.hover_at(pos2(layout.scale.x(500.0), y));
    harness.drag_at(pos2(layout.scale.x(500.0), y));
    harness.run();
    assert_eq!(harness.state().playhead, 500.0);
    harness.hover_at(pos2(layout.scale.x(2000.0), y));
    harness.run();
    assert_eq!(harness.state().playhead, 2000.0);
    // Past the end of the scene, and off the ruler entirely.
    harness.hover_at(pos2(layout.scale.x(4000.0) + 60.0, y + 80.0));
    harness.run();
    assert_eq!(harness.state().playhead, 4000.0);
    harness.drop_at(pos2(layout.scale.x(4000.0) + 60.0, y + 80.0));
    harness.run();
    assert_eq!(harness.state().playhead, 4000.0);
}

#[test]
fn clicking_a_bar_selects_it_and_empty_track_clears() {
    let mut harness = harness();
    let at = harness.state().bar_center(2, 200.0, 1100.0);
    click(&mut harness, at);
    assert_eq!(harness.state().selection, keys(&["move"]));

    // A click on another bar replaces the selection.
    let at = harness.state().bar_center(4, 900.0, 1500.0);
    click(&mut harness, at);
    assert_eq!(harness.state().selection, keys(&["grow"]));

    // Empty track on the same row, well after the bar.
    let layout = harness.state().layout();
    let empty = pos2(layout.scale.x(3000.0), at.y);
    click(&mut harness, empty);
    assert!(harness.state().selection.is_empty());
}

#[test]
fn clicking_a_layer_summary_selects_all_its_actions() {
    let mut harness = harness();
    // The Card layer's summary spans 0 to 1100 on row 0.
    let at = harness.state().bar_center(0, 0.0, 1100.0);
    click(&mut harness, at);
    assert_eq!(harness.state().selection, keys(&["fade", "move"]));
}

#[test]
fn the_arrow_of_a_layer_row_folds_its_actions() {
    let mut harness = harness();
    let layout = harness.state().layout();
    // The arrow sits at the left of the label, inside the row's inset.
    let arrow = pos2(layout.labels.left() + 20.0, layout.row_rect(0).center().y);
    click(&mut harness, arrow);
    assert!(
        harness
            .state()
            .events
            .contains(&TimelineEvent::GroupCollapsed {
                key: Key::new("Card"),
                collapsed: true
            })
    );
    // The name beside it selects the layer instead.
    let name = pos2(layout.labels.left() + 80.0, layout.row_rect(3).center().y);
    click(&mut harness, name);
    assert!(
        harness
            .state()
            .events
            .contains(&TimelineEvent::GroupClicked {
                key: Key::new("Badge")
            })
    );
}

#[test]
fn the_transport_buttons_report_what_was_asked() {
    let mut harness = harness();
    harness.get_by_label("Play").click();
    harness.run();
    harness.get_by_label("Loop playback").click();
    harness.run();
    harness.get_by_label("Jump to start").click();
    harness.run();
    let events = &harness.state().events;
    assert!(events.contains(&TimelineEvent::PlayToggled { playing: true }));
    assert!(events.contains(&TimelineEvent::LoopToggled { looping: true }));
    assert!(events.contains(&TimelineEvent::JumpedToStart));
}

#[test]
fn pinching_zooms_about_the_pointer() {
    let mut harness = harness();
    let layout = harness.state().layout();
    let pointer = pos2(layout.scale.x(2000.0), layout.tracks.center().y);
    harness.hover_at(pointer);
    harness.run();
    harness.event(Event::Zoom(2.0));
    harness.run();
    let state = harness.state();
    assert!((state.view.points_per_second - 400.0).abs() < 0.01);
    // Two seconds is still under the pointer.
    assert!((state.layout().scale.x(2000.0) - pointer.x).abs() < 0.5);
}

#[test]
fn the_view_never_scrolls_before_time_zero() {
    let mut harness = harness();
    let layout = harness.state().layout();
    harness.hover_at(layout.tracks.center());
    harness.run();
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(300.0, 0.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(30);
    assert_eq!(harness.state().view.scroll_x, 0.0);

    // Scrolling the other way moves forward in time.
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(-120.0, 0.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(30);
    assert!((harness.state().view.scroll_x - 120.0).abs() < 0.5);
}

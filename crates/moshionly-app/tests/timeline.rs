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
    /// The start and end of the bar with `key`.
    fn times(&self, key: &str) -> (f64, f64) {
        let bar = self
            .groups
            .iter()
            .flat_map(|group| &group.bars)
            .find(|bar| bar.key == key)
            .expect("the bar exists");
        (bar.start, bar.end)
    }

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
        instant: false,
        fixed: false,
    }
}

/// The marker of an action that happens at one moment, `at`.
fn marker(key: &str, at: f64) -> Bar {
    Bar {
        instant: true,
        ..bar(key, at, at)
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
        folder: false,
        bars,
    }
}

/// Rows: 0 Card, 1 fade, 2 move, 3 Badge, 4 grow. Four seconds at 200
/// points per second.
fn harness() -> Harness<'static, State> {
    harness_at(200.0)
}

fn harness_at(points_per_second: f32) -> Harness<'static, State> {
    let state = State {
        view: TimelineState::with_zoom(points_per_second),
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
                        TimelineEvent::BarMoved { key, start, end }
                        | TimelineEvent::BarResized {
                            key, start, end, ..
                        } => {
                            for bar in state.groups.iter_mut().flat_map(|g| &mut g.bars) {
                                if bar.key == *key {
                                    (bar.start, bar.end) = (*start, *end);
                                }
                            }
                        }
                        TimelineEvent::DurationChanged { duration } => {
                            state.duration = *duration;
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

/// Presses at `from`, moves to `to` in two steps and lets go.
fn drag(harness: &mut Harness<'static, State>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.drag_at(from);
    harness.hover_at(from + (to - from) / 2.0);
    harness.hover_at(to);
    harness.drop_at(to);
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

#[test]
fn dragging_a_bar_moves_it_and_closes_the_edit() {
    let mut harness = harness();
    let from = harness.state().bar_center(2, 200.0, 1100.0);
    // 100 points at 200 points per second is half a second.
    drag(&mut harness, from, from + vec2(100.0, 0.0));
    let state = harness.state();
    assert_eq!(state.times("move"), (700.0, 1600.0));
    // The other bars stay put, and the dragged bar became the selection.
    assert_eq!(state.times("fade"), (0.0, 600.0));
    assert_eq!(state.selection, keys(&["move"]));
    assert_eq!(state.events.last(), Some(&TimelineEvent::EditFinished));
    let finished = state
        .events
        .iter()
        .filter(|event| **event == TimelineEvent::EditFinished)
        .count();
    assert_eq!(finished, 1);
}

#[test]
fn a_dragged_bar_snaps_to_the_edge_of_another() {
    let mut harness = harness();
    // "grow" starts at 900. Dragging it 62 points left asks for 590, which
    // is within reach of the end of "fade" at 600.
    let from = harness.state().bar_center(4, 900.0, 1500.0);
    drag(&mut harness, from, from - vec2(62.0, 0.0));
    assert_eq!(harness.state().times("grow"), (600.0, 1200.0));
}

#[test]
fn away_from_other_bars_a_dragged_bar_falls_on_the_grid() {
    let mut harness = harness();
    // 33 points is 165 ms: "grow" would start at 1065, which the 50 ms grid
    // of this zoom rounds to 1050.
    let from = harness.state().bar_center(4, 900.0, 1500.0);
    drag(&mut harness, from, from + vec2(33.0, 0.0));
    assert_eq!(harness.state().times("grow"), (1050.0, 1650.0));
}

#[test]
fn dragging_an_end_resizes_the_bar() {
    let mut harness = harness();
    let rect = harness.state().layout().bar_rect(1, 0.0, 600.0);
    // The right end of "fade", 40 points to the right: 200 ms longer.
    let end = pos2(rect.right() - 1.0, rect.center().y);
    drag(&mut harness, end, end + vec2(40.0, 0.0));
    let state = harness.state();
    assert_eq!(state.times("fade"), (0.0, 800.0));
    assert!(state.events.iter().any(|event| matches!(
        event,
        TimelineEvent::BarResized {
            edge: moshionly_app::timeline::Edge::End,
            ..
        }
    )));
    assert_eq!(state.events.last(), Some(&TimelineEvent::EditFinished));

    // The left end of "move" cannot pass its right end.
    let rect = state.layout().bar_rect(2, 200.0, 1100.0);
    let start = pos2(rect.left() + 1.0, rect.center().y);
    drag(&mut harness, start, start + vec2(400.0, 0.0));
    assert_eq!(harness.state().times("move"), (1090.0, 1100.0));
}

#[test]
fn selected_bars_move_together_and_stop_at_time_zero() {
    let mut harness = harness();
    // Select both of Card's actions through the layer's summary bar.
    let summary = harness.state().bar_center(0, 0.0, 1100.0);
    click(&mut harness, summary);
    assert_eq!(harness.state().selection, keys(&["fade", "move"]));

    // "fade" already starts at zero, so the pair cannot go left.
    let from = harness.state().bar_center(2, 200.0, 1100.0);
    drag(&mut harness, from, from - vec2(100.0, 0.0));
    assert_eq!(harness.state().times("fade"), (0.0, 600.0));
    assert_eq!(harness.state().times("move"), (200.0, 1100.0));
    assert!(
        !harness
            .state()
            .events
            .contains(&TimelineEvent::EditFinished)
    );

    // To the right they go together.
    drag(&mut harness, from, from + vec2(50.0, 0.0));
    assert_eq!(harness.state().times("fade"), (250.0, 850.0));
    assert_eq!(harness.state().times("move"), (450.0, 1350.0));
    assert_eq!(harness.state().times("grow"), (900.0, 1500.0));
}

#[test]
fn a_rubber_band_selects_the_bars_it_touches() {
    let mut harness = harness();
    let layout = harness.state().layout();
    // From empty track right of every bar on the first action row, down and
    // left across all three bars.
    let from = pos2(layout.scale.x(1700.0), layout.row_rect(1).center().y);
    let to = pos2(layout.scale.x(300.0), layout.row_rect(4).center().y);
    drag(&mut harness, from, to);
    assert_eq!(harness.state().selection, keys(&["fade", "move", "grow"]));
    // Selecting is not an edit.
    assert!(
        !harness
            .state()
            .events
            .contains(&TimelineEvent::EditFinished)
    );
}

#[test]
fn dragging_the_tab_at_the_end_changes_the_scene_length() {
    // Zoomed out so the end of the scene is on screen.
    let mut harness = harness_at(100.0);
    let layout = harness.state().layout();
    let tab = pos2(layout.scale.x(4000.0), layout.ruler.top() + 6.0);
    drag(&mut harness, tab, tab - vec2(100.0, 0.0));
    let state = harness.state();
    assert_eq!(state.duration, 3000.0);
    assert_eq!(state.events.last(), Some(&TimelineEvent::EditFinished));
    // The playhead did not follow the pointer.
    assert_eq!(state.playhead, 0.0);
}

#[test]
fn bars_of_a_locked_layer_cannot_be_dragged() {
    let mut harness = harness();
    harness.state_mut().groups[1].locked = true;
    harness.run();
    let from = harness.state().bar_center(4, 900.0, 1500.0);
    drag(&mut harness, from, from + vec2(80.0, 0.0));
    assert_eq!(harness.state().times("grow"), (900.0, 1500.0));
}

#[test]
fn a_marker_moves_from_anywhere_on_it_and_never_resizes() {
    let mut harness = harness();
    // Row 5: a show at 2 s under Badge, clear of the other bars.
    harness.state_mut().groups[1]
        .bars
        .push(marker("show", 2000.0));
    harness.run();
    let center = harness.state().bar_center(5, 2000.0, 2000.0);
    // Near its left side, where a bar's start grip would be: 60 points at
    // 200 points per second is 300 ms later.
    let left = center - vec2(1.5, 0.0);
    drag(&mut harness, left, left + vec2(60.0, 0.0));
    assert_eq!(harness.state().times("show"), (2300.0, 2300.0));
    // Near its right side, where the end grip would be.
    let center = harness.state().bar_center(5, 2300.0, 2300.0);
    let right = center + vec2(1.5, 0.0);
    drag(&mut harness, right, right + vec2(60.0, 0.0));
    assert_eq!(harness.state().times("show"), (2600.0, 2600.0));
    // And back to the left.
    let center = harness.state().bar_center(5, 2600.0, 2600.0);
    let left = center - vec2(1.5, 0.0);
    drag(&mut harness, left, left - vec2(100.0, 0.0));
    assert_eq!(harness.state().times("show"), (2100.0, 2100.0));
    let state = harness.state();
    assert!(
        !state
            .events
            .iter()
            .any(|event| matches!(event, TimelineEvent::BarResized { .. })),
        "a marker has no ends to resize"
    );
    assert_eq!(state.selection, keys(&["show"]));
}

#[test]
fn a_bar_that_only_shows_something_is_not_selected_or_moved() {
    let mut harness = harness();
    // The Badge's grow becomes a bar that only shows what it plays.
    harness.state_mut().groups[1].bars[0].fixed = true;
    harness.run();
    let grow = harness.state().bar_center(4, 900.0, 1500.0);
    click(&mut harness, grow);
    assert!(harness.state().selection.is_empty());
    assert!(
        harness
            .state()
            .events
            .contains(&TimelineEvent::GroupClicked {
                key: Key::new("Badge")
            })
    );
    let later = grow + vec2(100.0, 0.0);
    drag(&mut harness, grow, later);
    assert_eq!(harness.state().times("grow"), (900.0, 1500.0));
    assert!(harness.state().selection.is_empty());
}

#[test]
fn a_layer_summary_snaps_the_actions_it_moves() {
    // The Badge pulses from 1000 to 1200, and may also play a component
    // from 365 to 865, which a bar only shows: the summary spans it too.
    for plays in [false, true] {
        let mut harness = harness();
        let badge = &mut harness.state_mut().groups[1];
        badge.bars = vec![bar("pulse", 1000.0, 1200.0)];
        if plays {
            let shown = Bar {
                fixed: true,
                ..bar("plays", 365.0, 865.0)
            };
            badge.bars.insert(0, shown);
        }
        harness.run();
        let span = if plays {
            (365.0, 1200.0)
        } else {
            (1000.0, 1200.0)
        };
        let from = harness.state().bar_center(3, span.0, span.1);
        // 46 points is 230 ms: the pulse would start at 1230, which the
        // 50 ms grid rounds to 1250. The fixed bar does not move, so its
        // edges are not the ones that snap.
        drag(&mut harness, from, from + vec2(46.0, 0.0));
        assert_eq!(
            harness.state().times("pulse"),
            (1250.0, 1450.0),
            "with a bar that plays a component: {plays}"
        );
        if plays {
            assert_eq!(harness.state().times("plays"), (365.0, 865.0));
        }
    }
}

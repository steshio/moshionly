//! Drives the canvas viewport the way a user would, without a window.

use egui::{Event, Key, MouseWheelUnit, Pos2, TouchPhase, Vec2, pos2, vec2};
use egui_kittest::Harness;
use moshionly_app::{
    theme,
    viewport::{Camera, Viewport, ViewportEvent},
};

/// The viewport fills a window of this size, so its centre is at 200, 150.
const SIZE: Vec2 = vec2(400.0, 300.0);

struct State {
    camera: Camera,
    events: Vec<ViewportEvent>,
}

fn harness(camera: Camera) -> Harness<'static, State> {
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_context_setup(theme::install)
        .build_ui_state(
            |ui, state: &mut State| {
                let rect = ui.ctx().content_rect();
                let output = Viewport::new(&mut state.camera).show(ui, rect);
                state.events.extend(output.events);
            },
            State {
                camera,
                events: Vec::new(),
            },
        );
    harness.run();
    harness
}

/// A camera looking at world point 100, 100 at 200%.
fn camera() -> Camera {
    Camera {
        center: pos2(100.0, 100.0),
        zoom: 2.0,
    }
}

fn close(a: Pos2, b: Pos2) -> bool {
    (a - b).length() < 0.01
}

#[test]
fn a_click_reports_the_world_position() {
    let mut harness = harness(camera());
    let at = pos2(250.0, 150.0);
    harness.hover_at(at);
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run();
    let events = &harness.state().events;
    assert_eq!(events.len(), 1, "{events:?}");
    let ViewportEvent::Clicked { world, .. } = events[0] else {
        panic!("expected a click, got {events:?}");
    };
    // 50 points right of the centre at 200% is 25 world units.
    assert!(close(world, pos2(125.0, 100.0)), "{world:?}");
}

#[test]
fn a_drag_reports_where_it_started_went_and_ended() {
    let mut harness = harness(camera());
    let (a, b, c) = (pos2(100.0, 100.0), pos2(140.0, 100.0), pos2(180.0, 130.0));
    harness.hover_at(a);
    harness.drag_at(a);
    harness.hover_at(b);
    harness.hover_at(c);
    harness.drop_at(c);
    harness.run();

    let state = harness.state();
    let world = |screen: Pos2| pos2(100.0, 100.0) + (screen - pos2(200.0, 150.0)) / 2.0;
    let events = &state.events;
    assert!(
        matches!(events.first(), Some(ViewportEvent::DragStarted { world: w, .. }) if close(*w, world(a))),
        "{events:?}"
    );
    assert!(
        matches!(events.last(), Some(ViewportEvent::DragStopped { world: w, .. }) if close(*w, world(c))),
        "{events:?}"
    );
    assert!(events.iter().any(
        |event| matches!(event, ViewportEvent::Dragged { world: w, .. } if close(*w, world(c)))
    ));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ViewportEvent::Clicked { .. }))
    );
    // Dragging the content does not move the camera.
    assert_eq!(state.camera, camera());
}

#[test]
fn dragging_with_space_held_pans() {
    let mut harness = harness(camera());
    let (a, b) = (pos2(100.0, 100.0), pos2(160.0, 80.0));
    harness.hover_at(a);
    harness.key_down(Key::Space);
    harness.drag_at(a);
    harness.hover_at(pos2(130.0, 90.0));
    harness.hover_at(b);
    harness.drop_at(b);
    harness.key_up(Key::Space);
    harness.run();

    let state = harness.state();
    assert!(state.events.is_empty(), "{:?}", state.events);
    // The content followed the pointer: 60 right and 20 up on screen is 30
    // and 10 world units, so the camera's centre moved the other way.
    assert!(
        close(state.camera.center, pos2(70.0, 110.0)),
        "{:?}",
        state.camera
    );
    assert_eq!(state.camera.zoom, 2.0);
}

#[test]
fn pinching_zooms_about_the_pointer() {
    let mut harness = harness(camera());
    let pointer = pos2(320.0, 60.0);
    let rect = egui::Rect::from_min_size(Pos2::ZERO, SIZE);
    let under = camera().screen_to_world(rect, pointer);
    harness.hover_at(pointer);
    harness.run();
    harness.event(Event::Zoom(1.5));
    harness.run();

    let state = harness.state();
    assert!((state.camera.zoom - 3.0).abs() < 1e-4, "{:?}", state.camera);
    assert!(close(state.camera.screen_to_world(rect, pointer), under));
}

#[test]
fn scrolling_pans() {
    let mut harness = harness(camera());
    harness.hover_at(pos2(200.0, 150.0));
    harness.run();
    harness.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -40.0),
        phase: TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    // Scrolling is smoothed over a few frames.
    harness.run_steps(40);

    let state = harness.state();
    // The content moved up by 40 points, 20 world units: the view moved down.
    assert!(
        (state.camera.center.y - 120.0).abs() < 0.5,
        "{:?}",
        state.camera
    );
    assert!((state.camera.center.x - 100.0).abs() < 0.01);
    assert_eq!(state.camera.zoom, 2.0);
}

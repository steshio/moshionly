//! Drives the canvas viewport the way a user would, without a window.

use egui::{Event, Key, MouseWheelUnit, Pos2, TouchPhase, Vec2, pos2, vec2};
use egui_kittest::Harness;
use moshionly_app::{
    theme,
    viewport::{Anchor, Camera, Handle, OrientedBox, Overlays, Viewport, ViewportEvent},
};

/// The viewport fills a window of this size, so its centre is at 200, 150.
const SIZE: Vec2 = vec2(400.0, 300.0);

struct State {
    camera: Camera,
    overlays: Overlays,
    events: Vec<ViewportEvent>,
    handle: Option<Handle>,
}

fn harness(camera: Camera) -> Harness<'static, State> {
    harness_with(camera, Overlays::default())
}

fn harness_with(camera: Camera, overlays: Overlays) -> Harness<'static, State> {
    let mut harness = Harness::builder()
        .with_size(SIZE)
        // Frames a sixtieth of a second apart, as on a screen, so that two
        // quick clicks are a double click.
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(theme::install)
        .build_ui_state(
            |ui, state: &mut State| {
                let rect = ui.ctx().content_rect();
                let output = Viewport::new(&mut state.camera)
                    .overlays(&state.overlays)
                    .show(ui, rect);
                state.events.extend(output.events);
                state.handle = output.handle;
            },
            State {
                camera,
                overlays,
                events: Vec::new(),
                handle: None,
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

/// A selection box that is at 120, 90 to 280, 210 on screen with [`camera`].
fn selection() -> Overlays {
    Overlays {
        selection: Some(OrientedBox::from_rect(egui::Rect::from_min_max(
            pos2(60.0, 70.0),
            pos2(140.0, 130.0),
        ))),
        ..Overlays::default()
    }
}

#[test]
fn dragging_a_corner_of_the_selection_reports_the_handle() {
    let mut harness = harness_with(camera(), selection());
    let (corner, end) = (pos2(280.0, 210.0), pos2(300.0, 240.0));
    harness.hover_at(corner);
    harness.run();
    assert_eq!(
        harness.state().handle,
        Some(Handle::Resize(Anchor::BottomRight))
    );
    harness.drag_at(corner);
    harness.hover_at(pos2(290.0, 225.0));
    harness.hover_at(end);
    harness.drop_at(end);
    harness.run();

    let events = &harness.state().events;
    let handle = Handle::Resize(Anchor::BottomRight);
    assert!(
        matches!(events.first(), Some(ViewportEvent::HandleDragStarted { handle: h, world, .. })
            if *h == handle && close(*world, pos2(140.0, 130.0))),
        "{events:?}"
    );
    assert!(
        matches!(events.last(), Some(ViewportEvent::HandleDragStopped { handle: h, world, .. })
            if *h == handle && close(*world, pos2(150.0, 145.0))),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, ViewportEvent::DragStarted { .. }))
    );
}

#[test]
fn dragging_from_just_outside_a_corner_rotates() {
    let mut harness = harness_with(camera(), selection());
    let (start, end) = (pos2(290.0, 220.0), pos2(260.0, 250.0));
    harness.hover_at(start);
    harness.drag_at(start);
    harness.hover_at(pos2(275.0, 235.0));
    harness.hover_at(end);
    harness.drop_at(end);
    harness.run();
    let events = &harness.state().events;
    assert!(
        matches!(
            events.first(),
            Some(ViewportEvent::HandleDragStarted {
                handle: Handle::Rotate(Anchor::BottomRight),
                ..
            })
        ),
        "{events:?}"
    );
}

#[test]
fn a_drag_inside_the_selection_is_an_ordinary_drag() {
    let mut harness = harness_with(camera(), selection());
    let (start, end) = (pos2(200.0, 150.0), pos2(230.0, 170.0));
    harness.hover_at(start);
    harness.run();
    assert_eq!(harness.state().handle, None);
    harness.drag_at(start);
    harness.hover_at(pos2(215.0, 160.0));
    harness.hover_at(end);
    harness.drop_at(end);
    harness.run();
    let events = &harness.state().events;
    assert!(
        matches!(events.first(), Some(ViewportEvent::DragStarted { .. })),
        "{events:?}"
    );
}

#[test]
fn two_quick_clicks_are_a_double_click() {
    let mut harness = harness(camera());
    let at = pos2(250.0, 150.0);
    harness.hover_at(at);
    harness.run();
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        harness.step();
    }
    harness.run();
    let events = &harness.state().events;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ViewportEvent::DoubleClicked { .. })),
        "{events:?}"
    );
}

//! The camera: which part of the infinite canvas the viewport shows.
//!
//! Two coordinate spaces meet here. **World** coordinates are the document's:
//! pixels of the design, y down, unbounded. **Screen** coordinates are egui
//! points in the window. A [`Camera`] plus the viewport's rectangle on screen
//! define the mapping between them.

use egui::{Pos2, Rect, Vec2};

/// The smallest zoom, 2%.
pub const MIN_ZOOM: f32 = 0.02;
/// The largest zoom, 25600%.
pub const MAX_ZOOM: f32 = 256.0;

/// The zoom levels that "zoom in" and "zoom out" step through.
pub const ZOOM_STOPS: [f32; 17] = [
    0.02, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0,
];

/// Where the viewport looks and how closely.
///
/// The camera stores the world point at the centre of the viewport, so
/// resizing the window or a panel keeps the same thing in the middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The world point shown at the centre of the viewport.
    pub center: Pos2,
    /// Screen points per world unit: 1 is 100%.
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Pos2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Where a world point lands on screen.
    pub fn world_to_screen(&self, viewport: Rect, world: Pos2) -> Pos2 {
        viewport.center() + (world - self.center) * self.zoom
    }

    /// The world point under a screen position.
    pub fn screen_to_world(&self, viewport: Rect, screen: Pos2) -> Pos2 {
        self.center + (screen - viewport.center()) / self.zoom
    }

    /// A world rectangle on screen.
    pub fn rect_to_screen(&self, viewport: Rect, world: Rect) -> Rect {
        Rect::from_min_max(
            self.world_to_screen(viewport, world.min),
            self.world_to_screen(viewport, world.max),
        )
    }

    /// A screen rectangle in world coordinates.
    pub fn rect_to_world(&self, viewport: Rect, screen: Rect) -> Rect {
        Rect::from_min_max(
            self.screen_to_world(viewport, screen.min),
            self.screen_to_world(viewport, screen.max),
        )
    }

    /// The part of the world the viewport shows.
    pub fn visible_world(&self, viewport: Rect) -> Rect {
        self.rect_to_world(viewport, viewport)
    }

    /// Moves the view so the content follows a pointer that moved by
    /// `screen_delta`.
    pub fn pan(&mut self, screen_delta: Vec2) {
        self.center -= screen_delta / self.zoom;
    }

    /// Multiplies the zoom by `factor`, keeping the world point under the
    /// screen position `anchor` where it is. This is what pinching and
    /// scroll-zooming do, with the pointer as the anchor.
    pub fn zoom_about(&mut self, viewport: Rect, anchor: Pos2, factor: f32) {
        self.set_zoom_about(viewport, anchor, self.zoom * factor);
    }

    /// Sets the zoom, keeping the world point under the screen position
    /// `anchor` where it is. The zoom is kept within [`MIN_ZOOM`] and
    /// [`MAX_ZOOM`].
    pub fn set_zoom_about(&mut self, viewport: Rect, anchor: Pos2, zoom: f32) {
        if !zoom.is_finite() || zoom <= 0.0 {
            return;
        }
        let fixed = self.screen_to_world(viewport, anchor);
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        // Put `fixed` back under the anchor.
        self.center = fixed - (anchor - viewport.center()) / self.zoom;
    }

    /// Sets the zoom, keeping the centre of the view where it is.
    pub fn set_zoom(&mut self, zoom: f32) {
        if zoom.is_finite() && zoom > 0.0 {
            self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    /// Steps to the next larger zoom stop.
    pub fn zoom_in(&mut self) {
        self.zoom = next_zoom_stop(self.zoom);
    }

    /// Steps to the next smaller zoom stop.
    pub fn zoom_out(&mut self) {
        self.zoom = previous_zoom_stop(self.zoom);
    }

    /// Moves and zooms so that the world rectangle `world` is as large as it
    /// can be inside `area`, centred in it. `area` is the part of the
    /// viewport that is free of floating controls.
    pub fn fit(&mut self, viewport: Rect, area: Rect, world: Rect) {
        if !world.is_finite() || !area.is_positive() {
            return;
        }
        let size = world.size().max(Vec2::splat(1.0));
        let zoom = (area.width() / size.x).min(area.height() / size.y);
        if !zoom.is_finite() || zoom <= 0.0 {
            return;
        }
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        // The centre of `world` goes to the centre of `area`.
        self.center = world.center() - (area.center() - viewport.center()) / self.zoom;
    }
}

/// The smallest zoom stop larger than `zoom`.
pub fn next_zoom_stop(zoom: f32) -> f32 {
    ZOOM_STOPS
        .into_iter()
        .find(|stop| *stop > zoom * 1.001)
        .unwrap_or(MAX_ZOOM)
}

/// The largest zoom stop smaller than `zoom`.
pub fn previous_zoom_stop(zoom: f32) -> f32 {
    ZOOM_STOPS
        .into_iter()
        .rev()
        .find(|stop| *stop < zoom * 0.999)
        .unwrap_or(MIN_ZOOM)
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};

    use super::*;

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(248.0, 44.0), vec2(928.0, 608.0))
    }

    fn close(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn the_center_of_the_world_view_is_the_center_of_the_viewport() {
        let camera = Camera {
            center: pos2(540.0, 540.0),
            zoom: 0.5,
        };
        let vp = viewport();
        assert!(close(
            camera.world_to_screen(vp, pos2(540.0, 540.0)),
            vp.center()
        ));
        // 100 world units to the right are 50 points at 50%.
        assert!(close(
            camera.world_to_screen(vp, pos2(640.0, 540.0)),
            vp.center() + vec2(50.0, 0.0)
        ));
    }

    #[test]
    fn screen_and_world_round_trip() {
        let vp = viewport();
        for zoom in [0.02, 0.37, 1.0, 3.0, 64.0] {
            let camera = Camera {
                center: pos2(-120.5, 870.25),
                zoom,
            };
            for world in [pos2(0.0, 0.0), pos2(-300.0, 42.5), pos2(1080.0, 1080.0)] {
                let screen = camera.world_to_screen(vp, world);
                let back = camera.screen_to_world(vp, screen);
                assert!((back - world).length() < 0.05, "zoom {zoom}: {back:?}");
            }
        }
        let camera = Camera {
            center: pos2(10.0, 20.0),
            zoom: 2.0,
        };
        let world = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 50.0));
        let screen = camera.rect_to_screen(vp, world);
        assert_eq!(screen.size(), vec2(200.0, 100.0));
        let back = camera.rect_to_world(vp, screen);
        assert!(close(back.min, world.min) && close(back.max, world.max));
    }

    #[test]
    fn panning_moves_the_content_with_the_pointer() {
        let vp = viewport();
        let mut camera = Camera {
            center: pos2(0.0, 0.0),
            zoom: 2.0,
        };
        let before = camera.world_to_screen(vp, pos2(30.0, 30.0));
        camera.pan(vec2(40.0, -10.0));
        let after = camera.world_to_screen(vp, pos2(30.0, 30.0));
        assert!(close(after, before + vec2(40.0, -10.0)));
        assert_eq!(camera.zoom, 2.0);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let vp = viewport();
        let mut camera = Camera {
            center: pos2(500.0, 300.0),
            zoom: 0.8,
        };
        let pointer = vp.min + vec2(123.0, 456.0);
        let under = camera.screen_to_world(vp, pointer);
        camera.zoom_about(vp, pointer, 1.7);
        assert!((camera.zoom - 1.36).abs() < 1e-5);
        assert!(close(camera.screen_to_world(vp, pointer), under));
        camera.zoom_about(vp, pointer, 0.1);
        assert!(close(camera.screen_to_world(vp, pointer), under));
    }

    #[test]
    fn zoom_stays_within_its_limits_and_ignores_nonsense() {
        let vp = viewport();
        let mut camera = Camera::default();
        camera.zoom_about(vp, vp.center(), 1e9);
        assert_eq!(camera.zoom, MAX_ZOOM);
        camera.zoom_about(vp, vp.center(), 1e-12);
        assert_eq!(camera.zoom, MIN_ZOOM);
        let before = camera;
        camera.zoom_about(vp, vp.center(), f32::NAN);
        camera.set_zoom_about(vp, vp.center(), -3.0);
        camera.set_zoom(0.0);
        camera.set_zoom(f32::INFINITY);
        assert_eq!(camera, before);
    }

    #[test]
    fn fitting_centres_the_content_in_the_free_area() {
        let vp = viewport();
        // Leave the bottom 80 points for a tool bar, and a margin all round.
        let area = Rect::from_min_max(vp.min + vec2(40.0, 40.0), vp.max - vec2(40.0, 80.0));
        let world = Rect::from_min_size(pos2(0.0, 0.0), vec2(1080.0, 1080.0));
        let mut camera = Camera::default();
        camera.fit(vp, area, world);
        let on_screen = camera.rect_to_screen(vp, world);
        assert!((on_screen.height() - area.height()).abs() < 0.01);
        assert!(on_screen.width() <= area.width());
        assert!(close(on_screen.center(), area.center()));

        // A wide rectangle is limited by the width instead.
        let wide = Rect::from_min_size(pos2(-500.0, 0.0), vec2(4000.0, 100.0));
        camera.fit(vp, area, wide);
        let on_screen = camera.rect_to_screen(vp, wide);
        assert!((on_screen.width() - area.width()).abs() < 0.01);
        assert!(close(on_screen.center(), area.center()));
    }

    #[test]
    fn fitting_nothing_changes_nothing() {
        let vp = viewport();
        let mut camera = Camera::default();
        camera.fit(vp, vp, Rect::NAN);
        camera.fit(
            vp,
            Rect::NOTHING,
            Rect::from_min_size(Pos2::ZERO, vec2(1.0, 1.0)),
        );
        assert_eq!(camera, Camera::default());
        // A point still gets a finite, capped zoom.
        camera.fit(vp, vp, Rect::from_min_size(pos2(5.0, 5.0), Vec2::ZERO));
        assert!(camera.zoom.is_finite() && camera.zoom <= MAX_ZOOM);
    }

    #[test]
    fn zoom_stops_step_up_and_down() {
        assert_eq!(next_zoom_stop(1.0), 1.5);
        assert_eq!(previous_zoom_stop(1.0), 0.75);
        assert_eq!(next_zoom_stop(0.87), 1.0);
        assert_eq!(previous_zoom_stop(0.87), 0.75);
        assert_eq!(next_zoom_stop(MAX_ZOOM), MAX_ZOOM);
        assert_eq!(previous_zoom_stop(MIN_ZOOM), MIN_ZOOM);
        let mut camera = Camera::default();
        camera.zoom_in();
        camera.zoom_in();
        assert_eq!(camera.zoom, 2.0);
        camera.zoom_out();
        assert_eq!(camera.zoom, 1.5);
    }
}

//! How sharp to render an artboard, and how much of it.
//!
//! An artboard is drawn at the pixel density the zoom needs, rounded up to a
//! small set of levels so that zooming a little does not render again. When
//! the whole artboard at that density would make a texture too large, only
//! the part on screen is rendered, with a margin, and that part is kept for
//! as long as the view stays inside it, so panning a little does not render
//! again either.

use egui::{Rect, Vec2, pos2, vec2};

/// The longest side of a rendered texture, in pixels.
pub const MAX_TEXTURE_SIDE: f32 = 4096.0;

/// The lowest density rendered: one pixel per 64 artboard pixels.
pub const MIN_DENSITY: f32 = 1.0 / 64.0;

/// The highest density rendered: 16 pixels per artboard pixel.
pub const MAX_DENSITY: f32 = 16.0;

/// What to render of one artboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// Output pixels per artboard pixel.
    pub scale: f32,
    /// The part of the artboard to render, in the artboard's own
    /// coordinates. `None` renders all of it.
    pub region: Option<Rect>,
}

/// The density level for a zoom: the smallest power of √2 that is at least
/// `zoom × pixels_per_point`, within [`MIN_DENSITY`] and [`MAX_DENSITY`].
pub fn density(zoom: f32, pixels_per_point: f32) -> f32 {
    let needed = zoom * pixels_per_point;
    if !needed.is_finite() || needed <= 0.0 {
        return 1.0;
    }
    // Half steps of a power of two: 0.5, 0.71, 1, 1.41, 2, ...
    let level = (needed.log2() * 2.0 - 1e-4).ceil() / 2.0;
    2f32.powf(level).clamp(MIN_DENSITY, MAX_DENSITY)
}

/// What to render of an artboard of `size` (in artboard pixels) when the
/// part of it on screen is `visible` (in the artboard's own coordinates,
/// possibly reaching beyond it) at `zoom` on a screen of `pixels_per_point`.
///
/// `previous` is the plan of the picture on screen. A partial picture at the
/// same density is kept while the view stays inside it.
pub fn plan(
    size: Vec2,
    visible: Rect,
    zoom: f32,
    pixels_per_point: f32,
    previous: Option<&Plan>,
) -> Plan {
    let mut scale = density(zoom, pixels_per_point);
    let whole = size * scale;
    if whole.x <= MAX_TEXTURE_SIDE && whole.y <= MAX_TEXTURE_SIDE {
        return Plan {
            scale,
            region: None,
        };
    }
    // Too large as a whole: render what is on screen, with a quarter of the
    // view's size as margin, snapped outwards to a grid of 256 pixels.
    let artboard = Rect::from_min_size(pos2(0.0, 0.0), size);
    if let Some(previous) = previous
        && let Some(region) = previous.region
        && previous.scale == scale
        && region.contains_rect(visible.intersect(artboard))
    {
        return *previous;
    }
    let margin = visible.size() * 0.25;
    let wanted = visible.expand2(margin).intersect(artboard);
    if !wanted.is_positive() {
        // Nothing of it is on screen: a small whole picture will do.
        let fit = (MAX_TEXTURE_SIDE / size.x.max(size.y)).min(scale);
        return Plan {
            scale: fit,
            region: None,
        };
    }
    let cell = 256.0 / scale;
    let snapped = Rect::from_min_max(
        pos2(
            (wanted.min.x / cell).floor() * cell,
            (wanted.min.y / cell).floor() * cell,
        ),
        pos2(
            (wanted.max.x / cell).ceil() * cell,
            (wanted.max.y / cell).ceil() * cell,
        ),
    )
    .intersect(artboard);
    let side = snapped.width().max(snapped.height()) * scale;
    if side > MAX_TEXTURE_SIDE {
        // A very large screen: fewer pixels than ideal rather than none.
        scale *= MAX_TEXTURE_SIDE / side;
    }
    Plan {
        scale,
        region: (snapped != artboard).then_some(snapped),
    }
}

/// The size in pixels of a render of `plan` for an artboard of `size`.
pub fn pixel_size(size: Vec2, plan: &Plan) -> Vec2 {
    let area = plan.region.map_or(size, |region| region.size());
    vec2(
        (area.x * plan.scale).ceil().max(1.0),
        (area.y * plan.scale).ceil().max(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_rounds_up_to_half_powers_of_two() {
        assert_eq!(density(1.0, 1.0), 1.0);
        assert_eq!(density(1.0, 2.0), 2.0);
        assert_eq!(density(0.5, 2.0), 1.0);
        assert_eq!(density(0.9, 1.0), 1.0);
        assert!((density(1.2, 1.0) - std::f32::consts::SQRT_2).abs() < 1e-4);
        assert!((density(0.3, 1.0) - 0.5_f32.powf(1.5)).abs() < 1e-4);
        // Capped at both ends, and nonsense zooms give 1.
        assert_eq!(density(1000.0, 2.0), MAX_DENSITY);
        assert_eq!(density(0.0001, 1.0), MIN_DENSITY);
        assert_eq!(density(f32::NAN, 1.0), 1.0);
        assert_eq!(density(0.0, 1.0), 1.0);
    }

    #[test]
    fn small_zoom_changes_keep_the_density() {
        assert_eq!(density(0.75, 2.0), density(0.72, 2.0));
        assert_eq!(density(1.05, 1.0), density(1.4, 1.0));
    }

    #[test]
    fn an_artboard_that_fits_is_rendered_whole() {
        let size = vec2(1080.0, 1080.0);
        let visible = Rect::from_min_size(pos2(-200.0, -100.0), vec2(1600.0, 1000.0));
        let plan = plan(size, visible, 0.8, 2.0, None);
        assert_eq!(plan.region, None);
        assert_eq!(plan.scale, 2.0);
        assert_eq!(pixel_size(size, &plan), vec2(2160.0, 2160.0));
    }

    #[test]
    fn close_up_only_the_visible_part_is_rendered() {
        let size = vec2(1080.0, 1080.0);
        // At 800% on a Retina screen, a 1440 point wide view shows 90
        // artboard pixels across.
        let visible = Rect::from_min_size(pos2(500.0, 500.0), vec2(90.0, 56.0));
        let plan = plan(size, visible, 8.0, 2.0, None);
        assert_eq!(plan.scale, 16.0);
        let region = plan.region.expect("a region");
        assert!(region.contains_rect(visible));
        let pixels = pixel_size(size, &plan);
        assert!(pixels.x <= MAX_TEXTURE_SIDE && pixels.y <= MAX_TEXTURE_SIDE);
        // Panning a little keeps the picture; panning far makes a new one.
        let nudged = visible.translate(vec2(3.0, 2.0));
        assert_eq!(super::plan(size, nudged, 8.0, 2.0, Some(&plan)), plan);
        let far = visible.translate(vec2(300.0, 0.0));
        let moved = super::plan(size, far, 8.0, 2.0, Some(&plan));
        assert!(moved.region.unwrap().contains_rect(far));
    }

    #[test]
    fn a_huge_view_lowers_the_density_rather_than_overflow() {
        let size = vec2(20_000.0, 20_000.0);
        let visible = Rect::from_min_size(pos2(0.0, 0.0), vec2(5000.0, 5000.0));
        let plan = plan(size, visible, 1.0, 2.0, None);
        let pixels = pixel_size(size, &plan);
        assert!(pixels.x <= MAX_TEXTURE_SIDE + 1.0, "{pixels:?}");
        assert!(plan.scale < 2.0);
    }

    #[test]
    fn an_artboard_off_screen_gets_a_small_whole_picture() {
        let size = vec2(8000.0, 8000.0);
        let visible = Rect::from_min_size(pos2(9000.0, 0.0), vec2(500.0, 500.0));
        let plan = plan(size, visible, 2.0, 2.0, None);
        assert_eq!(plan.region, None);
        assert!(pixel_size(size, &plan).x <= MAX_TEXTURE_SIDE);
    }
}

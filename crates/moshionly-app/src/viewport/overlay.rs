//! What is drawn over the artboards: selection, hover, marquee and guides.
//!
//! The viewport is handed these as plain geometry in world coordinates and
//! draws them at screen scale, so lines stay one point wide and handles stay
//! the same size at every zoom.

use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2, pos2};

use super::{
    camera::Camera,
    handles::{Anchor, OrientedBox},
};
use crate::{
    theme::Theme,
    widgets::{numeric::format_value, paint},
};

/// A distance shown on the canvas: a line between two points with a label.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// One end, in world coordinates.
    pub from: Pos2,
    /// The other end.
    pub to: Pos2,
    /// The text at the middle of the line, usually the distance.
    pub label: String,
}

/// A path being drawn with the pen, in world coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PenOverlay {
    /// The outline so far, flattened to a polyline, with a line on to the
    /// pointer.
    pub path: Vec<Pos2>,
    /// The anchors.
    pub anchors: Vec<Pos2>,
    /// Handle lines, from an anchor to its handle.
    pub handles: Vec<[Pos2; 2]>,
    /// The pointer is over the first anchor: a click closes the path.
    pub closing: bool,
}

/// Everything the viewport draws over the artboards, in world coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlays {
    /// The box of each selected layer, drawn as a thin outline.
    pub outlines: Vec<OrientedBox>,
    /// The box around the whole selection, drawn with resize handles. Its
    /// handles can be dragged, and just outside its corners it rotates.
    pub selection: Option<OrientedBox>,
    /// Shows the selection's size under its box.
    pub show_size: bool,
    /// The box of the layer under the pointer.
    pub hover: Option<OrientedBox>,
    /// A rubber-band selection in progress.
    pub marquee: Option<Rect>,
    /// Snapping guides: lines from one point to another.
    pub guides: Vec<[Pos2; 2]>,
    /// Distances to show.
    pub measurements: Vec<Measurement>,
    /// A path being drawn with the pen.
    pub pen: Option<PenOverlay>,
    /// A box drawn around text being edited on the canvas.
    pub editing: Option<OrientedBox>,
    /// Paths layers travel while animating: the points of each, with dots
    /// where a move starts or ends.
    pub paths: Vec<(Vec<Pos2>, Vec<Pos2>)>,
}

impl Overlays {
    /// True when there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Draws `overlays` for a viewport at `viewport` seen through `camera`.
pub(super) fn paint(ui: &Ui, viewport: Rect, camera: &Camera, overlays: &Overlays) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let painter = ui.painter_at(viewport);
    let accent = Stroke::new(1.0, c.accent);

    if let Some(hover) = overlays.hover
        && overlays.selection != Some(hover)
    {
        outline(
            &painter,
            &hover.to_screen(camera, viewport),
            Stroke::new(2.0, c.accent),
        );
    }
    for layer in &overlays.outlines {
        outline(&painter, &layer.to_screen(camera, viewport), accent);
    }

    if let Some(selection) = overlays.selection {
        let on_screen = selection.to_screen(camera, viewport);
        outline(&painter, &on_screen, accent);
        handles(&painter, &on_screen, theme);
        if overlays.show_size {
            size_label(ui, &painter, &on_screen, selection.size);
        }
    }

    for (points, stops) in &overlays.paths {
        let line: Vec<Pos2> = points
            .iter()
            .map(|point| camera.world_to_screen(viewport, *point))
            .collect();
        painter.add(Shape::dashed_line(
            &line,
            Stroke::new(1.5, c.accent),
            6.0,
            4.0,
        ));
        for stop in stops {
            let at = camera.world_to_screen(viewport, *stop);
            painter.circle(at, 3.5, c.surface, Stroke::new(1.5, c.accent));
        }
    }

    if let Some(marquee) = overlays.marquee {
        let rect = crisp(camera.rect_to_screen(viewport, marquee));
        painter.rect(rect, 0.0, c.marquee_fill, accent, StrokeKind::Inside);
    }

    for [from, to] in &overlays.guides {
        painter.line_segment(
            [
                crisp_point(camera.world_to_screen(viewport, *from)),
                crisp_point(camera.world_to_screen(viewport, *to)),
            ],
            Stroke::new(1.0, c.guide),
        );
    }

    if let Some(editing) = overlays.editing {
        outline(&painter, &editing.to_screen(camera, viewport), accent);
    }

    if let Some(pen) = &overlays.pen {
        paint_pen(&painter, viewport, camera, pen, theme);
    }

    for measurement in &overlays.measurements {
        let from = crisp_point(camera.world_to_screen(viewport, measurement.from));
        let to = crisp_point(camera.world_to_screen(viewport, measurement.to));
        painter.add(Shape::dashed_line(
            &[from, to],
            Stroke::new(1.0, c.measure),
            3.0,
            2.0,
        ));
        // A short bar across each end.
        let along = (to - from).normalized();
        let across = Vec2::new(-along.y, along.x) * 3.0;
        if along != Vec2::ZERO {
            for end in [from, to] {
                painter.line_segment([end - across, end + across], Stroke::new(1.0, c.measure));
            }
        }
        pill(
            ui,
            &painter,
            from + (to - from) / 2.0,
            Align2::CENTER_CENTER,
            &measurement.label,
            c.measure,
        );
    }
}

/// Draws a path being drawn with the pen: its outline so far, its handles
/// and its anchors.
fn paint_pen(painter: &Painter, viewport: Rect, camera: &Camera, pen: &PenOverlay, theme: &Theme) {
    let c = &theme.color;
    let screen = |p: Pos2| camera.world_to_screen(viewport, p);
    let accent = Stroke::new(1.5, c.accent);
    if pen.path.len() >= 2 {
        painter.add(Shape::line(
            pen.path.iter().map(|p| screen(*p)).collect(),
            accent,
        ));
    }
    for [anchor, handle] in &pen.handles {
        let (a, h) = (screen(*anchor), screen(*handle));
        painter.line_segment([a, h], Stroke::new(1.0, c.accent));
        painter.circle(h, 3.0, c.handle_fill, Stroke::new(1.0, c.accent));
    }
    for (n, anchor) in pen.anchors.iter().enumerate() {
        let at = screen(*anchor);
        // The first anchor grows when a click there would close the path.
        let side = if n == 0 && pen.closing { 10.0 } else { 7.0 };
        let rect = Rect::from_center_size(at, Vec2::splat(side));
        painter.rect(
            rect,
            1.0,
            c.handle_fill,
            Stroke::new(1.0, c.accent),
            StrokeKind::Inside,
        );
    }
}

/// Outlines a box. Upright boxes are put on whole pixels so the line is
/// crisp.
fn outline(painter: &Painter, screen_box: &OrientedBox, stroke: Stroke) {
    if screen_box.rotation == 0.0 {
        painter.rect_stroke(crisp(screen_box.bounds()), 0.0, stroke, StrokeKind::Middle);
    } else {
        painter.add(Shape::closed_line(screen_box.corners().to_vec(), stroke));
    }
}

/// Draws the resize handles of the selection box.
fn handles(painter: &Painter, screen_box: &OrientedBox, theme: &Theme) {
    let c = &theme.color;
    let side = theme.size.handle;
    let stroke = Stroke::new(1.0, c.accent);
    // Too small to grab by anything but its corners, or by nothing at all.
    let smallest = screen_box.size.x.min(screen_box.size.y);
    if smallest < side {
        return;
    }
    let upright = screen_box.rotation == 0.0;
    let crisp_box = if upright {
        OrientedBox::from_rect(crisp(screen_box.bounds()))
    } else {
        *screen_box
    };
    for anchor in Anchor::ALL {
        // Edge handles only where the edge is long enough to tell them
        // apart from the corners.
        if !anchor.is_corner() {
            let unit = anchor.unit();
            let length = if unit.x == 0.0 {
                screen_box.size.x
            } else {
                screen_box.size.y
            };
            if length < side * 6.0 {
                continue;
            }
        }
        let center = crisp_box.point(anchor.unit());
        if upright {
            let rect = Rect::from_center_size(center, Vec2::splat(side));
            painter.rect(rect, 1.0, c.handle_fill, stroke, StrokeKind::Inside);
        } else {
            let square = OrientedBox {
                center,
                size: Vec2::splat(side),
                rotation: screen_box.rotation,
            };
            painter.add(Shape::convex_polygon(
                square.corners().to_vec(),
                c.handle_fill,
                stroke,
            ));
        }
    }
}

/// The selection's size in a pill under its box.
fn size_label(ui: &Ui, painter: &Painter, screen_box: &OrientedBox, world_size: Vec2) {
    let theme = Theme::of(ui);
    let text = format!(
        "{} × {}",
        format_value(f64::from(world_size.x), 2),
        format_value(f64::from(world_size.y), 2)
    );
    let bounds = screen_box.bounds();
    pill(
        ui,
        painter,
        pos2(bounds.center().x, bounds.bottom() + theme.space.md),
        Align2::CENTER_TOP,
        &text,
        theme.color.accent,
    );
}

/// A small filled label with light text, placed so that `anchor` of it is at
/// `at`.
fn pill(ui: &Ui, painter: &Painter, at: Pos2, anchor: Align2, text: &str, fill: Color32) {
    let theme = Theme::of(ui);
    let galley = paint::line(ui, text, theme.small_medium(), theme.color.text_on_accent);
    let size = galley.size() + Vec2::new(theme.space.sm * 2.0, theme.space.xxs * 2.0);
    let rect = anchor.anchor_size(paint::snap(at), size);
    painter.rect_filled(rect, theme.radius.xs, fill);
    paint::galley_in(painter, rect, Align2::CENTER_CENTER, galley);
}

/// Moves a rectangle's edges to pixel centres, where a one-point line drawn
/// along them is crisp.
fn crisp(rect: Rect) -> Rect {
    Rect::from_min_max(crisp_point(rect.min), crisp_point(rect.max))
}

fn crisp_point(point: Pos2) -> Pos2 {
    pos2(point.x.floor() + 0.5, point.y.floor() + 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crisp_points_sit_on_pixel_centres() {
        assert_eq!(crisp_point(pos2(10.2, 20.9)), pos2(10.5, 20.5));
        assert_eq!(crisp_point(pos2(-0.3, 7.0)), pos2(-0.5, 7.5));
        let rect = crisp(Rect::from_min_max(pos2(1.4, 2.6), pos2(9.9, 12.1)));
        assert_eq!(rect, Rect::from_min_max(pos2(1.5, 2.5), pos2(9.5, 12.5)));
    }

    #[test]
    fn empty_overlays_know_they_are_empty() {
        assert!(Overlays::default().is_empty());
        let overlays = Overlays {
            marquee: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
            ..Overlays::default()
        };
        assert!(!overlays.is_empty());
    }
}

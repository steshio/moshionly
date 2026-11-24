//! The easing of the selected action: a picker of curve types, a drawing
//! of the curve, and its intensity (or, for a custom curve, its two
//! control points).
//!
//! The drawing's maths is kept in plain functions: [`curve`] samples an
//! easing, [`value_range`] says how much room the drawing needs above and
//! below for curves that overshoot, and [`to_screen`] places a point of the
//! curve in the drawing.

use egui::{Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2};
use moshionly_core::{Easing, Edit, Node, ease};

use super::Bind;
use crate::{
    theme::Theme,
    widgets::{
        Dropdown, NumericField, NumericResponse, pair, property_row, section, slider_with_value,
    },
};

/// The kinds of easing, as the picker lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EasingKind {
    Linear,
    Smooth,
    Natural,
    SlowDown,
    Accelerate,
    Overshoot,
    Elastic,
    Bounce,
    Bezier,
}

/// Every kind with its name, in picker order.
pub(super) const KINDS: [(EasingKind, &str); 9] = [
    (EasingKind::Smooth, "Smooth"),
    (EasingKind::SlowDown, "Slow down"),
    (EasingKind::Accelerate, "Speed up"),
    (EasingKind::Natural, "Natural"),
    (EasingKind::Overshoot, "Overshoot"),
    (EasingKind::Elastic, "Elastic"),
    (EasingKind::Bounce, "Bounce"),
    (EasingKind::Linear, "Linear"),
    (EasingKind::Bezier, "Custom curve"),
];

/// How many straight pieces the drawn curve is made of.
const SAMPLES: usize = 96;

/// The height of the drawing, in points.
const PREVIEW_HEIGHT: f32 = 84.0;

impl EasingKind {
    /// The kind of an easing.
    pub(super) fn of(easing: &Easing) -> Self {
        match easing {
            Easing::Linear => Self::Linear,
            Easing::Smooth { .. } => Self::Smooth,
            Easing::Natural { .. } => Self::Natural,
            Easing::SlowDown { .. } => Self::SlowDown,
            Easing::Accelerate { .. } => Self::Accelerate,
            Easing::Overshoot { .. } => Self::Overshoot,
            Easing::Elastic { .. } => Self::Elastic,
            Easing::Bounce { .. } => Self::Bounce,
            Easing::Bezier { .. } => Self::Bezier,
        }
    }

    /// An easing of this kind that keeps what it can of `previous`: its
    /// intensity, or for a custom curve the shape `previous` had.
    pub(super) fn with(self, previous: &Easing) -> Easing {
        let intensity = previous.intensity().unwrap_or(50.0);
        match self {
            Self::Linear => Easing::Linear,
            Self::Smooth => Easing::Smooth { intensity },
            Self::Natural => Easing::Natural { intensity },
            Self::SlowDown => Easing::SlowDown { intensity },
            Self::Accelerate => Easing::Accelerate { intensity },
            Self::Overshoot => Easing::Overshoot { intensity },
            Self::Elastic => Easing::Elastic { intensity },
            Self::Bounce => Easing::Bounce { intensity },
            Self::Bezier => {
                let [x1, y1, x2, y2] = previous.control_points().unwrap_or([0.25, 0.1, 0.25, 1.0]);
                Easing::Bezier { x1, y1, x2, y2 }
            }
        }
    }
}

/// The easing with another intensity, for the kinds that have one.
pub(super) fn with_intensity(easing: &Easing, intensity: f64) -> Easing {
    let intensity = intensity.clamp(0.0, 100.0);
    match *easing {
        Easing::Smooth { .. } => Easing::Smooth { intensity },
        Easing::Natural { .. } => Easing::Natural { intensity },
        Easing::SlowDown { .. } => Easing::SlowDown { intensity },
        Easing::Accelerate { .. } => Easing::Accelerate { intensity },
        Easing::Overshoot { .. } => Easing::Overshoot { intensity },
        Easing::Elastic { .. } => Easing::Elastic { intensity },
        Easing::Bounce { .. } => Easing::Bounce { intensity },
        other => other,
    }
}

/// The curve of an easing as `samples + 1` points `(progress, value)`,
/// from `(0, 0)` to `(1, 1)`.
pub(super) fn curve(easing: &Easing, samples: usize) -> Vec<(f64, f64)> {
    let samples = samples.max(1);
    (0..=samples)
        .map(|n| {
            let t = n as f64 / samples as f64;
            (t, ease(easing, t))
        })
        .collect()
}

/// The values a drawing of `points` must show, bottom to top: always 0 to
/// 1, widened to take in values that go past either end, with a little
/// room beyond them.
pub(super) fn value_range(points: &[(f64, f64)]) -> (f64, f64) {
    let low = points.iter().map(|(_, v)| *v).fold(0.0, f64::min);
    let high = points.iter().map(|(_, v)| *v).fold(1.0, f64::max);
    let margin = |beyond: f64| {
        if beyond > 0.0 {
            beyond * 0.15 + 0.02
        } else {
            0.0
        }
    };
    (low - margin(-low), high + margin(high - 1.0))
}

/// Where the point `(progress, value)` goes in `rect` when the drawing
/// shows the values in `range`: progress runs left to right, values bottom
/// to top.
pub(super) fn to_screen(rect: Rect, range: (f64, f64), (t, v): (f64, f64)) -> Pos2 {
    let (low, high) = range;
    let span = (high - low).max(f64::EPSILON);
    let x = rect.left() + rect.width() * t as f32;
    let y = rect.bottom() - rect.height() * ((v - low) / span) as f32;
    pos2(x, y)
}

/// Draws the curve of `easing` across the available width.
pub(super) fn curve_preview(ui: &mut Ui, easing: &Easing) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let (outer, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), PREVIEW_HEIGHT),
        Sense::hover(),
    );
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, theme.radius.sm, c.well);
    let rect = outer.shrink2(Vec2::new(theme.space.lg, theme.space.md));
    let points = curve(easing, SAMPLES);
    let range = value_range(&points);
    let place = |point| to_screen(rect, range, point);

    // The start and end values, and the straight line between them.
    let guide = Stroke::new(1.0, c.border_strong);
    for value in [0.0, 1.0] {
        let y = place((0.0, value)).y;
        painter.hline(rect.x_range(), y, guide);
    }
    painter.line_segment(
        [place((0.0, 0.0)), place((1.0, 1.0))],
        Stroke::new(1.0, c.border),
    );
    if let Easing::Bezier { x1, y1, x2, y2 } = *easing {
        let handle = Stroke::new(1.0, c.text_tertiary);
        for (anchor, control) in [((0.0, 0.0), (x1, y1)), ((1.0, 1.0), (x2, y2))] {
            painter.line_segment([place(anchor), place(control)], handle);
            painter.circle_filled(place(control), 3.0, c.text_tertiary);
        }
    }
    let line: Vec<Pos2> = points.iter().copied().map(place).collect();
    painter.add(Shape::line(line, Stroke::new(2.0, c.accent)));
    for end in [(0.0, 0.0), (1.0, 1.0)] {
        painter.circle_filled(place(end), 3.0, c.accent);
    }
}

/// The easing section of the selected action `node`.
pub(super) fn easing_section(ui: &mut Ui, bind: &mut Bind<'_>, node: &Node) {
    let Some(easing) = node.data.easing().copied() else {
        return;
    };
    let mut open = bind.state.open.easing;
    let mut picked: Option<Easing> = None;
    let mut fields: Vec<(NumericResponse, Easing)> = Vec::new();
    section(
        ui,
        "Easing",
        &mut open,
        |_| {},
        |ui| {
            let mut kind = EasingKind::of(&easing);
            property_row(ui, "Curve", |ui| {
                if Dropdown::new(&mut kind, "Easing", &KINDS)
                    .show(ui)
                    .changed()
                {
                    picked = Some(kind.with(&easing));
                }
            });
            let theme = Theme::of(ui);
            ui.add_space(theme.space.xs);
            curve_preview(ui, &picked.unwrap_or(easing));
            ui.add_space(theme.space.xs);
            if let Some(intensity) = easing.intensity() {
                let mut value = intensity;
                let response = property_row(ui, "Intensity", |ui| {
                    slider_with_value(ui, &mut value, "Intensity", 0.0..=100.0, "")
                });
                fields.push((response, with_intensity(&easing, value)));
            }
            if let Easing::Bezier { x1, y1, x2, y2 } = easing {
                let mut points = [x1, y1, x2, y2];
                let mut responses: Vec<NumericResponse> = Vec::new();
                for (row, label) in ["Point 1", "Point 2"].into_iter().enumerate() {
                    let (mut x, mut y) = (points[row * 2], points[row * 2 + 1]);
                    let (mut x_response, mut y_response) = (None, None);
                    property_row(ui, label, |ui| {
                        pair(
                            ui,
                            |ui| {
                                x_response = Some(
                                    NumericField::new(&mut x, "X")
                                        .label("X")
                                        .range(0.0..=1.0)
                                        .step(0.01)
                                        .decimals(2)
                                        .id_salt(("bezier-x", row))
                                        .show(ui),
                                );
                            },
                            |ui| {
                                y_response = Some(
                                    NumericField::new(&mut y, "Y")
                                        .label("Y")
                                        .range(-2.0..=3.0)
                                        .step(0.01)
                                        .decimals(2)
                                        .id_salt(("bezier-y", row))
                                        .show(ui),
                                );
                            },
                        );
                    });
                    points[row * 2] = x;
                    points[row * 2 + 1] = y;
                    responses.extend(x_response);
                    responses.extend(y_response);
                }
                let [x1, y1, x2, y2] = points;
                let curve = Easing::Bezier { x1, y1, x2, y2 };
                fields.extend(responses.into_iter().map(|response| (response, curve)));
            }
        },
    );
    bind.state.open.easing = open;
    let id = node.id.clone();
    let edit = |easing: &Easing| {
        vec![Edit::set(
            id.clone(),
            "easing",
            serde_json::to_value(easing).unwrap_or_default(),
        )]
    };
    if let Some(easing) = picked {
        bind.apply("Change easing", &edit(&easing));
    }
    for (response, easing) in fields {
        bind.field(&response, "Change easing", || edit(&easing));
    }
}

#[cfg(test)]
mod tests {
    use egui::vec2;

    use super::*;

    #[test]
    fn a_curve_runs_from_rest_to_the_end() {
        let points = curve(&Easing::smooth(), 4);
        assert_eq!(points.len(), 5);
        assert_eq!(points[0], (0.0, 0.0));
        assert_eq!(points[4], (1.0, 1.0));
        assert_eq!(points[2].0, 0.5);
        // Smooth is symmetric: halfway in time is halfway there.
        assert!((points[2].1 - 0.5).abs() < 1e-6);
        assert_eq!(curve(&Easing::Linear, 0).len(), 2);
    }

    #[test]
    fn the_drawing_makes_room_for_overshoot() {
        assert_eq!(value_range(&curve(&Easing::Linear, 8)), (0.0, 1.0));
        let (low, high) = value_range(&curve(&Easing::overshoot(), 64));
        assert_eq!(low, 0.0);
        // Overshoot at 50 goes about 12% past the end.
        assert!(high > 1.1 && high < 1.2, "{high}");
        let (low, _) = value_range(&[(0.0, 0.0), (0.3, -0.2), (1.0, 1.0)]);
        assert!(low < -0.2);
    }

    #[test]
    fn points_land_in_the_drawing() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 50.0));
        assert_eq!(to_screen(rect, (0.0, 1.0), (0.0, 0.0)), pos2(10.0, 70.0));
        assert_eq!(to_screen(rect, (0.0, 1.0), (1.0, 1.0)), pos2(110.0, 20.0));
        assert_eq!(to_screen(rect, (0.0, 1.0), (0.5, 0.5)), pos2(60.0, 45.0));
        // With room for overshoot, 1 sits below the top.
        let top = to_screen(rect, (0.0, 1.25), (1.0, 1.0));
        assert_eq!(top, pos2(110.0, 30.0));
    }

    #[test]
    fn switching_kinds_keeps_the_intensity() {
        let strong = Easing::SlowDown { intensity: 80.0 };
        assert_eq!(
            EasingKind::Bounce.with(&strong),
            Easing::Bounce { intensity: 80.0 }
        );
        assert_eq!(EasingKind::Linear.with(&strong), Easing::Linear);
        assert_eq!(
            EasingKind::Smooth.with(&Easing::Linear),
            Easing::Smooth { intensity: 50.0 }
        );
        // A custom curve starts from the shape the easing had.
        let custom = EasingKind::Bezier.with(&Easing::smooth());
        assert_eq!(
            custom.control_points(),
            Easing::smooth().control_points(),
            "same shape"
        );
        for (kind, _) in KINDS {
            assert_eq!(EasingKind::of(&kind.with(&strong)), kind);
        }
        assert_eq!(
            with_intensity(&Easing::elastic(), 140.0),
            Easing::Elastic { intensity: 100.0 }
        );
        assert_eq!(with_intensity(&Easing::Linear, 10.0), Easing::Linear);
    }
}

//! Easing: how an animation's progress is paced between its start and end.
//!
//! Every easing maps a progress from 0 to 1 onto a value that starts at 0 and
//! ends at 1. Most have an `intensity` from 0 to 100 that shapes the curve:
//! 50 is the tuned default, 0 is the gentlest usable version and 100 the most
//! pronounced.
//!
//! | Type | Shape | What intensity does |
//! |---|---|---|
//! | `linear` | constant speed | nothing |
//! | `smooth` | eases in and out evenly | sharper acceleration and braking |
//! | `natural` | quick start, long soft landing | longer, softer landing |
//! | `slow_down` | starts fast, brakes | brakes earlier and harder |
//! | `accelerate` | starts slowly, ends fast | later, harder launch |
//! | `overshoot` | passes the target, settles back | goes further past (1% to 33%) |
//! | `elastic` | a spring that wobbles into place | more and bigger wobbles |
//! | `bounce` | a dropped ball that settles | bouncier ball, more bounces |
//! | `bezier` | a CSS `cubic-bezier` you supply | no intensity |

use std::f64::consts::PI;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::defaults;

/// The pacing of an animation.
///
/// ```json
/// { "type": "smooth", "intensity": 50 }
/// { "type": "bezier", "x1": 0.2, "y1": 0, "x2": 0, "y2": 1 }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Easing {
    /// Constant speed.
    Linear,
    /// Eases in and out evenly. The default.
    Smooth {
        /// Shape of the curve, 0 to 100. Default 50.
        #[serde(
            default = "defaults::intensity",
            skip_serializing_if = "defaults::is_intensity"
        )]
        #[schemars(range(min = 0.0, max = 100.0))]
        intensity: f64,
    },
    /// A quick start and a long, soft landing.
    Natural {
        /// Shape of the curve, 0 to 100. Default 50.
        #[serde(
            default = "defaults::intensity",
            skip_serializing_if = "defaults::is_intensity"
        )]
        #[schemars(range(min = 0.0, max = 100.0))]
        intensity: f64,
    },
    /// Starts at full speed and brakes into the end. Good for things arriving.
    SlowDown {
        /// Shape of the curve, 0 to 100. Default 50.
        #[serde(
            default = "defaults::intensity",
            skip_serializing_if = "defaults::is_intensity"
        )]
        #[schemars(range(min = 0.0, max = 100.0))]
        intensity: f64,
    },
    /// Starts slowly and ends at full speed. Good for things leaving.
    Accelerate {
        /// Shape of the curve, 0 to 100. Default 50.
        #[serde(
            default = "defaults::intensity",
            skip_serializing_if = "defaults::is_intensity"
        )]
        #[schemars(range(min = 0.0, max = 100.0))]
        intensity: f64,
    },
    /// Goes past the end value, then settles back onto it.
    Overshoot {
        /// How far it goes past, 0 to 100. Default 50.
        #[serde(
            default = "defaults::intensity",
            skip_serializing_if = "defaults::is_intensity"
        )]
        #[schemars(range(min = 0.0, max = 100.0))]
        intensity: f64,
    },
    /// A damped spring: wobbles around the end value before resting on it.
    Elastic {
        /// How wobbly the spring is, 0 to 100. Default 50.
        #[serde(
            default = "defaults::intensity",
            skip_serializing_if = "defaults::is_intensity"
        )]
        #[schemars(range(min = 0.0, max = 100.0))]
        intensity: f64,
    },
    /// A dropped ball: hits the end value and bounces back off it a few times.
    Bounce {
        /// How bouncy the ball is, 0 to 100. Default 50.
        #[serde(
            default = "defaults::intensity",
            skip_serializing_if = "defaults::is_intensity"
        )]
        #[schemars(range(min = 0.0, max = 100.0))]
        intensity: f64,
    },
    /// A cubic Bezier curve from (0, 0) to (1, 1), as in CSS `cubic-bezier`.
    Bezier {
        /// Horizontal position of the first control point, 0 to 1.
        #[schemars(range(min = 0.0, max = 1.0))]
        x1: f64,
        /// Vertical position of the first control point. May leave 0 to 1.
        y1: f64,
        /// Horizontal position of the second control point, 0 to 1.
        #[schemars(range(min = 0.0, max = 1.0))]
        x2: f64,
        /// Vertical position of the second control point. May leave 0 to 1.
        y2: f64,
    },
}

impl Default for Easing {
    fn default() -> Self {
        Easing::smooth()
    }
}

impl Easing {
    /// `smooth` at the default intensity.
    pub const fn smooth() -> Self {
        Easing::Smooth { intensity: 50.0 }
    }

    /// `natural` at the default intensity.
    pub const fn natural() -> Self {
        Easing::Natural { intensity: 50.0 }
    }

    /// `slow_down` at the default intensity.
    pub const fn slow_down() -> Self {
        Easing::SlowDown { intensity: 50.0 }
    }

    /// `accelerate` at the default intensity.
    pub const fn accelerate() -> Self {
        Easing::Accelerate { intensity: 50.0 }
    }

    /// `overshoot` at the default intensity.
    pub const fn overshoot() -> Self {
        Easing::Overshoot { intensity: 50.0 }
    }

    /// `elastic` at the default intensity.
    pub const fn elastic() -> Self {
        Easing::Elastic { intensity: 50.0 }
    }

    /// `bounce` at the default intensity.
    pub const fn bounce() -> Self {
        Easing::Bounce { intensity: 50.0 }
    }

    /// The type name used in files, for example `"slow_down"`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Easing::Linear => "linear",
            Easing::Smooth { .. } => "smooth",
            Easing::Natural { .. } => "natural",
            Easing::SlowDown { .. } => "slow_down",
            Easing::Accelerate { .. } => "accelerate",
            Easing::Overshoot { .. } => "overshoot",
            Easing::Elastic { .. } => "elastic",
            Easing::Bounce { .. } => "bounce",
            Easing::Bezier { .. } => "bezier",
        }
    }

    /// The intensity, for the types that have one.
    pub fn intensity(&self) -> Option<f64> {
        match *self {
            Easing::Smooth { intensity }
            | Easing::Natural { intensity }
            | Easing::SlowDown { intensity }
            | Easing::Accelerate { intensity }
            | Easing::Overshoot { intensity }
            | Easing::Elastic { intensity }
            | Easing::Bounce { intensity } => Some(intensity),
            Easing::Linear | Easing::Bezier { .. } => None,
        }
    }

    /// The cubic Bezier control points `[x1, y1, x2, y2]` of this easing, when
    /// it is one. `linear` and all types except `elastic` and `bounce` are.
    pub fn control_points(&self) -> Option<[f64; 4]> {
        let amount = |intensity: f64| intensity.clamp(0.0, 100.0) / 100.0;
        match *self {
            Easing::Linear => Some([1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]),
            Easing::Smooth { intensity } => {
                // Symmetric: both handles pull the same distance.
                let pull = 0.2 + 0.6 * amount(intensity);
                Some([pull, 0.0, 1.0 - pull, 1.0])
            }
            Easing::Natural { intensity } => {
                let i = amount(intensity);
                Some([0.15 + 0.2 * i, 0.0, 0.35 - 0.3 * i, 1.0])
            }
            Easing::SlowDown { intensity } => Some(slow_down_points(amount(intensity))),
            Easing::Accelerate { intensity } => {
                // The slow-down curve turned around: what it does at the end,
                // this does at the start.
                let [x1, y1, x2, y2] = slow_down_points(amount(intensity));
                Some([1.0 - x2, 1.0 - y2, 1.0 - x1, 1.0 - y1])
            }
            Easing::Overshoot { intensity } => {
                // A cubic in time that leaves 0 with speed `tension + 3` and
                // arrives at 1 with speed 0, which makes it swing past 1 by
                // 4·s³ / (27·(s+1)²): about 1%, 12% and 33% at 0, 50 and 100.
                let i = amount(intensity);
                let tension = 0.6 + 2.2 * i + 0.8 * i * i;
                Some([1.0 / 3.0, 1.0 + tension / 3.0, 2.0 / 3.0, 1.0])
            }
            Easing::Elastic { .. } | Easing::Bounce { .. } => None,
            Easing::Bezier { x1, y1, x2, y2 } => Some([x1, y1, x2, y2]),
        }
    }

    /// The easing as a CSS `animation-timing-function` value.
    ///
    /// Bezier-shaped easings become `cubic-bezier(...)`. `elastic` and `bounce`
    /// have no closed CSS form and become a sampled `linear(...)` function.
    pub fn to_css(&self) -> String {
        match self {
            Easing::Linear => "linear".to_owned(),
            Easing::Elastic { .. } | Easing::Bounce { .. } => {
                const STEPS: usize = 60;
                let samples: Vec<String> = (0..=STEPS)
                    .map(|i| css_number(ease(self, i as f64 / STEPS as f64)))
                    .collect();
                format!("linear({})", samples.join(", "))
            }
            _ => {
                let [x1, y1, x2, y2] = self
                    .control_points()
                    .expect("every remaining easing is a Bezier curve");
                format!(
                    "cubic-bezier({}, {}, {}, {})",
                    css_number(x1),
                    css_number(y1),
                    css_number(x2),
                    css_number(y2)
                )
            }
        }
    }
}

/// Control points of `slow_down` for an intensity amount from 0 to 1.
fn slow_down_points(i: f64) -> [f64; 4] {
    [0.35 - 0.3 * i, 0.6 + 0.4 * i, 0.65 - 0.55 * i, 1.0]
}

/// Prints a number with at most three decimals and no trailing zeros.
fn css_number(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = format!("{rounded:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Evaluates an easing.
///
/// `t` is the progress through the animation and is clamped to 0 to 1. The
/// result is exactly 0 at 0 and exactly 1 at 1. In between it may leave that
/// range for `overshoot`, `elastic` and Bezier curves whose handles do.
pub fn ease(easing: &Easing, t: f64) -> f64 {
    if t.is_nan() || t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    match *easing {
        Easing::Linear => t,
        Easing::Elastic { intensity } => spring(intensity.clamp(0.0, 100.0) / 100.0, t),
        Easing::Bounce { intensity } => dropped_ball(intensity.clamp(0.0, 100.0) / 100.0, t),
        _ => {
            let [x1, y1, x2, y2] = easing
                .control_points()
                .expect("every remaining easing is a Bezier curve");
            cubic_bezier(x1, y1, x2, y2, t)
        }
    }
}

/// Value at `u` of a one-dimensional cubic Bezier from 0 to 1 with control
/// values `p1` and `p2`.
fn bezier_at(u: f64, p1: f64, p2: f64) -> f64 {
    let c = 3.0 * p1;
    let b = 3.0 * (p2 - p1) - c;
    let a = 1.0 - c - b;
    ((a * u + b) * u + c) * u
}

/// Slope at `u` of the same curve.
fn bezier_slope(u: f64, p1: f64, p2: f64) -> f64 {
    let c = 3.0 * p1;
    let b = 3.0 * (p2 - p1) - c;
    let a = 1.0 - c - b;
    (3.0 * a * u + 2.0 * b) * u + c
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at progress `x`.
///
/// `x1` and `x2` are clamped to 0 to 1, which makes the horizontal component
/// monotonic so there is exactly one answer.
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    bezier_at(solve_parameter(x, x1, x2), y1, y2)
}

/// Finds the curve parameter `u` at which the horizontal component equals `x`.
fn solve_parameter(x: f64, x1: f64, x2: f64) -> f64 {
    const TOLERANCE: f64 = 1e-10;

    // Newton's method converges in a few steps on almost every curve.
    let mut u = x;
    for _ in 0..8 {
        let error = bezier_at(u, x1, x2) - x;
        if error.abs() < TOLERANCE {
            return u;
        }
        let slope = bezier_slope(u, x1, x2);
        if slope.abs() < 1e-6 {
            break;
        }
        let next = u - error / slope;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        u = next;
    }

    // Flat spots (handles at 0 or 1) defeat Newton; bisection cannot fail on a
    // monotonic curve.
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    let mut u = x;
    for _ in 0..60 {
        let value = bezier_at(u, x1, x2);
        if (value - x).abs() < TOLERANCE {
            break;
        }
        if value < x {
            low = u;
        } else {
            high = u;
        }
        u = (low + high) / 2.0;
    }
    u
}

/// A damped spring released at 0 and pulled toward 1.
///
/// The spring starts at rest and its wobble fades by a fixed factor over the
/// animation, so it has always settled by the end; the result is divided by
/// the last value so it ends on exactly 1. Intensity sets how many wobbles fit
/// in: about 1, 2.4 and 4.5, with first swings of about 4%, 25% and 49%.
fn spring(i: f64, t: f64) -> f64 {
    const DECAY: f64 = 6.5;
    let wobbles = 1.0 + 2.0 * i + 1.5 * i * i;
    let frequency = 2.0 * PI * wobbles;
    let position = |t: f64| {
        let (sin, cos) = (frequency * t).sin_cos();
        1.0 - (-DECAY * t).exp() * (cos + DECAY / frequency * sin)
    };
    position(t) / position(1.0)
}

/// A ball dropped from 0 onto a floor at 1.
///
/// The first fall is a parabola. Each bounce after it is a parabola that
/// reaches `restitution²` of the previous height and takes `restitution` of
/// the previous time, as a real ball does. Bounces are added until they are
/// too small to see, and the whole run is fitted into the animation.
fn dropped_ball(i: f64, t: f64) -> f64 {
    const SMALLEST_VISIBLE_BOUNCE: f64 = 0.02;
    const MAX_BOUNCES: i32 = 8;

    let restitution = 0.25 + 0.45 * i;
    let mut bounces = 1;
    while bounces < MAX_BOUNCES && restitution.powi(2 * bounces) > SMALLEST_VISIBLE_BOUNCE {
        bounces += 1;
    }

    // Time of the first fall, in units where the whole run lasts 1.
    let total: f64 = 1.0 + 2.0 * (1..=bounces).map(|k| restitution.powi(k)).sum::<f64>();
    let fall = 1.0 / total;
    if t < fall {
        let u = t / fall;
        return u * u;
    }

    let mut start = fall;
    for k in 1..=bounces {
        let duration = 2.0 * fall * restitution.powi(k);
        if t < start + duration || k == bounces {
            let u = ((t - start) / duration).clamp(0.0, 1.0);
            let height = restitution.powi(2 * k) * 4.0 * u * (1.0 - u);
            return 1.0 - height;
        }
        start += duration;
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const INTENSITIES: [f64; 5] = [0.0, 25.0, 50.0, 75.0, 100.0];

    fn all_with_intensity(intensity: f64) -> Vec<Easing> {
        vec![
            Easing::Smooth { intensity },
            Easing::Natural { intensity },
            Easing::SlowDown { intensity },
            Easing::Accelerate { intensity },
            Easing::Overshoot { intensity },
            Easing::Elastic { intensity },
            Easing::Bounce { intensity },
        ]
    }

    fn samples(easing: &Easing) -> Vec<f64> {
        (0..=1000)
            .map(|i| ease(easing, f64::from(i) / 1000.0))
            .collect()
    }

    #[test]
    fn every_easing_starts_at_zero_and_ends_at_one() {
        let mut easings = vec![
            Easing::Linear,
            Easing::Bezier {
                x1: 0.2,
                y1: 0.0,
                x2: 0.0,
                y2: 1.0,
            },
        ];
        for intensity in INTENSITIES {
            easings.extend(all_with_intensity(intensity));
        }
        for easing in easings {
            assert_eq!(ease(&easing, 0.0), 0.0, "{easing:?}");
            assert_eq!(ease(&easing, 1.0), 1.0, "{easing:?}");
            assert_eq!(ease(&easing, -3.0), 0.0, "{easing:?}");
            assert_eq!(ease(&easing, 7.0), 1.0, "{easing:?}");
            assert_eq!(ease(&easing, f64::NAN), 0.0, "{easing:?}");
            // No jump on the way into either end.
            assert!(ease(&easing, 1e-6).abs() < 1e-3, "{easing:?}");
            assert!((ease(&easing, 1.0 - 1e-6) - 1.0).abs() < 1e-3, "{easing:?}");
        }
    }

    #[test]
    fn the_plain_curves_never_go_backwards() {
        for intensity in INTENSITIES {
            for easing in [
                Easing::Linear,
                Easing::Smooth { intensity },
                Easing::Natural { intensity },
                Easing::SlowDown { intensity },
                Easing::Accelerate { intensity },
            ] {
                let values = samples(&easing);
                for pair in values.windows(2) {
                    assert!(pair[1] >= pair[0] - 1e-9, "{easing:?} went backwards");
                }
                assert!(values.iter().all(|v| (-1e-9..=1.0 + 1e-9).contains(v)));
            }
        }
    }

    #[test]
    fn slow_down_leads_and_accelerate_trails_linear() {
        for intensity in INTENSITIES {
            for t in [0.2, 0.5, 0.8] {
                assert!(ease(&Easing::SlowDown { intensity }, t) > t);
                assert!(ease(&Easing::Accelerate { intensity }, t) < t);
            }
        }
        // They are mirror images of each other.
        let slow = ease(&Easing::slow_down(), 0.3);
        let fast = ease(&Easing::accelerate(), 0.7);
        assert!((slow - (1.0 - fast)).abs() < 1e-9);
    }

    #[test]
    fn smooth_is_symmetric_about_the_middle() {
        for intensity in INTENSITIES {
            let easing = Easing::Smooth { intensity };
            assert!((ease(&easing, 0.5) - 0.5).abs() < 1e-9);
            assert!((ease(&easing, 0.25) + ease(&easing, 0.75) - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn more_intensity_means_a_stronger_curve() {
        // Measured as the largest distance from the straight line.
        let strength = |easing: &Easing| {
            samples(easing)
                .iter()
                .enumerate()
                .map(|(i, v)| (v - i as f64 / 1000.0).abs())
                .fold(0.0_f64, f64::max)
        };
        let makers: [fn(f64) -> Easing; 5] = [
            |intensity| Easing::Smooth { intensity },
            |intensity| Easing::Natural { intensity },
            |intensity| Easing::SlowDown { intensity },
            |intensity| Easing::Accelerate { intensity },
            |intensity| Easing::Overshoot { intensity },
        ];
        for make in makers {
            let (low, mid, high) = (
                strength(&make(0.0)),
                strength(&make(50.0)),
                strength(&make(100.0)),
            );
            assert!(
                low < mid && mid < high,
                "{:?}: {low} {mid} {high}",
                make(50.0)
            );
        }
    }

    #[test]
    fn overshoot_passes_one_by_the_designed_amount() {
        let peak = |intensity| {
            samples(&Easing::Overshoot { intensity })
                .into_iter()
                .fold(0.0_f64, f64::max)
        };
        assert!((peak(0.0) - 1.012).abs() < 0.005, "{}", peak(0.0));
        assert!((peak(50.0) - 1.12).abs() < 0.01, "{}", peak(50.0));
        assert!((peak(100.0) - 1.33).abs() < 0.01, "{}", peak(100.0));
    }

    #[test]
    fn elastic_wobbles_then_settles() {
        for intensity in INTENSITIES {
            let values = samples(&Easing::Elastic { intensity });
            let peak = values.iter().copied().fold(0.0_f64, f64::max);
            assert!(
                peak > 1.02,
                "intensity {intensity} should pass 1, peak {peak}"
            );
            assert!(peak < 1.6, "intensity {intensity} peak {peak}");
            // Settled well before the end: the last tenth stays near 1.
            assert!(values[900..].iter().all(|v| (v - 1.0).abs() < 0.02));
        }
        let peak_of = |intensity| {
            samples(&Easing::Elastic { intensity })
                .into_iter()
                .fold(0.0_f64, f64::max)
        };
        assert!(peak_of(0.0) < peak_of(50.0) && peak_of(50.0) < peak_of(100.0));
    }

    #[test]
    fn bounce_hits_the_floor_and_comes_back_off_it() {
        for intensity in INTENSITIES {
            let values = samples(&Easing::Bounce { intensity });
            assert!(values.iter().all(|v| (0.0..=1.0 + 1e-9).contains(v)));
            // It reaches 1 before the end, then leaves it again.
            let first_hit = values.iter().position(|v| *v > 0.999).unwrap();
            assert!(
                first_hit < 900,
                "intensity {intensity} first hit at {first_hit}"
            );
            let lowest_after = values[first_hit..].iter().copied().fold(1.0_f64, f64::min);
            assert!(lowest_after < 0.99, "intensity {intensity} never bounced");
            // Continuous: no step larger than the steepest fall allows.
            for pair in values.windows(2) {
                assert!(
                    (pair[1] - pair[0]).abs() < 0.02,
                    "intensity {intensity} jumps"
                );
            }
        }
    }

    #[test]
    fn bezier_solver_matches_a_brute_force_search() {
        let curves = [
            [0.25, 0.1, 0.25, 1.0],
            [0.42, 0.0, 0.58, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 1.0, 0.0],
            [0.5, -0.5, 0.5, 1.5],
        ];
        for [x1, y1, x2, y2] in curves {
            let easing = Easing::Bezier { x1, y1, x2, y2 };
            for step in 1..20 {
                let x = f64::from(step) / 20.0;
                // Walk the curve finely and keep the point whose x is closest.
                let mut best = (f64::MAX, 0.0);
                for i in 0..=200_000 {
                    let u = f64::from(i) / 200_000.0;
                    let distance = (bezier_at(u, x1, x2) - x).abs();
                    if distance < best.0 {
                        best = (distance, bezier_at(u, y1, y2));
                    }
                }
                let solved = ease(&easing, x);
                assert!(
                    (solved - best.1).abs() < 1e-3,
                    "cubic-bezier({x1}, {y1}, {x2}, {y2}) at {x}: {solved} vs {}",
                    best.1
                );
            }
        }
    }

    #[test]
    fn bezier_with_out_of_range_handles_is_clamped_not_broken() {
        let wild = Easing::Bezier {
            x1: -2.0,
            y1: 0.0,
            x2: 3.0,
            y2: 1.0,
        };
        for value in samples(&wild) {
            assert!(value.is_finite());
        }
    }

    #[test]
    fn serde_uses_a_type_tag_and_omits_the_default_intensity() {
        assert_eq!(
            serde_json::to_value(Easing::smooth()).unwrap(),
            json!({ "type": "smooth" })
        );
        assert_eq!(
            serde_json::to_value(Easing::SlowDown { intensity: 80.0 }).unwrap(),
            json!({ "type": "slow_down", "intensity": 80.0 })
        );
        assert_eq!(
            serde_json::to_value(Easing::Linear).unwrap(),
            json!({ "type": "linear" })
        );

        let parsed: Easing =
            serde_json::from_value(json!({ "type": "smooth", "intensity": 50 })).unwrap();
        assert_eq!(parsed, Easing::smooth());
        let bezier: Easing = serde_json::from_value(
            json!({ "type": "bezier", "x1": 0.2, "y1": 0, "x2": 0, "y2": 1 }),
        )
        .unwrap();
        assert_eq!(
            bezier,
            Easing::Bezier {
                x1: 0.2,
                y1: 0.0,
                x2: 0.0,
                y2: 1.0
            }
        );
    }

    #[test]
    fn serde_rejects_unknown_types_and_fields() {
        let unknown_type =
            serde_json::from_value::<Easing>(json!({ "type": "wiggle" })).unwrap_err();
        assert!(
            unknown_type.to_string().contains("slow_down"),
            "{unknown_type}"
        );
        let unknown_field =
            serde_json::from_value::<Easing>(json!({ "type": "smooth", "strength": 3 }))
                .unwrap_err();
        assert!(
            unknown_field.to_string().contains("strength"),
            "{unknown_field}"
        );
        assert!(serde_json::from_value::<Easing>(json!({ "type": "bezier", "x1": 0.2 })).is_err());
    }

    #[test]
    fn css_export() {
        assert_eq!(Easing::Linear.to_css(), "linear");
        assert_eq!(Easing::smooth().to_css(), "cubic-bezier(0.5, 0, 0.5, 1)");
        assert_eq!(
            Easing::slow_down().to_css(),
            "cubic-bezier(0.2, 0.8, 0.375, 1)"
        );
        assert_eq!(
            Easing::Bezier {
                x1: 0.25,
                y1: 0.1,
                x2: 0.25,
                y2: 1.0
            }
            .to_css(),
            "cubic-bezier(0.25, 0.1, 0.25, 1)"
        );
        let overshoot = Easing::overshoot().to_css();
        assert!(
            overshoot.starts_with("cubic-bezier(0.333, 1.6"),
            "{overshoot}"
        );

        for easing in [Easing::elastic(), Easing::bounce()] {
            let css = easing.to_css();
            assert!(css.starts_with("linear(0, "), "{css}");
            assert!(css.ends_with(", 1)"), "{css}");
            assert_eq!(css.matches(',').count(), 60, "61 samples");
        }
    }

    #[test]
    fn css_numbers_are_short() {
        assert_eq!(css_number(0.5), "0.5");
        assert_eq!(css_number(1.0), "1");
        assert_eq!(css_number(-0.0004), "0");
        assert_eq!(css_number(1.23456), "1.235");
    }
}

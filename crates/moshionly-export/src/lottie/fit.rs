//! Keyframes: a property's value over time, as few Lottie keyframes as keep
//! within a tolerance of Moshionly's own value at every moment.
//!
//! Moshionly has no keyframes. A property's value is a function of time,
//! worked out by evaluation from the animations on the layer. Lottie
//! players interpolate between keyframes, each eased by a cubic Bezier
//! curve. This module finds keyframes that reproduce the function.
//!
//! Time is cut where animations start and end. Between two cuts:
//!
//! - a value that does not change becomes a hold keyframe;
//! - a value that follows one animation's easing becomes one keyframe
//!   whose handles are that easing, or the piece of it between the cuts.
//!   Linear and Bezier easings are cubic curves, and so is any piece of
//!   one, so this is exact;
//! - anything else (elastic and bouncing easings, several animations
//!   adding up, values computed from several properties) is fitted with
//!   cubic pieces, split where the error is largest until every piece keeps
//!   within the tolerance.
//!
//! Every candidate is checked against the true value eight times a frame,
//! after its numbers are rounded the way they are written, so nothing is
//! taken on trust. Where no animation runs, a value is expected to hold;
//! that it does is checked once a frame, and a value that does not hold
//! after all is fitted like any other. Values that jump (a `show`, an
//! animation that starts from a value of its own) get a hold keyframe up
//! to the jump.
//!
//! A fit looks at the cancellation token every few dozen values it works
//! out, and once the export is cancelled it ends at once with the value at
//! its start, which is thrown away with the rest of the export.

use std::collections::HashMap;

use moshionly_core::{Easing, ease};

use crate::progress::CancelToken;

use super::json::{places, round};

/// How a keyframe moves on to the next one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Ease {
    /// It keeps its value until the next keyframe.
    Hold,
    /// It moves along a cubic Bezier easing `[x1, y1, x2, y2]`: Lottie's out
    /// handle `(x1, y1)` and in handle `(x2, y2)`.
    Curve([f64; 4]),
}

/// The easing handles of a straight line.
pub(crate) const LINEAR: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

/// One keyframe.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Keyframe {
    /// When, in frames.
    pub time: f64,
    /// The value, rounded as it is written.
    pub value: Vec<f64>,
    /// How the value moves on to the next keyframe's. The last keyframe's
    /// is not used.
    pub ease: Ease,
}

/// A property's values: one value throughout, or keyframes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Animated {
    /// The same value the whole time.
    Static(Vec<f64>),
    /// Keyframes, at least two.
    Keyframes(Vec<Keyframe>),
}

#[cfg(test)]
impl Animated {
    /// The value at the first moment.
    pub fn first(&self) -> &[f64] {
        match self {
            Animated::Static(value) => value,
            Animated::Keyframes(keys) => &keys[0].value,
        }
    }

    /// The number of keyframes; 0 for a fixed value.
    pub fn keyframe_count(&self) -> usize {
        match self {
            Animated::Static(_) => 0,
            Animated::Keyframes(keys) => keys.len(),
        }
    }

    /// The value at `frame`, as a player works it out.
    pub fn at(&self, frame: f64) -> Vec<f64> {
        let Animated::Keyframes(keys) = self else {
            return self.first().to_vec();
        };
        let next = keys.partition_point(|key| key.time <= frame);
        if next == 0 {
            return keys[0].value.clone();
        }
        if next == keys.len() {
            return keys[keys.len() - 1].value.clone();
        }
        let (key, after) = (&keys[next - 1], &keys[next]);
        match key.ease {
            Ease::Hold => key.value.clone(),
            Ease::Curve(handles) => {
                let x = (frame - key.time) / (after.time - key.time);
                interpolate(&key.value, &after.value, handles, x)
            }
        }
    }
}

/// The value `x` of the way through a keyframe from `from` to `to` eased by
/// `handles`, as Lottie players compute it.
fn interpolate(from: &[f64], to: &[f64], handles: [f64; 4], x: f64) -> Vec<f64> {
    let [x1, y1, x2, y2] = handles;
    let progress = ease(&Easing::Bezier { x1, y1, x2, y2 }, x);
    from.iter()
        .zip(to)
        .map(|(from, to)| from + (to - from) * progress)
        .collect()
}

/// An animation's easing over its time, in frames: a hint for the fitter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EaseSpan {
    /// Where it starts, in frames.
    pub start: f64,
    /// Where it ends, in frames.
    pub end: f64,
    /// How it is paced.
    pub easing: Easing,
}

/// What the fitter is told about where a value may change and how.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Hints {
    /// Moments, in frames, where the value may change course or jump:
    /// where animations start and end.
    pub cuts: Vec<f64>,
    /// The easings of the animations, over their time.
    pub spans: Vec<EaseSpan>,
}

impl Hints {
    /// Adds an animation from `start` to `end` frames paced by `easing`.
    pub fn add(&mut self, start: f64, end: f64, easing: Easing) {
        self.cuts.push(start);
        self.cuts.push(end);
        if end > start {
            self.spans.push(EaseSpan { start, end, easing });
        }
    }

    /// Adds a moment where the value may jump.
    pub fn cut(&mut self, at: f64) {
        self.cuts.push(at);
    }
}

/// A value to fit: a function of time in frames, and how close keyframes
/// must keep to it.
pub(crate) struct Track<'a> {
    /// The value at a time in frames. Every call with the same time must
    /// give the same value.
    pub at: &'a dyn Fn(f64) -> Vec<f64>,
    /// The largest difference allowed in any component, in its units.
    pub tolerance: f64,
    /// Decimals the values are written with.
    pub places: i32,
    /// Ends the fit early once cancelled.
    pub cancel: Option<&'a CancelToken>,
}

/// The most times a piece is split in two.
const MAX_DEPTH: u32 = 14;
/// Pieces shorter than this, in frames, are not split again.
const MIN_PIECE: f64 = 0.02;
/// How long before a jump the value is held, in frames.
const BEFORE_JUMP: f64 = 0.01;
/// How many times a frame the fit is checked.
const CHECKS_PER_FRAME: f64 = 8.0;
/// The part of the tolerance a fit may use where it is checked, leaving
/// the rest for the moments between checks.
const MARGIN: f64 = 0.75;
/// How many values a fit works out between looks at the cancellation
/// token.
const VALUES_PER_CANCEL_CHECK: u32 = 64;

/// Fits keyframes to `track` from frame `start` to frame `end`.
pub(crate) fn fit(track: &Track<'_>, hints: &Hints, start: f64, end: f64) -> Animated {
    let start = round(start, places::TIME);
    let end = round(end, places::TIME).max(start + MIN_PIECE);
    let mut cuts: Vec<f64> = hints
        .cuts
        .iter()
        .map(|cut| round(*cut, places::TIME))
        .filter(|cut| *cut > start && *cut < end)
        .collect();
    cuts.push(start);
    cuts.push(end);
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < MIN_PIECE);
    if let Some(last) = cuts.last_mut() {
        *last = end;
    }

    let mut fitter = Fitter {
        track,
        hints,
        cache: HashMap::new(),
        worked_out: 0,
        stopped: false,
    };
    let first = fitter.value(start);
    let mut keys = Vec::new();
    for pair in cuts.windows(2) {
        fitter.piece(pair[0], pair[1], 0, &mut keys);
    }
    if fitter.stopped {
        return Animated::Static(first);
    }
    let last = fitter.before(end);
    let last = fitter.rounded(last);
    keys.push(Keyframe {
        time: end,
        value: last,
        ease: Ease::Hold,
    });
    let keys = fitter.merge(keys);
    if fitter.stopped {
        return Animated::Static(first);
    }
    simplify(keys)
}

/// Drops keyframes that change nothing.
fn simplify(keys: Vec<Keyframe>) -> Animated {
    let mut kept: Vec<Keyframe> = Vec::with_capacity(keys.len());
    for mut key in keys {
        if let Some(previous) = kept.last_mut()
            && previous.value == key.value
        {
            // Moving to the same value is holding it.
            previous.ease = Ease::Hold;
            if key.ease == Ease::Hold {
                continue;
            }
        }
        key.time = round(key.time, places::TIME);
        kept.push(key);
    }
    // A last keyframe that only repeats the hold before it adds nothing
    // but the end of the time, which players do not need.
    if kept.iter().all(|key| key.value == kept[0].value) {
        return Animated::Static(kept.swap_remove(0).value);
    }
    Animated::Keyframes(kept)
}

struct Fitter<'a, 'b> {
    track: &'a Track<'b>,
    hints: &'a Hints,
    /// Values already worked out, by the bits of their time.
    cache: HashMap<u64, Vec<f64>>,
    /// How many values have been worked out.
    worked_out: u32,
    /// Whether the export was cancelled; the fit then ends.
    stopped: bool,
}

/// How far a candidate is from the true values.
struct Miss {
    /// The largest difference.
    error: f64,
    /// Where it is, in frames.
    at: f64,
}

impl Fitter<'_, '_> {
    fn value(&mut self, frame: f64) -> Vec<f64> {
        if let Some(value) = self.cache.get(&frame.to_bits()) {
            return value.clone();
        }
        self.worked_out += 1;
        if self.worked_out.is_multiple_of(VALUES_PER_CANCEL_CHECK)
            && self.track.cancel.is_some_and(CancelToken::is_cancelled)
        {
            self.stopped = true;
        }
        if self.stopped
            && let Some(any) = self.cache.values().next()
        {
            // Cancelled: any value will do, the fit is thrown away.
            return any.clone();
        }
        let value = (self.track.at)(frame);
        self.cache.insert(frame.to_bits(), value.clone());
        value
    }

    /// The value just before `frame`: where a piece ending there arrives.
    fn before(&mut self, frame: f64) -> Vec<f64> {
        self.value(frame - 1e-6)
    }

    fn rounded(&self, value: Vec<f64>) -> Vec<f64> {
        value
            .into_iter()
            .map(|component| round(component, self.track.places))
            .collect()
    }

    fn close(&self, a: &[f64], b: &[f64]) -> bool {
        a.iter()
            .zip(b)
            .all(|(a, b)| (a - b).abs() <= self.track.tolerance)
    }

    /// Joins neighbouring curves into one wherever one curve still keeps
    /// within the tolerance over both: cuts made by animations of other
    /// properties do not need keyframes of their own.
    fn merge(&mut self, mut keys: Vec<Keyframe>) -> Vec<Keyframe> {
        let mut index = 0;
        while index + 2 < keys.len() && !self.stopped {
            let (Ease::Curve(_), Ease::Curve(_)) = (keys[index].ease, keys[index + 1].ease) else {
                index += 1;
                continue;
            };
            let (a, b) = (keys[index].time, keys[index + 2].time);
            let (from, to) = (keys[index].value.clone(), keys[index + 2].value.clone());
            let checks: Vec<(f64, Vec<f64>)> = checks(a, b)
                .into_iter()
                .map(|time| (time, self.value(time)))
                .collect();
            let joined = self
                .candidates(a, b, &from, &to)
                .into_iter()
                .find_map(|handles| {
                    let handles = handles.map(|handle| round(handle, places::EASE));
                    let miss = self.miss(&checks, a, b, &from, &to, handles);
                    (miss.error <= self.track.tolerance * MARGIN).then_some(handles)
                });
            match joined {
                Some(handles) => {
                    keys[index].ease = Ease::Curve(handles);
                    keys.remove(index + 1);
                }
                None => index += 1,
            }
        }
        keys
    }

    /// Fits the piece from `a` to `b`, adding its keyframes to `keys`.
    fn piece(&mut self, a: f64, b: f64, depth: u32, keys: &mut Vec<Keyframe>) {
        if self.stopped {
            return;
        }
        let from = self.value(a);
        let from = self.rounded(from);
        let arrival = self.before(b);
        let to = self.rounded(arrival.clone());
        let after = self.value(b);
        let jumps = !self.close(&arrival, &after);
        // A piece that jumps at its end arrives a little early and holds.
        let end = if jumps && b - a > 2.0 * BEFORE_JUMP {
            round(b - BEFORE_JUMP, places::TIME)
        } else {
            b
        };
        // Where no animation runs the value is expected to hold: once a
        // frame is enough to see that it does.
        let quiet = !self
            .hints
            .spans
            .iter()
            .any(|span| span.start < b && span.end > a);
        if quiet && self.close(&from, &to) {
            let holds = quiet_checks(a, b).into_iter().all(|time| {
                let value = self.value(time);
                self.close(&value, &from)
            });
            if holds {
                keys.push(Keyframe {
                    time: a,
                    value: from,
                    ease: Ease::Hold,
                });
                return;
            }
        }
        let checks: Vec<(f64, Vec<f64>)> = checks(a, b)
            .into_iter()
            .map(|time| (time, self.value(time)))
            .collect();

        if self.close(&from, &to) && checks.iter().all(|(_, value)| self.close(value, &from)) {
            keys.push(Keyframe {
                time: a,
                value: from,
                ease: Ease::Hold,
            });
            return;
        }

        let mut worst: Option<Miss> = None;
        for handles in self.candidates(a, b, &from, &to) {
            let handles = handles.map(|handle| round(handle, places::EASE));
            let miss = self.miss(&checks, a, end, &from, &to, handles);
            if miss.error <= self.track.tolerance * MARGIN {
                keys.push(Keyframe {
                    time: a,
                    value: from,
                    ease: Ease::Curve(handles),
                });
                if end < b {
                    keys.push(Keyframe {
                        time: end,
                        value: to,
                        ease: Ease::Hold,
                    });
                }
                return;
            }
            if worst.as_ref().is_none_or(|worst| miss.error > worst.error) {
                worst = Some(miss);
            }
        }

        if depth >= MAX_DEPTH || b - a < 2.0 * MIN_PIECE {
            // Too short to split again: whatever happens here happens
            // within a fraction of a frame, which a straight line shows.
            keys.push(Keyframe {
                time: a,
                value: from,
                ease: Ease::Curve(LINEAR),
            });
            if end < b {
                keys.push(Keyframe {
                    time: end,
                    value: to,
                    ease: Ease::Hold,
                });
            }
            return;
        }
        let split = self.split_point(a, b, &from, &to, &checks, worst);
        self.piece(a, split, depth + 1, keys);
        self.piece(split, b, depth + 1, keys);
    }

    /// The easings worth trying for the piece from `a` to `b`: the pieces of
    /// the animations running there, a straight line, and a cubic through
    /// the ends with the value's own slopes.
    fn candidates(&mut self, a: f64, b: f64, from: &[f64], to: &[f64]) -> Vec<[f64; 4]> {
        let mut candidates: Vec<[f64; 4]> = self
            .hints
            .spans
            .iter()
            .filter(|span| span.start <= a + 1e-6 && span.end >= b - 1e-6)
            .filter_map(|span| {
                let points = span.easing.control_points()?;
                let length = span.end - span.start;
                sub_curve(points, (a - span.start) / length, (b - span.start) / length)
            })
            .collect();
        candidates.push(LINEAR);
        if let Some(handles) = self.slopes(a, b, from, to) {
            candidates.push(handles);
        }
        candidates.dedup();
        candidates
    }

    /// The cubic easing that leaves `from` and arrives at `to` with the
    /// value's own slopes, measured on the component that changes most.
    fn slopes(&mut self, a: f64, b: f64, from: &[f64], to: &[f64]) -> Option<[f64; 4]> {
        let (component, change) = from
            .iter()
            .zip(to)
            .map(|(from, to)| to - from)
            .enumerate()
            .max_by(|(_, x), (_, y)| x.abs().total_cmp(&y.abs()))?;
        if change.abs() <= self.track.tolerance {
            return None;
        }
        let h = (b - a) * 1e-3;
        let start = (self.value(a + h)[component] - self.value(a)[component]) / h;
        let end = (self.before(b)[component] - self.value(b - h)[component]) / h;
        let scale = (b - a) / (3.0 * change);
        let handles = [1.0 / 3.0, start * scale, 2.0 / 3.0, 1.0 - end * scale];
        handles.iter().all(|h| h.is_finite()).then_some(handles)
    }

    /// The largest difference between the keyframe from `from` at `a` to
    /// `to` at `end` and the true values at the `checks`.
    fn miss(
        &self,
        checks: &[(f64, Vec<f64>)],
        a: f64,
        end: f64,
        from: &[f64],
        to: &[f64],
        handles: [f64; 4],
    ) -> Miss {
        let mut miss = Miss { error: 0.0, at: a };
        for (time, value) in checks {
            let x = ((time - a) / (end - a)).min(1.0);
            let predicted = if *time >= end {
                to.to_vec()
            } else {
                interpolate(from, to, handles, x)
            };
            let error = predicted
                .iter()
                .zip(value)
                .map(|(p, v)| (p - v).abs())
                .fold(0.0, f64::max);
            if error > miss.error || error.is_nan() {
                miss = Miss {
                    error: if error.is_nan() { f64::INFINITY } else { error },
                    at: *time,
                };
            }
        }
        miss
    }

    /// Where to split a piece that no candidate fits: at a turning point of
    /// the value when it has one, found exactly, else where the fit was
    /// worst.
    fn split_point(
        &mut self,
        a: f64,
        b: f64,
        from: &[f64],
        to: &[f64],
        checks: &[(f64, Vec<f64>)],
        worst: Option<Miss>,
    ) -> f64 {
        // How far a value is from the straight line between the ends, in
        // one component, signed.
        let off = |time: f64, value: &[f64], component: usize| {
            let x = (time - a) / (b - a);
            value[component] - (from[component] + (to[component] - from[component]) * x)
        };
        // The check furthest from that line: a peak or a dip when the
        // value turns.
        let mut furthest: Option<(usize, usize, f64)> = None;
        for (index, (time, value)) in checks.iter().enumerate() {
            for component in 0..value.len().min(from.len()) {
                let distance = off(*time, value, component).abs();
                if furthest.is_none_or(|(_, _, best)| distance > best) {
                    furthest = Some((index, component, distance));
                }
            }
        }
        let turns = checks.windows(3).any(|w| {
            w[0].1.iter().zip(&w[1].1).zip(&w[2].1).any(|((p, q), r)| {
                (q - p) * (r - q) < 0.0 && (q - p).abs() > 1e-9 && (r - q).abs() > 1e-9
            })
        });
        let at = match (turns, furthest, worst) {
            (true, Some((index, component, _)), _) => {
                // The peak lies between the checks either side of the
                // furthest one; find it to a thousandth of a frame.
                let low = if index == 0 { a } else { checks[index - 1].0 };
                let high = checks.get(index + 1).map_or(b, |check| check.0);
                let sign = off(checks[index].0, &checks[index].1, component).signum();
                self.peak(low, high, |time, value| sign * off(time, value, component))
            }
            (_, _, Some(miss)) => miss.at,
            (_, Some((index, _, _)), None) => checks[index].0,
            _ => (a + b) / 2.0,
        };
        let margin = (b - a) * 0.05;
        round(at.clamp(a + margin, b - margin), places::TIME).clamp(a + MIN_PIECE, b - MIN_PIECE)
    }

    /// Where `height` of the value is largest between `low` and `high`, by
    /// golden-section search.
    fn peak(&mut self, mut low: f64, mut high: f64, height: impl Fn(f64, &[f64]) -> f64) -> f64 {
        const GOLDEN: f64 = 0.618_033_988_749_895;
        let probe = |fitter: &mut Self, time: f64| {
            let value = fitter.value(time);
            height(time, &value)
        };
        let mut left = high - GOLDEN * (high - low);
        let mut right = low + GOLDEN * (high - low);
        let (mut at_left, mut at_right) = (probe(self, left), probe(self, right));
        while high - low > 1e-3 {
            if at_left >= at_right {
                high = right;
                right = left;
                at_right = at_left;
                left = high - GOLDEN * (high - low);
                at_left = probe(self, left);
            } else {
                low = left;
                left = right;
                at_left = at_right;
                right = low + GOLDEN * (high - low);
                at_right = probe(self, right);
            }
        }
        (low + high) / 2.0
    }
}

/// The moments a piece from `a` to `b` is checked at: eight times a frame
/// on the frame grid, and a few between the ends for pieces shorter than a
/// frame, which have few moments on the grid.
fn checks(a: f64, b: f64) -> Vec<f64> {
    let mut times: Vec<f64> = Vec::new();
    let first = (a * CHECKS_PER_FRAME).floor() as i64 + 1;
    let last = (b * CHECKS_PER_FRAME).ceil() as i64 - 1;
    for step in first..=last {
        times.push(step as f64 / CHECKS_PER_FRAME);
    }
    times.retain(|time| *time > a && *time < b);
    if times.len() < CHECKS_PER_FRAME as usize {
        for k in 1..8 {
            times.push(a + (b - a) * f64::from(k) / 8.0);
        }
        times.sort_by(f64::total_cmp);
        times.dedup();
    }
    times
}

/// The moments a piece from `a` to `b` where no animation runs is checked
/// at to see that its value holds: every frame, and the middle.
fn quiet_checks(a: f64, b: f64) -> Vec<f64> {
    let first = a.floor() as i64 + 1;
    let last = b.ceil() as i64 - 1;
    let mut times: Vec<f64> = (first..=last).map(|frame| frame as f64).collect();
    times.push((a + b) / 2.0);
    times.retain(|time| *time > a && *time < b);
    times
}

/// Control points of a cubic Bezier easing as `(x, y)`, from `(0, 0)` to
/// `(1, 1)`.
type Points = [(f64, f64); 4];

/// The piece of the easing with control points `[x1, y1, x2, y2]` between
/// progress `from` and progress `to`, stretched to run from `(0, 0)` to
/// `(1, 1)` again. A piece of a cubic curve is a cubic curve, so the result
/// is exact. `None` when the value does not change over the piece.
pub(crate) fn sub_curve(handles: [f64; 4], from: f64, to: f64) -> Option<[f64; 4]> {
    let [x1, y1, x2, y2] = handles;
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let points: Points = [(0.0, 0.0), (x1, y1), (x2, y2), (1.0, 1.0)];
    let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
    if to <= from {
        return None;
    }
    let (u0, u1) = (solve_x(from, x1, x2), solve_x(to, x1, x2));
    let left = split(points, u1).0;
    let piece = if u1 > 0.0 {
        split(left, u0 / u1).1
    } else {
        left
    };
    let [p0, p1, p2, p3] = piece;
    let (dx, dy) = (p3.0 - p0.0, p3.1 - p0.1);
    if dx.abs() < 1e-12 || dy.abs() < 1e-12 {
        return None;
    }
    let handles = [
        ((p1.0 - p0.0) / dx).clamp(0.0, 1.0),
        (p1.1 - p0.1) / dy,
        ((p2.0 - p0.0) / dx).clamp(0.0, 1.0),
        (p2.1 - p0.1) / dy,
    ];
    handles.iter().all(|h| h.is_finite()).then_some(handles)
}

/// Splits a cubic at parameter `u` into the parts before and after it.
fn split(p: Points, u: f64) -> (Points, Points) {
    let lerp = |a: (f64, f64), b: (f64, f64)| (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
    let (p01, p12, p23) = (lerp(p[0], p[1]), lerp(p[1], p[2]), lerp(p[2], p[3]));
    let (p012, p123) = (lerp(p01, p12), lerp(p12, p23));
    let mid = lerp(p012, p123);
    ([p[0], p01, p012, mid], [mid, p123, p23, p[3]])
}

/// The parameter where the horizontal part of the easing curve with
/// handles at `x1` and `x2` (both 0 to 1) reaches `x`.
fn solve_x(x: f64, x1: f64, x2: f64) -> f64 {
    let at =
        |u: f64| 3.0 * (1.0 - u) * (1.0 - u) * u * x1 + 3.0 * (1.0 - u) * u * u * x2 + u * u * u;
    // The curve rises from 0 to 1, so halving the interval always works.
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..64 {
        let mid = (low + high) / 2.0;
        if at(mid) < x {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f64 = 30.0;

    /// One animation from `start` to `end` milliseconds taking a value
    /// from 0 to 100 with `easing`, as a function of frames.
    fn tween(start: f64, end: f64, easing: Easing) -> impl Fn(f64) -> Vec<f64> {
        move |frame: f64| {
            let ms = frame * 1000.0 / FPS;
            let t = if end > start {
                (ms - start) / (end - start)
            } else {
                1.0
            };
            vec![100.0 * ease(&easing, t)]
        }
    }

    fn hints(spans: &[(f64, f64, Easing)]) -> Hints {
        let mut hints = Hints::default();
        for (start, end, easing) in spans {
            hints.add(start * FPS / 1000.0, end * FPS / 1000.0, *easing);
        }
        hints
    }

    /// Fits and checks every frame and many moments between: within the
    /// tolerance where the fitter checks, and nearly so between.
    fn fit_and_check(
        at: &dyn Fn(f64) -> Vec<f64>,
        hints: &Hints,
        frames: f64,
        tolerance: f64,
    ) -> Animated {
        let track = Track {
            at,
            tolerance,
            places: 3,
            cancel: None,
        };
        let animated = fit(&track, hints, 0.0, frames);
        for step in 0..(frames * 32.0) as u32 {
            let frame = f64::from(step) / 32.0;
            let checked = step % 4 == 0;
            let allowed = if checked { tolerance } else { tolerance * 1.5 };
            let (got, want) = (animated.at(frame), at(frame));
            for (got, want) in got.iter().zip(&want) {
                assert!(
                    (got - want).abs() <= allowed + 1e-9,
                    "at frame {frame}: {got} instead of {want}\n{animated:#?}"
                );
            }
        }
        animated
    }

    fn all_easings() -> Vec<Easing> {
        let mut easings = vec![
            Easing::Linear,
            Easing::Bezier {
                x1: 0.2,
                y1: -0.4,
                x2: 0.1,
                y2: 1.3,
            },
        ];
        for intensity in [0.0, 50.0, 100.0] {
            easings.extend([
                Easing::Smooth { intensity },
                Easing::Natural { intensity },
                Easing::SlowDown { intensity },
                Easing::Accelerate { intensity },
                Easing::Overshoot { intensity },
                Easing::Elastic { intensity },
                Easing::Bounce { intensity },
            ]);
        }
        easings
    }

    #[test]
    fn bezier_easings_become_one_exact_keyframe() {
        for easing in all_easings() {
            let at = tween(200.0, 1200.0, easing);
            let animated = fit_and_check(&at, &hints(&[(200.0, 1200.0, easing)]), 60.0, 0.01);
            if easing.control_points().is_some() {
                // Hold until the start, the eased keyframe, and the end.
                assert_eq!(animated.keyframe_count(), 3, "{easing:?}: {animated:#?}");
            }
        }
    }

    #[test]
    fn springs_and_bounces_take_a_few_keyframes() {
        for easing in [Easing::elastic(), Easing::bounce()] {
            let at = tween(0.0, 1000.0, easing);
            let animated = fit_and_check(&at, &hints(&[(0.0, 1000.0, easing)]), 30.0, 0.05);
            let count = animated.keyframe_count();
            assert!((3..=30).contains(&count), "{easing:?}: {count} keyframes");
        }
    }

    #[test]
    fn the_fit_works_without_hints() {
        for easing in all_easings() {
            let at = tween(100.0, 900.0, easing);
            fit_and_check(&at, &Hints::default(), 30.0, 0.05);
        }
    }

    #[test]
    fn cuts_that_change_nothing_add_no_keyframes() {
        // An animation cut in three by other animations' starts and ends:
        // each piece is exact, and the pieces join back into one.
        let easing = Easing::smooth();
        let at = tween(0.0, 1000.0, easing);
        let mut hints = hints(&[(0.0, 1000.0, easing)]);
        hints.cut(9.0);
        hints.cut(17.5);
        let animated = fit_and_check(&at, &hints, 30.0, 0.01);
        assert_eq!(animated.keyframe_count(), 2, "{animated:#?}");
        // A piece on its own is a piece of the curve.
        let piece = sub_curve([0.5, 0.0, 0.5, 1.0], 0.3, 0.6).unwrap();
        assert_ne!(piece, [0.5, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn overlapping_animations_add_up_within_tolerance() {
        let (a, b) = (
            tween(0.0, 800.0, Easing::smooth()),
            tween(300.0, 1000.0, Easing::overshoot()),
        );
        let at = move |frame: f64| vec![a(frame)[0] + 2.0 * b(frame)[0]];
        let hints = hints(&[
            (0.0, 800.0, Easing::smooth()),
            (300.0, 1000.0, Easing::overshoot()),
        ]);
        let animated = fit_and_check(&at, &hints, 36.0, 0.05);
        assert!(animated.keyframe_count() < 20, "{animated:#?}");
    }

    #[test]
    fn jumps_hold_until_they_happen() {
        // 0 until frame 10, 50 from then on, eased to 80 from frame 20.
        let at = |frame: f64| {
            vec![if frame < 10.0 {
                0.0
            } else if frame < 20.0 {
                50.0
            } else {
                50.0 + 30.0 * ease(&Easing::smooth(), (frame - 20.0) / 10.0)
            }]
        };
        let mut hints = Hints::default();
        hints.cut(10.0);
        hints.add(20.0, 30.0, Easing::smooth());
        let animated = fit_and_check(&at, &hints, 40.0, 0.01);
        let Animated::Keyframes(keys) = animated else {
            panic!("animated")
        };
        assert_eq!(keys[0].ease, Ease::Hold);
        assert_eq!((keys[1].time, keys[1].value[0]), (10.0, 50.0));
    }

    #[test]
    fn values_that_never_change_are_static() {
        let at = |_: f64| vec![3.0, 4.0];
        let hints = hints(&[(0.0, 500.0, Easing::smooth())]);
        assert_eq!(
            fit_and_check(&at, &hints, 30.0, 0.01),
            Animated::Static(vec![3.0, 4.0])
        );
    }

    #[test]
    fn several_components_share_keyframes() {
        let one = tween(0.0, 1000.0, Easing::slow_down());
        let at = move |frame: f64| {
            let v = one(frame)[0];
            vec![v, 2.0 * v + 10.0, 5.0]
        };
        let animated = fit_and_check(
            &at,
            &hints(&[(0.0, 1000.0, Easing::slow_down())]),
            30.0,
            0.01,
        );
        assert_eq!(animated.keyframe_count(), 2);
    }

    #[test]
    fn a_cancelled_fit_ends_at_once() {
        // An elastic value over ten seconds takes thousands of values to
        // fit; cancelled after a hundred, the fit ends at the next look.
        let token = CancelToken::new();
        let worked_out = std::cell::Cell::new(0_u32);
        let at = |frame: f64| {
            worked_out.set(worked_out.get() + 1);
            if worked_out.get() == 100 {
                token.cancel();
            }
            vec![100.0 * ease(&Easing::elastic(), frame / 300.0)]
        };
        let track = Track {
            at: &at,
            tolerance: 0.01,
            places: 3,
            cancel: Some(&token),
        };
        let animated = fit(&track, &Hints::default(), 0.0, 300.0);
        assert!(
            worked_out.get() <= 100 + VALUES_PER_CANCEL_CHECK,
            "{} values",
            worked_out.get()
        );
        assert_eq!(animated, Animated::Static(vec![0.0]));
    }

    #[test]
    fn values_hold_where_nothing_runs_with_a_look_a_frame() {
        // A short animation in a long export: the value holds before and
        // after it, which needs a look a frame, not eight.
        let worked_out = std::cell::Cell::new(0_u32);
        let tween = tween(4000.0, 4500.0, Easing::smooth());
        let at = |frame: f64| {
            worked_out.set(worked_out.get() + 1);
            tween(frame)
        };
        let hints = hints(&[(4000.0, 4500.0, Easing::smooth())]);
        let track = Track {
            at: &at,
            tolerance: 0.01,
            places: 3,
            cancel: None,
        };
        let animated = fit(&track, &hints, 0.0, 400.0);
        assert!(worked_out.get() < 600, "{} values", worked_out.get());
        assert_eq!(animated.keyframe_count(), 3, "{animated:#?}");
        for step in 0..(400 * 32) {
            let frame = f64::from(step) / 32.0;
            let (got, want) = (animated.at(frame)[0], tween(frame)[0]);
            assert!(
                (got - want).abs() <= 0.015,
                "at frame {frame}: {got} instead of {want}"
            );
        }
    }

    #[test]
    fn sub_curves_match_the_whole() {
        let handles = [0.4, 0.0, 0.2, 1.0];
        let whole = Easing::Bezier {
            x1: 0.4,
            y1: 0.0,
            x2: 0.2,
            y2: 1.0,
        };
        let (from, to) = (0.25, 0.7);
        let piece = sub_curve(handles, from, to).unwrap();
        let [x1, y1, x2, y2] = piece;
        let part = Easing::Bezier { x1, y1, x2, y2 };
        let (low, high) = (ease(&whole, from), ease(&whole, to));
        for step in 0..=20 {
            let x = f64::from(step) / 20.0;
            let expected = (ease(&whole, from + (to - from) * x) - low) / (high - low);
            assert!((ease(&part, x) - expected).abs() < 1e-6, "{x}");
        }
        assert!(sub_curve(handles, 0.5, 0.5).is_none());
    }
}

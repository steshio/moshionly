//! Snapping: pulling a dragged time onto something worth lining up with.
//!
//! Two things attract a time being dragged. **Targets** are moments that
//! matter: the edges of other bars, the playhead, the start and end of the
//! scene. Within a few points of one, the time jumps to it and a guide line
//! is shown there. Away from any target the time falls on the **grid**, a
//! round step that follows the zoom, so numbers stay tidy.

/// The result of snapping a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapped {
    /// The time to use, a whole number of milliseconds.
    pub time: f64,
    /// The target the time snapped to, where a guide line should be drawn.
    /// `None` when it only fell on the grid, or did not snap at all.
    pub guide: Option<f64>,
}

/// The things a dragged time can snap to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapper {
    /// Moments worth lining up with, in milliseconds.
    targets: Vec<f64>,
    /// The step of the time grid, in milliseconds.
    grid: Option<f64>,
    /// How near a target must be to attract, in milliseconds.
    threshold: f64,
}

impl Snapper {
    /// A snapper that leaves times alone, apart from rounding them to whole
    /// milliseconds.
    pub fn off() -> Self {
        Self::default()
    }

    /// A snapper for the given targets. `threshold` is how near a target
    /// must be to attract, in milliseconds (a few points at the current
    /// zoom). `grid` is the step times fall on away from any target.
    pub fn new(targets: Vec<f64>, grid: Option<f64>, threshold: f64) -> Self {
        Self {
            targets,
            grid: grid.filter(|step| step.is_finite() && *step > 0.0),
            threshold: threshold.max(0.0),
        }
    }

    /// The nearest target within reach of `time`.
    fn target_near(&self, time: f64) -> Option<f64> {
        self.targets
            .iter()
            .copied()
            .filter(|target| (target - time).abs() <= self.threshold)
            .min_by(|a, b| (a - time).abs().total_cmp(&(b - time).abs()))
    }

    fn on_grid(&self, time: f64) -> f64 {
        match self.grid {
            Some(step) => (time / step).round() * step,
            None => time.round(),
        }
    }

    /// Snaps one time: an edge being resized, the playhead, the end of the
    /// scene.
    pub fn snap(&self, time: f64) -> Snapped {
        match self.target_near(time) {
            Some(target) => Snapped {
                time: target.round(),
                guide: Some(target),
            },
            None => Snapped {
                time: self.on_grid(time).round(),
                guide: None,
            },
        }
    }

    /// Snaps a span that is being moved without changing its length, such as
    /// a bar from `start` to `end`. Whichever end is nearest a target decides.
    /// The result's `time` is the new start.
    pub fn snap_span(&self, start: f64, end: f64) -> Snapped {
        let by_start = self
            .target_near(start)
            .map(|target| (target - start, target));
        let by_end = self.target_near(end).map(|target| (target - end, target));
        let best = match (by_start, by_end) {
            (Some(a), Some(b)) => Some(if b.0.abs() < a.0.abs() { b } else { a }),
            (a, b) => a.or(b),
        };
        match best {
            Some((shift, target)) => Snapped {
                time: (start + shift).round(),
                guide: Some(target),
            },
            None => Snapped {
                time: self.on_grid(start).round(),
                guide: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Targets at 0, 600 and 1500 ms, a 50 ms grid, and a reach of 30 ms.
    fn snapper() -> Snapper {
        Snapper::new(vec![0.0, 600.0, 1500.0], Some(50.0), 30.0)
    }

    #[test]
    fn a_time_near_a_target_jumps_to_it_and_shows_a_guide() {
        let s = snapper();
        assert_eq!(
            s.snap(588.0),
            Snapped {
                time: 600.0,
                guide: Some(600.0)
            }
        );
        assert_eq!(s.snap(629.0).time, 600.0);
        assert_eq!(s.snap(-12.0).guide, Some(0.0));
    }

    #[test]
    fn a_time_away_from_targets_falls_on_the_grid_without_a_guide() {
        let s = snapper();
        assert_eq!(
            s.snap(680.0),
            Snapped {
                time: 700.0,
                guide: None
            }
        );
        assert_eq!(s.snap(1012.0).time, 1000.0);
        assert_eq!(s.snap(1026.0).time, 1050.0);
    }

    #[test]
    fn the_nearest_of_two_targets_wins() {
        let s = Snapper::new(vec![100.0, 140.0], None, 30.0);
        assert_eq!(s.snap(115.0).time, 100.0);
        assert_eq!(s.snap(125.0).time, 140.0);
    }

    #[test]
    fn a_moved_span_snaps_by_whichever_end_is_nearer() {
        let s = snapper();
        // The start is 20 from 600, the end 310 from anything: the start
        // decides, and the length is kept by the caller.
        assert_eq!(
            s.snap_span(620.0, 1190.0),
            Snapped {
                time: 600.0,
                guide: Some(600.0)
            }
        );
        // The end is 10 from 1500: the start moves by the same 10.
        assert_eq!(
            s.snap_span(1090.0, 1490.0),
            Snapped {
                time: 1100.0,
                guide: Some(1500.0)
            }
        );
        // Both within reach: the end is closer (5 against 20).
        let s = Snapper::new(vec![600.0, 1000.0], Some(50.0), 30.0);
        assert_eq!(
            s.snap_span(620.0, 995.0),
            Snapped {
                time: 625.0,
                guide: Some(1000.0)
            }
        );
        // Neither: the start falls on the grid.
        assert_eq!(
            snapper().snap_span(830.0, 1030.0),
            Snapped {
                time: 850.0,
                guide: None
            }
        );
    }

    #[test]
    fn switched_off_it_only_rounds_to_whole_milliseconds() {
        let s = Snapper::off();
        assert_eq!(
            s.snap(599.6),
            Snapped {
                time: 600.0,
                guide: None
            }
        );
        assert_eq!(s.snap_span(12.4, 512.4).time, 12.0);
    }

    #[test]
    fn a_broken_grid_is_ignored() {
        for grid in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let s = Snapper::new(Vec::new(), Some(grid), 30.0);
            assert_eq!(s.snap(123.4).time, 123.0, "grid {grid}");
        }
    }
}

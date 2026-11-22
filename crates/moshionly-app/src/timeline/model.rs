//! The plain data a timeline shows and the events it reports.
//!
//! The timeline never changes what it is given. The app builds [`Group`]s
//! from its document every frame, hands them over, and applies the
//! [`TimelineEvent`]s that come back.

use crate::{key::Key, theme::Tone};

/// One action on the timeline: a bar from `start` to `end` on its own row.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    /// The bar's key, handed back in events.
    pub key: Key,
    /// The action's name, shown on the bar and in the label column.
    pub label: String,
    /// An icon for the action's type, shown in the label column.
    pub icon: Option<&'static str>,
    /// The colour family, which tells kinds of actions apart.
    pub tone: Tone,
    /// Start, in milliseconds from the start of the scene.
    pub start: f64,
    /// End, in milliseconds. Never before `start`.
    pub end: f64,
}

/// A layer and the actions that target it: a header row with a summary bar,
/// then one row per action.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The group's key, handed back in events.
    pub key: Key,
    /// The layer's name.
    pub label: String,
    /// An icon for the layer's type.
    pub icon: Option<&'static str>,
    /// Whether the action rows are folded away.
    pub collapsed: bool,
    /// Whether the layer is hidden. Its bars are drawn faded.
    pub hidden: bool,
    /// Whether the layer is locked. Its bars cannot be edited.
    pub locked: bool,
    /// The layer's actions, top to bottom.
    pub bars: Vec<Bar>,
}

impl Group {
    /// The span from the earliest start to the latest end of the group's
    /// bars, or `None` when it has none.
    pub fn span(&self) -> Option<(f64, f64)> {
        let start = self.bars.iter().map(|bar| bar.start).reduce(f64::min)?;
        let end = self.bars.iter().map(|bar| bar.end).reduce(f64::max)?;
        Some((start, end))
    }
}

/// An end of a bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    /// The left end.
    Start,
    /// The right end.
    End,
}

/// Something the user asked for on the timeline.
///
/// Times are whole milliseconds. Events arrive in the order things happened
/// within a frame; a drag sends its change every frame the value changes and
/// [`EditFinished`](Self::EditFinished) when it ends.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineEvent {
    /// The playhead was moved to `time`.
    PlayheadMoved {
        /// The new current time.
        time: f64,
    },
    /// A bar was dragged along the timeline. Its length is unchanged.
    BarMoved {
        /// The bar.
        key: Key,
        /// Its new start.
        start: f64,
        /// Its new end.
        end: f64,
    },
    /// A bar was resized from one end.
    BarResized {
        /// The bar.
        key: Key,
        /// The end that was dragged.
        edge: Edge,
        /// Its new start.
        start: f64,
        /// Its new end.
        end: f64,
    },
    /// The end of the scene was dragged.
    DurationChanged {
        /// The new length of the scene.
        duration: f64,
    },
    /// A drag that moved or resized bars, or changed the duration, ended:
    /// the moment to close an undo step.
    EditFinished,
    /// The set of selected bars changed. `bars` is the whole new selection.
    SelectionChanged {
        /// Every selected bar.
        bars: Vec<Key>,
    },
    /// A bar was double-clicked.
    BarActivated {
        /// The bar.
        key: Key,
    },
    /// The label of a layer row was clicked.
    GroupClicked {
        /// The group.
        key: Key,
    },
    /// A layer's actions were folded away or unfolded.
    GroupCollapsed {
        /// The group.
        key: Key,
        /// The state asked for.
        collapsed: bool,
    },
    /// A layer's eye was clicked.
    GroupHidden {
        /// The group.
        key: Key,
        /// The state asked for.
        hidden: bool,
    },
    /// A layer's lock was clicked.
    GroupLocked {
        /// The group.
        key: Key,
        /// The state asked for.
        locked: bool,
    },
    /// Play or pause was clicked.
    PlayToggled {
        /// The state asked for.
        playing: bool,
    },
    /// The loop button was clicked.
    LoopToggled {
        /// The state asked for.
        looping: bool,
    },
    /// The jump-to-start button was clicked.
    JumpedToStart,
    /// The snapping button was clicked.
    SnappingToggled {
        /// The state asked for.
        snapping: bool,
    },
}

/// One row of the timeline, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// The header row of the group at this index.
    Group(usize),
    /// The row of a bar: group index, then bar index.
    Bar(usize, usize),
}

/// The rows that are showing: every group's header, followed by its bars
/// unless the group is collapsed.
pub fn rows(groups: &[Group]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        rows.push(Row::Group(g));
        if !group.collapsed {
            rows.extend((0..group.bars.len()).map(|b| Row::Bar(g, b)));
        }
    }
    rows
}

/// The latest end of any bar, or zero when there are none.
pub fn last_end(groups: &[Group]) -> f64 {
    groups
        .iter()
        .flat_map(|group| &group.bars)
        .map(|bar| bar.end)
        .fold(0.0, f64::max)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub fn bar(key: &str, start: f64, end: f64) -> Bar {
        Bar {
            key: Key::new(key),
            label: key.to_owned(),
            icon: None,
            tone: Tone::Accent,
            start,
            end,
        }
    }

    pub fn group(key: &str, bars: Vec<Bar>) -> Group {
        Group {
            key: Key::new(key),
            label: key.to_owned(),
            icon: None,
            collapsed: false,
            hidden: false,
            locked: false,
            bars,
        }
    }

    #[test]
    fn collapsed_groups_show_only_their_header() {
        let mut groups = vec![
            group("a", vec![bar("a1", 0.0, 100.0), bar("a2", 50.0, 400.0)]),
            group("b", vec![bar("b1", 200.0, 300.0)]),
            group("c", vec![]),
        ];
        assert_eq!(
            rows(&groups),
            [
                Row::Group(0),
                Row::Bar(0, 0),
                Row::Bar(0, 1),
                Row::Group(1),
                Row::Bar(1, 0),
                Row::Group(2),
            ]
        );
        groups[0].collapsed = true;
        assert_eq!(
            rows(&groups),
            [Row::Group(0), Row::Group(1), Row::Bar(1, 0), Row::Group(2)]
        );
    }

    #[test]
    fn a_group_spans_its_bars() {
        let g = group("a", vec![bar("a1", 300.0, 500.0), bar("a2", 50.0, 400.0)]);
        assert_eq!(g.span(), Some((50.0, 500.0)));
        assert_eq!(group("empty", vec![]).span(), None);
        assert_eq!(last_end(&[g, group("empty", vec![])]), 500.0);
        assert_eq!(last_end(&[]), 0.0);
    }
}

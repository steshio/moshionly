//! The timeline widget.

use egui::{
    Align, CursorIcon, Id, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2,
};

use super::{
    bars::{self, BarLook},
    edit::{self, Original},
    header,
    layout::{self, LABELS_MIN, TRACK_PAD, TRACKS_MIN, TimelineLayout},
    model::{self, BarCommand, Edge, Group, Row, TimelineEvent},
    scale::{self, clamp_zoom, scroll_after_zoom},
    snap::Snapper,
    state::{Drag, TimelineState},
};
use crate::{
    key::Key,
    theme::{Theme, icons},
    widgets::{Grow, ListRow, MenuItem, context_menu, menu_separator, row_toggle, splitter},
};

/// How near a snapping target must be to attract, in points.
const SNAP_REACH: f32 = 6.0;
/// The widest a step of the snapping grid may be, in points.
const GRID_WIDTH: f32 = 10.0;
/// Empty room under the last row.
const BOTTOM_PAD: f32 = 24.0;
/// Room kept under a row that is scrolled into view, clear of the scroll
/// handle along the bottom.
const REVEAL_MARGIN: f32 = 12.0;

/// A timeline: a ruler, a playhead, and one row per action, grouped under
/// the layer each action targets.
///
/// The timeline is driven by plain data and never changes it. Hand it the
/// [`Group`]s to show, the selection and the current time, and apply the
/// [`TimelineEvent`]s it returns. Only the [`TimelineState`] (zoom and
/// scroll) is the widget's to change.
///
/// - **Scroll** to move through time and rows; **pinch** or ⌘-scroll to zoom
///   about the pointer.
/// - **Click or drag in the ruler** to move the playhead.
/// - **Click a bar** to select it; Shift or ⌘ adds to or removes from the
///   selection. Click empty space to clear it, or drag over empty space to
///   select with a rubber band.
/// - **Drag a bar** to move it, together with the rest of the selection.
///   **Drag either end** to resize it; the marker of an action that
///   happens at one moment has no ends and only moves. A layer's summary
///   bar moves all of the layer's actions.
/// - **Drag the tab at the end of the scene** to change its length.
///
/// Dragged things snap to the edges of other bars, the playhead and the ends
/// of the scene, with a guide line where they meet, and otherwise to a time
/// grid that follows the zoom. Holding ⌘ while dragging turns snapping off
/// (or on, when it is switched off).
#[must_use = "call show()"]
pub struct Timeline<'a> {
    state: &'a mut TimelineState,
    groups: &'a [Group],
    selection: &'a [Key],
    selected_group: Option<&'a Key>,
    playhead: f64,
    duration: f64,
    playing: bool,
    looping: bool,
    snapping: bool,
    follow: bool,
}

/// What [`Timeline::show`] reports back.
pub struct TimelineOutput {
    /// What the user asked for this frame, in order.
    pub events: Vec<TimelineEvent>,
    /// Where the parts of the timeline were drawn.
    pub layout: TimelineLayout,
}

impl<'a> Timeline<'a> {
    /// A timeline showing `groups`, with its view state in `state`.
    pub fn new(state: &'a mut TimelineState, groups: &'a [Group]) -> Self {
        Self {
            state,
            groups,
            selection: &[],
            selected_group: None,
            playhead: 0.0,
            duration: 0.0,
            playing: false,
            looping: false,
            snapping: true,
            follow: false,
        }
    }

    /// Scrolls along with the playhead so it stays in view, a page at a
    /// time: for playback.
    pub fn follow_playhead(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }

    /// The keys of the selected bars.
    pub fn selection(mut self, selection: &'a [Key]) -> Self {
        self.selection = selection;
        self
    }

    /// The layer that is selected in the rest of the app; its row is tinted.
    pub fn selected_group(mut self, key: Option<&'a Key>) -> Self {
        self.selected_group = key;
        self
    }

    /// The current time, in milliseconds.
    pub fn playhead(mut self, time: f64) -> Self {
        self.playhead = time;
        self
    }

    /// The length of the scene, in milliseconds.
    pub fn duration(mut self, duration: f64) -> Self {
        self.duration = duration;
        self
    }

    /// Whether the animation is playing, for the play button.
    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    /// Whether playback loops, for the loop button.
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Whether dragged things snap, for the snapping button.
    pub fn snapping(mut self, snapping: bool) -> Self {
        self.snapping = snapping;
        self
    }

    /// Draws the timeline in `rect`.
    pub fn show(self, ui: &mut Ui, rect: Rect) -> TimelineOutput {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let Self {
            state,
            groups,
            selection,
            selected_group,
            playhead,
            duration,
            playing,
            looping,
            snapping,
            follow,
        } = self;
        let id = ui.make_persistent_id("timeline");
        let mut events = Vec::new();
        let rows = model::rows(groups);
        let duration = duration.max(0.0);
        let end = duration.max(model::last_end(groups));

        // Zoom so the scene fits: on first show and when asked.
        if state.fit_requested && rect.width() > 0.0 {
            let room = TimelineLayout::compute(rect, state, theme).fit_width();
            state.points_per_second = scale::zoom_to_fit(end.max(1.0), room);
            state.scroll_x = 0.0;
            state.fit_requested = false;
        }
        state.label_width = layout::label_width(state.label_width, rect.width());

        // Bring a row into view when asked.
        if let Some(key) = state.reveal.take()
            && let Some((first, last)) = model::rows_of(groups, &rows, &key)
        {
            let layout = TimelineLayout::compute(rect, state, theme);
            state.scroll_y = scroll_to_reveal(
                state.scroll_y,
                layout.tracks.height() - REVEAL_MARGIN,
                first as f32 * layout.row_height,
                (last + 1) as f32 * layout.row_height,
            );
        }

        // Scrolling and zooming change the layout, so they come first.
        scroll_and_zoom(ui, &TimelineLayout::compute(rect, state, theme), state);
        let content = content_size(
            &TimelineLayout::compute(rect, state, theme),
            rows.len(),
            end,
        );
        clamp_scroll(state, &TimelineLayout::compute(rect, state, theme), content);
        if follow && state.drag.is_none() {
            let layout = TimelineLayout::compute(rect, state, theme);
            let x = layout.scale.x(playhead);
            state.scroll_x = scroll_to_follow(
                state.scroll_x,
                x,
                layout.tracks.left(),
                layout.tracks.right(),
            );
            clamp_scroll(state, &layout, content);
        }
        let layout = TimelineLayout::compute(rect, state, theme);
        let s = layout.scale;

        // The pointer, and how dragged things snap this frame. Holding the
        // command key inverts the snapping switch.
        let (toggle, command, primary_down, press_origin, pointer) = ui.input(|i| {
            (
                i.modifiers.shift || i.modifiers.command,
                i.modifiers.command,
                i.pointer.primary_down(),
                i.pointer.press_origin(),
                i.pointer.latest_pos(),
            )
        });
        let snap_on = snapping != command;
        let snapper = |targets: Vec<f64>, with_grid: bool| {
            if !snap_on {
                return Snapper::off();
            }
            let grid = with_grid.then(|| scale::grid_step(s.points_per_ms, GRID_WIDTH));
            Snapper::new(targets, grid, f64::from(SNAP_REACH / s.points_per_ms))
        };

        // Backgrounds. Outside the scene the tracks are tinted.
        let body = Rect::from_min_max(layout.labels.min, rect.max);
        let body_painter = ui.painter_at(body);
        let tracks_painter = ui.painter_at(layout.tracks);
        body_painter.rect_filled(body, 0.0, c.surface);
        for outside in header::outside_scene(layout.tracks, s.x(0.0), s.x(duration)) {
            tracks_painter.rect_filled(outside, 0.0, c.timeline_outside);
        }

        // Empty track: a click clears the selection, a drag is a rubber band.
        let background = ui.interact(layout.tracks, id.with("tracks"), Sense::click_and_drag());
        if background.clicked() && !toggle && !selection.is_empty() {
            events.push(TimelineEvent::SelectionChanged { bars: Vec::new() });
        }
        if background.drag_started()
            && state.drag.is_none()
            && let Some(origin) = press_origin.or(pointer)
        {
            state.drag = Some(Drag::Marquee {
                time: s.time(origin.x),
                y: origin.y - layout.row_top(0),
                base: if toggle {
                    selection.to_vec()
                } else {
                    Vec::new()
                },
            });
        }

        // Rows: tint, bar, label. `placed` collects where every editable bar
        // is, for the rubber band.
        let mut placed: Vec<(Key, Rect)> = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let row_rect = layout.row_rect(index);
            let on_screen = row_rect.bottom() >= body.top() && row_rect.top() <= body.bottom();
            match *row {
                Row::Group(g) => {
                    if !on_screen {
                        continue;
                    }
                    let group = &groups[g];
                    if selected_group == Some(&group.key) {
                        body_painter.rect_filled(row_rect, 0.0, c.accent_soft.gamma_multiply(0.5));
                    }
                    if let Some(span) = group.span() {
                        let bar_rect = layout.bar_rect(index, span.0, span.1);
                        // The summary stands for the group's actions, not
                        // for bars that only show something.
                        let all: Vec<Key> = group
                            .bars
                            .iter()
                            .filter(|bar| !bar.fixed)
                            .map(|bar| bar.key.clone())
                            .collect();
                        let editable = !group.locked && !all.is_empty();
                        let sense = if editable {
                            Sense::click_and_drag()
                        } else {
                            Sense::hover()
                        };
                        let response = interact_in(
                            ui,
                            bar_rect,
                            layout.tracks,
                            id.with(("summary", group.key.as_str())),
                            sense,
                        );
                        let look = BarLook {
                            selected: !all.is_empty()
                                && all.iter().all(|key| selection.contains(key)),
                            hovered: response.as_ref().is_some_and(|r| r.hovered()) && editable,
                            faded: group.hidden || group.locked,
                        };
                        bars::paint_summary(ui, &tracks_painter, bar_rect, look);
                        // The summary stands for all the layer's actions:
                        // a click selects them, a drag moves them.
                        if let Some(response) = response {
                            let grabbed = response.drag_started() && state.drag.is_none();
                            if (response.clicked() || grabbed) && selection != all.as_slice() {
                                events.push(TimelineEvent::SelectionChanged { bars: all.clone() });
                            }
                            if grabbed {
                                state.drag = Some(Drag::Bars {
                                    span,
                                    grab_time: press_origin.map_or(span.0, |p| s.time(p.x)),
                                    originals: originals_of(groups, &all),
                                });
                            }
                        }
                    }
                    group_label(ui, &layout, index, group, &mut events);
                }
                Row::Bar(g, b) => {
                    let group = &groups[g];
                    let bar = &group.bars[b];
                    let bar_rect = layout.bar_rect(index, bar.start, bar.end);
                    let editable = !group.locked && !bar.fixed;
                    if editable {
                        placed.push((bar.key.clone(), bar_rect));
                    }
                    if !on_screen {
                        continue;
                    }
                    let selected = selection.contains(&bar.key);
                    if selected {
                        body_painter.rect_filled(row_rect, 0.0, c.accent_soft);
                    }
                    let sense = if editable {
                        Sense::click_and_drag()
                    } else if bar.fixed {
                        Sense::click()
                    } else {
                        Sense::hover()
                    };
                    let response = interact_in(
                        ui,
                        bar_rect,
                        layout.tracks,
                        id.with(("bar", bar.key.as_str())),
                        sense,
                    );
                    let look = BarLook {
                        selected,
                        hovered: response.as_ref().is_some_and(|r| r.hovered()) && editable,
                        faded: group.hidden || group.locked,
                    };
                    bars::paint_bar(ui, &tracks_painter, bar_rect, &bar.label, bar.tone, look);

                    // The ends: grabbing one resizes the bar. A marker of
                    // an instant has no length to change: all of it moves.
                    for edge in [Edge::Start, Edge::End] {
                        if !editable || bar.instant {
                            break;
                        }
                        let zone = edge_zone(bar_rect, edge, theme.size.bar_edge_grab);
                        let Some(grip) = interact_in(
                            ui,
                            zone,
                            layout.tracks,
                            id.with(("edge", bar.key.as_str(), edge)),
                            Sense::drag(),
                        ) else {
                            continue;
                        };
                        if grip.hovered() {
                            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                        }
                        if grip.drag_started() && state.drag.is_none() {
                            let edge_time = match edge {
                                Edge::Start => bar.start,
                                Edge::End => bar.end,
                            };
                            let grabbed_at =
                                press_origin.or(pointer).map_or(edge_time, |p| s.time(p.x));
                            state.drag = Some(Drag::Edge {
                                key: bar.key.clone(),
                                edge,
                                original: (bar.start, bar.end),
                                grab_offset: grabbed_at - edge_time,
                            });
                            if !selected {
                                events.push(TimelineEvent::SelectionChanged {
                                    bars: vec![bar.key.clone()],
                                });
                            }
                        }
                    }

                    // The body: a click selects, a drag moves the selection.
                    let label_clicked = bar_label(ui, &layout, index, bar, selected);
                    let clicked = response.as_ref().is_some_and(|r| r.clicked()) || label_clicked;
                    if bar.fixed {
                        // Nothing of its own to select or change: it is
                        // about its group's layer.
                        if clicked {
                            events.push(TimelineEvent::GroupClicked {
                                key: group.key.clone(),
                            });
                        }
                        continue;
                    }
                    if clicked && !group.locked {
                        events.push(TimelineEvent::SelectionChanged {
                            bars: selection_after_click(selection, &bar.key, toggle),
                        });
                    }
                    if response.as_ref().is_some_and(|r| r.double_clicked()) {
                        events.push(TimelineEvent::BarActivated {
                            key: bar.key.clone(),
                        });
                    }
                    // A right click works on the selection, which takes in
                    // the bar first.
                    if let Some(response) = response.as_ref().filter(|_| !group.locked) {
                        if response.secondary_clicked() && !selected {
                            events.push(TimelineEvent::SelectionChanged {
                                bars: vec![bar.key.clone()],
                            });
                        }
                        bar_menu(response, &mut events);
                    }
                    if response.is_some_and(|r| r.drag_started()) && state.drag.is_none() {
                        // Dragging a bar that is not selected selects it first.
                        let moving = if selected {
                            selection.to_vec()
                        } else {
                            let next = selection_after_click(selection, &bar.key, toggle);
                            events.push(TimelineEvent::SelectionChanged { bars: next.clone() });
                            next
                        };
                        state.drag = Some(Drag::Bars {
                            span: (bar.start, bar.end),
                            grab_time: press_origin.map_or(bar.start, |p| s.time(p.x)),
                            originals: originals_of(groups, &moving),
                        });
                    }
                }
            }
        }

        // The drag in progress. Everything is worked out from where things
        // were when the drag began.
        if !primary_down {
            if state.drag.is_some() && state.drag_changed {
                events.push(TimelineEvent::EditFinished);
            }
            state.drag = None;
            state.guide = None;
            state.drag_changed = false;
        } else if let (Some(drag), Some(pointer)) = (state.drag.clone(), pointer) {
            state.guide = None;
            match drag {
                Drag::Bars {
                    span,
                    grab_time,
                    originals,
                } => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    let delta = s.time(pointer.x) - grab_time;
                    let moving = |key: &Key| originals.iter().any(|(k, ..)| k == key);
                    let mut targets = bar_edges(groups, moving);
                    targets.extend([0.0, duration, playhead]);
                    let snapped = snapper(targets, true).snap_span(span.0 + delta, span.1 + delta);
                    let wanted = snapped.time - span.0;
                    let delta = edit::clamp_move(&originals, wanted);
                    // No guide when the move stopped short of the target.
                    state.guide = snapped.guide.filter(|_| delta == wanted);
                    for (key, start, end) in edit::moved(&originals, delta) {
                        // A bar that has gone since the drag began is left out.
                        if times_of(groups, &key).is_some_and(|now| now != (start, end)) {
                            events.push(TimelineEvent::BarMoved { key, start, end });
                            state.drag_changed = true;
                        }
                    }
                }
                Drag::Edge {
                    key,
                    edge,
                    original,
                    grab_offset,
                } => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let mut targets = bar_edges(groups, |k| *k == key);
                    targets.extend([0.0, duration, playhead]);
                    let snapped = snapper(targets, true).snap(s.time(pointer.x) - grab_offset);
                    let (start, end) = edit::resized(original, edge, snapped.time);
                    let landed = match edge {
                        Edge::Start => start,
                        Edge::End => end,
                    };
                    state.guide = snapped.guide.filter(|_| landed == snapped.time);
                    if times_of(groups, &key).is_some_and(|now| now != (start, end)) {
                        events.push(TimelineEvent::BarResized {
                            key,
                            edge,
                            start,
                            end,
                        });
                        state.drag_changed = true;
                    }
                }
                Drag::Duration { grab_offset } => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let mut targets = bar_edges(groups, |_| false);
                    targets.push(playhead);
                    let snapped = snapper(targets, true).snap(s.time(pointer.x) - grab_offset);
                    let next = edit::scene_end(snapped.time);
                    state.guide = snapped.guide.filter(|_| next == snapped.time);
                    if next != duration {
                        events.push(TimelineEvent::DurationChanged { duration: next });
                        state.drag_changed = true;
                    }
                }
                Drag::Marquee { time, y, base } => {
                    let origin = pos2(s.x(time), layout.row_top(0) + y);
                    let band = Rect::from_two_pos(origin, pointer);
                    let next = edit::merged(&base, edit::marquee_hits(&placed, band));
                    if next != selection {
                        events.push(TimelineEvent::SelectionChanged { bars: next });
                    }
                    tracks_painter.rect(
                        band.intersect(layout.tracks),
                        0.0,
                        c.marquee_fill,
                        Stroke::new(1.0, c.accent),
                        StrokeKind::Inside,
                    );
                }
            }
        }

        // The header goes over the body, so rows scroll away under it.
        header::ruler(ui, &layout, duration);
        let ruler = ui.interact(layout.ruler, id.with("ruler"), Sense::click_and_drag());
        if ruler.is_pointer_button_down_on()
            && state.drag.is_none()
            && let Some(pointer) = ruler.interact_pointer_pos()
        {
            // The playhead snaps to the edges of bars but not to the grid,
            // so scrubbing stays smooth.
            let mut targets = bar_edges(groups, |_| false);
            targets.extend([0.0, duration]);
            let time = snapper(targets, false)
                .snap(s.time(pointer.x))
                .time
                .clamp(0.0, duration);
            if time != playhead {
                events.push(TimelineEvent::PlayheadMoved { time });
            }
        }

        // The tab at the end of the scene.
        let end_tab = interact_in(
            ui,
            header::duration_grab(&layout, duration),
            layout.ruler,
            id.with("duration"),
            Sense::drag(),
        );
        let end_tab_hot = end_tab.as_ref().is_some_and(|r| r.hovered())
            || matches!(state.drag, Some(Drag::Duration { .. }));
        if end_tab_hot {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if end_tab.is_some_and(|r| r.drag_started()) && state.drag.is_none() {
            let grabbed_at = press_origin.or(pointer).map_or(duration, |p| s.time(p.x));
            state.drag = Some(Drag::Duration {
                grab_offset: grabbed_at - duration,
            });
        }
        header::duration_marker(ui, &layout, duration, end_tab_hot);

        // The snapping guide, where a dragged thing met a target.
        if let Some(guide) = state.guide {
            let area = Rect::from_min_max(layout.ruler.left_top(), rect.max);
            ui.painter_at(area).vline(
                s.x(guide).floor() + 0.5,
                (area.top() + theme.size.ruler_ticks)..=area.bottom(),
                Stroke::new(1.0, c.guide),
            );
        }

        header::playhead(ui, &layout, playhead);
        header::transport(
            ui,
            &layout,
            playhead,
            duration,
            playing,
            looping,
            &mut events,
        );
        header::zoom_controls(ui, &layout, state, snapping, &mut events);

        // Hairlines: under the header and beside the label column.
        let line = ui.painter_at(rect);
        let header_bottom = layout.ruler.bottom();
        line.rect_filled(
            Rect::from_min_max(
                pos2(rect.left(), header_bottom - 1.0),
                pos2(rect.right(), header_bottom),
            ),
            0.0,
            c.border,
        );
        let split = layout.labels.right();
        line.rect_filled(
            Rect::from_min_max(pos2(split - 1.0, rect.top()), pos2(split, rect.bottom())),
            0.0,
            c.border,
        );

        // The label column can be dragged wider or narrower.
        splitter(
            ui,
            "timeline-labels",
            Rect::from_min_max(pos2(split, rect.top()), pos2(split, rect.bottom())),
            Grow::Right,
            &mut state.label_width,
            LABELS_MIN..=(rect.width() - TRACKS_MIN).max(LABELS_MIN),
            theme.size.timeline_labels,
        );

        scrollbars(ui, id, &layout, content, state);

        TimelineOutput { events, layout }
    }
}

/// The start and end of every bar whose key `skip` does not match: the
/// moments a dragged thing can line up with.
fn bar_edges(groups: &[Group], skip: impl Fn(&Key) -> bool) -> Vec<f64> {
    groups
        .iter()
        .flat_map(|group| &group.bars)
        .filter(|bar| !skip(&bar.key))
        .flat_map(|bar| [bar.start, bar.end])
        .collect()
}

/// The current start and end of the bar with `key`.
fn times_of(groups: &[Group], key: &Key) -> Option<(f64, f64)> {
    groups
        .iter()
        .flat_map(|group| &group.bars)
        .find(|bar| bar.key == *key)
        .map(|bar| (bar.start, bar.end))
}

/// The bars named by `keys` with their current times, leaving out those in
/// locked layers and those that only show something: what a drag of the
/// selection moves.
fn originals_of(groups: &[Group], keys: &[Key]) -> Vec<Original> {
    groups
        .iter()
        .filter(|group| !group.locked)
        .flat_map(|group| &group.bars)
        .filter(|bar| !bar.fixed && keys.contains(&bar.key))
        .map(|bar| (bar.key.clone(), bar.start, bar.end))
        .collect()
}

/// The context menu of a bar: what can be done with the selected bars.
fn bar_menu(response: &egui::Response, events: &mut Vec<TimelineEvent>) {
    context_menu(response, |ui| {
        for command in BarCommand::ALL {
            let mut item = MenuItem::new(command.label()).danger(command == BarCommand::Delete);
            if let Some(icon) = command_icon(command) {
                item = item.icon(icon);
            }
            if command == BarCommand::Delete || command == BarCommand::Stagger {
                menu_separator(ui);
            }
            if ui.add(item).clicked() {
                events.push(TimelineEvent::Command { command });
            }
        }
    });
}

/// The icon of a context menu command.
fn command_icon(command: BarCommand) -> Option<&'static str> {
    match command {
        BarCommand::Duplicate => Some(icons::COPY),
        BarCommand::Delete => Some(icons::TRASH),
        BarCommand::Group => Some(icons::FOLDER_SIMPLE),
        BarCommand::Ungroup => None,
        BarCommand::Stagger => Some(icons::ROWS),
    }
}

/// The strip at one end of a bar that resizes it when dragged. Part of it
/// hangs outside the bar, and inside it takes at most a third, so even a
/// short bar keeps a middle to move it by.
pub fn edge_zone(bar: Rect, edge: Edge, grab: f32) -> Rect {
    let inside = grab.min(bar.width() / 3.0);
    let outside = grab / 2.0;
    match edge {
        Edge::Start => Rect::from_min_max(
            pos2(bar.left() - outside, bar.top()),
            pos2(bar.left() + inside, bar.bottom()),
        ),
        Edge::End => Rect::from_min_max(
            pos2(bar.right() - inside, bar.top()),
            pos2(bar.right() + outside, bar.bottom()),
        ),
    }
}

/// The selection after a bar was clicked: just that bar, or with `toggle`
/// (Shift or ⌘ held) the old selection with the bar added or removed.
pub fn selection_after_click(selection: &[Key], clicked: &Key, toggle: bool) -> Vec<Key> {
    if !toggle {
        return vec![clicked.clone()];
    }
    let mut next: Vec<Key> = selection.to_vec();
    match next.iter().position(|key| key == clicked) {
        Some(index) => {
            next.remove(index);
        }
        None => next.push(clicked.clone()),
    }
    next
}

/// Senses `rect` where it is inside `clip`. Returns `None` when no part of
/// it is.
fn interact_in(ui: &Ui, rect: Rect, clip: Rect, id: Id, sense: Sense) -> Option<egui::Response> {
    let visible = rect.intersect(clip);
    visible
        .is_positive()
        .then(|| ui.interact(visible, id, sense))
}

/// A child `Ui` for the label of the row at `index`, clipped to the label
/// column.
fn label_ui(ui: &mut Ui, layout: &TimelineLayout, index: usize, key: &Key) -> Ui {
    let top = layout.row_top(index);
    let rect = Rect::from_min_max(
        pos2(layout.labels.left(), top),
        pos2(layout.labels.right() - 1.0, top + layout.row_height),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(("timeline-label", key.as_str()))
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(layout.labels.intersect(ui.clip_rect()));
    child
}

/// The label of a layer row: expand arrow, icon, name, and the hide and
/// lock toggles.
fn group_label(
    ui: &mut Ui,
    layout: &TimelineLayout,
    index: usize,
    group: &Group,
    events: &mut Vec<TimelineEvent>,
) {
    let mut child = label_ui(ui, layout, index, &group.key);
    let mut row = ListRow::new(&group.label)
        .dimmed(group.hidden)
        .height(layout.row_height)
        .id_salt(group.key.as_str());
    if let Some(icon) = group.icon {
        row = row.icon(icon);
    }
    if !group.bars.is_empty() {
        row = row.expanded(!group.collapsed);
    }
    let (mut hidden, mut locked) = (group.hidden, group.locked);
    let (mut hide_clicked, mut lock_clicked) = (false, false);
    let shown = row.show(&mut child, |ui, hovered| {
        // A folder is not a layer: there is nothing to hide or lock.
        if group.folder {
            return;
        }
        hide_clicked = row_toggle(ui, &mut hidden, icons::EYE_SLASH, icons::EYE, hovered);
        lock_clicked = row_toggle(
            ui,
            &mut locked,
            icons::LOCK_SIMPLE,
            icons::LOCK_SIMPLE_OPEN,
            hovered,
        );
    });
    let key = || group.key.clone();
    if hide_clicked {
        events.push(TimelineEvent::GroupHidden { key: key(), hidden });
    }
    if lock_clicked {
        events.push(TimelineEvent::GroupLocked { key: key(), locked });
    }
    if shown.toggled {
        events.push(TimelineEvent::GroupCollapsed {
            key: key(),
            collapsed: !group.collapsed,
        });
    } else if shown.response.clicked() {
        events.push(TimelineEvent::GroupClicked { key: key() });
    }
}

/// The label of an action row. Returns true when it was clicked.
fn bar_label(
    ui: &mut Ui,
    layout: &TimelineLayout,
    index: usize,
    bar: &model::Bar,
    selected: bool,
) -> bool {
    let mut child = label_ui(ui, layout, index, &bar.key);
    let mut row = ListRow::new(&bar.label)
        .depth(1)
        .quiet(true)
        .selected(selected)
        .height(layout.row_height)
        .id_salt(bar.key.as_str());
    if let Some(icon) = bar.icon {
        row = row.icon(icon);
    }
    row.show(&mut child, |_, _| {}).response.clicked()
}

/// Applies this frame's scrolling and pinching to the view.
fn scroll_and_zoom(ui: &Ui, layout: &TimelineLayout, state: &mut TimelineState) {
    if !ui.rect_contains_pointer(layout.rect) {
        return;
    }
    let (zoom, scroll, pointer) = ui.input(|i| {
        (
            i.zoom_delta(),
            i.smooth_scroll_delta(),
            i.pointer.hover_pos(),
        )
    });
    let Some(pointer) = pointer else {
        return;
    };
    // Time only moves when the pointer is over the part that shows time.
    let over_time = pointer.x >= layout.tracks.left();
    if zoom != 1.0 && over_time {
        let old = state.points_per_second;
        let new = clamp_zoom(old * zoom);
        let anchor = pointer.x - layout.tracks.left() - TRACK_PAD;
        state.scroll_x = scroll_after_zoom(state.scroll_x, anchor, old, new);
        state.points_per_second = new;
    } else if scroll != Vec2::ZERO {
        if over_time {
            state.scroll_x -= scroll.x;
        }
        state.scroll_y -= scroll.y;
    }
}

/// The scroll offset that brings the span from `top` to `bottom` (measured
/// from the top of the content) into a view `view` tall, moving as little as
/// possible. A span taller than the view is shown from its top.
pub fn scroll_to_reveal(scroll: f32, view: f32, top: f32, bottom: f32) -> f32 {
    if bottom - top > view || top < scroll {
        top
    } else if bottom > scroll + view {
        bottom - view
    } else {
        scroll
    }
}

/// The horizontal scroll that keeps a playhead at `x` in view in tracks
/// from `left` to `right`. Nothing moves while it is in view; when it runs
/// off either side the view turns a page, so it lands near the left again.
pub fn scroll_to_follow(scroll_x: f32, x: f32, left: f32, right: f32) -> f32 {
    let width = (right - left).max(0.0);
    let margin = (width * 0.05).min(24.0);
    let landing = left + width * 0.1;
    if x > right - margin || x < left {
        (scroll_x + x - landing).max(0.0)
    } else {
        scroll_x
    }
}

/// The size of everything that can be scrolled to: all rows, and time up to
/// `end` plus half a view of empty room.
fn content_size(layout: &TimelineLayout, rows: usize, end: f64) -> Vec2 {
    Vec2::new(
        TRACK_PAD + layout.scale.width(end) + layout.tracks.width() * 0.5,
        rows as f32 * layout.row_height + BOTTOM_PAD,
    )
}

/// Keeps the scroll offsets within the content.
fn clamp_scroll(state: &mut TimelineState, layout: &TimelineLayout, content: Vec2) {
    let max = (content - layout.tracks.size()).max(Vec2::ZERO);
    state.scroll_x = state.scroll_x.clamp(0.0, max.x);
    state.scroll_y = state.scroll_y.clamp(0.0, max.y);
}

/// Thin scroll handles along the bottom and the right of the tracks, shown
/// when there is more than fits. They can be dragged.
fn scrollbars(
    ui: &mut Ui,
    id: Id,
    layout: &TimelineLayout,
    content: Vec2,
    state: &mut TimelineState,
) {
    let tracks = layout.tracks;
    let thickness = 6.0;
    let horizontal = Rect::from_min_max(
        pos2(tracks.left() + 2.0, tracks.bottom() - thickness - 2.0),
        pos2(tracks.right() - thickness - 4.0, tracks.bottom() - 2.0),
    );
    scrollbar(
        ui,
        id.with("scroll-x"),
        horizontal,
        true,
        content.x,
        tracks.width(),
        &mut state.scroll_x,
    );
    let vertical = Rect::from_min_max(
        pos2(tracks.right() - thickness - 2.0, tracks.top() + 2.0),
        pos2(tracks.right() - 2.0, tracks.bottom() - thickness - 4.0),
    );
    scrollbar(
        ui,
        id.with("scroll-y"),
        vertical,
        false,
        content.y,
        tracks.height(),
        &mut state.scroll_y,
    );
}

/// One scroll handle in `groove`. `content` and `view` are lengths along the
/// scrolling direction.
fn scrollbar(
    ui: &mut Ui,
    id: Id,
    groove: Rect,
    horizontal: bool,
    content: f32,
    view: f32,
    scroll: &mut f32,
) {
    let length = if horizontal {
        groove.width()
    } else {
        groove.height()
    };
    let max_scroll = content - view;
    if max_scroll <= 0.5 || length <= 24.0 {
        return;
    }
    let (start, handle_length) = scroll_handle(length, content, view, *scroll);
    let handle = if horizontal {
        Rect::from_min_max(
            pos2(groove.left() + start, groove.top()),
            pos2(groove.left() + start + handle_length, groove.bottom()),
        )
    } else {
        Rect::from_min_max(
            pos2(groove.left(), groove.top() + start),
            pos2(groove.right(), groove.top() + start + handle_length),
        )
    };
    let response = ui.interact(handle.expand(3.0), id, Sense::drag());
    if response.dragged() {
        let delta = response.drag_delta();
        let moved = if horizontal { delta.x } else { delta.y };
        let travel = (length - handle_length).max(1.0);
        *scroll = (*scroll + moved * max_scroll / travel).clamp(0.0, max_scroll);
    }
    let c = &Theme::of(ui).color;
    let ink = if response.dragged() || response.hovered() {
        c.text_tertiary
    } else {
        c.border_strong
    };
    ui.painter()
        .rect_filled(handle, handle.width().min(handle.height()) / 2.0, ink);
}

/// Where a scroll handle sits in a groove of `length`: its offset from the
/// start and its own length.
fn scroll_handle(length: f32, content: f32, view: f32, scroll: f32) -> (f32, f32) {
    let handle = (view / content * length).clamp(24.0_f32.min(length), length);
    let max_scroll = (content - view).max(f32::EPSILON);
    let t = (scroll / max_scroll).clamp(0.0, 1.0);
    (t * (length - handle), handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(names: &[&str]) -> Vec<Key> {
        names.iter().map(Key::new).collect()
    }

    #[test]
    fn a_plain_click_selects_only_that_bar() {
        let selection = keys(&["a", "b"]);
        assert_eq!(
            selection_after_click(&selection, &Key::new("c"), false),
            keys(&["c"])
        );
        assert_eq!(
            selection_after_click(&selection, &Key::new("a"), false),
            keys(&["a"])
        );
    }

    #[test]
    fn a_modified_click_adds_or_removes() {
        let selection = keys(&["a", "b"]);
        assert_eq!(
            selection_after_click(&selection, &Key::new("c"), true),
            keys(&["a", "b", "c"])
        );
        assert_eq!(
            selection_after_click(&selection, &Key::new("a"), true),
            keys(&["b"])
        );
        assert_eq!(
            selection_after_click(&[], &Key::new("a"), true),
            keys(&["a"])
        );
    }

    #[test]
    fn the_ends_of_a_bar_have_grab_strips() {
        let bar = Rect::from_min_max(pos2(100.0, 10.0), pos2(200.0, 30.0));
        let start = edge_zone(bar, Edge::Start, 7.0);
        let end = edge_zone(bar, Edge::End, 7.0);
        assert_eq!(
            start,
            Rect::from_min_max(pos2(96.5, 10.0), pos2(107.0, 30.0))
        );
        assert_eq!(
            end,
            Rect::from_min_max(pos2(193.0, 10.0), pos2(203.5, 30.0))
        );
        // A short bar keeps a third of itself for moving.
        let short = Rect::from_min_max(pos2(100.0, 10.0), pos2(106.0, 30.0));
        let start = edge_zone(short, Edge::Start, 7.0);
        let end = edge_zone(short, Edge::End, 7.0);
        assert_eq!(start.right(), 102.0);
        assert_eq!(end.left(), 104.0);
    }

    #[test]
    fn locked_layers_stay_out_of_a_drag_and_everything_else_is_a_target() {
        use super::super::model::tests::{bar, group};
        let mut groups = vec![
            group("a", vec![bar("a1", 0.0, 100.0), bar("a2", 50.0, 400.0)]),
            group("b", vec![bar("b1", 200.0, 300.0)]),
        ];
        groups[1].locked = true;
        let all = keys(&["a1", "a2", "b1"]);
        assert_eq!(
            originals_of(&groups, &all),
            [(Key::new("a1"), 0.0, 100.0), (Key::new("a2"), 50.0, 400.0)]
        );
        assert_eq!(
            bar_edges(&groups, |key| *key == "a1"),
            [50.0, 400.0, 200.0, 300.0]
        );
        assert_eq!(times_of(&groups, &Key::new("b1")), Some((200.0, 300.0)));
        assert_eq!(times_of(&groups, &Key::new("zz")), None);
    }

    #[test]
    fn bars_that_only_show_something_stay_out_of_a_drag() {
        use super::super::model::tests::{bar, group};
        let plays = model::Bar {
            fixed: true,
            ..bar("plays", 300.0, 900.0)
        };
        let groups = vec![group("a", vec![plays, bar("a1", 0.0, 100.0)])];
        assert_eq!(
            originals_of(&groups, &keys(&["plays", "a1"])),
            [(Key::new("a1"), 0.0, 100.0)]
        );
        // Other bars still line up with it.
        assert_eq!(bar_edges(&groups, |key| *key == "a1"), [300.0, 900.0]);
    }

    #[test]
    fn the_view_follows_the_playhead_a_page_at_a_time() {
        // Tracks from 100 to 1100, scrolled to 0.
        assert_eq!(scroll_to_follow(0.0, 600.0, 100.0, 1100.0), 0.0);
        // Near the right edge: the playhead goes back to 10% in.
        assert_eq!(scroll_to_follow(0.0, 1090.0, 100.0, 1100.0), 890.0);
        // Gone off the left (a loop started over): back to the start.
        assert_eq!(scroll_to_follow(890.0, 20.0, 100.0, 1100.0), 710.0);
        assert_eq!(scroll_to_follow(50.0, -900.0, 100.0, 1100.0), 0.0);
    }

    #[test]
    fn revealing_scrolls_no_more_than_needed() {
        // A view 200 tall, scrolled to 100: it shows 100 to 300.
        assert_eq!(scroll_to_reveal(100.0, 200.0, 150.0, 250.0), 100.0);
        // Above the view: its top comes to the top.
        assert_eq!(scroll_to_reveal(100.0, 200.0, 40.0, 90.0), 40.0);
        // Below: its bottom comes to the bottom.
        assert_eq!(scroll_to_reveal(100.0, 200.0, 380.0, 430.0), 230.0);
        // Half in view at the bottom.
        assert_eq!(scroll_to_reveal(100.0, 200.0, 280.0, 330.0), 130.0);
        // Too tall to fit: show its beginning.
        assert_eq!(scroll_to_reveal(100.0, 200.0, 250.0, 600.0), 250.0);
    }

    #[test]
    fn the_scroll_handle_shows_how_much_is_in_view() {
        // Half the content is in view: the handle is half the groove.
        assert_eq!(scroll_handle(200.0, 1000.0, 500.0, 0.0), (0.0, 100.0));
        assert_eq!(scroll_handle(200.0, 1000.0, 500.0, 500.0), (100.0, 100.0));
        assert_eq!(scroll_handle(200.0, 1000.0, 500.0, 250.0), (50.0, 100.0));
        // A lot of content: the handle does not get too small to grab.
        let (start, handle) = scroll_handle(200.0, 100_000.0, 500.0, 99_500.0);
        assert_eq!(handle, 24.0);
        assert_eq!(start, 176.0);
        // Scroll offsets outside the content are tolerated.
        assert_eq!(scroll_handle(200.0, 1000.0, 500.0, 9_999.0).0, 100.0);
    }

    #[test]
    fn the_view_cannot_scroll_past_the_content() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1000.0, 200.0));
        let mut state = TimelineState {
            scroll_x: 99_999.0,
            scroll_y: -40.0,
            ..TimelineState::with_zoom(200.0)
        };
        let layout = TimelineLayout::compute(rect, &state, Theme::light());
        // Three rows fit without scrolling; four seconds do not.
        let content = content_size(&layout, 3, 4000.0);
        clamp_scroll(&mut state, &layout, content);
        assert_eq!(state.scroll_y, 0.0);
        let tracks = layout.tracks.width();
        let expected = TRACK_PAD + 800.0 + tracks * 0.5 - tracks;
        assert!((state.scroll_x - expected).abs() < 1e-3);
    }
}

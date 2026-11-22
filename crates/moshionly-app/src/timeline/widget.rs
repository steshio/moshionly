//! The timeline widget.

use egui::{Align, Id, Layout, Rect, Sense, Ui, UiBuilder, Vec2, pos2};

use super::{
    bars::{self, BarLook},
    header,
    layout::{self, LABELS_MIN, TRACK_PAD, TRACKS_MIN, TimelineLayout},
    model::{self, Group, Row, TimelineEvent},
    scale::{self, clamp_zoom, scroll_after_zoom},
    state::TimelineState,
};
use crate::{
    key::Key,
    theme::{Theme, icons},
    widgets::{Grow, ListRow, row_toggle, splitter},
};

/// How much of the tracks' width the scene takes after "fit".
const FIT_FILL: f64 = 0.86;
/// Empty room under the last row.
const BOTTOM_PAD: f32 = 24.0;

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
///   selection. Click empty space to clear it.
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
        }
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
        } = self;
        let id = ui.make_persistent_id("timeline");
        let mut events = Vec::new();
        let rows = model::rows(groups);
        let duration = duration.max(0.0);
        let end = duration.max(model::last_end(groups));

        // Zoom so the scene fits: on first show and when asked.
        if state.fit_requested && rect.width() > 0.0 {
            let labels = layout::label_width(state.label_width, rect.width());
            let tracks = rect.width() - labels - TRACK_PAD * 2.0;
            state.points_per_second = scale::zoom_to_fit(end.max(1.0) / FIT_FILL, tracks);
            state.scroll_x = 0.0;
            state.fit_requested = false;
        }
        state.label_width = layout::label_width(state.label_width, rect.width());

        // Scrolling and zooming change the layout, so they come first.
        scroll_and_zoom(ui, &TimelineLayout::compute(rect, state, theme), state);
        let content = content_size(
            &TimelineLayout::compute(rect, state, theme),
            rows.len(),
            end,
        );
        clamp_scroll(state, &TimelineLayout::compute(rect, state, theme), content);
        let layout = TimelineLayout::compute(rect, state, theme);
        let s = layout.scale;

        // Backgrounds. Outside the scene the tracks are tinted.
        let body = Rect::from_min_max(layout.labels.min, rect.max);
        let body_painter = ui.painter_at(body);
        let tracks_painter = ui.painter_at(layout.tracks);
        body_painter.rect_filled(body, 0.0, c.surface);
        for outside in header::outside_scene(layout.tracks, s.x(0.0), s.x(duration)) {
            tracks_painter.rect_filled(outside, 0.0, c.timeline_outside);
        }

        // A click on empty track clears the selection.
        let background = ui.interact(layout.tracks, id.with("tracks"), Sense::click());
        if background.clicked() && !selection.is_empty() {
            events.push(TimelineEvent::SelectionChanged { bars: Vec::new() });
        }

        // Rows: tint, bar, label.
        let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        for (index, row) in rows.iter().enumerate() {
            let row_rect = layout.row_rect(index);
            if row_rect.bottom() < body.top() || row_rect.top() > body.bottom() {
                continue;
            }
            match *row {
                Row::Group(g) => {
                    let group = &groups[g];
                    if selected_group == Some(&group.key) {
                        body_painter.rect_filled(row_rect, 0.0, c.accent_soft.gamma_multiply(0.5));
                    }
                    if let Some((start, end)) = group.span() {
                        let bar_rect = layout.bar_rect(index, start, end);
                        let response = interact_in(
                            ui,
                            bar_rect,
                            layout.tracks,
                            id.with(("summary", group.key.as_str())),
                            Sense::click(),
                        );
                        let look = BarLook {
                            selected: group.bars.iter().all(|bar| selection.contains(&bar.key)),
                            hovered: response.as_ref().is_some_and(|r| r.hovered()),
                            faded: group.hidden || group.locked,
                        };
                        bars::paint_summary(ui, &tracks_painter, bar_rect, look);
                        // The summary stands for all the layer's actions.
                        if response.is_some_and(|r| r.clicked()) && !group.locked {
                            events.push(TimelineEvent::SelectionChanged {
                                bars: group.bars.iter().map(|bar| bar.key.clone()).collect(),
                            });
                        }
                    }
                    group_label(ui, &layout, index, group, &mut events);
                }
                Row::Bar(g, b) => {
                    let group = &groups[g];
                    let bar = &group.bars[b];
                    let selected = selection.contains(&bar.key);
                    if selected {
                        body_painter.rect_filled(row_rect, 0.0, c.accent_soft);
                    }
                    let bar_rect = layout.bar_rect(index, bar.start, bar.end);
                    let sense = if group.locked {
                        Sense::hover()
                    } else {
                        Sense::click()
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
                        hovered: response.as_ref().is_some_and(|r| r.hovered()) && !group.locked,
                        faded: group.hidden || group.locked,
                    };
                    bars::paint_bar(ui, &tracks_painter, bar_rect, &bar.label, bar.tone, look);

                    let label_clicked = bar_label(ui, &layout, index, bar, selected);
                    let clicked = response.as_ref().is_some_and(|r| r.clicked()) || label_clicked;
                    if clicked && !group.locked {
                        events.push(TimelineEvent::SelectionChanged {
                            bars: selection_after_click(selection, &bar.key, toggle),
                        });
                    }
                    if response.is_some_and(|r| r.double_clicked()) {
                        events.push(TimelineEvent::BarActivated {
                            key: bar.key.clone(),
                        });
                    }
                }
            }
        }

        // The header goes over the body, so rows scroll away under it.
        header::ruler(ui, &layout, duration);
        let ruler = ui.interact(layout.ruler, id.with("ruler"), Sense::click_and_drag());
        if ruler.is_pointer_button_down_on()
            && let Some(pointer) = ruler.interact_pointer_pos()
        {
            let time = s.time(pointer.x).round().clamp(0.0, duration);
            if time != playhead {
                events.push(TimelineEvent::PlayheadMoved { time });
            }
        }
        header::duration_marker(ui, &layout, duration, false);
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
            fit_requested: false,
            ..TimelineState::default()
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

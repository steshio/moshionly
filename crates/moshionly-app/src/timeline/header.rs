//! The header strip of the timeline: transport, ruler and zoom controls.

use egui::{
    Align, Align2, Color32, Layout, Pos2, Rect, Shape, Stroke, Ui, UiBuilder, Vec2, pos2,
    text::LayoutJob,
};

use super::{
    layout::{TRACK_PAD, TimelineLayout},
    model::TimelineEvent,
    scale::{self, clamp_zoom, scroll_after_zoom, slider_to_zoom, zoom_to_slider},
    state::TimelineState,
};
use crate::{
    theme::{Theme, icons},
    widgets::{IconButton, Slider, paint},
};

/// Least distance between two ruler labels, in points.
const LABEL_SPACING: f32 = 72.0;

/// The transport controls and the time readout, above the label column.
pub(super) fn transport(
    ui: &mut Ui,
    layout: &TimelineLayout,
    playhead: f64,
    duration: f64,
    playing: bool,
    looping: bool,
    events: &mut Vec<TimelineEvent>,
) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let rect = layout.transport;
    ui.painter_at(rect).rect_filled(rect, 0.0, c.surface);

    let mut bar = ui.new_child(
        UiBuilder::new()
            .id_salt("timeline-transport")
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    bar.set_clip_rect(rect.intersect(ui.clip_rect()));
    bar.spacing_mut().item_spacing.x = theme.space.xxs;
    bar.add_space(theme.space.md);

    if bar
        .add(IconButton::new(icons::SKIP_BACK).tooltip("Jump to start"))
        .clicked()
    {
        events.push(TimelineEvent::JumpedToStart);
    }
    let (icon, tip) = if playing {
        (icons::PAUSE, "Pause")
    } else {
        (icons::PLAY, "Play")
    };
    if bar.add(IconButton::new(icon).tooltip(tip)).clicked() {
        events.push(TimelineEvent::PlayToggled { playing: !playing });
    }
    if bar
        .add(
            IconButton::new(icons::REPEAT)
                .selected(looping)
                .dimmed(!looping)
                .tooltip("Loop playback"),
        )
        .clicked()
    {
        events.push(TimelineEvent::LoopToggled { looping: !looping });
    }
    bar.add_space(theme.space.sm);

    // The readout: current time in full strength, the scene's length quiet.
    // A monospaced face keeps it the same width while it counts.
    let mut job = LayoutJob::default();
    let format = |color: Color32| egui::TextFormat {
        font_id: theme.mono(),
        color,
        ..Default::default()
    };
    job.append(&scale::timecode(playhead), 0.0, format(c.text));
    job.append(" / ", 0.0, format(c.text_tertiary));
    job.append(&scale::timecode(duration), 0.0, format(c.text_secondary));
    let galley = bar.painter().layout_job(job);
    let size = Vec2::new(galley.size().x + theme.space.md * 2.0, theme.size.control);
    let (well, response) = bar.allocate_exact_size(size, egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Label,
            true,
            format!(
                "Time {} of {}",
                scale::timecode(playhead),
                scale::timecode(duration)
            ),
        )
    });
    bar.painter().rect_filled(well, theme.radius.sm, c.well);
    paint::galley_in(bar.painter(), well, Align2::CENTER_CENTER, galley);
}

/// Paints the ruler: ticks, labels and the tint beyond the end of the scene.
pub(super) fn ruler(ui: &Ui, layout: &TimelineLayout, duration: f64) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let rect = layout.ruler;
    let painter = ui.painter_at(rect);
    let s = layout.scale;
    painter.rect_filled(rect, 0.0, c.surface);

    // The band under the ticks is tinted outside the scene.
    let band_bottom = rect.top() + theme.size.ruler_ticks.min(rect.height());
    let labels = Rect::from_min_max(pos2(rect.left(), band_bottom), rect.max);
    for outside in outside_scene(labels, s.x(0.0), s.x(duration)) {
        painter.rect_filled(outside, 0.0, c.timeline_outside);
    }

    let ticks = scale::ticks(s.points_per_ms, LABEL_SPACING);
    let per_major = (ticks.major / ticks.minor).round().max(1.0) as i64;
    let first = (s.time(rect.left()) / ticks.minor).floor().max(0.0) as i64;
    let last = (s.time(rect.right()) / ticks.minor).ceil() as i64;
    for index in first..=last {
        let time = index as f64 * ticks.minor;
        let x = s.x(time).floor() + 0.5;
        if index % per_major == 0 {
            painter.vline(
                x,
                (rect.top() + 4.0)..=band_bottom,
                Stroke::new(1.0, c.border_strong),
            );
            painter.text(
                paint::snap(pos2(x + 4.0, labels.center().y)),
                Align2::LEFT_CENTER,
                scale::tick_label(time, ticks.major),
                theme.small(),
                c.text_secondary,
            );
        } else {
            painter.vline(
                x,
                (band_bottom - 4.0)..=band_bottom,
                Stroke::new(1.0, c.border_strong),
            );
        }
    }
}

/// The parts of `rect` that lie before time zero and after the end of the
/// scene, given the screen x of both.
pub(super) fn outside_scene(rect: Rect, zero_x: f32, end_x: f32) -> impl Iterator<Item = Rect> {
    let before = Rect::from_min_max(rect.min, pos2(zero_x.min(rect.right()), rect.bottom()));
    let after = Rect::from_min_max(pos2(end_x.max(rect.left()), rect.top()), rect.max);
    [before, after]
        .into_iter()
        .filter(|part| part.width() > 0.0 && part.height() > 0.0)
}

/// Paints the playhead: a line through the ruler and the rows, with a
/// shield-shaped head in the tick band.
pub(super) fn playhead(ui: &Ui, layout: &TimelineLayout, time: f64) {
    let theme = Theme::of(ui);
    let accent = theme.color.accent;
    let area = Rect::from_min_max(layout.ruler.left_top(), layout.rect.max);
    let x = layout.scale.x(time).floor() + 0.5;
    if x < area.left() - 8.0 || x > area.right() + 8.0 {
        return;
    }
    let painter = ui.painter_at(area);
    let top = layout.ruler.top();
    painter.vline(x, (top + 6.0)..=area.bottom(), Stroke::new(1.0, accent));
    painter.add(Shape::convex_polygon(
        playhead_shape(pos2(x, top)),
        accent,
        Stroke::NONE,
    ));
}

/// The outline of the playhead's head: flat on top with softened corners,
/// coming to a point at the bottom. `tip_top` is the middle of its top edge.
fn playhead_shape(tip_top: Pos2) -> Vec<Pos2> {
    let (half, shoulder, height, soften) = (5.5, 7.0, 12.0, 1.5);
    let p = |dx: f32, dy: f32| tip_top + Vec2::new(dx, dy + 0.5);
    vec![
        p(-half + soften, 0.0),
        p(half - soften, 0.0),
        p(half, soften),
        p(half, shoulder),
        p(0.0, height),
        p(-half, shoulder),
        p(-half, soften),
    ]
}

/// Paints the marker at the end of the scene: a line through the rows and a
/// small tab in the tick band that can be dragged.
pub(super) fn duration_marker(ui: &Ui, layout: &TimelineLayout, duration: f64, active: bool) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let area = Rect::from_min_max(layout.ruler.left_top(), layout.rect.max);
    let x = layout.scale.x(duration).floor() + 0.5;
    if x < area.left() - 8.0 || x > area.right() + 8.0 {
        return;
    }
    let ink = if active { c.text } else { c.text_tertiary };
    let painter = ui.painter_at(area);
    let band_bottom = layout.ruler.top() + theme.size.ruler_ticks;
    painter.vline(
        x,
        band_bottom..=area.bottom(),
        Stroke::new(1.0, c.border_strong),
    );
    let tab = duration_grab(layout, duration).shrink2(Vec2::new(3.5, 1.5));
    painter.rect_filled(tab, 2.0, ink);
}

/// The area in the ruler where the end-of-scene marker can be grabbed.
pub(super) fn duration_grab(layout: &TimelineLayout, duration: f64) -> Rect {
    let x = layout.scale.x(duration).floor() + 0.5;
    Rect::from_center_size(pos2(x, layout.ruler.top() + 6.0), Vec2::new(14.0, 12.0))
}

/// The zoom controls at the right end of the header: snapping, the zoom
/// slider and fit.
pub(super) fn zoom_controls(
    ui: &mut Ui,
    layout: &TimelineLayout,
    state: &mut TimelineState,
    snapping: bool,
    events: &mut Vec<TimelineEvent>,
) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let rect = layout.zoom;
    if rect.width() < 120.0 {
        return;
    }
    // Swallow clicks so they do not scrub the playhead underneath.
    ui.interact(
        rect,
        ui.make_persistent_id("timeline-zoom-area"),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, c.surface);
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.left() + 1.0, rect.bottom())),
        0.0,
        c.border,
    );

    let mut bar = ui.new_child(
        UiBuilder::new()
            .id_salt("timeline-zoom")
            .max_rect(rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    bar.set_clip_rect(rect.intersect(ui.clip_rect()));
    bar.spacing_mut().item_spacing.x = theme.space.xs;
    bar.add_space(theme.space.md);

    if bar
        .add(
            IconButton::new(icons::ARROWS_IN_LINE_HORIZONTAL)
                .dimmed(true)
                .tooltip("Fit the scene in the timeline"),
        )
        .clicked()
    {
        state.fit_requested = true;
    }

    let mut position = f64::from(zoom_to_slider(state.points_per_second)) * 100.0;
    let slider = Slider::new(&mut position, "Timeline zoom", 0.0..=100.0)
        .step(0.0)
        .width(72.0)
        .show(&mut bar);
    if slider.changed() {
        // Zoom about the middle of the tracks.
        let old = state.points_per_second;
        let new = clamp_zoom(slider_to_zoom(position as f32 / 100.0));
        let anchor = layout.tracks.width() / 2.0 - TRACK_PAD;
        state.scroll_x = scroll_after_zoom(state.scroll_x, anchor, old, new).max(0.0);
        state.points_per_second = new;
    }

    if bar
        .add(
            IconButton::new(icons::MAGNET)
                .selected(snapping)
                .dimmed(!snapping)
                .tooltip("Snap to the time grid and to other actions"),
        )
        .clicked()
    {
        events.push(TimelineEvent::SnappingToggled {
            snapping: !snapping,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_splits_a_strip_into_before_and_after() {
        let strip = Rect::from_min_max(pos2(100.0, 0.0), pos2(500.0, 20.0));
        let parts: Vec<Rect> = outside_scene(strip, 116.0, 400.0).collect();
        assert_eq!(
            parts,
            [
                Rect::from_min_max(pos2(100.0, 0.0), pos2(116.0, 20.0)),
                Rect::from_min_max(pos2(400.0, 0.0), pos2(500.0, 20.0)),
            ]
        );
        // Scrolled so the scene covers the whole strip: nothing is outside.
        assert_eq!(outside_scene(strip, -50.0, 900.0).count(), 0);
        // The scene ended before the strip begins: all of it is after.
        let parts: Vec<Rect> = outside_scene(strip, -900.0, -100.0).collect();
        assert_eq!(parts, [strip]);
    }

    #[test]
    fn the_playhead_head_is_a_closed_shape_around_its_line() {
        let shape = playhead_shape(pos2(200.5, 10.0));
        assert_eq!(shape.len(), 7);
        let (min, max) = shape.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p.x), hi.max(p.x))
        });
        assert_eq!((min + max) / 2.0, 200.5);
        assert!(shape.iter().all(|p| p.y >= 10.0 && p.y <= 23.0));
    }
}

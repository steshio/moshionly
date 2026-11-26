//! Toasts: short messages that appear over the canvas and fade away.

use egui::{Align2, Area, Context, Id, Order, Sense, Stroke, Vec2};

use super::paint;
use crate::theme::{Theme, icons};

/// What kind of message a toast carries. It decides the icon and its colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastKind {
    /// Neutral information.
    #[default]
    Info,
    /// Something worked.
    Success,
    /// Something needs attention.
    Warning,
    /// Something failed.
    Error,
}

/// One message.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// The kind of message.
    pub kind: ToastKind,
    /// The text, ideally one short sentence.
    pub text: String,
    /// How long it stays, in seconds.
    pub duration: f64,
}

impl Toast {
    /// A neutral message shown for four seconds.
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(ToastKind::Info, text)
    }

    /// A success message.
    pub fn success(text: impl Into<String>) -> Self {
        Self::new(ToastKind::Success, text)
    }

    /// A warning.
    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(ToastKind::Warning, text)
    }

    /// An error, shown a little longer.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            duration: 7.0,
            ..Self::new(ToastKind::Error, text)
        }
    }

    fn new(kind: ToastKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            duration: 4.0,
        }
    }
}

/// The queue of toasts currently on screen.
///
/// Keep one in the app, [`push`](Self::push) messages into it from anywhere,
/// and call [`show`](Self::show) once per frame.
#[derive(Clone, Debug, Default)]
pub struct Toasts {
    /// Each toast with the time it was first shown (`None` until then).
    items: Vec<(Toast, Option<f64>)>,
}

/// Most toasts shown at once; older ones are dropped.
const MAX_VISIBLE: usize = 4;
/// Seconds a toast takes to fade out at the end of its life.
const FADE: f64 = 0.25;

impl Toasts {
    /// Adds a message.
    pub fn push(&mut self, toast: Toast) {
        self.items.push((toast, None));
        if self.items.len() > MAX_VISIBLE {
            let extra = self.items.len() - MAX_VISIBLE;
            self.items.drain(..extra);
        }
    }

    /// The messages currently queued, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Toast> {
        self.items.iter().map(|(toast, _)| toast)
    }

    /// True when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Drops the toasts whose time is up at `now` (seconds).
    pub fn expire(&mut self, now: f64) {
        self.items
            .retain(|(toast, shown)| shown.is_none_or(|start| now - start < toast.duration));
    }

    /// Draws the queue as a stack centred on `anchor`, growing upwards, and
    /// drops expired toasts. `anchor` is usually a point near the bottom of
    /// the canvas. A click on a toast dismisses it.
    pub fn show(&mut self, ctx: &Context, anchor: egui::Pos2) {
        let now = ctx.input(|i| i.time);
        self.expire(now);
        if self.items.is_empty() {
            return;
        }
        let theme = Theme::of_ctx(ctx);
        let c = &theme.color;
        let height = 36.0;
        let gap = theme.space.md;
        let mut dismissed = None;

        for (index, (toast, shown)) in self.items.iter_mut().rev().enumerate() {
            let start = *shown.get_or_insert(now);
            let remaining = toast.duration - (now - start);
            let opacity = (remaining / FADE).clamp(0.0, 1.0) as f32;
            let center = anchor - Vec2::new(0.0, index as f32 * (height + gap) + height / 2.0);

            Area::new(Id::unique(("moshionly-toast", index)))
                .order(Order::Foreground)
                .fixed_pos(center)
                .pivot(Align2::CENTER_CENTER)
                .interactable(true)
                .show(ctx, |ui| {
                    ui.set_opacity(opacity);
                    let (icon, ink) = match toast.kind {
                        ToastKind::Info => (icons::INFO, c.text_secondary),
                        ToastKind::Success => (icons::CHECK_CIRCLE, c.success),
                        ToastKind::Warning => (icons::WARNING, c.warning),
                        ToastKind::Error => (icons::X_CIRCLE, c.danger),
                    };
                    let max_text = theme.size.toast_width - 56.0;
                    let galley = paint::elided(ui, &toast.text, theme.body(), c.text, max_text);
                    let width = galley.size().x + theme.space.lg * 2.0 + theme.size.icon + 8.0;
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
                    // Screen readers announce it.
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Status, true, &toast.text)
                    });
                    paint::shadows(ui.painter(), rect, theme.radius.md, &theme.shadow.float);
                    paint::panel(
                        ui.painter(),
                        rect,
                        theme.radius.md,
                        c.raised,
                        Stroke::new(1.0, c.border),
                    );
                    icons::paint(
                        ui.painter(),
                        egui::pos2(
                            rect.left() + theme.space.lg + theme.size.icon / 2.0,
                            rect.center().y,
                        ),
                        icon,
                        theme.size.icon,
                        ink,
                    );
                    let text_rect = egui::Rect::from_min_max(
                        egui::pos2(
                            rect.left() + theme.space.lg + theme.size.icon + 8.0,
                            rect.top(),
                        ),
                        rect.max,
                    );
                    paint::galley_in(ui.painter(), text_rect, Align2::LEFT_CENTER, galley);
                    if response.clicked() {
                        dismissed = Some(index);
                    }
                });
        }

        if let Some(index) = dismissed {
            let position = self.items.len() - 1 - index;
            self.items.remove(position);
        }
        // Keep animating until the queue is empty.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_keeps_only_the_newest() {
        let mut toasts = Toasts::default();
        for n in 0..7 {
            toasts.push(Toast::info(format!("message {n}")));
        }
        let texts: Vec<_> = toasts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["message 3", "message 4", "message 5", "message 6"]);
    }

    #[test]
    fn toasts_expire_after_their_duration() {
        let mut toasts = Toasts::default();
        toasts.push(Toast::info("short"));
        toasts.push(Toast::error("long"));
        // Not shown yet: nothing expires.
        toasts.expire(1000.0);
        assert_eq!(toasts.iter().count(), 2);
        for (_, shown) in &mut toasts.items {
            *shown = Some(10.0);
        }
        toasts.expire(13.9);
        assert_eq!(toasts.iter().count(), 2);
        toasts.expire(14.1);
        assert_eq!(toasts.iter().count(), 1);
        assert_eq!(toasts.iter().next().unwrap().kind, ToastKind::Error);
        toasts.expire(17.1);
        assert!(toasts.is_empty());
    }
}

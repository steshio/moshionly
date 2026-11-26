//! How bars are drawn.

use egui::{Align2, Color32, Painter, Rect, Stroke, Ui, pos2};

use crate::{
    theme::{Theme, Tone},
    widgets::paint,
};

/// How a bar looks right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct BarLook {
    /// The bar is selected: solid fill.
    pub selected: bool,
    /// The pointer is over the bar.
    pub hovered: bool,
    /// The bar's layer is hidden or locked: drawn faded.
    pub faded: bool,
    /// The bar only shows something and cannot be resized: no grips.
    pub fixed: bool,
}

/// Paints one action bar: a rounded tile in its tone with the action's name
/// and, unless it is fixed, a grip mark at each end.
pub(super) fn paint_bar(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    label: &str,
    tone: Tone,
    look: BarLook,
) {
    let theme = Theme::of(ui);
    let t = theme.color.tone(tone);
    let fade = |color: Color32| {
        if look.faded {
            color.gamma_multiply(0.45)
        } else {
            color
        }
    };
    let (fill, outline, ink) = if look.selected {
        (t.solid, t.solid, t.on_solid)
    } else if look.hovered {
        (t.soft, t.solid, t.text)
    } else {
        (t.soft, t.outline, t.text)
    };
    paint::panel(
        painter,
        rect,
        theme.radius.sm,
        fade(fill),
        Stroke::new(1.0, fade(outline)),
    );

    // Grip marks, where there is room for them.
    let grip_inset = 5.0;
    let grips = rect.width() >= 28.0 && !look.fixed;
    if grips {
        let grip_ink = fade(if look.selected {
            t.on_solid.gamma_multiply(0.7)
        } else {
            t.outline
        });
        for x in [rect.left() + grip_inset, rect.right() - grip_inset] {
            painter.vline(
                x.floor() + 0.5,
                rect.y_range().shrink(6.0),
                Stroke::new(1.5, grip_ink),
            );
        }
    }

    // The name, when the bar is long enough to say something.
    let pad = if grips { grip_inset + 6.0 } else { 6.0 };
    let text_rect = Rect::from_min_max(
        pos2(rect.left() + pad, rect.top()),
        pos2(rect.right() - pad, rect.bottom()),
    );
    if text_rect.width() >= 24.0 {
        let galley = paint::elided(
            ui,
            label,
            theme.small_medium(),
            fade(ink),
            text_rect.width(),
        );
        // A name cut down to its ellipsis says nothing.
        if galley.size().x >= 16.0 {
            let pos = Align2::LEFT_CENTER
                .align_size_within_rect(galley.size(), text_rect)
                .min;
            painter.galley(paint::snap(pos), galley, fade(ink));
        }
    }
}

/// Paints the summary bar of a layer row: a quiet neutral span from the
/// first start to the last end of the layer's actions.
pub(super) fn paint_summary(ui: &Ui, painter: &Painter, rect: Rect, look: BarLook) {
    let theme = Theme::of(ui);
    let t = theme.color.tone(Tone::Neutral);
    let fade = if look.faded { 0.45 } else { 1.0 };
    let (fill, outline) = if look.selected {
        (theme.color.accent, theme.color.accent)
    } else if look.hovered {
        (t.soft, t.solid)
    } else {
        (t.soft, t.outline)
    };
    // Slimmer than an action bar, so the two kinds of row read differently.
    let slim = rect.shrink2(egui::vec2(0.0, 5.0));
    paint::panel(
        painter,
        slim,
        slim.height() / 2.0,
        fill.gamma_multiply(fade),
        Stroke::new(1.0, outline.gamma_multiply(fade)),
    );
}

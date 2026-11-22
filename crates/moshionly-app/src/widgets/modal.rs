//! The frame of modal dialogs.

use egui::{
    Align, Align2, Context, Frame, Id, Layout, Margin, Modal, Rect, Sense, Stroke, Ui, Vec2,
};

use super::{IconButton, paint};
use crate::theme::{Theme, corner, icons};

/// A modal dialog: a dimmed backdrop and a centred card with a title bar, a
/// body and an optional footer for buttons.
///
/// ```ignore
/// let result = ModalFrame::new("Export").width(420.0).show(ctx, |ui| {
///     ui.label("…");
///     modal_footer(ui, |ui| {
///         ui.add(TextButton::primary("Export"));
///         ui.add(TextButton::new("Cancel"));
///     });
/// });
/// if result.close_requested { open = false; }
/// ```
#[must_use = "call show()"]
pub struct ModalFrame<'a> {
    title: &'a str,
    width: f32,
    id: Id,
}

/// What a [`ModalFrame`] reports back.
pub struct ModalFrameResponse<R> {
    /// What the body closure returned.
    pub inner: R,
    /// True when the user asked to close the dialog: the close button, a
    /// click on the backdrop or Escape.
    pub close_requested: bool,
}

impl<'a> ModalFrame<'a> {
    /// A dialog titled `title`.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            width: 400.0,
            id: Id::unique(("moshionly-modal", title)),
        }
    }

    /// The width of the dialog's body.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Draws the dialog over everything else.
    pub fn show<R>(self, ctx: &Context, body: impl FnOnce(&mut Ui) -> R) -> ModalFrameResponse<R> {
        let theme = Theme::of_ctx(ctx);
        let c = &theme.color;
        let frame = Frame::new()
            .fill(c.raised)
            .stroke(Stroke::new(1.0, c.border))
            .corner_radius(corner(theme.radius.lg))
            .shadow(theme.shadow.modal)
            .inner_margin(Margin::ZERO);

        let mut close_clicked = false;
        let response = Modal::new(self.id)
            .backdrop_color(c.scrim)
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(self.width);
                ui.spacing_mut().item_spacing.y = 0.0;

                // Title bar.
                let (bar, _) = ui
                    .allocate_exact_size(Vec2::new(self.width, theme.size.top_bar), Sense::hover());
                paint::text_in(
                    ui,
                    bar.shrink2(Vec2::new(theme.space.xl, 0.0)),
                    Align2::LEFT_CENTER,
                    self.title,
                    theme.title(),
                    c.text,
                );
                let close_rect = Rect::from_center_size(
                    egui::pos2(bar.right() - theme.space.xl - 8.0, bar.center().y),
                    Vec2::splat(theme.size.control),
                );
                close_clicked = ui
                    .put(close_rect, IconButton::new(icons::X).tooltip("Close"))
                    .clicked();
                ui.painter()
                    .hline(bar.x_range(), bar.bottom(), theme.hairline());

                // Body.
                Frame::new()
                    .inner_margin(Margin::same(theme.space.xl as i8))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = theme.space.md;
                        body(ui)
                    })
                    .inner
            });

        ModalFrameResponse {
            close_requested: close_clicked || response.should_close(),
            inner: response.inner,
        }
    }
}

/// The button row at the bottom of a dialog. Buttons are laid out right to
/// left, so add the primary action first.
pub fn modal_footer(ui: &mut Ui, buttons: impl FnOnce(&mut Ui)) {
    let theme = Theme::of(ui);
    ui.add_space(theme.space.md);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = theme.space.md;
        buttons(ui);
    });
}

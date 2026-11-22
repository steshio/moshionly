//! The chat panel: where the user asks an AI agent to edit the file.
//!
//! This round only builds the frame of it: a header, an empty state and the
//! composer. The conversation itself arrives when the chat providers are
//! connected.

use egui::{
    Align, Color32, Frame, Key, Layout, Margin, Rect, RichText, Sense, Stroke, TextEdit, Ui,
    UiBuilder, Vec2, pos2,
};

use super::parts::sample_tag;
use crate::{
    theme::{Theme, icons},
    widgets::{IconButton, divider, empty_state, paint, section_header, tooltip},
};

/// What happened in the chat panel this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatOutput {
    /// The close button was clicked.
    pub close: bool,
    /// A message was sent: the text of it.
    pub sent: Option<String>,
}

/// Height of the composer block at the bottom of the panel.
const COMPOSER_HEIGHT: f32 = 116.0;

/// Draws the chat panel. `draft` is the text in the composer.
pub fn chat_panel(ui: &mut Ui, draft: &mut String) -> ChatOutput {
    let theme = Theme::of(ui);
    let mut out = ChatOutput::default();

    section_header(ui, "Chat", None, |ui| {
        out.close = ui
            .add(IconButton::new(icons::X).dimmed(true).tooltip("Close chat"))
            .clicked();
        sample_tag(ui);
    });
    divider(ui);

    let rest = ui.available_rect_before_wrap();
    let composer_top = (rest.bottom() - COMPOSER_HEIGHT).max(rest.top());
    let body = Rect::from_min_max(rest.min, pos2(rest.right(), composer_top));
    let composer = Rect::from_min_max(pos2(rest.left(), composer_top), rest.max);

    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        ui.add_space(theme.space.xxl);
        empty_state(
            ui,
            icons::SPARKLE,
            "Design with an agent",
            "Describe a change and an AI agent edits this file with you, using your own \
             subscription or API key.",
            Some("Connect a provider"),
        );
    });

    out.sent = composer_box(ui, composer, draft);
    ui.allocate_rect(rest, Sense::hover());
    out
}

/// The message box: a text area with attach and send buttons under it.
/// Returns the message when it is sent, with Enter or the send button.
fn composer_box(ui: &mut Ui, rect: Rect, draft: &mut String) -> Option<String> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let frame = rect.shrink2(Vec2::new(theme.space.lg, theme.space.lg));
    let id = ui.make_persistent_id("chat-composer");
    let focused = ui.memory(|m| m.has_focus(id));
    paint::panel(
        ui.painter(),
        frame,
        theme.radius.md,
        c.surface,
        Stroke::new(
            1.0,
            if focused {
                c.focus_ring
            } else {
                c.border_strong
            },
        ),
    );

    let inner = frame.shrink(theme.space.md);
    let buttons = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 24.0), inner.max);
    let text = Rect::from_min_max(
        pos2(inner.left() + theme.space.xs, inner.top() + theme.space.xxs),
        pos2(
            inner.right() - theme.space.xs,
            buttons.top() - theme.space.xs,
        ),
    );

    // Enter sends; Shift+Enter starts a new line.
    let enter = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));

    ui.scope_builder(UiBuilder::new().max_rect(text), |ui| {
        ui.set_clip_rect(text.intersect(ui.clip_rect()));
        ui.add(
            TextEdit::multiline(draft)
                .id(id)
                .hint_text(RichText::new("Describe a change…").color(c.text_tertiary))
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .font(theme.body())
                .text_color(c.text)
                .background_color(Color32::TRANSPARENT)
                .desired_width(text.width())
                .desired_rows(2),
        )
        .accessible_name("Message");
    });

    let mut send_clicked = false;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(buttons)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme.space.xxs;
            ui.add(
                IconButton::new(icons::PAPERCLIP)
                    .dimmed(true)
                    .tooltip("Attach an image"),
            );
            ui.add(
                IconButton::new(icons::AT)
                    .dimmed(true)
                    .tooltip("Mention a layer"),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                send_clicked = send_button(ui, !draft.trim().is_empty());
            });
        },
    );

    if (enter || send_clicked) && !draft.trim().is_empty() {
        Some(std::mem::take(draft).trim().to_owned())
    } else {
        None
    }
}

/// The round-cornered send button: solid accent when there is something to
/// send.
fn send_button(ui: &mut Ui, ready: bool) -> bool {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(theme.size.control), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ready, "Send"));
    let (fill, ink) = if !ready {
        (c.well, c.text_tertiary)
    } else if response.is_pointer_button_down_on() {
        (c.accent_pressed, c.text_on_accent)
    } else if response.hovered() {
        (c.accent_hover, c.text_on_accent)
    } else {
        (c.accent, c.text_on_accent)
    };
    ui.painter().rect_filled(rect, theme.radius.sm, fill);
    icons::paint(ui.painter(), rect.center(), icons::ARROW_UP, 14.0, ink);
    let clicked = ready && response.clicked();
    tooltip(response, "Send", Some("↵"));
    clicked
}

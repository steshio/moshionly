//! Drawing the conversation: the person's messages as bubbles, the
//! assistant's answers as formatted text with collapsed thinking, tool
//! chips that expand, pictures, and errors with a way forward.

use egui::{
    Align2, Color32, FontFamily, FontId, Rect, Response, Sense, Stroke, Ui, Vec2,
    text::{LayoutJob, TextFormat},
};
use moshionly_ai::{ImageData, StopReason};

use super::conversation::{
    AnswerState, Author, ChatError, Message, Part, ToolCall, ToolState, Usage,
};
use super::format::{self, Block, Span};
use super::help::{self, Remedy};
use super::images::ImageCache;
use crate::theme::{Theme, Tone, icons};
use crate::widgets::{ButtonKind, TextButton, paint};

/// What the person asked for in the message list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAction {
    /// Open the AI settings.
    OpenSettings,
    /// Send the last message again.
    TryAgain,
}

/// The longest tool input shown when a chip is opened, in characters.
const MAX_INPUT_CHARS: usize = 1500;

/// Draws every message, top to bottom, in the full width of `ui`.
pub fn message_list(
    ui: &mut Ui,
    messages: &[Message],
    provider_id: &str,
    images: &mut ImageCache,
) -> Option<ListAction> {
    let theme = Theme::of(ui);
    let mut action = None;
    let last = messages.len().saturating_sub(1);
    for (index, message) in messages.iter().enumerate() {
        if index > 0 {
            ui.add_space(theme.space.lg);
        }
        let found = match message.author {
            Author::Person => {
                person_message(ui, message, images);
                None
            }
            Author::Assistant => answer(ui, message, index == last, provider_id, images),
        };
        action = action.or(found);
    }
    action
}

/// The person's message: a bubble on the right.
fn person_message(ui: &mut Ui, message: &Message, images: &mut ImageCache) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let width = ui.available_width();
    let max_inner = (width * 0.85 - theme.space.lg * 2.0).max(40.0);
    let text = message.text();
    let galley = (!text.is_empty()).then(|| {
        let job = LayoutJob::simple(text, theme.body(), c.text, max_inner);
        ui.painter().layout_job(job)
    });
    let pictures: Vec<&ImageData> = message.images().collect();
    let (thumbs, thumbs_size) = thumbs(ui.ctx(), &pictures, max_inner, images);
    let text_size = galley.as_ref().map_or(Vec2::ZERO, |g| g.size());
    let gap = if thumbs_size.y > 0.0 && text_size.y > 0.0 {
        theme.space.sm
    } else {
        0.0
    };
    let inner = Vec2::new(
        text_size.x.max(thumbs_size.x),
        text_size.y + gap + thumbs_size.y,
    );
    let bubble_size = inner + Vec2::new(theme.space.lg * 2.0, theme.space.md * 2.0);
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, bubble_size.y), Sense::hover());
    let bubble = Rect::from_min_size(
        egui::pos2(row.right() - bubble_size.x, row.top()),
        bubble_size,
    );
    ui.painter()
        .rect_filled(bubble, theme.radius.lg, c.accent_soft);
    let mut cursor = bubble.min + Vec2::new(theme.space.lg, theme.space.md);
    if thumbs_size.y > 0.0 {
        paint_thumbs(ui, cursor, &thumbs);
        cursor.y += thumbs_size.y + gap;
    }
    if let Some(galley) = galley {
        let rect = Rect::from_min_size(cursor, galley.size());
        ui.place(rect, egui::Label::new(galley).selectable(true));
    }
}

/// One answer of the assistant.
fn answer(
    ui: &mut Ui,
    message: &Message,
    is_last: bool,
    provider_id: &str,
    images: &mut ImageCache,
) -> Option<ListAction> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut action = None;
    let running = message.is_running();
    let last_part = message.parts.len().saturating_sub(1);
    for (index, part) in message.parts.iter().enumerate() {
        if index > 0 {
            ui.add_space(theme.space.md);
        }
        let id = ui.make_persistent_id(("chat-part", message.id, index));
        match part {
            Part::Text(text) => formatted(ui, text),
            Part::Thinking(text) => {
                let thinking_now = running && index == last_part;
                thinking(ui, id, text, thinking_now);
            }
            Part::Tool(call) => tool_chip(ui, id, call, images),
            Part::Image(image) => thumbnail_row(ui, &[image], images),
            Part::Error(error) => {
                let name = message.provider.as_deref().unwrap_or("The AI");
                let found = error_card(ui, error, provider_id, name, is_last);
                action = action.or(found);
            }
        }
    }

    // A quiet line under the answer: working, stopped, what it used.
    let status = match message.state {
        AnswerState::Running => None,
        AnswerState::Finished(StopReason::Cancelled) => Some("Stopped".to_owned()),
        AnswerState::Finished(StopReason::MaxTokens) => {
            Some("The answer reached its length limit".to_owned())
        }
        AnswerState::Finished(StopReason::ToolLoopLimit) => {
            Some("Stopped after many tool calls in a row".to_owned())
        }
        AnswerState::Finished(StopReason::Refusal) => Some("The model declined".to_owned()),
        AnswerState::Finished(StopReason::EndTurn | StopReason::Error) => None,
    };
    let usage = message.usage.as_ref().map(usage_text);
    let footer = match (status, usage) {
        (Some(status), Some(usage)) => Some(format!("{status} · {usage}")),
        (Some(text), None) | (None, Some(text)) => Some(text),
        (None, None) => None,
    };
    if running {
        if !message.parts.is_empty() {
            ui.add_space(theme.space.md);
        }
        working(ui);
    } else if let Some(footer) = footer {
        ui.add_space(theme.space.sm);
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), theme.type_scale.caption * 1.6),
            Sense::hover(),
        );
        paint::text_in(
            ui,
            rect,
            Align2::LEFT_CENTER,
            &footer,
            theme.caption(),
            c.text_tertiary,
        );
    }
    action
}

/// A spinner and "Working…" while the answer arrives.
fn working(ui: &mut Ui) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let height = theme.size.control;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let spinner = Rect::from_center_size(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        Vec2::splat(12.0),
    );
    ui.place(spinner, egui::Spinner::new().size(12.0).color(c.accent));
    paint::text_in(
        ui,
        Rect::from_min_max(
            egui::pos2(spinner.right() + theme.space.sm, rect.top()),
            rect.max,
        ),
        Align2::LEFT_CENTER,
        "Working…",
        theme.small(),
        c.text_secondary,
    );
}

/// Answer text with its formatting, selectable for copying.
pub fn formatted(ui: &mut Ui, text: &str) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let width = ui.available_width();
    for (index, block) in format::parse(text).iter().enumerate() {
        if index > 0 {
            ui.add_space(theme.space.sm);
        }
        match block {
            Block::Paragraph(spans) => {
                text_block(ui, spans, theme.body(), 0.0, width);
            }
            Block::Heading(spans) => {
                text_block(ui, spans, theme.body_strong(), 0.0, width);
            }
            Block::Bullet { depth, spans } => {
                let indent = 14.0 + *depth as f32 * 14.0;
                let rect = text_block(ui, spans, theme.body(), indent, width);
                let dot = egui::pos2(
                    rect.left() + indent - 8.0,
                    rect.top() + theme.type_scale.body * 0.7,
                );
                ui.painter().circle_filled(dot, 1.8, c.text_secondary);
            }
            Block::Numbered { marker, spans } => {
                let indent = 20.0;
                let rect = text_block(ui, spans, theme.body(), indent, width);
                paint::text_in(
                    ui,
                    Rect::from_min_size(
                        rect.min,
                        Vec2::new(indent - 4.0, theme.type_scale.body * 1.4),
                    ),
                    Align2::RIGHT_TOP,
                    marker,
                    theme.body(),
                    c.text_secondary,
                );
            }
            Block::Code(code) => code_block(ui, code, width),
        }
    }
}

/// Lays out `spans` with a left `indent` and adds them as a selectable
/// label. Returns the whole row's rectangle.
fn text_block(ui: &mut Ui, spans: &[Span], font: FontId, indent: f32, width: f32) -> Rect {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut job = LayoutJob::default();
    job.wrap.max_width = (width - indent).max(20.0);
    let mono = FontId::new(theme.type_scale.small, FontFamily::Monospace);
    let bold = FontId::new(font.size, theme.body_strong().family);
    for span in spans {
        let (text, format) = match span {
            Span::Plain(text) => (text, TextFormat::simple(font.clone(), c.text)),
            Span::Bold(text) => (text, TextFormat::simple(bold.clone(), c.text)),
            Span::Italic(text) => (
                text,
                TextFormat {
                    italics: true,
                    ..TextFormat::simple(font.clone(), c.text)
                },
            ),
            Span::Code(text) => (
                text,
                TextFormat {
                    background: c.well,
                    expand_bg: 1.5,
                    ..TextFormat::simple(mono.clone(), c.text)
                },
            ),
        };
        job.append(text, 0.0, format);
    }
    let galley = ui.painter().layout_job(job);
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, galley.size().y), Sense::hover());
    let rect = Rect::from_min_size(row.min + Vec2::new(indent, 0.0), galley.size());
    ui.place(rect, egui::Label::new(galley).selectable(true));
    row
}

/// A fenced code block: monospaced text on a well.
fn code_block(ui: &mut Ui, code: &str, width: f32) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let pad = theme.space.md;
    let job = LayoutJob::simple(code.to_owned(), theme.mono(), c.text, width - pad * 2.0);
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(width, galley.size().y + pad * 2.0),
        Sense::hover(),
    );
    paint::panel(ui.painter(), rect, theme.radius.sm, c.well, Stroke::NONE);
    let inner = Rect::from_min_size(rect.min + Vec2::splat(pad), galley.size());
    ui.place(inner, egui::Label::new(galley).selectable(true));
}

/// The model's reasoning: a quiet disclosure row, closed until clicked.
fn thinking(ui: &mut Ui, id: egui::Id, text: &str, now: bool) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut open = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let label = if now { "Thinking…" } else { "Thought" };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.control),
        Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label}, show reasoning"))
    });
    let ink = if response.hovered() {
        c.text
    } else {
        c.text_secondary
    };
    paint::disclosure(
        ui.painter(),
        egui::pos2(rect.left() + 5.0, rect.center().y),
        open,
        ink,
    );
    paint::text_in(
        ui,
        Rect::from_min_max(egui::pos2(rect.left() + 16.0, rect.top()), rect.max),
        Align2::LEFT_CENTER,
        label,
        theme.small_medium(),
        ink,
    );
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    if open {
        let indent = 16.0;
        let width = ui.available_width() - indent;
        let job = LayoutJob::simple(
            text.trim().to_owned(),
            theme.small(),
            c.text_secondary,
            width,
        );
        let galley = ui.painter().layout_job(job);
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), galley.size().y),
            Sense::hover(),
        );
        ui.painter().vline(
            row.left() + 5.0,
            row.y_range(),
            Stroke::new(1.0, c.border_strong),
        );
        let rect = Rect::from_min_size(row.min + Vec2::new(indent, 0.0), galley.size());
        ui.place(rect, egui::Label::new(galley).selectable(true));
    }
}

/// A tool call: a chip with its state, title and summary that opens to
/// show its input, its result and its pictures.
fn tool_chip(ui: &mut Ui, id: egui::Id, call: &ToolCall, images: &mut ImageCache) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut open = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let width = ui.available_width();
    let height = theme.size.row - 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            format!(
                "{}: {} ({})",
                call.title,
                call.summary,
                state_name(call.state)
            ),
        )
    });
    let fill = if response.hovered() {
        c.well_hover
    } else {
        c.well
    };
    paint::panel(ui.painter(), rect, theme.radius.sm, fill, Stroke::NONE);

    let icon_center = egui::pos2(rect.left() + theme.space.md + 7.0, rect.center().y);
    match call.state {
        ToolState::Running => {
            ui.place(
                Rect::from_center_size(icon_center, Vec2::splat(12.0)),
                egui::Spinner::new().size(12.0).color(c.accent),
            );
        }
        ToolState::Succeeded => {
            icons::paint(
                ui.painter(),
                icon_center,
                icons::CHECK_CIRCLE,
                14.0,
                c.success,
            );
        }
        ToolState::Failed => {
            icons::paint(ui.painter(), icon_center, icons::X_CIRCLE, 14.0, c.danger);
        }
        ToolState::Interrupted => {
            icons::paint(
                ui.painter(),
                icon_center,
                icons::CIRCLE_DASHED,
                14.0,
                c.text_tertiary,
            );
        }
    }
    let caret = egui::pos2(rect.right() - theme.space.md - 4.0, rect.center().y);
    icons::paint(
        ui.painter(),
        caret,
        if open {
            icons::CARET_UP
        } else {
            icons::CARET_DOWN
        },
        10.0,
        c.text_secondary,
    );
    let text_left = icon_center.x + 7.0 + theme.space.sm;
    let title = paint::line(ui, &call.title, theme.body_medium(), c.text);
    let title_width = title.size().x;
    ui.painter().galley(
        paint::snap(egui::pos2(
            text_left,
            rect.center().y - title.size().y / 2.0,
        )),
        title,
        c.text,
    );
    let summary_left = text_left + title_width + theme.space.sm;
    let summary_room = caret.x - 10.0 - summary_left;
    if !call.summary.is_empty() && summary_room > 24.0 {
        let summary = paint::elided(
            ui,
            &call.summary,
            theme.body(),
            c.text_secondary,
            summary_room,
        );
        ui.painter().galley(
            paint::snap(egui::pos2(
                summary_left,
                rect.center().y - summary.size().y / 2.0,
            )),
            summary,
            c.text_secondary,
        );
    }
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }

    if open {
        ui.add_space(theme.space.xs);
        detail_line(ui, "Tool", &call.name, true);
        let input = pretty_input(&call.input);
        if !input.is_empty() {
            detail_line(ui, "Input", &input, true);
        }
        let result = call.result.clone().unwrap_or_else(|| {
            match call.state {
                ToolState::Running => "Running…",
                ToolState::Interrupted => "Stopped before it finished",
                ToolState::Succeeded | ToolState::Failed => "No details",
            }
            .to_owned()
        });
        detail_line(ui, "Result", &result, false);
    }
    if !call.images.is_empty() {
        ui.add_space(theme.space.sm);
        let pictures: Vec<&ImageData> = call.images.iter().collect();
        thumbnail_row(ui, &pictures, images);
    }
}

/// One labelled line in an opened tool chip.
fn detail_line(ui: &mut Ui, label: &str, value: &str, mono: bool) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let label_width = 44.0;
    let indent = theme.space.md;
    let width = ui.available_width() - indent - label_width;
    let font = if mono { theme.mono() } else { theme.small() };
    let job = LayoutJob::simple(value.to_owned(), font, c.text_secondary, width.max(40.0));
    let galley = ui.painter().layout_job(job);
    let height = galley.size().y.max(theme.type_scale.small * 1.5) + theme.space.xs;
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    paint::text_in(
        ui,
        Rect::from_min_size(
            row.min + Vec2::new(indent, 0.0),
            Vec2::new(label_width, theme.type_scale.small * 1.4),
        ),
        Align2::LEFT_CENTER,
        label,
        theme.small_medium(),
        c.text_tertiary,
    );
    let rect = Rect::from_min_size(
        row.min + Vec2::new(indent + label_width, 1.0),
        galley.size(),
    );
    ui.place(rect, egui::Label::new(galley).selectable(true));
}

/// A tool's input as indented JSON, cut short when long.
fn pretty_input(input: &serde_json::Value) -> String {
    if input.as_object().is_some_and(serde_json::Map::is_empty) || input.is_null() {
        return String::new();
    }
    let text = serde_json::to_string_pretty(input).unwrap_or_default();
    if text.chars().count() <= MAX_INPUT_CHARS {
        return text;
    }
    let mut short: String = text.chars().take(MAX_INPUT_CHARS).collect();
    short.push_str("\n…");
    short
}

fn state_name(state: ToolState) -> &'static str {
    match state {
        ToolState::Running => "running",
        ToolState::Succeeded => "done",
        ToolState::Failed => "failed",
        ToolState::Interrupted => "stopped",
    }
}

/// An error inside an answer: what happened, what to do, and buttons for
/// it. Only the newest answer offers the buttons.
fn error_card(
    ui: &mut Ui,
    error: &ChatError,
    provider_id: &str,
    provider_name: &str,
    actionable: bool,
) -> Option<ListAction> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let tone = c.tone(Tone::Rose);
    let help = help::help(error.kind, provider_id, provider_name);
    let width = ui.available_width();
    let pad = theme.space.lg;
    let text_left = pad + theme.size.icon + theme.space.md;
    let text_width = (width - text_left - pad).max(40.0);

    let title = ui.painter().layout_job(LayoutJob::simple(
        help.title.clone(),
        theme.body_strong(),
        c.text,
        text_width,
    ));
    let mut advice_job = LayoutJob::default();
    advice_job.wrap.max_width = text_width;
    for span in format::spans(&help.advice) {
        let (text, format) = match span {
            Span::Code(text) => (
                text,
                TextFormat {
                    background: c.surface,
                    ..TextFormat::simple(theme.mono(), c.text)
                },
            ),
            Span::Plain(text) | Span::Bold(text) | Span::Italic(text) => {
                (text, TextFormat::simple(theme.body(), c.text))
            }
        };
        advice_job.append(&text, 0.0, format);
    }
    let advice = ui.painter().layout_job(advice_job);
    let detail = ui.painter().layout_job(LayoutJob::simple(
        error.message.clone(),
        theme.small(),
        c.text_secondary,
        text_width,
    ));
    let buttons = actionable && !help.remedies.is_empty();
    let buttons_height = if buttons {
        theme.size.control + theme.space.md
    } else {
        0.0
    };
    let height = pad * 2.0
        + title.size().y
        + theme.space.xs
        + advice.size().y
        + theme.space.xs
        + detail.size().y
        + buttons_height;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    paint::panel(
        ui.painter(),
        rect,
        theme.radius.md,
        tone.soft,
        Stroke::new(1.0, tone.outline),
    );
    icons::paint(
        ui.painter(),
        egui::pos2(
            rect.left() + pad + theme.size.icon / 2.0,
            rect.top() + pad + title.size().y / 2.0,
        ),
        icons::WARNING,
        theme.size.icon,
        tone.text,
    );
    let mut y = rect.top() + pad;
    let x = rect.left() + text_left;
    let title_height = title.size().y;
    ui.painter().galley(egui::pos2(x, y), title, c.text);
    y += title_height + theme.space.xs;
    let advice_rect = Rect::from_min_size(egui::pos2(x, y), advice.size());
    y += advice.size().y + theme.space.xs;
    ui.place(advice_rect, egui::Label::new(advice).selectable(true));
    let detail_rect = Rect::from_min_size(egui::pos2(x, y), detail.size());
    y += detail.size().y;
    ui.place(detail_rect, egui::Label::new(detail).selectable(true));

    let mut action = None;
    if buttons {
        let row = Rect::from_min_max(
            egui::pos2(x, y + theme.space.md),
            egui::pos2(rect.right() - pad, y + theme.space.md + theme.size.control),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = theme.space.sm;
        for (index, remedy) in help.remedies.iter().enumerate() {
            let kind = if index == 0 {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            };
            let button = TextButton::new(help::remedy_label(*remedy)).kind(kind);
            if child.add(button).clicked() {
                action = Some(match remedy {
                    Remedy::OpenSettings => ListAction::OpenSettings,
                    Remedy::TryAgain => ListAction::TryAgain,
                });
            }
        }
    }
    action
}

/// The tallest a thumbnail is drawn, in points.
const THUMB_HEIGHT: f32 = 160.0;
/// The space between thumbnails, in points.
const THUMB_GAP: f32 = 6.0;
/// The side of the stand-in for a picture that cannot be read.
const THUMB_MISSING: f32 = 48.0;

/// A picture ready to draw at its thumbnail size.
struct Thumb {
    texture: Option<egui::TextureHandle>,
    size: Vec2,
}

/// The thumbnails of `pictures` in a row no wider than `max_width`, and the
/// row's size.
fn thumbs(
    ctx: &egui::Context,
    pictures: &[&ImageData],
    max_width: f32,
    images: &mut ImageCache,
) -> (Vec<Thumb>, Vec2) {
    let textures: Vec<Option<egui::TextureHandle>> = pictures
        .iter()
        .map(|picture| images.texture(ctx, picture))
        .collect();
    let natural: Vec<Option<Vec2>> = textures
        .iter()
        .map(|texture| texture.as_ref().map(egui::TextureHandle::size_vec2))
        .collect();
    let sizes = thumbnail_sizes(&natural, max_width);
    let row = Vec2::new(
        sizes.iter().map(|size| size.x).sum::<f32>()
            + THUMB_GAP * sizes.len().saturating_sub(1) as f32,
        sizes.iter().map(|size| size.y).fold(0.0, f32::max),
    );
    let thumbs = textures
        .into_iter()
        .zip(sizes)
        .map(|(texture, size)| Thumb { texture, size })
        .collect();
    (thumbs, row)
}

/// Each picture's thumbnail size, keeping its shape: no taller than
/// [`THUMB_HEIGHT`], never enlarged, and all of them side by side within
/// `max_width`. `None` stands for a picture that cannot be read.
fn thumbnail_sizes(natural: &[Option<Vec2>], max_width: f32) -> Vec<Vec2> {
    let count = natural.len().max(1) as f32;
    let each = ((max_width - THUMB_GAP * (count - 1.0)) / count).max(16.0);
    natural
        .iter()
        .map(|size| match size {
            Some(size) if size.x > 0.0 && size.y > 0.0 => {
                let scale = (each / size.x).min(THUMB_HEIGHT / size.y).min(1.0);
                *size * scale
            }
            _ => Vec2::splat(THUMB_MISSING.min(each)),
        })
        .collect()
}

/// Draws thumbnails left to right from `origin`.
fn paint_thumbs(ui: &mut Ui, origin: egui::Pos2, thumbs: &[Thumb]) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut x = origin.x;
    for thumb in thumbs {
        let rect = Rect::from_min_size(egui::pos2(x, origin.y), thumb.size);
        match &thumb.texture {
            Some(texture) => {
                ui.place(
                    rect,
                    egui::Image::from_texture((texture.id(), thumb.size))
                        .corner_radius(crate::theme::corner(theme.radius.sm)),
                );
            }
            None => {
                ui.painter().rect_filled(rect, theme.radius.sm, c.well);
                icons::paint(
                    ui.painter(),
                    rect.center(),
                    icons::IMAGE,
                    20.0,
                    c.text_tertiary,
                );
            }
        }
        x += thumb.size.x + THUMB_GAP;
    }
}

/// Allocates a row of thumbnails in `ui` and draws them.
fn thumbnail_row(ui: &mut Ui, pictures: &[&ImageData], images: &mut ImageCache) {
    let (thumbs, size) = thumbs(ui.ctx(), pictures, ui.available_width(), images);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    paint_thumbs(ui, rect.min, &thumbs);
}

/// Token counts and cost in a few characters, such as
/// `1.2k in · 340 out · $0.012`.
pub fn usage_text(usage: &Usage) -> String {
    let mut text = format!(
        "{} in · {} out",
        tokens(usage.input_tokens),
        tokens(usage.output_tokens)
    );
    if let Some(cost) = usage.cost_usd {
        text.push_str(&format!(" · {}", dollars(cost)));
    }
    text
}

fn tokens(count: u64) -> String {
    match count {
        0..1_000 => count.to_string(),
        1_000..1_000_000 => trim_zero(format!("{:.1}", count as f64 / 1_000.0)) + "k",
        _ => trim_zero(format!("{:.1}", count as f64 / 1_000_000.0)) + "M",
    }
}

fn trim_zero(text: String) -> String {
    text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
}

fn dollars(cost: f64) -> String {
    if cost < 0.1 {
        format!("${cost:.3}")
    } else {
        format!("${cost:.2}")
    }
}

/// A clickable card holding one suggested prompt, as wide as `ui`.
pub fn suggestion(ui: &mut Ui, text: &str) -> Response {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let width = ui.available_width();
    let pad = Vec2::new(theme.space.lg, theme.space.md);
    let icon_room = theme.size.icon + theme.space.sm;
    let job = LayoutJob::simple(
        text.to_owned(),
        theme.body(),
        c.text,
        width - pad.x * 2.0 - icon_room,
    );
    let galley = ui.painter().layout_job(job);
    let height = galley.size().y.max(theme.size.icon) + pad.y * 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let fill = if response.is_pointer_button_down_on() {
        c.well_pressed
    } else if response.hovered() {
        c.well_hover
    } else {
        Color32::TRANSPARENT
    };
    paint::panel(
        ui.painter(),
        rect,
        theme.radius.md,
        fill,
        Stroke::new(1.0, c.border),
    );
    icons::paint(
        ui.painter(),
        egui::pos2(
            rect.left() + pad.x + theme.size.icon / 2.0,
            rect.top() + pad.y + theme.type_scale.body * 0.7,
        ),
        icons::SPARKLE,
        theme.size.icon - 2.0,
        c.accent_text,
    );
    ui.painter()
        .galley(rect.min + pad + Vec2::new(icon_room, 0.0), galley, c.text);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_reads_in_a_few_characters() {
        let usage = Usage {
            input_tokens: 1234,
            output_tokens: 340,
            cost_usd: Some(0.0123),
        };
        assert_eq!(usage_text(&usage), "1.2k in · 340 out · $0.012");
        let usage = Usage {
            input_tokens: 2_000_000,
            output_tokens: 15_000,
            cost_usd: Some(1.5),
        };
        assert_eq!(usage_text(&usage), "2M in · 15k out · $1.50");
        let usage = Usage {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: None,
        };
        assert_eq!(usage_text(&usage), "0 in · 0 out");
    }

    #[test]
    fn long_tool_input_is_cut_short() {
        let input = serde_json::json!({ "text": "x".repeat(5000) });
        let shown = pretty_input(&input);
        assert!(shown.chars().count() <= MAX_INPUT_CHARS + 2);
        assert!(shown.ends_with('…'));
        assert_eq!(pretty_input(&serde_json::json!({})), "");
    }

    #[test]
    fn thumbnails_keep_their_shape_and_fit_the_width() {
        let wide = Some(Vec2::new(800.0, 400.0));
        assert_eq!(
            thumbnail_sizes(&[wide], 1000.0),
            vec![Vec2::new(320.0, 160.0)]
        );
        assert_eq!(
            thumbnail_sizes(&[wide], 200.0),
            vec![Vec2::new(200.0, 100.0)]
        );
        let two = thumbnail_sizes(&[wide, wide], 206.0);
        assert_eq!(two, vec![Vec2::new(100.0, 50.0); 2]);
        // Small pictures are not enlarged; unreadable ones get a stand-in.
        let small = Some(Vec2::new(40.0, 30.0));
        assert_eq!(
            thumbnail_sizes(&[small], 300.0),
            vec![Vec2::new(40.0, 30.0)]
        );
        assert_eq!(
            thumbnail_sizes(&[None], 300.0),
            vec![Vec2::splat(THUMB_MISSING)]
        );
        assert!(thumbnail_sizes(&[], 300.0).is_empty());
    }
}

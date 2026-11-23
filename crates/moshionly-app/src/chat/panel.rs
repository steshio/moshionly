//! The panel's frame: the header with the AI picker, the conversation or
//! an empty state, and the composer.
//!
//! Drawing never changes the chat. Everything the person does comes back
//! as [`PanelAction`]s, which [`Chat`](super::Chat) carries out after the
//! frame is drawn.

use egui::{
    Align, Align2, Color32, Event, Frame, InputState, Key, Layout, Margin, Popup,
    PopupCloseBehavior, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder, Vec2,
    pos2,
};
use moshionly_ai::{ImageData, ProviderStatus, ToolTransport};

use super::controller::{Controller, ModelList};
use super::images::ImageCache;
use super::messages::{self, ListAction};
use crate::theme::{Theme, icons};
use crate::widgets::{
    IconButton, MenuItem, divider, empty_state, menu_heading, menu_separator, paint, popup_frame,
    tooltip,
};

/// Something the person did in the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelAction {
    /// Close the panel.
    Close,
    /// Start a new conversation.
    NewChat,
    /// Open the AI settings.
    OpenSettings,
    /// Send this text with the attached pictures.
    Send(String),
    /// Stop the answer that is arriving.
    Stop,
    /// Send the last message again.
    TryAgain,
    /// Answer with this provider from now on.
    SelectProvider(String),
    /// Use this model of the selected provider (`None` for its default).
    SelectModel(Option<String>),
    /// Pick pictures to attach.
    Attach,
    /// Remove the attached picture at this position.
    RemoveAttachment(usize),
}

/// What the panel needs to know beyond the controller.
pub struct PanelView<'a> {
    /// The text being written.
    pub draft: &'a mut String,
    /// Pictures attached to the next message.
    pub attachments: &'a [ImageData],
    /// Whether pictures can be attached at all (a file dialog is there).
    pub can_attach: bool,
    /// Whether the app's MCP server is running, for agent CLIs.
    pub mcp_running: bool,
    /// Whether layers are selected, which changes the suggestions.
    pub has_selection: bool,
    /// A short note to show above the composer, such as a picture that
    /// could not be attached.
    pub notice: Option<&'a str>,
}

/// The fewest lines the composer shows.
const COMPOSER_MIN_ROWS: usize = 2;
/// The most lines the composer grows to before it scrolls.
const COMPOSER_MAX_ROWS: usize = 8;

/// Draws the whole panel into `ui`'s available rectangle.
pub fn panel(
    ui: &mut Ui,
    controller: &Controller,
    view: PanelView<'_>,
    images: &mut ImageCache,
) -> Vec<PanelAction> {
    let mut actions = Vec::new();
    header(ui, controller, &mut actions);
    divider(ui);

    let rest = ui.available_rect_before_wrap();
    let composer_height = composer_height(ui, view.draft, &view);
    let composer_top = (rest.bottom() - composer_height).max(rest.top());
    let body = Rect::from_min_max(rest.min, pos2(rest.right(), composer_top));
    let composer = Rect::from_min_max(pos2(rest.left(), composer_top), rest.max);

    ui.scope_builder(UiBuilder::new().max_rect(body).id_salt("chat-body"), |ui| {
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        body_contents(ui, controller, &view, images, &mut actions);
    });
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(composer)
            .id_salt("chat-composer-area"),
        |ui| {
            composer_box(ui, composer, controller, view, images, &mut actions);
        },
    );
    ui.allocate_rect(rest, Sense::hover());
    actions
}

/// The header: the AI picker on the left, new chat, settings and close on
/// the right.
fn header(ui: &mut Ui, controller: &Controller, actions: &mut Vec<PanelAction>) {
    let theme = Theme::of(ui);
    let pad = theme.space.lg;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.section_header),
        Sense::hover(),
    );
    let buttons_rect = Rect::from_min_max(
        pos2(rect.left() + pad, rect.top()),
        pos2(rect.right() - pad + theme.space.xs, rect.bottom()),
    );
    let mut buttons = ui.new_child(
        UiBuilder::new()
            .max_rect(buttons_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = theme.space.xxs;
    if buttons
        .add(IconButton::new(icons::X).dimmed(true).tooltip("Close chat"))
        .clicked()
    {
        actions.push(PanelAction::Close);
    }
    if buttons
        .add(
            IconButton::new(icons::GEAR_SIX)
                .dimmed(true)
                .tooltip("AI settings"),
        )
        .clicked()
    {
        actions.push(PanelAction::OpenSettings);
    }
    let can_start_over = !controller.conversation().is_empty();
    let new_chat = buttons.add_enabled(
        can_start_over,
        IconButton::new(icons::PLUS)
            .dimmed(true)
            .tooltip("New chat"),
    );
    if new_chat.clicked() {
        actions.push(PanelAction::NewChat);
    }
    let picker_right = buttons.min_rect().left() - theme.space.sm;
    let picker_rect = Rect::from_min_max(
        pos2(
            rect.left() + pad - theme.space.sm,
            rect.center().y - theme.size.control / 2.0,
        ),
        pos2(picker_right, rect.center().y + theme.size.control / 2.0),
    );
    picker(ui, picker_rect, controller, actions);
}

/// The AI picker: the provider and model in use, opening a menu of the
/// providers that are ready, the provider's models and the settings.
fn picker(ui: &mut Ui, rect: Rect, controller: &Controller, actions: &mut Vec<PanelAction>) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let selected = controller.selected();
    let label = match selected {
        Some(status) => match model_label(controller, status) {
            Some(model) => format!("{} · {model}", status.info.name),
            None => status.info.name.clone(),
        },
        None if controller.statuses().is_none() => "Looking for your AI…".to_owned(),
        None => "Choose an AI".to_owned(),
    };
    let id = ui.make_persistent_id("chat-ai-picker");
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, format!("AI: {label}"))
    });
    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));
    let fill = if open || response.is_pointer_button_down_on() {
        c.well_pressed
    } else if response.hovered() {
        c.well_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme.radius.sm, fill);
    let caret_x = rect.left() + theme.space.sm;
    let text = paint::elided(
        ui,
        &label,
        theme.body_strong(),
        c.text,
        (rect.width() - theme.space.sm * 2.0 - 16.0).max(10.0),
    );
    let text_width = text.size().x;
    ui.painter().galley(
        paint::snap(pos2(caret_x, rect.center().y - text.size().y / 2.0)),
        text,
        c.text,
    );
    icons::paint(
        ui.painter(),
        pos2(caret_x + text_width + 10.0, rect.center().y),
        icons::CARET_DOWN,
        10.0,
        c.text_secondary,
    );

    Popup::menu(&response)
        .frame(popup_frame(ui))
        .gap(theme.space.xs)
        .width(theme.size.menu_width)
        .close_behavior(PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
            ui.set_min_width(theme.size.menu_width - theme.space.xs * 2.0);
            picker_menu(ui, controller, actions);
        });
}

fn picker_menu(ui: &mut Ui, controller: &Controller, actions: &mut Vec<PanelAction>) {
    let selected_id = controller.selected().map(|status| status.info.id.clone());
    menu_heading(ui, "AI");
    let usable: Vec<&ProviderStatus> = controller
        .statuses()
        .unwrap_or_default()
        .iter()
        .filter(|status| status.is_usable())
        .collect();
    if usable.is_empty() {
        let note = if controller.statuses().is_none() {
            "Checking…"
        } else {
            "No AI is ready yet"
        };
        ui.add_enabled(false, MenuItem::new(note));
    }
    for status in usable {
        let checked = selected_id.as_deref() == Some(status.info.id.as_str());
        if ui
            .add(MenuItem::new(&status.info.name).checked(checked))
            .clicked()
            && !checked
        {
            actions.push(PanelAction::SelectProvider(status.info.id.clone()));
        }
    }

    if let Some(id) = &selected_id {
        menu_separator(ui);
        menu_heading(ui, "Model");
        let chosen = controller
            .settings()
            .provider(id)
            .model()
            .map(str::to_owned);
        if ui
            .add(MenuItem::new("Default").checked(chosen.is_none()))
            .clicked()
            && chosen.is_some()
        {
            actions.push(PanelAction::SelectModel(None));
        }
        match controller.models(id) {
            Some(ModelList::Loaded(models)) => {
                for model in models {
                    let checked = chosen.as_deref() == Some(model.id.as_str());
                    if ui
                        .add(MenuItem::new(&model.name).checked(checked))
                        .clicked()
                        && !checked
                    {
                        actions.push(PanelAction::SelectModel(Some(model.id.clone())));
                    }
                }
            }
            Some(ModelList::Failed(_)) => {
                ui.add_enabled(false, MenuItem::new("Models could not be listed"));
            }
            Some(ModelList::Loading) | None => {
                ui.add_enabled(false, MenuItem::new("Loading models…"));
            }
        }
    }

    menu_separator(ui);
    if ui
        .add(MenuItem::new("AI settings…").icon(icons::GEAR_SIX))
        .clicked()
    {
        actions.push(PanelAction::OpenSettings);
    }
}

/// The name of the model the selected provider will use, when one is
/// known: the one chosen in the settings, else the provider's first.
fn model_label(controller: &Controller, status: &ProviderStatus) -> Option<String> {
    let chosen = controller.settings().provider(&status.info.id).model();
    let models = match controller.models(&status.info.id) {
        Some(ModelList::Loaded(models)) => models.as_slice(),
        _ => &[],
    };
    match chosen {
        Some(chosen) => Some(
            models
                .iter()
                .find(|model| model.id == chosen)
                .map_or_else(|| chosen.to_owned(), |model| model.name.clone()),
        ),
        None => models.first().map(|model| model.name.clone()),
    }
}

/// The conversation, or what to show before there is one.
fn body_contents(
    ui: &mut Ui,
    controller: &Controller,
    view: &PanelView<'_>,
    images: &mut ImageCache,
    actions: &mut Vec<PanelAction>,
) {
    let theme = Theme::of(ui);
    let conversation = controller.conversation();
    if conversation.is_empty() {
        ui.add_space(theme.space.xl);
        empty_body(ui, controller, view, actions);
        return;
    }
    let provider_id = controller
        .selected()
        .map(|status| status.info.id.clone())
        .unwrap_or_default();
    ScrollArea::vertical()
        .id_salt("chat-messages")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            let pad = theme.space.lg;
            ui.add_space(theme.space.lg);
            let width = ui.available_width() - pad * 2.0;
            let (top, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 0.0), Sense::hover());
            let mut inner = ui.new_child(
                UiBuilder::new()
                    .max_rect(Rect::from_min_size(
                        top.min + Vec2::new(pad, 0.0),
                        Vec2::new(width, f32::INFINITY),
                    ))
                    .layout(Layout::top_down(Align::Min)),
            );
            inner.spacing_mut().item_spacing.y = 0.0;
            let action =
                messages::message_list(&mut inner, conversation.messages(), &provider_id, images);
            let used = inner.min_rect();
            ui.allocate_rect(
                Rect::from_min_max(top.min, pos2(top.right(), used.bottom() + theme.space.lg)),
                Sense::hover(),
            );
            match action {
                Some(ListAction::OpenSettings) => actions.push(PanelAction::OpenSettings),
                Some(ListAction::TryAgain) => actions.push(PanelAction::TryAgain),
                None => {}
            }
        });
}

/// Before the first message: a welcome with suggestions, a note that the
/// providers are being checked, or a way to set one up.
fn empty_body(
    ui: &mut Ui,
    controller: &Controller,
    view: &PanelView<'_>,
    actions: &mut Vec<PanelAction>,
) {
    let theme = Theme::of(ui);
    if let Some(problem) = controller.runtime_problem() {
        empty_state(
            ui,
            icons::WARNING,
            "The chat could not start",
            &format!("Moshionly could not start the chat's background work: {problem}"),
            None,
        );
        return;
    }
    let Some(status) = controller.selected() else {
        if controller.statuses().is_none() {
            empty_state(
                ui,
                icons::SPARKLE,
                "Looking for your AI",
                "Checking which assistants are installed on this computer and which keys are set.",
                None,
            );
        } else {
            let set_up = empty_state(
                ui,
                icons::ROBOT,
                "Choose your AI",
                "Moshionly works with an AI you already use: an assistant installed on this \
                 computer, such as Claude Code, or an API key.",
                Some("Set up AI"),
            );
            if set_up.is_some_and(|response| response.clicked()) {
                actions.push(PanelAction::OpenSettings);
            }
        }
        return;
    };

    empty_state(
        ui,
        icons::SPARKLE,
        "Design with AI",
        &format!(
            "Describe a change and {} makes it in this file. You can undo anything it does.",
            status.info.name
        ),
        None,
    );
    ui.add_space(theme.space.md);
    let pad = theme.space.xl;
    let area = ui.available_rect_before_wrap();
    let mut list = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(area.left() + pad, area.top()),
                pos2(area.right() - pad, area.bottom()),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    list.spacing_mut().item_spacing.y = theme.space.sm;
    for prompt in suggestions(view.has_selection) {
        if messages::suggestion(&mut list, prompt).clicked() {
            actions.push(PanelAction::Send(prompt.to_owned()));
        }
    }
}

/// Prompts that show what the chat can do.
pub fn suggestions(has_selection: bool) -> [&'static str; 3] {
    if has_selection {
        [
            "Make the selected layers fade and slide in, one after another",
            "Give the selection a gentle bounce when it appears",
            "Describe the selected layers and their animations",
        ]
    } else {
        [
            "Describe what is on this artboard",
            "Make every layer fade in, one after another",
            "Suggest a smoother way to animate this scene",
        ]
    }
}

/// How tall the composer is for `draft`: two lines at least, eight at most.
fn composer_height(ui: &Ui, draft: &str, view: &PanelView<'_>) -> f32 {
    let theme = Theme::of(ui);
    let line = theme.type_scale.body * 1.4;
    // A trailing line break counts: the caret sits on the new, empty line.
    let rows = draft
        .split('\n')
        .count()
        .clamp(COMPOSER_MIN_ROWS, COMPOSER_MAX_ROWS) as f32;
    let mut height =
        theme.space.lg * 2.0 + theme.space.md * 2.0 + rows * line + 24.0 + theme.space.xs;
    if !view.attachments.is_empty() {
        height += 48.0 + theme.space.sm;
    }
    if view.notice.is_some() {
        height += theme.size.control;
    }
    height
}

/// The message box: pictures to send, the text, and the attach and send
/// (or stop) buttons.
fn composer_box(
    ui: &mut Ui,
    rect: Rect,
    controller: &Controller,
    view: PanelView<'_>,
    images: &mut ImageCache,
    actions: &mut Vec<PanelAction>,
) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let busy = controller.is_busy();
    let selected = controller.selected();
    let mut frame = rect.shrink2(Vec2::new(theme.space.lg, theme.space.lg));
    frame.min.y = rect.top() + theme.space.xs;
    frame.max.y = rect.bottom() - theme.space.lg;

    // An agent CLI reaches the tools only through the app's MCP server.
    let mcp_note = selected.and_then(|status| {
        (status.capabilities.tools == ToolTransport::Mcp && !view.mcp_running).then(|| {
            format!(
                "{} can talk but cannot edit this file until Moshionly's agent server is running.",
                status.info.name
            )
        })
    });
    // A note above the box, such as a picture that could not be attached.
    if let Some(note) = view.notice {
        let line = Rect::from_min_size(frame.min, Vec2::new(frame.width(), theme.size.control));
        notice(ui, line, note);
        frame.min.y += theme.size.control;
    }

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
    let mut inner = frame.shrink(theme.space.md);

    if !view.attachments.is_empty() {
        let strip = Rect::from_min_size(inner.min, Vec2::new(inner.width(), 48.0));
        attachment_strip(ui, strip, view.attachments, busy, images, actions);
        inner.min.y += 48.0 + theme.space.sm;
    }

    let buttons = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 24.0), inner.max);
    let text = Rect::from_min_max(
        pos2(inner.left() + theme.space.xs, inner.top() + theme.space.xxs),
        pos2(
            inner.right() - theme.space.xs,
            buttons.top() - theme.space.xs,
        ),
    );

    // Enter sends; Shift+Enter starts a new line.
    let can_send = !busy && selected.is_some();
    let enter = focused && can_send && ui.input_mut(take_plain_enter);
    let hint = match (selected, busy) {
        (_, true) => "Wait for the answer, or stop it".to_owned(),
        (Some(status), false) => format!("Ask {} to change the design…", status.info.name),
        (None, false) => "Set up an AI to start".to_owned(),
    };
    ui.scope_builder(UiBuilder::new().max_rect(text), |ui| {
        ui.set_clip_rect(text.intersect(ui.clip_rect()));
        ScrollArea::vertical()
            .id_salt("chat-composer-scroll")
            .auto_shrink([false, false])
            .max_height(text.height())
            .show(ui, |ui| {
                ui.add_enabled(
                    !busy,
                    TextEdit::multiline(view.draft)
                        .id(id)
                        .hint_text(RichText::new(hint).color(c.text_tertiary))
                        .frame(Frame::NONE)
                        .margin(Margin::ZERO)
                        .font(theme.body())
                        .text_color(c.text)
                        .background_color(Color32::TRANSPARENT)
                        .desired_width(text.width())
                        .desired_rows(COMPOSER_MIN_ROWS),
                )
                .accessible_name("Message");
            });
    });

    let mut send_clicked = false;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(buttons)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme.space.xxs;
            if view.can_attach {
                let images_ok = selected.is_none_or(|status| status.capabilities.images);
                let tip = match selected {
                    Some(status) if !images_ok => {
                        format!("{} cannot be sent pictures", status.info.name)
                    }
                    _ => "Attach a picture".to_owned(),
                };
                let attach = ui.add_enabled(
                    !busy && images_ok,
                    IconButton::new(icons::PAPERCLIP).dimmed(true).tooltip(&tip),
                );
                if attach.clicked() {
                    actions.push(PanelAction::Attach);
                }
            }
            if let Some(note) = &mcp_note {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(theme.size.control), Sense::hover());
                icons::paint(ui.painter(), rect.center(), icons::WARNING, 14.0, c.warning);
                tooltip(response, note, None);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if busy {
                    if stop_button(ui, controller.is_stopping()) {
                        actions.push(PanelAction::Stop);
                    }
                } else {
                    let ready =
                        can_send && (!view.draft.trim().is_empty() || !view.attachments.is_empty());
                    send_clicked = send_button(ui, ready);
                }
            });
        },
    );

    let has_content = !view.draft.trim().is_empty() || !view.attachments.is_empty();
    if (enter || send_clicked) && can_send && has_content {
        actions.push(PanelAction::Send(
            std::mem::take(view.draft).trim().to_owned(),
        ));
    }
}

/// Removes a plain Enter press from this frame's input and says whether
/// there was one. Enter with Shift (or another modifier) is left for the
/// text box, which starts a new line.
fn take_plain_enter(input: &mut InputState) -> bool {
    let mut found = false;
    input.events.retain(|event| {
        let plain_enter = matches!(
            event,
            Event::Key { key: Key::Enter, pressed: true, modifiers, .. }
                if !modifiers.shift && !modifiers.alt && !modifiers.ctrl && !modifiers.command
        );
        found |= plain_enter;
        !plain_enter
    });
    found
}

/// A quiet line with a warning icon.
fn notice(ui: &mut Ui, rect: Rect, text: &str) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    icons::paint(
        ui.painter(),
        pos2(rect.left() + 7.0, rect.center().y),
        icons::WARNING,
        14.0,
        c.warning,
    );
    let text_rect = Rect::from_min_max(pos2(rect.left() + 20.0, rect.top()), rect.max);
    let galley = paint::elided(ui, text, theme.small(), c.text_secondary, text_rect.width());
    let response = ui.interact(rect, ui.make_persistent_id("chat-notice"), Sense::hover());
    paint::galley_in(ui.painter(), text_rect, Align2::LEFT_CENTER, galley);
    tooltip(response, text, None);
}

/// The pictures waiting to be sent, each with a remove button.
fn attachment_strip(
    ui: &mut Ui,
    rect: Rect,
    attachments: &[ImageData],
    busy: bool,
    images: &mut ImageCache,
    actions: &mut Vec<PanelAction>,
) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let side = rect.height();
    for (index, picture) in attachments.iter().enumerate() {
        let cell = Rect::from_min_size(
            rect.min + Vec2::new(index as f32 * (side + theme.space.sm), 0.0),
            Vec2::splat(side),
        );
        if cell.right() > rect.right() {
            break;
        }
        ui.painter().rect_filled(cell, theme.radius.sm, c.well);
        match images.texture(ui.ctx(), picture) {
            Some(texture) => {
                let size = texture.size_vec2();
                let scale = (side / size.x).max(side / size.y);
                let uv = Rect::from_center_size(
                    pos2(0.5, 0.5),
                    Vec2::new(side / (size.x * scale), side / (size.y * scale)),
                );
                ui.painter().add(
                    egui::epaint::RectShape::filled(
                        cell,
                        crate::theme::corner(theme.radius.sm),
                        Color32::WHITE,
                    )
                    .with_texture(texture.id(), uv),
                );
            }
            None => icons::paint(
                ui.painter(),
                cell.center(),
                icons::IMAGE,
                18.0,
                c.text_tertiary,
            ),
        }
        if !busy {
            let remove =
                Rect::from_center_size(cell.right_top() + Vec2::new(-8.0, 8.0), Vec2::splat(14.0));
            let response = ui.interact(
                remove,
                ui.make_persistent_id(("chat-remove", index)),
                Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, "Remove picture")
            });
            ui.painter().circle_filled(remove.center(), 7.0, c.raised);
            ui.painter()
                .circle_stroke(remove.center(), 7.0, Stroke::new(1.0, c.border_strong));
            icons::paint(ui.painter(), remove.center(), icons::X, 9.0, c.text);
            if response.clicked() {
                actions.push(PanelAction::RemoveAttachment(index));
            }
        }
    }
}

/// The send button: solid accent when there is something to send.
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

/// The stop button shown while an answer arrives.
fn stop_button(ui: &mut Ui, stopping: bool) -> bool {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(theme.size.control), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !stopping, "Stop"));
    let fill = if stopping {
        c.well
    } else if response.is_pointer_button_down_on() {
        c.well_pressed
    } else if response.hovered() {
        c.well_hover
    } else {
        c.well
    };
    ui.painter().rect_filled(rect, theme.radius.sm, fill);
    let ink = if stopping { c.text_tertiary } else { c.text };
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), Vec2::splat(9.0)),
        2.0,
        ink,
    );
    let clicked = !stopping && response.clicked();
    tooltip(response, if stopping { "Stopping…" } else { "Stop" }, None);
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_follow_the_selection() {
        assert!(suggestions(true)[0].contains("selected"));
        assert!(!suggestions(false)[0].contains("selected"));
    }
}

//! The AI settings: every provider with whether it can be used, switches to
//! offer it or not, API keys, server addresses and models, and the AI the
//! chat opens with.
//!
//! Everything that is not a secret is kept in `moshionly-ai`'s
//! `AiSettings` and saved as soon as it changes. Keys go to the system's
//! keychain through the controller and are never shown again: a key field
//! is always empty, and only says whether a key is stored.

use std::collections::HashMap;

use egui::{
    Align, Align2, Color32, Context, Layout, Rect, Response, ScrollArea, Sense, Stroke, Ui, Vec2,
    pos2,
};
use moshionly_ai::{ApiKey, ProviderKind, ProviderStatus, ReadyState, known_providers};

use super::controller::{Controller, KeyNote};
use crate::theme::{Theme, Tone, icons};
use crate::widgets::{
    ButtonKind, Dropdown, ModalFrame, TextButton, TextField, modal_footer, paint, tooltip,
};

/// The width of the settings dialog.
const WIDTH: f32 = 520.0;
/// The tallest the provider list grows before it scrolls.
const LIST_HEIGHT: f32 = 380.0;
/// The width of the labels in a provider's details.
const LABEL_WIDTH: f32 = 124.0;

/// What the settings dialog keeps between frames: which provider is open
/// and what is being typed. Typed keys live here only until they are saved.
#[derive(Default)]
pub struct SettingsView {
    expanded: Option<String>,
    /// Scroll the open provider into view on the next frame.
    reveal: bool,
    keys: HashMap<String, String>,
    addresses: HashMap<String, String>,
    models: HashMap<String, String>,
}

impl SettingsView {
    /// Opens the details of one provider, closing any other.
    pub fn expand(&mut self, id: Option<&str>) {
        self.expanded = id.map(str::to_owned);
        self.reveal = id.is_some();
    }

    /// Forgets everything typed, keys included. Called when the dialog
    /// closes.
    pub fn reset(&mut self) {
        self.keys.clear();
        self.addresses.clear();
        self.models.clear();
    }

    /// Draws the dialog. Returns true when it should close.
    pub fn show(&mut self, ctx: &Context, controller: &mut Controller) -> bool {
        let mut done = false;
        let response = ModalFrame::new("AI settings").width(WIDTH).show(ctx, |ui| {
            self.body(ui, controller);
            modal_footer(ui, |ui| {
                done = ui.add(TextButton::primary("Done")).clicked();
                let checking = controller.is_checking();
                let check = ui.add_enabled(
                    !checking,
                    TextButton::new(if checking {
                        "Checking…"
                    } else {
                        "Check again"
                    })
                    .kind(ButtonKind::Quiet),
                );
                if check.clicked() {
                    controller.refresh_statuses();
                }
            });
        });
        let close = done || response.close_requested;
        if close {
            self.reset();
        }
        close
    }

    fn body(&mut self, ui: &mut Ui, controller: &mut Controller) {
        let theme = Theme::of(ui);
        let c = &theme.color;
        paragraph(
            ui,
            "Choose the AI the chat works with. Assistants installed on this computer use \
             your own subscription; the others use an API key, kept in your system's keychain.",
            c.text_secondary,
        );
        if let Some(problem) = controller.settings_problem() {
            warning(ui, problem);
        }
        self.default_row(ui, controller);

        let statuses: Vec<ProviderStatus> = match controller.statuses() {
            Some(statuses) => statuses.to_vec(),
            None => {
                paragraph(
                    ui,
                    "Checking which assistants are installed…",
                    c.text_secondary,
                );
                return;
            }
        };
        egui::Frame::new()
            .stroke(Stroke::new(1.0, c.border))
            .corner_radius(crate::theme::corner(theme.radius.md))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt("ai-settings-providers")
                    .max_height(LIST_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let groups = [
                            ("On this computer", ProviderKind::Subscription),
                            ("With an API key or a local server", ProviderKind::Api),
                        ];
                        for (index, (title, kind)) in groups.into_iter().enumerate() {
                            group_heading(ui, title, index > 0);
                            for status in statuses.iter().filter(|s| s.info.kind == kind) {
                                self.provider_row(ui, controller, status);
                            }
                        }
                    });
            });
    }

    /// The AI the chat opens with, and its model.
    fn default_row(&mut self, ui: &mut Ui, controller: &mut Controller) {
        let theme = Theme::of(ui);
        let names: Vec<(String, String)> = known_providers()
            .into_iter()
            .map(|descriptor| (descriptor.info.id, descriptor.info.name))
            .collect();
        let mut options: Vec<(usize, &str)> = vec![(0, "The first one that is ready")];
        options.extend(
            names
                .iter()
                .enumerate()
                .map(|(index, (_, name))| (index + 1, name.as_str())),
        );
        let current = controller.settings().default_provider().map(str::to_owned);
        let mut chosen = current
            .as_deref()
            .and_then(|id| names.iter().position(|(known, _)| known == id))
            .map_or(0, |index| index + 1);
        labelled(ui, "Open the chat with", |ui| {
            let response = Dropdown::new(&mut chosen, "Open the chat with", &options).show(ui);
            if response.changed() {
                let id = chosen.checked_sub(1).map(|index| names[index].0.clone());
                controller.change_settings(|settings| settings.default_provider = id);
            }
        });
        ui.add_space(theme.space.xs);
    }

    fn provider_row(&mut self, ui: &mut Ui, controller: &mut Controller, status: &ProviderStatus) {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let id = status.info.id.as_str();
        let open = self.expanded.as_deref() == Some(id);
        let pad = theme.space.lg;
        let width = ui.available_width();
        // The row's background goes under everything, so it is set last.
        let background = ui.painter().add(egui::Shape::Noop);
        let message = status.readiness.message.trim();
        let message_job = egui::text::LayoutJob::simple(
            message.to_owned(),
            theme.small(),
            c.text_secondary,
            width - pad * 2.0 - 60.0 - 22.0,
        );
        let message_galley = ui.painter().layout_job(message_job);
        let height = theme.space.md * 2.0
            + theme.type_scale.body * 1.4
            + if message.is_empty() {
                0.0
            } else {
                message_galley.size().y + 2.0
            };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!("{}: {}", status.info.name, ready_word(status)),
            )
        });
        let top = rect.top() + theme.space.md;
        let line_height = theme.type_scale.body * 1.4;
        let dot = pos2(rect.left() + pad + 4.0, top + line_height / 2.0);
        ui.painter()
            .circle_filled(dot, 4.0, dot_color(theme, status));
        let name_x = rect.left() + pad + 18.0;
        let name = paint::line(ui, &status.info.name, theme.body_medium(), c.text);
        let name_width = name.size().x;
        ui.painter().galley(pos2(name_x, top), name, c.text);
        let mut tag_x = name_x + name_width + theme.space.sm;
        if !status.capabilities.verified {
            let untested = tag(
                ui,
                pos2(tag_x, top + line_height / 2.0),
                "Untested",
                Tone::Amber,
            );
            let hover = ui.interact(
                untested,
                ui.make_persistent_id(("untested", id)),
                Sense::hover(),
            );
            tooltip(
                hover,
                "Moshionly's makers have not tried this one with the real service yet. \
                 It should work; tell us if it does not.",
                None,
            );
            tag_x += untested.width() + theme.space.xs;
        }
        if status.key_stored {
            tag(
                ui,
                pos2(tag_x, top + line_height / 2.0),
                "Key stored",
                Tone::Neutral,
            );
        }
        if !message.is_empty() {
            ui.painter().galley(
                pos2(name_x, top + line_height + 2.0),
                message_galley,
                c.text_secondary,
            );
        }

        // The switch that offers the provider in the chat.
        let switch_rect = Rect::from_center_size(
            pos2(rect.right() - pad - 14.0, top + line_height / 2.0),
            Vec2::new(28.0, 16.0),
        );
        let mut enabled = status.enabled;
        if switch(
            ui,
            switch_rect,
            &mut enabled,
            &format!("Offer {} in the chat", status.info.name),
        ) {
            controller.change_settings(|settings| settings.provider_mut(id).enabled = enabled);
            controller.refresh_status(id);
        }
        let caret = pos2(switch_rect.left() - theme.space.lg, switch_rect.center().y);
        icons::paint(
            ui.painter(),
            caret,
            if open {
                icons::CARET_UP
            } else {
                icons::CARET_DOWN
            },
            10.0,
            if response.hovered() {
                c.text
            } else {
                c.text_tertiary
            },
        );
        if response.clicked() {
            self.expanded = if open { None } else { Some(id.to_owned()) };
            self.reveal = !open;
        }
        let mut whole = rect;
        if open {
            let details = self.details(ui, controller, status);
            whole = whole.union(details);
            if std::mem::take(&mut self.reveal) {
                ui.scroll_to_rect_animation(
                    whole,
                    Some(Align::Center),
                    egui::style::ScrollAnimation::none(),
                );
            }
        }
        if response.hovered() {
            ui.painter().set(
                background,
                egui::epaint::RectShape::filled(rect, 0.0, c.well),
            );
        }
        ui.painter()
            .hline(whole.x_range(), whole.bottom(), Stroke::new(1.0, c.border));
    }

    /// What can be set for one provider, below its row. Returns the
    /// rectangle it took.
    fn details(
        &mut self,
        ui: &mut Ui,
        controller: &mut Controller,
        status: &ProviderStatus,
    ) -> Rect {
        let theme = Theme::of(ui);
        let pad = theme.space.lg;
        let outer = ui.available_rect_before_wrap();
        let inner = Rect::from_min_max(
            pos2(outer.left() + pad + 18.0, outer.top()),
            pos2(outer.right() - pad, outer.bottom()),
        );
        let mut details = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Min)),
        );
        details.spacing_mut().item_spacing.y = theme.space.sm;
        self.detail_fields(&mut details, controller, status);
        let used = Rect::from_min_max(
            outer.min,
            pos2(outer.right(), details.min_rect().bottom() + theme.space.md),
        );
        ui.allocate_rect(used, Sense::hover());
        used
    }

    fn detail_fields(&mut self, ui: &mut Ui, controller: &mut Controller, status: &ProviderStatus) {
        let theme = Theme::of(ui);
        let id = status.info.id.clone();
        let descriptor = known_providers()
            .into_iter()
            .find(|descriptor| descriptor.info.id == id);

        if status.capabilities.needs_api_key || status.info.id == "custom" {
            self.key_field(ui, controller, status);
        }
        if status.info.kind == ProviderKind::Api {
            let default_address = descriptor
                .as_ref()
                .and_then(|descriptor| descriptor.default_base_url.clone())
                .unwrap_or_else(|| "https://host/v1".to_owned());
            let current = controller
                .settings()
                .provider(&id)
                .base_url
                .clone()
                .unwrap_or_default();
            let draft = self.addresses.entry(id.clone()).or_insert(current);
            let mut commit = None;
            labelled(ui, "Server address", |ui| {
                let field = TextField::new(draft)
                    .hint(&default_address)
                    .name("Server address")
                    .show(ui);
                if field.submitted || field.response.lost_focus() {
                    commit = Some(draft.trim().to_owned());
                }
            });
            if let Some(address) = commit {
                let address = (!address.is_empty()).then_some(address);
                if controller.change_settings(|s| s.provider_mut(&id).base_url = address) {
                    controller.refresh_status(&id);
                }
            }
        }

        let current = controller
            .settings()
            .provider(&id)
            .model
            .clone()
            .unwrap_or_default();
        let draft = self.models.entry(id.clone()).or_insert(current);
        let mut commit = None;
        labelled(ui, "Model", |ui| {
            let field = TextField::new(draft)
                .hint("The provider's default")
                .name("Model")
                .show(ui);
            if field.submitted || field.response.lost_focus() {
                commit = Some(draft.trim().to_owned());
            }
        });
        if let Some(model) = commit {
            let model = (!model.is_empty()).then_some(model);
            controller.change_settings(|s| s.provider_mut(&id).model = model);
        }

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme.space.sm;
            if ui.add(TextButton::new("Check again")).clicked() {
                controller.refresh_status(&id);
            }
            if let Some(url) = descriptor.as_ref().and_then(|d| d.key_url.clone())
                && status.capabilities.needs_api_key
            {
                ui.hyperlink_to("Get a key", url);
            }
        });
    }

    /// The API key: a field that is always empty, a button to store what is
    /// typed, and one to remove the stored key.
    fn key_field(&mut self, ui: &mut Ui, controller: &mut Controller, status: &ProviderStatus) {
        let theme = Theme::of(ui);
        let c = &theme.color;
        let id = status.info.id.as_str();
        let draft = self.keys.entry(id.to_owned()).or_default();
        let mut save = false;
        let mut remove = false;
        labelled(ui, "API key", |ui| {
            let hint = if status.key_stored {
                "A key is stored. Paste a new one to replace it"
            } else {
                "Paste your key"
            };
            let buttons = if status.key_stored { 132.0 } else { 64.0 };
            let field_width = (ui.available_width() - buttons).max(80.0);
            let field = key_input(ui, id, draft, hint, field_width);
            save = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            save |= ui
                .add_enabled(!draft.trim().is_empty(), TextButton::new("Save"))
                .clicked();
            if status.key_stored {
                remove = ui
                    .add(TextButton::new("Remove").kind(ButtonKind::Quiet))
                    .clicked();
            }
        });
        if save && !draft.trim().is_empty() {
            controller.save_key(id, ApiKey::new(std::mem::take(draft)));
        }
        if remove {
            draft.clear();
            controller.forget_key(id);
        }
        let note = match controller.key_note(id) {
            Some(KeyNote::Working) => Some(("Saving…".to_owned(), c.text_secondary)),
            Some(KeyNote::Saved) => Some(("Key saved in the keychain.".to_owned(), c.success)),
            Some(KeyNote::Removed) => Some(("Key removed.".to_owned(), c.text_secondary)),
            Some(KeyNote::Failed(problem)) => Some((problem.clone(), c.danger)),
            None => None,
        };
        if let Some((text, color)) = note {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH);
                ui.label(egui::RichText::new(text).font(theme.small()).color(color));
            });
        }
    }
}

/// A one-line password field on a well. The key typed is hidden.
fn key_input(ui: &mut Ui, provider: &str, text: &mut String, hint: &str, width: f32) -> Response {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, theme.size.control), Sense::hover());
    let id = ui.make_persistent_id(("api-key-field", provider));
    let focused = ui.memory(|m| m.has_focus(id));
    paint::panel(
        ui.painter(),
        rect,
        theme.radius.sm,
        c.well,
        if focused {
            Stroke::new(1.0, c.focus_ring)
        } else {
            Stroke::NONE
        },
    );
    let text_height = ui.text_style_height(&egui::TextStyle::Body);
    let edit = Rect::from_min_max(
        pos2(
            rect.left() + theme.space.md,
            rect.center().y - text_height / 2.0,
        ),
        pos2(
            rect.right() - theme.space.md,
            rect.center().y + text_height / 2.0,
        ),
    );
    ui.place(
        edit,
        egui::TextEdit::singleline(text)
            .id(id)
            .password(true)
            .hint_text(egui::RichText::new(hint).color(c.text_tertiary))
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(theme.body())
            .text_color(c.text)
            .background_color(Color32::TRANSPARENT)
            .desired_width(edit.width()),
    )
    .accessible_name("API key")
}

/// A label on the left and controls on the right, on one line.
fn labelled(ui: &mut Ui, label: &str, controls: impl FnOnce(&mut Ui)) {
    let theme = Theme::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.sm;
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(LABEL_WIDTH - theme.space.sm, theme.size.control),
            Sense::hover(),
        );
        paint::text_in(
            ui,
            rect,
            Align2::LEFT_CENTER,
            label,
            theme.small_medium(),
            theme.color.text_secondary,
        );
        controls(ui);
    });
}

/// A wrapped paragraph in the small type.
fn paragraph(ui: &mut Ui, text: &str, color: Color32) {
    let theme = Theme::of(ui);
    let job =
        egui::text::LayoutJob::simple(text.to_owned(), theme.small(), color, ui.available_width());
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
}

/// A warning line about the settings file.
fn warning(ui: &mut Ui, text: &str) {
    let theme = Theme::of(ui);
    let tone = theme.color.tone(Tone::Amber);
    let width = ui.available_width();
    let pad = theme.space.md;
    let job = egui::text::LayoutJob::simple(
        text.to_owned(),
        theme.small(),
        theme.color.text,
        width - pad * 2.0 - 22.0,
    );
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(width, galley.size().y + pad * 2.0),
        Sense::hover(),
    );
    paint::panel(
        ui.painter(),
        rect,
        theme.radius.sm,
        tone.soft,
        Stroke::new(1.0, tone.outline),
    );
    icons::paint(
        ui.painter(),
        pos2(rect.left() + pad + 7.0, rect.top() + pad + 7.0),
        icons::WARNING,
        14.0,
        tone.text,
    );
    ui.painter().galley(
        rect.min + Vec2::new(pad + 22.0, pad),
        galley,
        theme.color.text,
    );
}

/// A small heading above a group of providers.
fn group_heading(ui: &mut Ui, title: &str, rule: bool) {
    let theme = Theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme.size.control + theme.space.xs),
        Sense::hover(),
    );
    if rule {
        ui.painter()
            .hline(rect.x_range(), rect.top(), theme.hairline());
    }
    paint::text_in(
        ui,
        rect.shrink2(Vec2::new(theme.space.lg, 0.0)),
        Align2::LEFT_BOTTOM,
        title,
        theme.small_medium(),
        theme.color.text_secondary,
    );
    ui.add_space(theme.space.xs);
}

/// A small rounded tag. Returns where it is.
fn tag(ui: &Ui, left_center: egui::Pos2, text: &str, tone: Tone) -> Rect {
    let theme = Theme::of(ui);
    let colors = theme.color.tone(tone);
    let galley = paint::line(ui, text, theme.caption(), colors.text);
    let size = galley.size() + Vec2::new(theme.space.sm * 2.0, 4.0);
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - size.y / 2.0), size);
    ui.painter().rect_filled(rect, theme.radius.xs, colors.soft);
    paint::galley_in(ui.painter(), rect, Align2::CENTER_CENTER, galley);
    rect
}

/// An on and off switch. Returns true when it was flipped.
fn switch(ui: &mut Ui, rect: Rect, on: &mut bool, name: &str) -> bool {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let response = ui.interact(
        rect.expand(4.0),
        ui.make_persistent_id(("switch", name)),
        Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, true, *on, name));
    let flipped = response.clicked();
    if flipped {
        *on = !*on;
    }
    let track = if *on { c.accent } else { c.border_strong };
    ui.painter().rect_filled(rect, rect.height() / 2.0, track);
    let knob_x = if *on {
        rect.right() - rect.height() / 2.0
    } else {
        rect.left() + rect.height() / 2.0
    };
    ui.painter().circle_filled(
        pos2(knob_x, rect.center().y),
        rect.height() / 2.0 - 2.0,
        Color32::WHITE,
    );
    tooltip(response, name, None);
    flipped
}

/// The colour of a provider's status dot.
fn dot_color(theme: &Theme, status: &ProviderStatus) -> Color32 {
    let c = &theme.color;
    if !status.enabled {
        c.text_tertiary
    } else if status.readiness.is_ready() {
        c.success
    } else {
        c.warning
    }
}

/// One or two words about a provider, for screen readers.
fn ready_word(status: &ProviderStatus) -> &'static str {
    if !status.enabled {
        return "turned off";
    }
    match status.readiness.state {
        ReadyState::Ready => "ready",
        ReadyState::NotInstalled => "not installed",
        ReadyState::NotSignedIn => "not signed in",
        ReadyState::MissingKey => "needs a key",
        ReadyState::InvalidKey => "key refused",
        ReadyState::Unreachable => "not reachable",
        ReadyState::Unknown => "unknown",
    }
}

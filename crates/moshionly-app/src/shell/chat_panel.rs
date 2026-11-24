//! The chat panel: the AI chat placed in the window.
//!
//! The chat itself is the self-contained [`Chat`] module. The shell hands
//! it the agent tools over the open document, the MCP server's address
//! while it runs, and what the person is looking at (the artboard, the
//! selection and the playhead). The AI's edits go through the shared
//! session, which tells the editor to repaint.

use egui::{Align, Context, Layout, Rect, Ui, UiBuilder};

use super::{Shell, parts};
use crate::{
    chat::{Chat, ChatInputs, Focus},
    theme::{Theme, icons},
    widgets::{IconButton, divider, empty_state, section_header},
};

impl Shell {
    /// Puts `chat` in the chat panel. The app passes the real one, with
    /// the person's AI settings; tests pass one with scripted providers.
    /// Without one the panel says the chat is not set up.
    pub fn set_chat(&mut self, chat: Chat) {
        self.chat = Some(Box::new(chat));
    }

    /// The chat in the panel, if one was set.
    pub fn chat(&self) -> Option<&Chat> {
        self.chat.as_deref()
    }

    /// The chat in the panel, to change it.
    pub fn chat_mut(&mut self) -> Option<&mut Chat> {
        self.chat.as_deref_mut()
    }

    /// Draws the chat panel in `rect`, or keeps a hidden chat's running
    /// answer moving when `rect` is `None`. Returns whether the person
    /// closed the panel.
    pub(super) fn chat_panel(&mut self, ui: &mut Ui, rect: Option<Rect>) -> bool {
        let ctx = ui.ctx().clone();
        let Some(rect) = rect else {
            if let Some(chat) = self.chat.as_deref_mut()
                && chat.poll(&ctx).document_changed
            {
                ctx.request_repaint();
            }
            return false;
        };
        let surface = Theme::of(ui).color.surface;
        let focus = Focus {
            artboard: self.editor.artboard(),
            selection: self.editor.selection.ids(),
            time_ms: self.editor.view_time(),
        };
        let mcp_url = self.server.url();
        let dialogs = Some(self.dialogs.as_ref());
        let tools = &self.tools;
        let chat = self.chat.as_deref_mut();
        let close = parts::panel(ui, "chat", rect, surface, |ui| {
            let mut column = ui.new_child(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            column.spacing_mut().item_spacing.y = 0.0;
            match chat {
                Some(chat) => {
                    let output = chat.show(
                        &mut column,
                        ChatInputs {
                            tools,
                            mcp_url,
                            focus,
                            dialogs,
                        },
                    );
                    if output.document_changed {
                        repaint(&ctx);
                    }
                    output.close
                }
                None => not_set_up(&mut column),
            }
        });
        parts::edge_line(ui, rect, parts::Edge::Left);
        close
    }
}

fn repaint(ctx: &Context) {
    ctx.request_repaint();
}

/// The panel when no chat was set: a header with a close button and a
/// short note.
fn not_set_up(ui: &mut Ui) -> bool {
    let theme = Theme::of(ui);
    let mut close = false;
    section_header(ui, "Chat", None, |ui| {
        close = ui
            .add(IconButton::new(icons::X).dimmed(true).tooltip("Close chat"))
            .clicked();
    });
    divider(ui);
    ui.add_space(theme.space.xxl);
    empty_state(
        ui,
        icons::SPARKLE,
        "The chat is not set up",
        "This window was opened without the AI chat.",
        None,
    );
    close
}

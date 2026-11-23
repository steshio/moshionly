//! The AI chat: a panel where the person asks an AI to change the design,
//! and the settings where they choose which AI that is.
//!
//! The chat talks to the AI through `moshionly-ai` and lets it work in the
//! document through the tools of `moshionly-agent`. It is self-contained:
//! the shell owns one chat, hands it plain inputs every frame and reads back
//! what happened.
//!
//! The parts, from the data inwards:
//!
//! - [`conversation`]: the messages, built up from a provider's events.
//! - [`tool_summary`]: a few readable words for each tool call.

pub mod conversation;
pub mod tool_summary;

pub use conversation::{
    AnswerState, Author, ChatError, Conversation, Message, Part, ProviderSession, ToolCall,
    ToolState, Usage,
};

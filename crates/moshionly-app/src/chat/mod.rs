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
//! - [`executor`]: the agent tools as API providers call them.
//! - [`source`]: where providers come from: the registry, or in tests
//!   [`testing`]'s scripted stand-ins.
//! - [`controller`]: runs turns and provider checks in the background.

pub mod controller;
pub mod conversation;
pub mod executor;
pub mod source;
pub mod testing;
pub mod tool_summary;

pub use controller::{Controller, KeyNote, MCP_SERVER_NAME, ModelList, Polled, SendError};
pub use conversation::{
    AnswerState, Author, ChatError, Conversation, Message, Part, ProviderSession, ToolCall,
    ToolState, Usage,
};
pub use executor::AgentTools;
pub use source::{ProviderSource, RegistrySource};

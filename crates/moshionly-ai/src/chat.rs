//! What the app sends to a provider: messages and the request around them.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::tool::ToolExecutor;

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The person using the app.
    User,
    /// The model.
    Assistant,
}

/// An image attached to a message or returned by a tool.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageData {
    /// MIME type, for example `image/png`.
    pub media_type: String,
    /// The encoded image file, not decoded pixels.
    #[serde(with = "base64_bytes")]
    pub data: Vec<u8>,
}

impl ImageData {
    /// An image from the bytes of a PNG file.
    pub fn png(data: impl Into<Vec<u8>>) -> Self {
        Self {
            media_type: "image/png".to_owned(),
            data: data.into(),
        }
    }

    /// The image bytes as standard base64, the form every model API expects.
    pub fn to_base64(&self) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }
}

impl fmt::Debug for ImageData {
    // Image bytes would drown any log line, so only their size is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("media_type", &self.media_type)
            .field("bytes", &self.data.len())
            .finish()
    }
}

/// One turn of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Who wrote it.
    pub role: Role,
    /// The message text.
    pub text: String,
    /// Images attached to the message. Providers that cannot accept images
    /// (see [`Capabilities::images`](crate::Capabilities)) ignore them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageData>,
}

impl ChatMessage {
    /// A message from the user.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
            images: Vec::new(),
        }
    }

    /// A message from the model.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            images: Vec::new(),
        }
    }

    /// Attaches an image.
    pub fn with_image(mut self, image: ImageData) -> Self {
        self.images.push(image);
        self
    }
}

/// Where a subscription provider finds the Moshionly tools: an MCP server
/// that the app serves over local HTTP while it is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpEndpoint {
    /// The server name the agent sees. Tool names are derived from it, so it
    /// should be short, lowercase and stable (for example `moshionly`).
    pub name: String,
    /// The streamable HTTP URL, for example `http://127.0.0.1:41234/mcp`.
    pub url: String,
}

impl McpEndpoint {
    /// An endpoint with the given server name and URL.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
        }
    }
}

/// Everything a provider needs for one turn.
///
/// The two ways of reaching tools sit side by side: subscription providers
/// attach the MCP server in [`mcp`](Self::mcp), API providers call
/// [`tools`](Self::tools) directly. An app that wants either kind of provider
/// to work fills in both.
#[derive(Clone, Default)]
pub struct ChatRequest {
    /// The conversation so far, oldest first, ending with the user message to
    /// answer.
    pub messages: Vec<ChatMessage>,
    /// Instructions for the model. Subscription providers append it to the
    /// agent's own system prompt.
    pub system_prompt: Option<String>,
    /// The model to use, as one of the ids from
    /// [`ChatProvider::list_models`](crate::ChatProvider::list_models). `None`
    /// means the provider's default.
    pub model: Option<String>,
    /// A session to continue, from an earlier
    /// [`ChatEvent::SessionStarted`](crate::ChatEvent::SessionStarted). Only
    /// providers with [`Capabilities::resume`](crate::Capabilities) use it;
    /// they then send just the last user message, because the agent already
    /// holds the rest.
    pub session_id: Option<String>,
    /// The Moshionly MCP server, for subscription providers.
    pub mcp: Option<McpEndpoint>,
    /// The Moshionly tools, for API providers.
    pub tools: Option<Arc<dyn ToolExecutor>>,
    /// The directory an agent CLI runs in. It should be an empty, stable
    /// directory owned by the app: stable because agents look up sessions by
    /// directory, empty so that the agent has nothing of the user's to read.
    /// `None` uses [`default_working_dir`](crate::default_working_dir).
    pub working_dir: Option<PathBuf>,
}

impl ChatRequest {
    /// A request holding a single user message.
    pub fn from_prompt(prompt: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::user(prompt)],
            ..Self::default()
        }
    }

    /// The last user message, which is the one being answered.
    pub fn last_user_message(&self) -> Option<&ChatMessage> {
        self.messages.iter().rev().find(|m| m.role == Role::User)
    }
}

impl fmt::Debug for ChatRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatRequest")
            .field("messages", &self.messages)
            .field("system_prompt", &self.system_prompt)
            .field("model", &self.model)
            .field("session_id", &self.session_id)
            .field("mcp", &self.mcp)
            .field("tools", &self.tools.as_ref().map(|t| t.tools().len()))
            .field("working_dir", &self.working_dir)
            .finish()
    }
}

/// Serde helper that stores bytes as a base64 string.
mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_with_base64_image() {
        let message = ChatMessage::user("look").with_image(ImageData::png(vec![1, 2, 3]));
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["images"][0]["data"], "AQID");
        let back: ChatMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, message);
    }

    #[test]
    fn image_debug_output_hides_the_bytes() {
        let image = ImageData::png(vec![0u8; 4096]);
        let debug = format!("{image:?}");
        assert!(debug.contains("4096"));
        assert!(debug.len() < 100);
    }

    #[test]
    fn last_user_message_skips_trailing_assistant_text() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage::user("first"),
                ChatMessage::assistant("reply"),
                ChatMessage::user("second"),
                ChatMessage::assistant("partial"),
            ],
            ..ChatRequest::default()
        };
        assert_eq!(request.last_user_message().unwrap().text, "second");
    }
}

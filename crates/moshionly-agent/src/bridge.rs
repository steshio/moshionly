//! The tools for the built-in chat, as plain data.
//!
//! The chat providers in `moshionly-ai` run the tool loop for API models
//! and call tools through its `ToolExecutor` trait. This crate does not
//! depend on that one, so [`ToolBridge`] offers the tools in the shapes the
//! trait needs, and the app joins the two in a few lines:
//!
//! ```text
//! struct AppTools(moshionly_agent::ToolBridge);
//!
//! #[async_trait]
//! impl moshionly_ai::ToolExecutor for AppTools {
//!     fn tools(&self) -> Vec<moshionly_ai::ToolSpec> {
//!         self.0.list().into_iter()
//!             .map(|tool| moshionly_ai::ToolSpec::new(tool.name, tool.description, tool.input_schema))
//!             .collect()
//!     }
//!     async fn call(&self, name: &str, input: Value) -> moshionly_ai::ToolOutput {
//!         let bridge = self.0.clone();
//!         let name = name.to_owned();
//!         // Screenshots and exports take a moment: keep them off the async threads.
//!         let result = tokio::task::spawn_blocking(move || bridge.call(&name, input)).await;
//!         // ...turn each BridgePart into a moshionly_ai::ToolContent...
//!     }
//! }
//! ```

use serde_json::Value;

use crate::guide::{Focus, system_prompt};
use crate::tool::ToolContent;
use crate::tools::Tools;

/// A tool as a chat provider offers it to a model.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTool {
    /// The name the model calls it by.
    pub name: String,
    /// What it does, written for a model.
    pub description: String,
    /// JSON Schema of its input object.
    pub input_schema: Value,
}

/// One piece of a tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgePart {
    /// Text, often JSON.
    Text(String),
    /// An image file, such as a screenshot.
    Image {
        /// The media type, for example `image/png`.
        media_type: String,
        /// The bytes of the image file.
        data: Vec<u8>,
    },
}

/// What a tool returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeResult {
    /// The pieces, in order.
    pub parts: Vec<BridgePart>,
    /// Whether the call failed; the text then says what to do instead.
    pub is_error: bool,
}

/// The tools of a session, for the built-in chat.
///
/// ```
/// use moshionly_agent::{BridgePart, Host, SessionHandle, ToolBridge, Tools};
/// use moshionly_core::Document;
/// use serde_json::json;
///
/// let session = SessionHandle::from_document(Document::new("Hello"));
/// let bridge = ToolBridge::new(Tools::new(session, Host::new()));
/// assert!(bridge.list().iter().any(|tool| tool.name == "apply_edits"));
///
/// let result = bridge.call("get_overview", json!({}));
/// assert!(!result.is_error);
/// assert!(matches!(&result.parts[0], BridgePart::Text(text) if text.contains("\"artboards\"")));
/// ```
#[derive(Debug, Clone)]
pub struct ToolBridge {
    tools: Tools,
}

impl ToolBridge {
    /// The bridge to `tools`.
    pub fn new(tools: Tools) -> Self {
        Self { tools }
    }

    /// Every tool, always in the same order.
    pub fn list(&self) -> Vec<BridgeTool> {
        self.tools
            .list()
            .into_iter()
            .map(|spec| BridgeTool {
                name: spec.name.to_owned(),
                description: spec.description.to_owned(),
                input_schema: spec.input_schema,
            })
            .collect()
    }

    /// Runs a tool. This never fails as a function: a problem comes back as
    /// a result with `is_error` set and a message for the model. It may take
    /// a moment for screenshots and exports, so call it off the UI thread.
    pub fn call(&self, name: &str, input: Value) -> BridgeResult {
        let output = self.tools.call(name, input);
        BridgeResult {
            parts: output
                .content
                .into_iter()
                .map(|part| match part {
                    ToolContent::Text { text } => BridgePart::Text(text),
                    ToolContent::Image { media_type, data } => {
                        BridgePart::Image { media_type, data }
                    }
                })
                .collect(),
            is_error: output.is_error,
        }
    }

    /// The system prompt for the chat: the agent guide, and what the person
    /// is looking at when `focus` says.
    pub fn system_prompt(&self, focus: Option<&Focus<'_>>) -> String {
        system_prompt(&self.tools.session().document(), focus)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use moshionly_core::NodeId;
    use serde_json::json;

    use super::*;
    use crate::Host;
    use crate::tools::testing::{StubFrames, demo_session};

    fn bridge() -> ToolBridge {
        ToolBridge::new(Tools::new(
            demo_session(),
            Host::new().with_frames(Arc::new(StubFrames::default())),
        ))
    }

    #[test]
    fn lists_the_same_tools_in_the_same_order() {
        let bridge = bridge();
        let names: Vec<String> = bridge.list().into_iter().map(|tool| tool.name).collect();
        let direct: Vec<&str> = bridge.tools.list().iter().map(|spec| spec.name).collect();
        assert_eq!(names, direct);
        let apply = bridge
            .list()
            .into_iter()
            .find(|tool| tool.name == "apply_edits")
            .unwrap();
        assert!(apply.description.starts_with("Changes the file."));
        assert_eq!(apply.input_schema["required"], json!(["operations"]));
    }

    #[test]
    fn calls_give_text_and_image_parts() {
        let bridge = bridge();
        let shot = bridge.call("screenshot", json!({ "artboard": "scene" }));
        assert!(!shot.is_error);
        assert!(matches!(&shot.parts[0], BridgePart::Text(text) if text.contains("\"shots\"")));
        assert_eq!(
            shot.parts[1],
            BridgePart::Image {
                media_type: "image/png".into(),
                data: b"\x89PNG stand-in".to_vec()
            }
        );

        let failed = bridge.call("get_node", json!({ "id": "nope" }));
        assert!(failed.is_error);
        assert!(matches!(&failed.parts[0], BridgePart::Text(text) if text.contains("no node")));
    }

    #[test]
    fn the_system_prompt_follows_the_open_document() {
        let bridge = bridge();
        let scene = NodeId::from("scene");
        let prompt = bridge.system_prompt(Some(&Focus {
            artboard: Some(&scene),
            ..Focus::default()
        }));
        assert!(prompt.contains("artboard \"Launch\""), "{prompt}");
        assert!(
            bridge
                .system_prompt(None)
                .starts_with("You are working in Moshionly")
        );
    }
}

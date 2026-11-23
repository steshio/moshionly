//! The Moshionly tools as an API provider calls them.
//!
//! API providers (Anthropic and the OpenAI-compatible ones) run the tool
//! loop themselves and call a [`ToolExecutor`]. [`AgentTools`] is that
//! executor over the agent crate's [`ToolBridge`], so a model reached with
//! an API key edits the same document, through the same validator and undo
//! history, as an agent CLI that reaches the tools over MCP.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use moshionly_agent::{BridgePart, BridgeResult, ToolBridge};
use moshionly_ai::{ImageData, ToolContent, ToolExecutor, ToolOutput, ToolSpec};
use serde_json::Value;

/// Told about the pictures a tool returned, with the tool's name and input,
/// so the chat can show them under the call.
pub type ImageSink = Arc<dyn Fn(&str, &Value, Vec<ImageData>) + Send + Sync>;

/// The Moshionly tools behind `moshionly-ai`'s [`ToolExecutor`].
///
/// Tool calls are synchronous and can take a moment (screenshots, exports),
/// so each one runs on Tokio's blocking pool, never on the async threads
/// that stream the answer.
#[derive(Clone)]
pub struct AgentTools {
    bridge: ToolBridge,
    images: Option<ImageSink>,
}

impl fmt::Debug for AgentTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentTools")
            .field("tools", &self.bridge.list().len())
            .field("image_sink", &self.images.is_some())
            .finish()
    }
}

impl AgentTools {
    /// The tools of `bridge`.
    pub fn new(bridge: ToolBridge) -> Self {
        Self {
            bridge,
            images: None,
        }
    }

    /// Also hands every picture a tool returns to `sink`.
    pub fn with_image_sink(mut self, sink: ImageSink) -> Self {
        self.images = Some(sink);
        self
    }
}

#[async_trait]
impl ToolExecutor for AgentTools {
    fn tools(&self) -> Vec<ToolSpec> {
        self.bridge
            .list()
            .into_iter()
            .map(|tool| ToolSpec::new(tool.name, tool.description, tool.input_schema))
            .collect()
    }

    async fn call(&self, name: &str, input: Value) -> ToolOutput {
        let bridge = self.bridge.clone();
        let (tool, arguments) = (name.to_owned(), input.clone());
        let result = tokio::task::spawn_blocking(move || bridge.call(&tool, arguments)).await;
        let output = match result {
            Ok(result) => to_output(result),
            Err(error) => {
                let why = if error.is_panic() {
                    "stopped with an internal error"
                } else {
                    "was cancelled"
                };
                ToolOutput::error(format!(
                    "The tool {name} {why}. The document was not changed by this call. \
                     Try again, or try another way."
                ))
            }
        };
        if let Some(sink) = &self.images {
            let images: Vec<ImageData> = output.images().cloned().collect();
            if !images.is_empty() {
                sink(name, &input, images);
            }
        }
        output
    }
}

/// The agent crate's result in the AI crate's terms.
fn to_output(result: BridgeResult) -> ToolOutput {
    ToolOutput {
        content: result
            .parts
            .into_iter()
            .map(|part| match part {
                BridgePart::Text(text) => ToolContent::Text { text },
                BridgePart::Image { media_type, data } => ToolContent::Image {
                    image: ImageData { media_type, data },
                },
            })
            .collect(),
        is_error: result.is_error,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use moshionly_agent::{
        Frame, FrameRequest, FrameSource, Host, HostError, SessionHandle, Tools,
    };
    use moshionly_core::{Document, build};
    use serde_json::json;

    use super::*;

    /// A document with one artboard holding one rectangle, and the
    /// artboard's and the rectangle's ids.
    fn small_document() -> (Document, String, String) {
        let mut document = Document::new("Test");
        let (artboard, edits) = build::add_artboard(&document, "Launch", 800.0, 600.0).unwrap();
        document.apply(&edits).unwrap();
        let rect = build::default_layer(moshionly_core::NodeType::Rect).unwrap();
        let (layer, edits) = build::add_layer(&document, &artboard, rect).unwrap();
        document.apply(&edits).unwrap();
        (document, artboard.to_string(), layer.to_string())
    }

    /// Frames that are a fixed stand-in PNG, so no renderer is needed.
    struct StandInFrames;

    impl FrameSource for StandInFrames {
        fn render(&self, _: &FrameRequest<'_>) -> Result<Frame, HostError> {
            Ok(Frame {
                png: b"\x89PNG stand-in".to_vec(),
                width: 4,
                height: 3,
                region: moshionly_core::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 4.0,
                    height: 3.0,
                },
                layer_box: None,
            })
        }
    }

    fn tools(session: &SessionHandle) -> AgentTools {
        let host = Host::new().with_frames(Arc::new(StandInFrames));
        AgentTools::new(ToolBridge::new(Tools::new(session.clone(), host)))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn offers_every_agent_tool() {
        let session = SessionHandle::from_document(Document::new("Empty"));
        let specs = tools(&session).tools();
        let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
        for expected in [
            "get_overview",
            "apply_edits",
            "screenshot",
            "undo_last_agent_change",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} missing from {names:?}"
            );
        }
        let apply = specs
            .iter()
            .find(|spec| spec.name == "apply_edits")
            .unwrap();
        assert_eq!(apply.input_schema["type"], "object");
        assert!(!apply.description.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_api_tool_call_changes_the_document_and_one_undo_restores_it() {
        let (document, _, layer) = small_document();
        let session = SessionHandle::from_document(document);
        let before = session.document();
        let revision = session.revision();

        let output = tools(&session)
            .call(
                "apply_edits",
                json!({
                    "label": "Move the card",
                    "operations": [{ "op": "update", "id": layer, "set": { "x": 321 } }]
                }),
            )
            .await;
        assert!(!output.is_error, "{}", output.joined_text());
        assert!(session.revision() > revision);
        let moved = session.document();
        let node = moved.get(&layer.as_str().into()).unwrap();
        assert_eq!(node.layer().unwrap().x, 321.0);

        let undone = session.undo().unwrap().expect("one step to undo");
        assert_eq!(undone.origin, moshionly_agent::Origin::Agent);
        assert_eq!(*session.document(), *before);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_edit_comes_back_as_an_error_for_the_model() {
        let (document, _, _) = small_document();
        let session = SessionHandle::from_document(document);
        let revision = session.revision();
        let output = tools(&session)
            .call(
                "apply_edits",
                json!({ "operations": [{ "op": "update", "id": "nope", "set": { "x": 1 } }] }),
            )
            .await;
        assert!(output.is_error);
        assert!(!output.joined_text().is_empty());
        assert_eq!(session.revision(), revision);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screenshots_reach_the_model_and_the_image_sink() {
        let (document, artboard, _) = small_document();
        let session = SessionHandle::from_document(document);
        let seen: Arc<Mutex<Vec<(String, Value, usize)>>> = Arc::default();
        let sink_seen = Arc::clone(&seen);
        let executor = tools(&session).with_image_sink(Arc::new(move |name, input, images| {
            sink_seen
                .lock()
                .unwrap()
                .push((name.to_owned(), input.clone(), images.len()));
        }));

        let input = json!({ "artboard": artboard });
        let output = executor.call("screenshot", input.clone()).await;
        assert!(!output.is_error, "{}", output.joined_text());
        let images: Vec<&ImageData> = output.images().collect();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].media_type, "image/png");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![("screenshot".to_owned(), input, 1)]
        );

        // Calls without pictures do not bother the sink.
        executor.call("get_overview", json!({})).await;
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unknown_tool_is_an_error_not_a_panic() {
        let session = SessionHandle::from_document(Document::new("Empty"));
        let output = tools(&session).call("make_coffee", json!({})).await;
        assert!(output.is_error);
    }
}

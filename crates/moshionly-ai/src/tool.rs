//! The bridge between API providers and the app's tools.
//!
//! API providers run the tool loop themselves: the model asks for a tool, the
//! provider calls [`ToolExecutor::call`], and the result goes back to the
//! model. Subscription providers do not use this module; their agent CLI
//! reaches the same tools through the Moshionly MCP server.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::ImageData;

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// The name the model uses to call the tool. Letters, digits, `_` and `-`.
    pub name: String,
    /// What the tool does and when to use it. The model reads this.
    pub description: String,
    /// JSON Schema for the tool's input object.
    pub input_schema: Value,
}

impl ToolSpec {
    /// A tool description.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// One piece of a tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    /// Plain text, usually JSON or a short message.
    Text {
        /// The text.
        text: String,
    },
    /// An image, for example a rendered frame.
    Image {
        /// The image.
        image: ImageData,
    },
}

/// What a tool returned.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolOutput {
    /// The result, in order.
    pub content: Vec<ToolContent>,
    /// True when the tool failed. The content then says what to fix, and the
    /// model gets a chance to try again.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful text result.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text { text: text.into() }],
            is_error: false,
        }
    }

    /// A failed call, with a message that tells the model what went wrong.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text {
                text: message.into(),
            }],
            is_error: true,
        }
    }

    /// Adds a PNG image to the result.
    pub fn with_png(mut self, png: impl Into<Vec<u8>>) -> Self {
        self.content.push(ToolContent::Image {
            image: ImageData::png(png),
        });
        self
    }

    /// All text parts joined with newlines.
    pub fn joined_text(&self) -> String {
        let parts: Vec<&str> = self
            .content
            .iter()
            .filter_map(|part| match part {
                ToolContent::Text { text } => Some(text.as_str()),
                ToolContent::Image { .. } => None,
            })
            .collect();
        parts.join("\n")
    }

    /// The images in the result.
    pub fn images(&self) -> impl Iterator<Item = &ImageData> {
        self.content.iter().filter_map(|part| match part {
            ToolContent::Image { image } => Some(image),
            ToolContent::Text { .. } => None,
        })
    }

    /// A one-line description for the chat panel, such as `ok, 1 image` or
    /// the first line of an error.
    pub fn summary(&self) -> String {
        let text = self.joined_text();
        let images = self.images().count();
        let mut summary = summarize_text(&text);
        if images > 0 {
            if !summary.is_empty() {
                summary.push_str(", ");
            }
            summary.push_str(&format!(
                "{images} image{}",
                if images == 1 { "" } else { "s" }
            ));
        }
        if summary.is_empty() {
            summary.push_str(if self.is_error { "failed" } else { "ok" });
        }
        summary
    }
}

/// The longest summary shown for a tool result, in characters.
const SUMMARY_CHARS: usize = 160;

/// Shortens tool output to its first line, capped at [`SUMMARY_CHARS`].
pub(crate) fn summarize_text(text: &str) -> String {
    let first_line = text.lines().find(|line| !line.trim().is_empty());
    let Some(line) = first_line else {
        return String::new();
    };
    let line = line.trim();
    if line.chars().count() <= SUMMARY_CHARS {
        return line.to_owned();
    }
    let mut short: String = line.chars().take(SUMMARY_CHARS).collect();
    short.push('…');
    short
}

/// The app's tools, as an API provider sees them.
///
/// Implementations must be cheap to share: providers hold an
/// `Arc<dyn ToolExecutor>` and may call several tools at the same time when
/// the model asks for them in one step.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// The tools to offer the model. Called once per request.
    fn tools(&self) -> Vec<ToolSpec>;

    /// Runs one tool. Failures are returned as a [`ToolOutput`] with
    /// `is_error` set, not as a panic, so the model can read the message and
    /// correct itself.
    async fn call(&self, name: &str, input: Value) -> ToolOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_uses_the_first_non_empty_line() {
        let output = ToolOutput::text("\n  applied 3 edits  \nmore detail");
        assert_eq!(output.summary(), "applied 3 edits");
    }

    #[test]
    fn summary_counts_images() {
        let output = ToolOutput::default().with_png(vec![1]).with_png(vec![2]);
        assert_eq!(output.summary(), "2 images");
        let output = ToolOutput::text("frame at 500 ms").with_png(vec![1]);
        assert_eq!(output.summary(), "frame at 500 ms, 1 image");
    }

    #[test]
    fn summary_of_empty_output_states_the_outcome() {
        assert_eq!(ToolOutput::default().summary(), "ok");
        let failed = ToolOutput {
            content: Vec::new(),
            is_error: true,
        };
        assert_eq!(failed.summary(), "failed");
    }

    #[test]
    fn summary_is_cut_on_a_character_boundary() {
        let long = "é".repeat(500);
        let summary = ToolOutput::text(long).summary();
        assert_eq!(summary.chars().count(), SUMMARY_CHARS + 1);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn tool_output_serialises_with_tagged_parts() {
        let output = ToolOutput::text("hi").with_png(vec![1, 2, 3]);
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["content"][0]["type"], "text");
        assert_eq!(json["content"][1]["type"], "image");
        assert_eq!(json["content"][1]["image"]["media_type"], "image/png");
    }
}

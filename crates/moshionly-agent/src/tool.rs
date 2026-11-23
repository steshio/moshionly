//! What a tool looks like from outside: its description, its result and the
//! ways it can fail.
//!
//! These types are plain data with no tie to a transport. The MCP server
//! turns them into protocol messages, and the built-in chat hands them to a
//! model API as they are.

use serde_json::{Value, json};

use crate::host::HostError;

/// A tool as a model sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// The name the model calls the tool by.
    pub name: &'static str,
    /// The name to show a person, for example in a list of tool calls.
    pub title: &'static str,
    /// What the tool does and when to use it. Written for a model.
    pub description: &'static str,
    /// JSON Schema (draft 2020-12) of the tool's input object.
    pub input_schema: Value,
    /// Whether the tool only reads. A tool that is not read-only changes
    /// the document or writes files.
    pub read_only: bool,
    /// Whether the tool can take away or overwrite what is there, as
    /// opposed to only adding to it.
    pub destructive: bool,
}

/// One piece of a tool's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolContent {
    /// Text: a short message, or JSON.
    Text {
        /// The text.
        text: String,
    },
    /// An image, for example a rendered frame.
    Image {
        /// The media type, for example `image/png`.
        media_type: String,
        /// The bytes of the image file.
        data: Vec<u8>,
    },
}

/// What a tool returned.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolOutput {
    /// The pieces of the result, in order.
    pub content: Vec<ToolContent>,
    /// Whether the call failed. The text then says what went wrong and what
    /// to do instead, so the model can try again.
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful result that is one piece of text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text { text: text.into() }],
            is_error: false,
        }
    }

    /// A successful result that is one JSON value, written compactly.
    pub fn json(value: &Value) -> Self {
        Self::text(value.to_string())
    }

    /// A failed call, with a message for the model.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text {
                text: message.into(),
            }],
            is_error: true,
        }
    }

    /// The same result with a PNG image added.
    pub fn with_png(mut self, png: Vec<u8>) -> Self {
        self.content.push(ToolContent::Image {
            media_type: "image/png".to_owned(),
            data: png,
        });
        self
    }

    /// Every text piece, joined with line breaks.
    pub fn joined_text(&self) -> String {
        let texts: Vec<&str> = self
            .content
            .iter()
            .filter_map(|part| match part {
                ToolContent::Text { text } => Some(text.as_str()),
                ToolContent::Image { .. } => None,
            })
            .collect();
        texts.join("\n")
    }

    /// The images in the result, as media type and bytes.
    pub fn images(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.content.iter().filter_map(|part| match part {
            ToolContent::Image { media_type, data } => Some((media_type.as_str(), data.as_slice())),
            ToolContent::Text { .. } => None,
        })
    }
}

/// Why one operation of an `apply_edits` batch was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditFailure {
    /// Position of the operation in the batch, counting from 0.
    pub operation: usize,
    /// The kind of operation: `add`, `update` and so on. Empty when the
    /// operation did not say.
    pub op: String,
    /// The field at fault, as a path inside the operation, such as
    /// `node.fill` or `parent`. `None` when no single field is to blame.
    pub field: Option<String>,
    /// What is wrong.
    pub message: String,
    /// What to do instead. Empty when the message already says.
    pub fix: String,
}

impl EditFailure {
    /// The failure as the JSON a model receives.
    pub fn to_value(&self) -> Value {
        let mut error = json!({ "operation": self.operation });
        if !self.op.is_empty() {
            error["op"] = json!(self.op);
        }
        if let Some(field) = &self.field {
            error["field"] = json!(field);
        }
        error["message"] = json!(self.message);
        if !self.fix.is_empty() {
            error["fix"] = json!(self.fix);
        }
        json!({
            "applied": 0,
            "note": "Nothing was applied: a batch is all or nothing. Correct this operation and send the whole batch again.",
            "error": error,
        })
    }
}

/// Why a tool call failed.
///
/// Every message is written for the model that made the call: it names what
/// was wrong and what would be accepted instead.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// No tool has the name that was called.
    #[error("There is no tool named \"{name}\". The tools are: {}.", .tools.join(", "))]
    UnknownTool {
        /// The name that was called.
        name: String,
        /// The names that exist.
        tools: Vec<&'static str>,
    },
    /// An argument is missing, unknown or of the wrong shape.
    #[error("{message}")]
    Argument {
        /// The argument at fault, as a path such as `times` or
        /// `operations[2].parent`.
        field: String,
        /// What is wrong and what would be accepted.
        message: String,
    },
    /// The call is well formed but cannot be done as asked: a node that
    /// does not exist, a file in the wrong place.
    #[error("{0}")]
    Rejected(String),
    /// The tool needs something this host does not have.
    #[error("{what} not available in this host: {why} {instead}")]
    NotAvailable {
        /// What is missing, as the start of a sentence: `Screenshots are`.
        what: &'static str,
        /// Why it is missing here.
        why: &'static str,
        /// What the model can do instead.
        instead: &'static str,
    },
    /// An operation of an `apply_edits` batch was refused.
    #[error("Operation {} failed, so nothing was applied. {}", .0.operation, .0.message)]
    Edit(Box<EditFailure>),
    /// The host's renderer or exporter failed.
    #[error(transparent)]
    Host(#[from] HostError),
}

impl ToolError {
    /// A [`ToolError::Argument`] about `field`.
    pub(crate) fn argument(field: impl Into<String>, message: impl Into<String>) -> Self {
        ToolError::Argument {
            field: field.into(),
            message: message.into(),
        }
    }

    /// The failure as the result the model receives.
    pub fn to_output(&self) -> ToolOutput {
        match self {
            ToolError::Edit(failure) => ToolOutput::error(failure.to_value().to_string()),
            other => ToolOutput::error(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_images_are_told_apart() {
        let output = ToolOutput::text("frame at 500 ms").with_png(vec![1, 2, 3]);
        assert!(!output.is_error);
        assert_eq!(output.joined_text(), "frame at 500 ms");
        let images: Vec<_> = output.images().collect();
        assert_eq!(images, [("image/png", &[1u8, 2, 3][..])]);
        assert_eq!(
            ToolOutput::json(&json!({ "a": 1 })).joined_text(),
            r#"{"a":1}"#
        );
    }

    #[test]
    fn errors_become_results_the_model_can_act_on() {
        let unknown = ToolError::UnknownTool {
            name: "get_nodes".into(),
            tools: vec!["get_node", "query_nodes"],
        };
        let output = unknown.to_output();
        assert!(output.is_error);
        assert_eq!(
            output.joined_text(),
            "There is no tool named \"get_nodes\". The tools are: get_node, query_nodes."
        );

        let missing = ToolError::NotAvailable {
            what: "Screenshots are",
            why: "no renderer is attached.",
            instead: "Use evaluate to check values as numbers.",
        };
        assert_eq!(
            missing.to_string(),
            "Screenshots are not available in this host: no renderer is attached. Use evaluate \
             to check values as numbers."
        );
    }

    #[test]
    fn an_edit_failure_names_the_operation_the_field_and_the_fix() {
        let failure = EditFailure {
            operation: 2,
            op: "add".into(),
            field: Some("node.fil".into()),
            message: "\"fil\" is not a field of rect.".into(),
            fix: "Use \"fill\".".into(),
        };
        let output = ToolError::Edit(Box::new(failure)).to_output();
        assert!(output.is_error);
        let value: Value = serde_json::from_str(&output.joined_text()).unwrap();
        assert_eq!(value["applied"], 0);
        assert_eq!(
            value["error"],
            json!({
                "operation": 2,
                "op": "add",
                "field": "node.fil",
                "message": "\"fil\" is not a field of rect.",
                "fix": "Use \"fill\"."
            })
        );
    }
}

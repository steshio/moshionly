//! Builds Chat Completions request bodies.

use serde_json::{Map, Value, json};

use super::stream::AssistantTurn;
use crate::api::ToolCall;
use crate::chat::{ChatMessage, ImageData, Role};
use crate::tool::{ToolOutput, ToolSpec};

/// Everything about a turn that stays the same from request to request.
#[derive(Debug, Clone)]
pub(super) struct RequestShape<'a> {
    pub model: &'a str,
    pub tools: &'a [ToolSpec],
    /// Ask for token counts in the stream.
    pub stream_usage: bool,
}

/// The request body for the conversation in `messages`.
pub(super) fn body(shape: &RequestShape<'_>, messages: &[Value]) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(shape.model));
    body.insert("stream".into(), json!(true));
    if shape.stream_usage {
        body.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    if !shape.tools.is_empty() {
        let tools: Vec<Value> = shape
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    body.insert("messages".into(), Value::Array(messages.to_vec()));
    Value::Object(body)
}

/// The system prompt and conversation as Chat Completions messages.
///
/// With `images` off, pictures are left out and the text says so, which is
/// kinder than a server error from a model that cannot see.
pub(super) fn initial_messages(
    system_prompt: Option<&str>,
    conversation: &[ChatMessage],
    images: bool,
) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = system_prompt.map(str::trim).filter(|text| !text.is_empty()) {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in conversation {
        match message.role {
            Role::Assistant if message.text.trim().is_empty() => {}
            Role::Assistant => {
                messages.push(json!({ "role": "assistant", "content": message.text }));
            }
            Role::User => {
                if let Some(content) = user_content(message, images) {
                    messages.push(json!({ "role": "user", "content": content }));
                }
            }
        }
    }
    messages
}

/// A user message's content: plain text, or text and image parts.
fn user_content(message: &ChatMessage, images: bool) -> Option<Value> {
    if message.images.is_empty() {
        return (!message.text.trim().is_empty()).then(|| json!(message.text));
    }
    if !images {
        let note = omitted_note(message.images.len());
        return Some(json!(format!("{}\n\n{note}", message.text).trim()));
    }
    let mut parts = Vec::new();
    if !message.text.trim().is_empty() {
        parts.push(json!({ "type": "text", "text": message.text }));
    }
    parts.extend(message.images.iter().map(image_part));
    Some(Value::Array(parts))
}

fn image_part(image: &ImageData) -> Value {
    json!({
        "type": "image_url",
        "image_url": {
            "url": format!("data:{};base64,{}", image.media_type, image.to_base64())
        }
    })
}

fn omitted_note(count: usize) -> String {
    let (noun, verb) = if count == 1 {
        ("image", "was")
    } else {
        ("images", "were")
    };
    format!("[{count} {noun} {verb} left out: this model cannot be sent images.]")
}

/// The assistant message that asked for tools, to send back with their
/// results.
pub(super) fn assistant_message(turn: &AssistantTurn) -> Value {
    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert(
        "content".into(),
        if turn.text.is_empty() {
            Value::Null
        } else {
            json!(turn.text)
        },
    );
    if !turn.raw_tool_calls.is_empty() {
        message.insert(
            "tool_calls".into(),
            Value::Array(turn.raw_tool_calls.clone()),
        );
    }
    Value::Object(message)
}

/// The messages that answer a step's tool calls: one `tool` message per
/// call, in order.
///
/// The format has no place for images in a tool message. Where the model
/// can see, they follow in a user message; where it cannot, the tool
/// message says they were left out.
pub(super) fn tool_result_messages(
    calls: &[ToolCall],
    outputs: &[ToolOutput],
    images: bool,
) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut pictures = Vec::new();
    for (call, output) in calls.iter().zip(outputs) {
        let mut text = output.joined_text();
        let count = output.images().count();
        if count > 0 {
            let note = if images {
                let (noun, verb) = if count == 1 {
                    ("image", "follows")
                } else {
                    ("images", "follow")
                };
                format!("[{count} {noun} from this tool {verb} in the next message.]")
            } else {
                omitted_note(count)
            };
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&note);
        }
        if text.is_empty() {
            text.push_str(if output.is_error { "failed" } else { "ok" });
        }
        messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": text }));
        if images && count > 0 {
            pictures.push(json!({
                "type": "text",
                "text": format!("Images returned by the {} tool:", call.name),
            }));
            pictures.extend(output.images().map(image_part));
        }
    }
    if !pictures.is_empty() {
        messages.push(json!({ "role": "user", "content": pictures }));
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape<'a>(tools: &'a [ToolSpec]) -> RequestShape<'a> {
        RequestShape {
            model: "gpt-6.2-sol",
            tools,
            stream_usage: true,
        }
    }

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_owned(),
            name: name.to_owned(),
            input: Ok(json!({})),
        }
    }

    #[test]
    fn the_body_streams_and_asks_for_token_counts() {
        let body = body(&shape(&[]), &[json!({"role":"user","content":"hi"})]);
        assert_eq!(body["model"], "gpt-6.2-sol");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"], json!({ "include_usage": true }));
        assert!(body.get("tools").is_none());
        assert!(body.get("max_tokens").is_none());

        let mut quiet = shape(&[]);
        quiet.stream_usage = false;
        assert!(super::body(&quiet, &[]).get("stream_options").is_none());
    }

    #[test]
    fn tools_are_offered_as_functions() {
        let tools = [ToolSpec::new(
            "apply_edits",
            "Applies a batch of edits.",
            json!({ "type": "object" }),
        )];
        let body = body(&shape(&tools), &[]);
        assert_eq!(
            body["tools"],
            json!([{ "type": "function", "function": {
                "name": "apply_edits",
                "description": "Applies a batch of edits.",
                "parameters": { "type": "object" } } }])
        );
    }

    #[test]
    fn the_system_prompt_comes_first() {
        let messages = initial_messages(
            Some("You edit motion designs."),
            &[
                ChatMessage::user("one"),
                ChatMessage::assistant("reply"),
                ChatMessage::assistant("  "),
                ChatMessage::user("two"),
            ],
            true,
        );
        assert_eq!(
            messages,
            vec![
                json!({"role":"system","content":"You edit motion designs."}),
                json!({"role":"user","content":"one"}),
                json!({"role":"assistant","content":"reply"}),
                json!({"role":"user","content":"two"}),
            ]
        );
        assert_eq!(initial_messages(Some(" "), &[], true), Vec::<Value>::new());
    }

    #[test]
    fn images_become_data_urls_where_the_model_can_see() {
        let message = ChatMessage::user("match this").with_image(ImageData::png(vec![1, 2, 3]));
        let messages = initial_messages(None, std::slice::from_ref(&message), true);
        assert_eq!(
            messages[0]["content"],
            json!([
                { "type": "text", "text": "match this" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AQID" } }
            ])
        );

        let messages = initial_messages(None, &[message], false);
        assert_eq!(
            messages[0]["content"],
            "match this\n\n[1 image was left out: this model cannot be sent images.]"
        );
    }

    #[test]
    fn each_call_gets_a_tool_message_and_images_follow_as_a_user_message() {
        let calls = [call("call_1", "screenshot"), call("call_2", "apply_edits")];
        let outputs = [
            ToolOutput::text("frame at 0 ms").with_png(vec![1, 2, 3]),
            ToolOutput::error("node cd34 does not exist"),
        ];
        let messages = tool_result_messages(&calls, &outputs, true);
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[0],
            json!({"role":"tool","tool_call_id":"call_1",
                "content":"frame at 0 ms\n\n[1 image from this tool follows in the next message.]"})
        );
        assert_eq!(messages[1]["content"], "node cd34 does not exist");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(
            messages[2]["content"][0]["text"],
            "Images returned by the screenshot tool:"
        );
        assert_eq!(
            messages[2]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AQID"
        );
    }

    #[test]
    fn tool_images_degrade_to_a_note_where_the_model_cannot_see() {
        let calls = [call("call_1", "screenshot"), call("call_2", "export")];
        let outputs = [
            ToolOutput::default().with_png(vec![1]).with_png(vec![2]),
            ToolOutput::default(),
        ];
        let messages = tool_result_messages(&calls, &outputs, false);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0]["content"],
            "[2 images were left out: this model cannot be sent images.]"
        );
        assert_eq!(messages[1]["content"], "ok");
    }
}

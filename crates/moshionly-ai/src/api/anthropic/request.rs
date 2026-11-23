//! Builds Messages API request bodies.
//!
//! The body is put together the same way for every request of a turn, and
//! messages are only ever appended. Both matter: prompt caching matches on
//! an unchanged prefix, and newer models check that the history a thinking
//! block was produced in comes back unedited.

use serde_json::{Map, Value, json};

use crate::api::ToolCall;
use crate::chat::{ChatMessage, ImageData, Role};
use crate::tool::{ToolContent, ToolOutput, ToolSpec};

/// The beta that turns on server-side refusal fallbacks.
pub(super) const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Models that accept the `fallbacks` parameter.
const FALLBACK_MODELS: &[&str] = &[
    "claude-fable-5-2",
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
];

/// Model families that predate adaptive thinking. They are sent no
/// `thinking` or `effort` settings and answer without extended thinking.
const PRE_ADAPTIVE_MODELS: &[&str] = &[
    "claude-haiku-4-5",
    "claude-sonnet-4-5",
    "claude-opus-4-5",
    "claude-mythos-preview",
    "claude-3",
];

/// Everything about a turn that stays the same from request to request.
#[derive(Debug, Clone)]
pub(super) struct RequestShape<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    pub effort: Option<&'a str>,
    pub system: Option<&'a str>,
    pub tools: &'a [ToolSpec],
    /// The request goes to Anthropic's own API, which accepts fields that a
    /// gateway in front of it may not.
    pub first_party: bool,
    /// Ask the API to re-run a request that a safety classifier declined on
    /// another model.
    pub refusal_fallback: bool,
}

impl RequestShape<'_> {
    /// True when this request carries the `fallbacks` parameter, which needs
    /// its beta header.
    pub fn uses_fallbacks(&self) -> bool {
        self.first_party && self.refusal_fallback && FALLBACK_MODELS.contains(&self.model)
    }
}

/// True when `model` takes `thinking: {"type": "adaptive"}`.
pub(super) fn supports_adaptive_thinking(model: &str) -> bool {
    !PRE_ADAPTIVE_MODELS
        .iter()
        .any(|prefix| model.starts_with(prefix))
}

/// The request body for the conversation in `messages`.
pub(super) fn body(shape: &RequestShape<'_>, messages: &[Value]) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(shape.model));
    body.insert("max_tokens".into(), json!(shape.max_tokens));
    body.insert("stream".into(), json!(true));

    if let Some(system) = shape.system.map(str::trim).filter(|text| !text.is_empty()) {
        // Tools are rendered before the system prompt, so this one mark
        // caches both.
        body.insert(
            "system".into(),
            json!([{ "type": "text", "text": system, "cache_control": cache_mark() }]),
        );
    }
    if !shape.tools.is_empty() {
        let tools: Vec<Value> = shape
            .tools
            .iter()
            .map(|tool| tool_definition(tool, shape.first_party))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if supports_adaptive_thinking(shape.model) {
        // Summaries, so that the reasoning can be shown while it happens.
        body.insert(
            "thinking".into(),
            json!({ "type": "adaptive", "display": "summarized" }),
        );
        if let Some(effort) = shape.effort {
            body.insert("output_config".into(), json!({ "effort": effort }));
        }
    }
    if shape.uses_fallbacks() {
        body.insert("fallbacks".into(), json!("default"));
    }

    let mut messages = messages.to_vec();
    if shape.first_party {
        // The API keeps a cache mark on the end of the conversation by
        // itself, moving it forward as the turn grows.
        body.insert("cache_control".into(), cache_mark());
    } else {
        mark_last_block(&mut messages);
    }
    body.insert("messages".into(), Value::Array(messages));
    Value::Object(body)
}

fn cache_mark() -> Value {
    json!({ "type": "ephemeral" })
}

/// Puts a cache mark on the last block of the conversation.
fn mark_last_block(messages: &mut [Value]) {
    if let Some(block) = messages
        .last_mut()
        .and_then(|message| message.get_mut("content"))
        .and_then(Value::as_array_mut)
        .and_then(|blocks| blocks.last_mut())
        .and_then(Value::as_object_mut)
    {
        block.insert("cache_control".into(), cache_mark());
    }
}

fn tool_definition(tool: &ToolSpec, first_party: bool) -> Value {
    let mut definition = json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.input_schema,
    });
    if first_party {
        // Large arguments, such as a batch of edits, then arrive as they are
        // written. The price is that the API no longer checks them, which is
        // why the stream reader parses them strictly.
        definition["eager_input_streaming"] = json!(true);
    }
    definition
}

/// The conversation as API messages: roles alternate, nothing is empty, and
/// the first message is the user's.
pub(super) fn initial_messages(conversation: &[ChatMessage]) -> Vec<Value> {
    let mut messages: Vec<(Role, Vec<Value>)> = Vec::new();
    for message in conversation {
        let blocks = match message.role {
            Role::User => user_blocks(message),
            Role::Assistant => text_blocks(&message.text),
        };
        if blocks.is_empty() {
            continue;
        }
        match messages.last_mut() {
            // Two messages in a row from the same side become one.
            Some((role, existing)) if *role == message.role => existing.extend(blocks),
            // An answer with no question before it has nothing to follow.
            None if message.role == Role::Assistant => {}
            _ => messages.push((message.role, blocks)),
        }
    }
    messages
        .into_iter()
        .map(|(role, blocks)| {
            let role = match role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            json!({ "role": role, "content": blocks })
        })
        .collect()
}

/// Images first, then the text that talks about them.
fn user_blocks(message: &ChatMessage) -> Vec<Value> {
    let mut blocks: Vec<Value> = message.images.iter().map(image_block).collect();
    blocks.extend(text_blocks(&message.text));
    blocks
}

fn text_blocks(text: &str) -> Vec<Value> {
    if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![json!({ "type": "text", "text": text })]
    }
}

fn image_block(image: &ImageData) -> Value {
    json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": image.media_type,
            "data": image.to_base64(),
        }
    })
}

/// The user message that answers a step's tool calls: every result in one
/// message, in the order of the calls.
pub(super) fn tool_results_message(calls: &[ToolCall], outputs: &[ToolOutput]) -> Value {
    let results: Vec<Value> = calls
        .iter()
        .zip(outputs)
        .map(|(call, output)| {
            let mut content: Vec<Value> = output
                .content
                .iter()
                .filter_map(|part| match part {
                    ToolContent::Text { text } if text.is_empty() => None,
                    ToolContent::Text { text } => Some(json!({ "type": "text", "text": text })),
                    ToolContent::Image { image } => Some(image_block(image)),
                })
                .collect();
            if content.is_empty() {
                let text = if output.is_error { "failed" } else { "ok" };
                content.push(json!({ "type": "text", "text": text }));
            }
            let mut result = json!({
                "type": "tool_result",
                "tool_use_id": call.id,
                "content": content,
            });
            if output.is_error {
                result["is_error"] = json!(true);
            }
            result
        })
        .collect();
    json!({ "role": "user", "content": results })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape<'a>(tools: &'a [ToolSpec]) -> RequestShape<'a> {
        RequestShape {
            model: "claude-opus-5-5",
            max_tokens: 64000,
            effort: None,
            system: Some("You edit motion designs."),
            tools,
            first_party: true,
            refusal_fallback: true,
        }
    }

    fn one_tool() -> Vec<ToolSpec> {
        vec![ToolSpec::new(
            "apply_edits",
            "Applies a batch of edits.",
            json!({ "type": "object", "properties": {} }),
        )]
    }

    fn user(text: &str) -> Vec<Value> {
        initial_messages(&[ChatMessage::user(text)])
    }

    #[test]
    fn the_body_streams_with_the_model_and_token_limit() {
        let body = body(&shape(&[]), &user("hi"));
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["max_tokens"], 64000);
        assert_eq!(body["stream"], true);
        assert!(body.get("tools").is_none());
        // Sampling settings are rejected by the current API version.
        for removed in ["temperature", "top_p", "top_k"] {
            assert!(body.get(removed).is_none(), "{removed}");
        }
    }

    #[test]
    fn the_system_prompt_is_a_cached_block() {
        let body = body(&shape(&[]), &user("hi"));
        assert_eq!(
            body["system"],
            json!([{ "type": "text", "text": "You edit motion designs.",
                     "cache_control": { "type": "ephemeral" } }])
        );
        let mut no_prompt = shape(&[]);
        no_prompt.system = Some("  ");
        assert!(super::body(&no_prompt, &user("hi")).get("system").is_none());
    }

    #[test]
    fn current_models_think_adaptively_with_summaries() {
        let body = body(&shape(&[]), &user("hi"));
        assert_eq!(
            body["thinking"],
            json!({ "type": "adaptive", "display": "summarized" })
        );
        assert!(body.get("output_config").is_none());

        let mut with_effort = shape(&[]);
        with_effort.effort = Some("high");
        let body = super::body(&with_effort, &user("hi"));
        assert_eq!(body["output_config"], json!({ "effort": "high" }));
    }

    #[test]
    fn older_models_get_no_thinking_or_effort_settings() {
        for model in [
            "claude-haiku-4-5",
            "claude-haiku-4-5-20251001",
            "claude-opus-4-5",
        ] {
            let mut older = shape(&[]);
            older.model = model;
            older.effort = Some("high");
            let body = body(&older, &user("hi"));
            assert!(body.get("thinking").is_none(), "{model}");
            assert!(body.get("output_config").is_none(), "{model}");
        }
        for model in [
            "claude-opus-5-5",
            "claude-sonnet-4-6",
            "claude-haiku-5-5",
            "claude-fable-5-2",
        ] {
            assert!(supports_adaptive_thinking(model), "{model}");
        }
    }

    #[test]
    fn the_real_api_gets_eager_tool_input_automatic_caching_and_fallbacks() {
        let tools = one_tool();
        let shape = shape(&tools);
        let body = body(&shape, &user("hi"));
        assert_eq!(body["tools"][0]["name"], "apply_edits");
        assert_eq!(body["tools"][0]["eager_input_streaming"], true);
        assert_eq!(body["cache_control"], json!({ "type": "ephemeral" }));
        assert_eq!(body["fallbacks"], "default");
        assert!(shape.uses_fallbacks());
        // No mark inside the messages: the API places it.
        assert!(
            body["messages"][0]["content"][0]
                .get("cache_control")
                .is_none()
        );
    }

    #[test]
    fn a_gateway_gets_only_fields_every_deployment_accepts() {
        let tools = one_tool();
        let mut gateway = shape(&tools);
        gateway.first_party = false;
        let body = body(&gateway, &user("hi"));
        assert!(body["tools"][0].get("eager_input_streaming").is_none());
        assert!(body.get("cache_control").is_none());
        assert!(body.get("fallbacks").is_none());
        assert!(!gateway.uses_fallbacks());
        // The end of the conversation is marked for caching by hand.
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"],
            json!({ "type": "ephemeral" })
        );
    }

    #[test]
    fn fallbacks_are_only_sent_for_models_that_take_them() {
        for (model, expected) in [
            ("claude-opus-5-5", true),
            ("claude-fable-5-2", true),
            ("claude-sonnet-5-5", true),
            ("claude-haiku-5-5", false),
            ("claude-opus-4-8", false),
        ] {
            let mut shape = shape(&[]);
            shape.model = model;
            assert_eq!(shape.uses_fallbacks(), expected, "{model}");
        }
        let mut declined = shape(&[]);
        declined.refusal_fallback = false;
        assert!(body(&declined, &user("hi")).get("fallbacks").is_none());
    }

    #[test]
    fn the_same_inputs_always_give_the_same_bytes() {
        let tools = one_tool();
        let first = body(&shape(&tools), &user("hi")).to_string();
        let second = body(&shape(&tools), &user("hi")).to_string();
        assert_eq!(first, second);
    }

    #[test]
    fn images_come_before_the_text_of_a_user_message() {
        let messages = initial_messages(&[
            ChatMessage::user("match this").with_image(ImageData::png(vec![1, 2, 3]))
        ]);
        assert_eq!(
            messages,
            vec![json!({ "role": "user", "content": [
                { "type": "image", "source": {
                    "type": "base64", "media_type": "image/png", "data": "AQID" } },
                { "type": "text", "text": "match this" }
            ] })]
        );
    }

    #[test]
    fn messages_alternate_and_start_with_the_user() {
        let messages = initial_messages(&[
            ChatMessage::assistant("stray greeting"),
            ChatMessage::user("one"),
            ChatMessage::user("two"),
            ChatMessage::assistant(""),
            ChatMessage::assistant("reply"),
            ChatMessage::user("  "),
            ChatMessage::user("three"),
        ]);
        let roles: Vec<&str> = messages
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(messages[0]["content"].as_array().unwrap().len(), 2);
        assert_eq!(messages[2]["content"][0]["text"], "three");
    }

    #[test]
    fn all_results_of_a_step_go_back_in_one_message() {
        let calls = [
            ToolCall {
                id: "toolu_1".to_owned(),
                name: "screenshot".to_owned(),
                input: Ok(json!({})),
            },
            ToolCall {
                id: "toolu_2".to_owned(),
                name: "apply_edits".to_owned(),
                input: Ok(json!({})),
            },
            ToolCall {
                id: "toolu_3".to_owned(),
                name: "export".to_owned(),
                input: Ok(json!({})),
            },
        ];
        let outputs = [
            ToolOutput::text("frame at 0 ms").with_png(vec![1, 2, 3]),
            ToolOutput::error("node cd34 does not exist"),
            ToolOutput::default(),
        ];
        let message = tool_results_message(&calls, &outputs);
        assert_eq!(message["role"], "user");
        let results = message["content"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["tool_use_id"], "toolu_1");
        assert_eq!(results[0]["content"][1]["type"], "image");
        assert_eq!(results[0]["content"][1]["source"]["data"], "AQID");
        assert!(results[0].get("is_error").is_none());
        assert_eq!(results[1]["is_error"], true);
        assert_eq!(results[1]["content"][0]["text"], "node cd34 does not exist");
        // A result with nothing in it still says something.
        assert_eq!(results[2]["content"][0]["text"], "ok");
    }
}

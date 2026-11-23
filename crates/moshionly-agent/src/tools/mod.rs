//! The tools an agent works with.
//!
//! [`Tools`] is the whole surface: [`Tools::list`] describes the tools and
//! [`Tools::call`] runs one. The MCP server and the built-in chat are both
//! thin layers over those two functions, so an agent gets the same tools
//! whichever way it arrives.
//!
//! Each tool lives in its own module as a [`ToolDef`]: a name, a description
//! written for a model, a JSON Schema for its input and a function that does
//! the work. Adding a tool is writing such a module and listing it in
//! [`TOOLS`].

mod apply;
mod args;
mod common;
mod evaluate;
mod export;
mod files;
mod fonts;
mod image;
mod node;
mod overview;
mod query;
mod schema;
mod screenshot;
#[cfg(test)]
pub(crate) mod testing;
mod undo;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::host::Host;
use crate::session::SessionHandle;
use crate::tool::{ToolError, ToolOutput, ToolSpec};
use args::Args;

/// Limits and locations for the tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsConfig {
    /// The most text one result may carry, in characters. Tools that could
    /// return more cut their answer short and say how to ask for less.
    pub max_result_chars: usize,
    /// The folder exports are written to. `None` uses a folder named
    /// `exports` next to the project file.
    pub export_dir: Option<PathBuf>,
    /// Whether `export` may write to any path the agent names. When false,
    /// which is the default, exports stay inside the export folder.
    pub export_anywhere: bool,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            max_result_chars: 16_000,
            export_dir: None,
            export_anywhere: false,
        }
    }
}

/// What a tool gets to work with.
pub(crate) struct Context<'a> {
    /// The document the tools read and edit.
    pub session: &'a SessionHandle,
    /// What the host can do.
    pub host: &'a Host,
    /// Limits and locations.
    pub config: &'a ToolsConfig,
}

/// One tool: how a model sees it, and the function that runs it.
pub(crate) struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub read_only: bool,
    /// The JSON Schema of the input. Its `properties` are also the list of
    /// argument names the tool accepts.
    pub schema: fn() -> Value,
    pub run: fn(&Context<'_>, &Args) -> Result<ToolOutput, ToolError>,
}

/// Every tool, in the order they are listed to a model. The order is fixed
/// so that clients can cache the list.
const TOOLS: &[&ToolDef] = &[
    &overview::GET_OVERVIEW,
    &query::QUERY_NODES,
    &node::GET_NODE,
    &schema::GET_SCHEMA,
    &apply::APPLY_EDITS,
    &screenshot::SCREENSHOT,
    &evaluate::EVALUATE,
    &fonts::LIST_FONTS,
    &image::IMPORT_IMAGE,
    &export::EXPORT,
    &undo::UNDO_LAST_AGENT_CHANGE,
];

/// The JSON Schema of an input object with these properties, of which
/// `required` have to be given. Other properties are refused.
pub(crate) fn input_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

struct Inner {
    session: SessionHandle,
    host: Host,
    config: ToolsConfig,
}

/// The tools of one session.
///
/// Cloning is cheap, and every clone works on the same session. Calls may
/// come from several threads at once.
///
/// ```
/// use moshionly_agent::{Host, SessionHandle, Tools};
/// use moshionly_core::Document;
/// use serde_json::json;
///
/// let session = SessionHandle::from_document(Document::new("Hello"));
/// let tools = Tools::new(session, Host::new());
///
/// let output = tools.call("no_such_tool", json!({}));
/// assert!(output.is_error);
/// ```
#[derive(Clone)]
pub struct Tools {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Tools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tools")
            .field("session", &self.inner.session)
            .field("host", &self.inner.host)
            .finish()
    }
}

impl Tools {
    /// The tools for `session`, using what `host` can do.
    pub fn new(session: SessionHandle, host: Host) -> Self {
        Self::with_config(session, host, ToolsConfig::default())
    }

    /// The tools for `session` with other limits and locations.
    pub fn with_config(session: SessionHandle, host: Host, config: ToolsConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                session,
                host,
                config,
            }),
        }
    }

    /// The session the tools work on.
    pub fn session(&self) -> &SessionHandle {
        &self.inner.session
    }

    /// Every tool, always in the same order.
    pub fn list(&self) -> Vec<ToolSpec> {
        TOOLS
            .iter()
            .map(|def| ToolSpec {
                name: def.name,
                title: def.title,
                description: def.description,
                input_schema: (def.schema)(),
                read_only: def.read_only,
            })
            .collect()
    }

    /// Whether a tool with this name exists.
    pub fn has(&self, name: &str) -> bool {
        TOOLS.iter().any(|def| def.name == name)
    }

    /// Runs the tool `name` with `arguments`, a JSON object.
    ///
    /// This never fails as a function: an unknown tool, bad arguments, a
    /// refused edit or a broken renderer all come back as a result with
    /// `is_error` set and a message that says what to do instead.
    pub fn call(&self, name: &str, arguments: Value) -> ToolOutput {
        match TOOLS.iter().find(|def| def.name == name) {
            Some(def) => self.run(def, &arguments),
            None => ToolError::UnknownTool {
                name: name.to_owned(),
                tools: TOOLS.iter().map(|def| def.name).collect(),
            }
            .to_output(),
        }
    }

    fn run(&self, def: &ToolDef, arguments: &Value) -> ToolOutput {
        let context = Context {
            session: &self.inner.session,
            host: &self.inner.host,
            config: &self.inner.config,
        };
        let schema = (def.schema)();
        let allowed: Vec<&str> = schema["properties"]
            .as_object()
            .map(|properties| properties.keys().map(String::as_str).collect())
            .unwrap_or_default();
        // A bug in a tool must not take the editor down with it.
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let args = Args::new(arguments, &allowed)?;
            (def.run)(&context, &args)
        }));
        match outcome {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => error.to_output(),
            Err(_) => ToolOutput::error(format!(
                "The tool {} stopped on an internal error and changed nothing. This is a bug in \
                 Moshionly, not in the call; try a simpler request.",
                def.name
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::Document;

    fn tools() -> Tools {
        Tools::new(
            SessionHandle::from_document(Document::new("Test")),
            Host::new(),
        )
    }

    #[test]
    fn an_unknown_tool_is_an_error_result_that_lists_the_tools() {
        let tools = tools();
        let output = tools.call("fly", json!({}));
        assert!(output.is_error);
        assert!(
            output
                .joined_text()
                .starts_with("There is no tool named \"fly\". The tools are: ")
        );
        assert!(!tools.has("fly"));
    }

    #[test]
    fn tool_names_are_distinct_and_schemas_are_closed_objects() {
        let specs = tools().list();
        let mut names: Vec<&str> = specs.iter().map(|spec| spec.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), specs.len());
        for spec in &specs {
            assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
            assert_eq!(
                spec.input_schema["additionalProperties"], false,
                "{}",
                spec.name
            );
            assert!(spec.input_schema["properties"].is_object(), "{}", spec.name);
            assert!(spec.description.len() > 40, "{}", spec.name);
            for required in spec.input_schema["required"].as_array().unwrap() {
                let name = required.as_str().unwrap();
                assert!(
                    spec.input_schema["properties"].get(name).is_some(),
                    "{}: required \"{name}\" is not a property",
                    spec.name
                );
            }
        }
    }

    static ECHO: ToolDef = ToolDef {
        name: "echo",
        title: "Echo",
        description: "Repeats its text, or fails on request.",
        read_only: true,
        schema: || input_schema(json!({ "text": { "type": "string" } }), &["text"]),
        run: |_, args| {
            let text = args.required_string("text", "what to repeat")?;
            assert!(text != "panic", "asked to panic");
            Ok(ToolOutput::text(text))
        },
    };

    #[test]
    fn a_tool_gets_checked_arguments() {
        let tools = tools();
        assert_eq!(
            tools.run(&ECHO, &json!({ "text": "hi" })).joined_text(),
            "hi"
        );
        let output = tools.run(&ECHO, &json!({ "txt": "hi" }));
        assert!(output.is_error);
        assert_eq!(
            output.joined_text(),
            "\"txt\" is not an argument of this tool. Did you mean \"text\"? It takes: text."
        );
        let output = tools.run(&ECHO, &json!({}));
        assert_eq!(
            output.joined_text(),
            "\"text\" is required: what to repeat."
        );
    }

    #[test]
    fn a_panicking_tool_is_an_error_result() {
        let tools = tools();
        let output = tools.run(&ECHO, &json!({ "text": "panic" }));
        assert!(output.is_error);
        assert!(output.joined_text().contains("internal error"));
        // The tools still work afterwards.
        assert_eq!(
            tools.run(&ECHO, &json!({ "text": "ok" })).joined_text(),
            "ok"
        );
    }
}

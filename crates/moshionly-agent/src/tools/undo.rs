//! `undo_last_agent_change`: take back the most recent agent edit, and
//! nothing the person did.

use serde_json::json;

use super::args::Args;
use super::{Context, ToolDef, input_schema};
use crate::session::Origin;
use crate::tool::{ToolError, ToolOutput};

pub(crate) static UNDO_LAST_AGENT_CHANGE: ToolDef = ToolDef {
    name: "undo_last_agent_change",
    title: "Undo the last agent change",
    description: "Undoes the most recent apply_edits call (or other agent change) as a whole, \
        when it is still the latest change to the file. It never undoes the person's own work: \
        if they changed something since, it refuses and says what their change was, and you \
        should make the reverse edit with apply_edits instead.",
    read_only: false,
    schema: || input_schema(json!({}), &[]),
    run,
};

fn run(context: &Context<'_>, _args: &Args) -> Result<ToolOutput, ToolError> {
    context.session.write(|session| {
        if session.in_gesture() {
            return Err(ToolError::Rejected(
                "The person is in the middle of changing something (a drag, for example), so \
                 nothing was undone. Try again in a moment."
                    .to_owned(),
            ));
        }
        let Some(last) = session.undo_steps().last() else {
            return Err(ToolError::Rejected(
                "There is nothing to undo: the file has no changes since it was opened.".to_owned(),
            ));
        };
        if last.origin != Origin::Agent {
            let who = match last.origin {
                Origin::Import => "an import from another tool",
                _ => "the person",
            };
            let what = if last.summary.is_empty() {
                String::new()
            } else {
                format!(" (\"{}\")", last.summary)
            };
            return Err(ToolError::Rejected(format!(
                "The most recent change{what} was made by {who}, not by an agent. Undoing it \
                 would take back their work, so nothing was undone. Make the reverse change with \
                 apply_edits instead."
            )));
        }
        match session.undo() {
            Ok(Some(step)) => Ok(ToolOutput::json(&json!({
                "undone": step.summary,
                "revision": session.revision(),
            }))),
            Ok(None) => Err(ToolError::Rejected("There is nothing to undo.".to_owned())),
            Err(error) => Err(ToolError::Rejected(format!(
                "The change could not be undone, and the undo history was cleared: {error}"
            ))),
        }
    })
}

#[cfg(test)]
mod tests {
    use moshionly_core::{Edit, NodeId};
    use serde_json::json;

    use crate::Origin;
    use crate::tools::testing::{call, call_error, demo_tools};

    fn x_of_badge(tools: &crate::Tools) -> serde_json::Value {
        tools
            .session()
            .document()
            .get(&NodeId::from("badge"))
            .unwrap()
            .to_full_value()["x"]
            .clone()
    }

    #[test]
    fn undoes_the_last_agent_batch_as_a_whole() {
        let tools = demo_tools();
        call(
            &tools,
            "apply_edits",
            json!({ "label": "Tidy up", "operations": [
                { "op": "update", "id": "badge", "set": { "x": 80 } },
                { "op": "remove", "id": "dot" },
            ]}),
        );
        let result = call(&tools, "undo_last_agent_change", json!({}));
        assert_eq!(result["undone"], "Tidy up");
        assert_eq!(x_of_badge(&tools), 40);
        assert!(tools.session().document().contains(&NodeId::from("dot")));
        // The undone batch can be redone by the person.
        assert!(tools.session().read(|session| session.can_redo()));
    }

    #[test]
    fn never_undoes_the_persons_work() {
        let tools = demo_tools();
        call(
            &tools,
            "apply_edits",
            json!({ "operations": [{ "op": "update", "id": "badge", "set": { "x": 80 } }] }),
        );
        tools
            .session()
            .apply(Origin::User, "Nudge badge", &[Edit::set("badge", "x", 90)])
            .unwrap();
        let error = call_error(&tools, "undo_last_agent_change", json!({}));
        assert!(
            error.starts_with(
                "The most recent change (\"Nudge badge\") was made by the person, not by an agent."
            ),
            "{error}"
        );
        assert_eq!(x_of_badge(&tools), 90);
    }

    #[test]
    fn says_when_there_is_nothing_to_undo_or_a_drag_is_open() {
        let tools = demo_tools();
        let error = call_error(&tools, "undo_last_agent_change", json!({}));
        assert!(error.starts_with("There is nothing to undo"), "{error}");

        call(
            &tools,
            "apply_edits",
            json!({ "operations": [{ "op": "update", "id": "badge", "set": { "x": 80 } }] }),
        );
        tools
            .session()
            .write(|session| session.begin_gesture(Origin::User, "Drag"));
        let error = call_error(&tools, "undo_last_agent_change", json!({}));
        assert!(
            error.contains("in the middle of changing something"),
            "{error}"
        );
        assert_eq!(x_of_badge(&tools), 80);
    }
}

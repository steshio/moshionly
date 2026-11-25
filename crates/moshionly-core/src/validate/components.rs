//! The rules that tie instances to their main components.
//!
//! - An instance's `component` names a `component` layer, in any artboard.
//! - A component is never inside another component.
//! - A component never shows itself: it holds no instance of itself, and no
//!   instance of a component that shows it.
//!
//! The first rule is about one node and is checked with its placement. The
//! other two can be broken by a node that is not touched at all (a group
//! with an instance in it is moved into a component, say), so they are
//! checked over the whole document.

use std::collections::{HashMap, HashSet};

use super::{Problem, ProblemKind};
use crate::document::Document;
use crate::id::NodeId;
use crate::model::{Node, NodeType};

/// Checks that an instance names a component. Every other node passes.
pub(crate) fn check_reference(document: &Document, node: &Node) -> Result<(), Problem> {
    let Some(component) = node.component() else {
        return Ok(());
    };
    let label = node.label();
    let problem = |message: String| {
        Err(Problem::new(
            ProblemKind::Component,
            &node.id,
            Some("component"),
            message,
        ))
    };
    match document.get(component) {
        None => problem(format!(
            "{label}: \"component\" is \"{component}\", but no node has that id. Set \
             \"component\" to the id of a component layer, and add it in the same batch if \
             it is new."
        )),
        Some(main) if main.node_type() != NodeType::Component => problem(format!(
            "{label}: \"component\" is {}, which is not a component. Set \"component\" to the \
             id of a component layer; a group can be made into one.",
            main.label()
        )),
        Some(_) => Ok(()),
    }
}

/// Checks the rules about components over the whole document: every
/// instance names a component, no component is inside another, and no
/// component shows itself. Problems are reported in order of node id, so
/// the same document always gives the same message.
pub(crate) fn check_components(document: &Document) -> Result<(), Problem> {
    let mut components: Vec<&Node> = Vec::new();
    let mut instances: Vec<&Node> = Vec::new();
    for node in document.nodes_unordered() {
        match node.node_type() {
            NodeType::Component => components.push(node),
            NodeType::Instance => instances.push(node),
            _ => {}
        }
    }
    if instances.is_empty() && components.is_empty() {
        return Ok(());
    }
    components.sort_by(|a, b| a.id.cmp(&b.id));
    instances.sort_by(|a, b| a.id.cmp(&b.id));

    for component in &components {
        if let Some(outer) = document.enclosing_component(&component.id) {
            return Err(Problem::new(
                ProblemKind::NestedComponent,
                &component.id,
                Some("parent"),
                format!(
                    "{}: it is inside {}, but a component cannot be inside another \
                     component. Move it out; to show it there, put an instance of it there \
                     instead.",
                    component.label(),
                    outer.label()
                ),
            ));
        }
    }
    for instance in &instances {
        check_reference(document, instance)?;
    }

    // Which components each component shows directly, through which
    // instance.
    let mut shows: HashMap<&NodeId, Vec<&NodeId>> = HashMap::new();
    for instance in &instances {
        if let (Some(outer), Some(shown)) = (
            document.enclosing_component(&instance.id),
            instance.component(),
        ) {
            shows.entry(&outer.id).or_default().push(shown);
        }
    }
    let reaches = |from: &NodeId, to: &NodeId| {
        let mut seen = HashSet::new();
        let mut pending = vec![from];
        while let Some(next) = pending.pop() {
            if next == to {
                return true;
            }
            if seen.insert(next) {
                pending.extend(shows.get(next).into_iter().flatten().copied());
            }
        }
        false
    };
    for instance in &instances {
        let (Some(outer), Some(shown)) = (
            document.enclosing_component(&instance.id),
            instance.component(),
        ) else {
            continue;
        };
        if !reaches(shown, &outer.id) {
            continue;
        }
        let outer_label = outer.label();
        let message = if shown == &outer.id {
            format!(
                "{}: \"component\" is {outer_label}, but this instance is inside that very \
                 component, so it would have to show itself inside itself forever. Use an \
                 instance of another component, or move this one out of {outer_label}.",
                instance.label()
            )
        } else {
            let shown_label = document
                .get(shown)
                .map_or_else(|| format!("\"{shown}\""), Node::label);
            format!(
                "{}: \"component\" is {shown_label}, which shows {outer_label} through its \
                 instances, and this instance is inside {outer_label}: each would have to \
                 show the other forever. Pick another component, or move this instance out \
                 of {outer_label}.",
                instance.label()
            )
        };
        return Err(Problem::new(
            ProblemKind::ComponentLoop,
            &instance.id,
            Some("component"),
            message,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::document::Document;
    use crate::edit::{Edit, EditError};
    use crate::id::NodeId;
    use crate::model::{Node, NodeType};
    use crate::validate::ProblemKind;

    fn add(value: Value) -> Edit {
        Edit::add(Node::from_value(value).unwrap())
    }

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    /// Two artboards. In the first, a component "card" with a rect, an
    /// instance "copy" of it and a loose group "box"; in the second, a
    /// component "badge" with an instance of "card" inside it.
    fn sample() -> Document {
        let mut document = Document::new("Components");
        document
            .apply(&[
                add(json!({ "id": "ab", "index": "a0", "type": "artboard", "width": 800, "height": 600 })),
                add(json!({ "id": "ab2", "index": "a1", "type": "artboard", "x": 900 })),
                add(json!({ "id": "card", "parent": "ab", "index": "a0", "type": "component",
                            "width": 200, "height": 100 })),
                add(json!({ "id": "bg", "parent": "card", "index": "a0", "type": "rect" })),
                add(json!({ "id": "copy", "parent": "ab", "index": "a1", "type": "instance",
                            "component": "card" })),
                add(json!({ "id": "box", "parent": "ab", "index": "a2", "type": "group" })),
                add(json!({ "id": "badge", "parent": "ab2", "index": "a0", "type": "component" })),
                add(json!({ "id": "inner", "parent": "badge", "index": "a0", "type": "instance",
                            "component": "card" })),
            ])
            .unwrap();
        document
    }

    fn refused(document: &Document, edits: &[Edit]) -> EditError {
        let mut copy = document.clone();
        let error = copy.apply(edits).unwrap_err();
        assert_eq!(&copy, document, "a refused batch changes nothing");
        error
    }

    #[test]
    fn instances_name_an_existing_component_in_any_artboard() {
        let document = sample();
        document.validate().unwrap();
        assert_eq!(document.instances_of(&id("card")).len(), 2);
        assert!(document.component_shows(&id("badge"), &id("card")));
        assert!(!document.component_shows(&id("card"), &id("badge")));
        assert_eq!(
            document
                .enclosing_component(&id("inner"))
                .map(|node| &node.id),
            Some(&id("badge"))
        );

        let missing = refused(
            &document,
            &[add(
                json!({ "id": "x", "parent": "ab", "index": "a5", "type": "instance",
                         "component": "nope" }),
            )],
        );
        assert_eq!(missing.problem.kind, ProblemKind::Component);
        assert_eq!(missing.problem.field.as_deref(), Some("component"));
        assert!(
            missing.to_string().contains("no node has that id"),
            "{missing}"
        );

        let not_one = refused(&document, &[Edit::set("copy", "component", "bg")]);
        assert_eq!(not_one.problem.kind, ProblemKind::Component);
        assert!(
            not_one.to_string().contains("is not a component"),
            "{not_one}"
        );
    }

    #[test]
    fn removing_a_component_its_instances_still_show_is_refused() {
        let document = sample();
        let error = refused(&document, &[Edit::remove("card")]);
        assert_eq!(error.problem.kind, ProblemKind::Component);
        assert_eq!(error.index, 0, "the removal is to blame");
        // Removing the instances in the same batch is fine.
        let mut document = document;
        document
            .apply(&[
                Edit::remove("copy"),
                Edit::remove("inner"),
                Edit::remove("card"),
            ])
            .unwrap();
        document.validate().unwrap();
    }

    #[test]
    fn a_component_cannot_show_itself() {
        let document = sample();
        let direct = refused(
            &document,
            &[add(
                json!({ "id": "x", "parent": "card", "index": "a5", "type": "instance",
                         "component": "card" }),
            )],
        );
        assert_eq!(direct.problem.kind, ProblemKind::ComponentLoop);
        assert!(direct.to_string().contains("inside itself"), "{direct}");

        // "badge" shows "card", so "card" cannot show "badge".
        let through = refused(
            &document,
            &[add(
                json!({ "id": "x", "parent": "card", "index": "a5", "type": "instance",
                         "component": "badge" }),
            )],
        );
        assert_eq!(through.problem.kind, ProblemKind::ComponentLoop);
        assert!(
            through.to_string().contains("each would have to"),
            "{through}"
        );

        // Moving a group that holds an instance of "card" into "card": the
        // instance is not touched, and the loop is still found.
        let mut holder = document.clone();
        holder
            .apply(&[Edit::update(
                "copy",
                [("parent", json!("box")), ("index", json!("a0"))],
            )])
            .unwrap();
        let moved = refused(
            &holder,
            &[Edit::update(
                "box",
                [("parent", json!("card")), ("index", json!("a5"))],
            )],
        );
        assert_eq!(moved.problem.kind, ProblemKind::ComponentLoop);
        assert_eq!(moved.problem.node, Some(id("copy")));
    }

    #[test]
    fn components_do_not_nest() {
        let document = sample();
        let nested = refused(
            &document,
            &[add(
                json!({ "id": "x", "parent": "card", "index": "a5", "type": "component" }),
            )],
        );
        assert_eq!(nested.problem.kind, ProblemKind::NestedComponent);
        assert_eq!(nested.problem.node, Some(id("x")));

        let mut holder = document.clone();
        holder
            .apply(&[add(
                json!({ "id": "x", "parent": "box", "index": "a0", "type": "component" }),
            )])
            .unwrap();
        let moved = refused(
            &holder,
            &[Edit::update(
                "box",
                [("parent", json!("card")), ("index", json!("a5"))],
            )],
        );
        assert_eq!(moved.problem.kind, ProblemKind::NestedComponent);
    }

    #[test]
    fn instances_hold_no_layers() {
        let error = refused(
            &sample(),
            &[add(
                json!({ "id": "x", "parent": "copy", "index": "a0", "type": "rect" }),
            )],
        );
        assert_eq!(error.problem.kind, ProblemKind::Parent);
        assert!(error.to_string().contains("or a component"), "{error}");
    }

    #[test]
    fn a_group_and_a_component_turn_into_each_other() {
        let mut document = sample();
        let before = document.clone();
        let inverse = document
            .apply(&[Edit::set("box", "type", "component")])
            .unwrap();
        assert_eq!(
            document.get(&id("box")).unwrap().node_type(),
            NodeType::Component
        );
        document.validate().unwrap();
        document.apply(&inverse).unwrap();
        assert_eq!(document, before, "undo turns it back into a group");

        // A component that instances show cannot stop being one, and no
        // other type can change.
        let shown = refused(&document, &[Edit::set("card", "type", "group")]);
        assert_eq!(shown.problem.kind, ProblemKind::Component);
        let other = refused(&document, &[Edit::set("box", "type", "mask_group")]);
        assert_eq!(other.problem.kind, ProblemKind::Immutable);
        let rect = refused(&document, &[Edit::set("bg", "type", "component")]);
        assert_eq!(rect.problem.kind, ProblemKind::Immutable);
    }

    #[test]
    fn files_that_break_the_rules_do_not_load() {
        let mut nodes: Vec<Node> = sample().nodes().cloned().collect();
        nodes.push(
            Node::from_value(json!({ "id": "x", "parent": "card", "index": "a5",
                                     "type": "instance", "component": "card" }))
            .unwrap(),
        );
        let problem = Document::from_nodes("Loop", nodes).unwrap_err();
        assert_eq!(problem.kind, ProblemKind::ComponentLoop);
    }
}

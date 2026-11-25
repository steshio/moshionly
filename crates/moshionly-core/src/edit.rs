//! Edits: the one way to change a document.
//!
//! Every change, whether it comes from a mouse drag, undo, a file import or
//! an AI agent, is a batch of three edits: add a node, update a node, remove
//! a node. [`Document::apply`] checks a batch as a whole, applies all of it
//! or none of it, and returns the batch that undoes it.
//!
//! # How a batch is checked
//!
//! Edits are applied in order to a working copy. Three things are checked at
//! each edit: the node it names exists (or, for an add, does not exist yet),
//! a patch produces a node of the same type with no unknown fields, and every
//! value is inside its range.
//!
//! Everything about where nodes sit (parents, indices, targets, loops) is
//! checked once at the end, on the result. So the order of edits inside a
//! batch is free: a child may be added before its parent, and two siblings
//! may swap indices.
//!
//! # Patches
//!
//! An [`Edit::Update`] carries a JSON merge patch on the node's own fields,
//! including `parent` and `index`:
//!
//! - a value replaces the field, `null` removes an optional field (it goes
//!   back to its default);
//! - an object is merged into the object that is there, field by field, so
//!   `{ "stroke": { "width": 4 } }` keeps the stroke's paint;
//! - an object that says its `type` (a gradient, an easing) replaces what is
//!   there as a whole, because its other fields depend on the type;
//! - arrays are replaced as a whole.
//!
//! `id` and `type` are fixed, with one exception: a `group` and a
//! `component` have the same fields, so a patch can turn one into the
//! other by setting `type`.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::document::Document;
use crate::id::NodeId;
use crate::model::{Family, Node, NodeType};
use crate::validate::{self, Problem, ProblemKind};

/// One change to a document.
///
/// ```json
/// { "op": "add", "node": { "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect" } }
/// { "op": "update", "id": "cd34", "patch": { "x": 120, "fill": "#0D99FF", "shadow": null } }
/// { "op": "remove", "id": "cd34" }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
// The shape is the documented one; edits are short-lived, so the size of
// `Add` next to the others does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Edit {
    /// Adds a node. Its id must not be in use.
    Add {
        /// The node to add.
        node: Node,
    },
    /// Changes fields of a node with a JSON merge patch. Moving or reordering
    /// a node is an update of `parent` and `index`. `id` and `type` cannot
    /// be changed, except that a `group` and a `component`, which have the
    /// same fields, can turn into each other.
    Update {
        /// The node to change.
        id: NodeId,
        /// Fields to set. `null` removes an optional field.
        patch: Map<String, Value>,
    },
    /// Removes a node together with its descendants and with every animation
    /// that targets the node or one of its descendants.
    Remove {
        /// The node to remove.
        id: NodeId,
    },
}

impl Edit {
    /// An edit that adds `node`.
    pub fn add(node: Node) -> Self {
        Edit::Add { node }
    }

    /// An edit that patches the node `id` with `(field, value)` pairs.
    ///
    /// A `serde_json::Map` can be passed directly.
    pub fn update<K: Into<String>>(
        id: impl Into<NodeId>,
        patch: impl IntoIterator<Item = (K, Value)>,
    ) -> Self {
        Edit::Update {
            id: id.into(),
            patch: patch
                .into_iter()
                .map(|(field, value)| (field.into(), value))
                .collect(),
        }
    }

    /// An edit that sets one field of the node `id`.
    pub fn set(id: impl Into<NodeId>, field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::update(id, [(field.into(), value.into())])
    }

    /// An edit that removes the node `id`.
    pub fn remove(id: impl Into<NodeId>) -> Self {
        Edit::Remove { id: id.into() }
    }

    /// The id of the node the edit is about.
    pub fn node_id(&self) -> &NodeId {
        match self {
            Edit::Add { node } => &node.id,
            Edit::Update { id, .. } | Edit::Remove { id } => id,
        }
    }

    /// A few words naming the edit, for messages: `update "cd34"`.
    pub fn summary(&self) -> String {
        match self {
            Edit::Add { node } => format!("add {}", node.label()),
            Edit::Update { id, .. } => format!("update \"{id}\""),
            Edit::Remove { id } => format!("remove \"{id}\""),
        }
    }
}

/// Why a batch of edits was refused.
///
/// The document is unchanged when this is returned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "Edit {} of {total} ({summary}) cannot be applied, so none of the edits were applied. {problem}",
    .index + 1
)]
pub struct EditError {
    /// Position in the batch of the edit that broke a rule, counting from 0.
    pub index: usize,
    /// Number of edits in the batch.
    pub total: usize,
    /// A few words naming that edit, for example `update "cd34"`.
    pub summary: String,
    /// The rule that was broken and how to fix it.
    pub problem: Problem,
}

/// Whether a patch object replaces the value it meets instead of merging
/// into it: it does when it says its `type`.
fn replaces(patch: &Map<String, Value>) -> bool {
    patch.get("type").is_some_and(Value::is_string)
}

/// Removes `null` members from objects, as a merge patch does to new values.
fn without_nulls(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(name, value)| (name.clone(), without_nulls(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Applies a merge patch to a JSON object, by the rules in the module docs.
pub fn merge_patch(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (name, value) in patch {
        match value {
            Value::Null => {
                target.remove(name);
            }
            Value::Object(fields) if !replaces(fields) => match target.get_mut(name) {
                Some(Value::Object(existing)) => merge_patch(existing, fields),
                _ => {
                    target.insert(name.clone(), without_nulls(value));
                }
            },
            _ => {
                target.insert(name.clone(), without_nulls(value));
            }
        }
    }
}

/// The merge patch that turns the JSON object `from` into `to`.
///
/// Neither object may contain `null` members below the top level.
pub fn diff_patch(from: &Map<String, Value>, to: &Map<String, Value>) -> Map<String, Value> {
    let mut patch = Map::new();
    for name in from.keys() {
        if !to.contains_key(name) {
            patch.insert(name.clone(), Value::Null);
        }
    }
    for (name, wanted) in to {
        match (from.get(name), wanted) {
            (Some(current), _) if current == wanted => {}
            // A patch object without a type merges into the object it meets,
            // so only the difference is needed.
            (Some(Value::Object(current)), Value::Object(fields))
                if !fields.contains_key("type") =>
            {
                patch.insert(name.clone(), Value::Object(diff_patch(current, fields)));
            }
            _ => {
                patch.insert(name.clone(), wanted.clone());
            }
        }
    }
    patch
}

/// The fields of a node as a JSON object.
fn fields_of(node: &Node) -> Map<String, Value> {
    match node.to_value() {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

/// What a batch, or a run of batches that will be undone together, has
/// touched: every node it changed, as that node was before the first change.
#[derive(Debug, Clone, Default)]
pub(crate) struct Journal {
    /// The first-seen state per id. `None` means the node did not exist.
    before: HashMap<NodeId, Option<Node>>,
    /// Ids in the order they were first touched.
    order: Vec<NodeId>,
}

impl Journal {
    fn record(&mut self, id: &NodeId, before: Option<&Node>) {
        if !self.before.contains_key(id) {
            self.before.insert(id.clone(), before.cloned());
            self.order.push(id.clone());
        }
    }

    /// Adds what `later` touched after this journal's changes.
    pub(crate) fn absorb(&mut self, later: Journal) {
        let Journal { mut before, order } = later;
        for id in order {
            if let Some(state) = before.remove(&id)
                && !self.before.contains_key(&id)
            {
                self.before.insert(id.clone(), state);
                self.order.push(id);
            }
        }
    }
}

/// Book-keeping for one batch while it is being applied.
#[derive(Default)]
struct Batch {
    journal: Journal,
    /// The last edit that touched each node, to say which edit is at fault.
    touched_by: HashMap<NodeId, usize>,
    /// Whether any node changed parent, which can move a whole subtree to
    /// another artboard.
    reparented: Option<usize>,
    /// The last edit that added, changed or removed a component or an
    /// instance, after which the component rules are checked again.
    components: Option<usize>,
}

impl Batch {
    fn touch(&mut self, id: &NodeId, before: Option<&Node>, edit: usize) {
        self.journal.record(id, before);
        self.touched_by.insert(id.clone(), edit);
    }

    /// Notes that `node` is involved in an edit, which matters to the
    /// component rules when it is a component or an instance.
    fn note(&mut self, node: &Node, edit: usize) {
        if matches!(node.node_type(), NodeType::Component | NodeType::Instance) {
            self.components = Some(edit);
        }
    }
}

impl Document {
    /// Builds a document from a list of nodes, checking every rule.
    pub fn from_nodes(
        name: impl Into<String>,
        nodes: impl IntoIterator<Item = Node>,
    ) -> Result<Document, Problem> {
        let mut document = Document::new(name);
        for node in nodes {
            if let Some(existing) = document.get(&node.id) {
                return Err(duplicate_id(&node, existing));
            }
            validate::check_values(&node)?;
            document.insert_raw(node);
        }
        // Report placement problems in a stable order.
        let mut nodes: Vec<&Node> = document.nodes_unordered().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        for node in nodes {
            validate::check_placement(&document, node)?;
        }
        validate::check_components(&document)?;
        Ok(document)
    }

    /// Applies a batch of edits, all or nothing, and returns the batch that
    /// undoes it.
    ///
    /// On an error the document is unchanged. See the [module docs](self)
    /// for what is checked and how patches merge.
    pub fn apply(&mut self, edits: &[Edit]) -> Result<Vec<Edit>, EditError> {
        let mut journal = Journal::default();
        self.apply_journaled(edits, &mut journal)?;
        Ok(self.inverse_of(&journal))
    }

    /// Checks a batch without changing the document.
    pub fn check(&self, edits: &[Edit]) -> Result<(), EditError> {
        self.clone().apply(edits).map(drop)
    }

    /// Applies a batch and adds what it touched to `journal`, which may
    /// already hold earlier batches that will be undone together with it.
    pub(crate) fn apply_journaled(
        &mut self,
        edits: &[Edit],
        journal: &mut Journal,
    ) -> Result<(), EditError> {
        let mut batch = Batch::default();
        let fail = |document: &mut Document, batch: &Batch, index: usize, problem: Problem| {
            document.restore(&batch.journal);
            EditError {
                index,
                total: edits.len(),
                summary: edits[index].summary(),
                problem,
            }
        };
        for (index, edit) in edits.iter().enumerate() {
            if let Err(problem) = self.apply_one(edit, index, &mut batch, journal) {
                return Err(fail(self, &batch, index, problem));
            }
        }
        if let Err((index, problem)) = self.check_touched(&batch) {
            return Err(fail(self, &batch, index, problem));
        }
        journal.absorb(batch.journal);
        Ok(())
    }

    fn apply_one(
        &mut self,
        edit: &Edit,
        index: usize,
        batch: &mut Batch,
        earlier: &Journal,
    ) -> Result<(), Problem> {
        match edit {
            Edit::Add { node } => {
                validate::check_values(node)?;
                if let Some(existing) = self.get(&node.id) {
                    return Err(duplicate_id(node, existing));
                }
                // The undo of this step restores or patches nodes by id, and
                // a patch cannot turn one type into another.
                let original = batch
                    .journal
                    .before
                    .get(&node.id)
                    .or_else(|| earlier.before.get(&node.id));
                if let Some(Some(original)) = original
                    && original.node_type() != node.node_type()
                {
                    return Err(Problem::new(
                        ProblemKind::DuplicateId,
                        &node.id,
                        Some("id"),
                        format!(
                            "{}: the id \"{}\" belonged to a {} that was removed in the same \
                             undo step. Give the new {} a different id.",
                            node.label(),
                            node.id,
                            original.node_type(),
                            node.node_type()
                        ),
                    ));
                }
                batch.touch(&node.id, None, index);
                batch.note(node, index);
                self.insert_raw(node.clone());
            }
            Edit::Update { id, patch } => {
                let current = self.get(id).ok_or_else(|| unknown_node(id))?;
                let updated = patched(current, patch)?;
                validate::check_values(&updated)?;
                if updated != *current {
                    if updated.parent != current.parent {
                        batch.reparented = Some(index);
                    }
                    // Moving or restyling an instance cannot break the
                    // component rules; changing what it shows, or turning
                    // a group into a component, can.
                    if updated.node_type() != current.node_type()
                        || updated.component() != current.component()
                    {
                        batch.note(current, index);
                        batch.note(&updated, index);
                    }
                    batch.touch(id, Some(current), index);
                    self.remove_raw(id);
                    self.insert_raw(updated);
                }
            }
            Edit::Remove { id } => {
                if !self.contains(id) {
                    return Err(unknown_node(id));
                }
                for doomed in self.removed_with(id) {
                    if let Some(node) = self.remove_raw(&doomed) {
                        batch.note(&node, index);
                        batch.touch(&doomed, Some(&node), index);
                    }
                }
            }
        }
        Ok(())
    }

    /// The ids that go when `id` is removed: the node, its descendants of
    /// both families, and every animation that targets one of those.
    ///
    /// In the middle of a batch the parents may form a loop (which the batch
    /// will be refused for unless this removal breaks it), so this keeps
    /// track of what it has seen instead of trusting the tree to end.
    fn removed_with<'a>(&'a self, id: &'a NodeId) -> Vec<NodeId> {
        let mut doomed: HashSet<&NodeId> = HashSet::new();
        let mut pending = vec![id];
        while let Some(next) = pending.pop() {
            if doomed.insert(next) {
                pending.extend(self.children(next, Family::Layer));
                pending.extend(self.children(next, Family::Animation));
            }
        }
        let orphaned: Vec<&NodeId> = self
            .nodes_unordered()
            .filter(|node| node.target().is_some_and(|target| doomed.contains(target)))
            .map(|node| &node.id)
            .collect();
        doomed.extend(orphaned);
        doomed.into_iter().cloned().collect()
    }

    /// Checks the placement rules for everything a batch touched.
    fn check_touched(&self, batch: &Batch) -> Result<(), (usize, Problem)> {
        for id in &batch.journal.order {
            if let Some(node) = self.get(id) {
                validate::check_placement(self, node)
                    .map_err(|problem| (batch.touched_by[id], problem))?;
            }
        }
        // Moving a layer or an animation group takes everything inside it
        // along, possibly to another artboard, away from animations or
        // targets that were not touched themselves.
        if let Some(index) = batch.reparented {
            for node in self.nodes_unordered() {
                if node.target().is_some() && !batch.touched_by.contains_key(&node.id) {
                    validate::check_placement(self, node).map_err(|problem| (index, problem))?;
                }
            }
        }
        // The component rules span the tree: removing a component leaves
        // its instances without one, and moving a group can put an
        // instance inside its own component.
        if let Some(index) = batch.components.max(batch.reparented) {
            validate::check_components(self).map_err(|problem| {
                let blamed = problem
                    .node
                    .as_ref()
                    .and_then(|node| batch.touched_by.get(node))
                    .copied()
                    .unwrap_or(index);
                (blamed, problem)
            })?;
        }
        Ok(())
    }

    /// Puts back every node a journal touched, as it was before.
    fn restore(&mut self, journal: &Journal) {
        for id in &journal.order {
            self.remove_raw(id);
        }
        for id in &journal.order {
            if let Some(Some(node)) = journal.before.get(id) {
                self.insert_raw(node.clone());
            }
        }
    }

    /// The batch that takes the document from its current state back to how
    /// it was before the changes recorded in `journal`.
    ///
    /// It restores removed nodes (parents and targets first), then patches
    /// changed nodes, then removes added nodes (dependants first, so that no
    /// removal sweeps away a node the batch still needs).
    pub(crate) fn inverse_of(&self, journal: &Journal) -> Vec<Edit> {
        let mut restored: Vec<&Node> = Vec::new();
        let mut updates: Vec<Edit> = Vec::new();
        let mut added: Vec<&Node> = Vec::new();
        for id in &journal.order {
            match (
                journal.before.get(id).and_then(Option::as_ref),
                self.get(id),
            ) {
                (Some(before), None) => restored.push(before),
                (None, Some(now)) => added.push(now),
                (Some(before), Some(now)) if before != now => updates.push(Edit::Update {
                    id: id.clone(),
                    patch: diff_patch(&fields_of(now), &fields_of(before)),
                }),
                _ => {}
            }
        }
        sort_parents_first(&mut restored);
        sort_parents_first(&mut added);
        restored
            .into_iter()
            .map(|node| Edit::add(node.clone()))
            .chain(updates)
            .chain(added.into_iter().rev().map(|node| Edit::remove(&node.id)))
            .collect()
    }
}

/// Orders nodes so that each comes after the nodes of the same list it
/// depends on: layers before animations, and parents before children.
fn sort_parents_first(nodes: &mut [&Node]) {
    let by_id: HashMap<&NodeId, &Node> = nodes.iter().map(|node| (&node.id, *node)).collect();
    let depth = |node: &Node| {
        let mut depth = 0;
        let mut parent = node.parent.as_ref();
        while let Some(above) = parent.and_then(|id| by_id.get(id))
            && depth <= by_id.len()
        {
            depth += 1;
            parent = above.parent.as_ref();
        }
        depth
    };
    let family = |node: &Node| match node.family() {
        Family::Layer => 0,
        Family::Animation => 1,
    };
    nodes.sort_by_cached_key(|node| {
        (
            family(node),
            depth(node),
            node.parent.clone(),
            node.index.clone(),
            node.id.clone(),
        )
    });
}

/// Whether a patch may change a node of type `from` to the type `to`: a
/// group and a component have the same fields and can turn into each
/// other, which is how a group becomes a component in place.
fn interchangeable(from: NodeType, to: Option<&Value>) -> bool {
    let both = [NodeType::Group, NodeType::Component];
    both.contains(&from)
        && to
            .and_then(Value::as_str)
            .and_then(NodeType::parse)
            .is_some_and(|to| both.contains(&to))
}

/// The node that results from applying `patch` to `node`.
pub(crate) fn patched(node: &Node, patch: &Map<String, Value>) -> Result<Node, Problem> {
    let label = node.label();
    let fixed = |field: &str, current: &str| match patch.get(field) {
        Some(value) if value.as_str() != Some(current) => Err(Problem::new(
            ProblemKind::Immutable,
            &node.id,
            Some(field),
            format!(
                "{label}: \"{field}\" cannot be changed (the patch sets it to {value}). To get \
                 a node with another {field}, remove this one and add a new node."
            ),
        )),
        _ => Ok(()),
    };
    fixed("id", node.id.as_str())?;
    if !interchangeable(node.node_type(), patch.get("type")) {
        fixed("type", node.node_type().as_str())?;
    }

    let mut fields = fields_of(node);
    merge_patch(&mut fields, patch);
    crate::schema::read_node(Value::Object(fields))
}

fn unknown_node(id: &NodeId) -> Problem {
    Problem::new(
        ProblemKind::UnknownNode,
        id,
        Some("id"),
        format!(
            "There is no node with the id \"{id}\". Check the id; the node may have been \
             removed, also by an earlier edit in this batch."
        ),
    )
}

fn duplicate_id(node: &Node, existing: &Node) -> Problem {
    Problem::new(
        ProblemKind::DuplicateId,
        &node.id,
        Some("id"),
        format!(
            "{}: the id \"{}\" is already used by {}. Give the new node a different id, or \
             update the existing node instead of adding one.",
            node.label(),
            node.id,
            existing.label()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    fn node(value: Value) -> Node {
        Node::from_value(value).unwrap()
    }

    fn add(value: Value) -> Edit {
        Edit::add(node(value))
    }

    fn update(target: &str, patch: Value) -> Edit {
        Edit::update(target, patch.as_object().unwrap().clone())
    }

    /// An artboard with a group holding two rects, a loose rect, and two
    /// animations, one of them in a folder.
    fn sample() -> Document {
        let mut document = Document::new("Sample");
        document
            .apply(&[
                add(json!({ "id": "ab", "index": "a0", "type": "artboard", "width": 800, "height": 600 })),
                add(json!({ "id": "grp", "parent": "ab", "index": "a0", "type": "group", "name": "Card" })),
                add(json!({ "id": "r1", "parent": "grp", "index": "a0", "type": "rect", "fill": "#FF0000" })),
                add(json!({ "id": "r2", "parent": "grp", "index": "a1", "type": "rect",
                            "stroke": { "paint": "#00FF00", "width": 2 } })),
                add(json!({ "id": "r3", "parent": "ab", "index": "a1", "type": "rect" })),
                add(json!({ "id": "fade", "parent": "ab", "index": "a0", "type": "fade_in",
                            "target": "r1", "start": 0, "end": 500 })),
                add(json!({ "id": "folder", "parent": "ab", "index": "a1", "type": "animation_group" })),
                add(json!({ "id": "move", "parent": "folder", "index": "a0", "type": "move",
                            "target": "grp", "start": 0, "end": 500, "to": { "x": 100 } })),
            ])
            .unwrap();
        document
    }

    fn refused(document: &mut Document, edits: &[Edit]) -> EditError {
        let before = document.clone();
        let error = document.apply(edits).unwrap_err();
        assert_eq!(*document, before, "a refused batch changes nothing");
        document.validate().unwrap();
        error
    }

    /// Applies a batch, then checks that its inverse undoes it exactly and
    /// that the inverse of the inverse redoes it exactly.
    fn round_trip(document: &mut Document, edits: &[Edit]) -> Vec<Edit> {
        let before = document.clone();
        let inverse = document.apply(edits).unwrap();
        document.validate().unwrap();
        let after = document.clone();
        let redo = document.apply(&inverse).unwrap();
        assert_eq!(*document, before, "the inverse restores the document");
        document.apply(&redo).unwrap();
        assert_eq!(*document, after, "the inverse of the inverse redoes it");
        inverse
    }

    #[test]
    fn add_update_remove_and_their_inverses() {
        let mut document = sample();
        assert_eq!(document.len(), 8);

        let inverse = round_trip(
            &mut document,
            &[add(
                json!({ "id": "new", "parent": "ab", "index": "a2", "type": "ellipse" }),
            )],
        );
        assert_eq!(inverse, [Edit::remove("new")]);

        let inverse = round_trip(
            &mut document,
            &[update(
                "r1",
                json!({ "x": 40, "fill": "#0000FF", "name": "Box" }),
            )],
        );
        assert_eq!(
            inverse,
            [update(
                "r1",
                json!({ "name": null, "x": null, "fill": "#FF0000" })
            )]
        );
        let r1 = document.get(&id("r1")).unwrap();
        assert_eq!((r1.name(), r1.layer().unwrap().x), ("Box", 40.0));

        let inverse = round_trip(&mut document, &[Edit::remove("new")]);
        assert_eq!(inverse.len(), 1);
        assert!(matches!(&inverse[0], Edit::Add { node } if node.id == id("new")));
    }

    #[test]
    fn removing_a_layer_takes_descendants_and_their_animations() {
        let mut document = sample();
        let inverse = round_trip(&mut document, &[Edit::remove("grp")]);
        let left: Vec<&str> = document.nodes().map(|node| node.id.as_str()).collect();
        assert_eq!(left, ["ab", "r3", "folder"], "the empty folder stays");

        // The inverse restores parents before children and layers before the
        // animations that target them.
        let order: Vec<&str> = inverse.iter().map(|edit| edit.node_id().as_str()).collect();
        assert_eq!(order, ["grp", "r1", "r2", "fade", "move"]);
    }

    #[test]
    fn removing_an_artboard_takes_everything_in_it() {
        let mut document = sample();
        round_trip(&mut document, &[Edit::remove("ab")]);
        assert!(document.is_empty());
    }

    #[test]
    fn patches_merge_objects_and_replace_typed_ones() {
        let mut document = sample();
        round_trip(
            &mut document,
            &[update("r2", json!({ "stroke": { "width": 6 } }))],
        );
        assert_eq!(
            document.get(&id("r2")).unwrap().to_value()["stroke"],
            json!({ "paint": "#00FF00", "width": 6 }),
            "the paint survives a patch of the width"
        );

        let linear = json!({ "type": "linear_gradient", "start": [0, 0], "stops": [
            { "offset": 0, "color": "#FF0000" }, { "offset": 1, "color": "#0000FF" } ] });
        round_trip(&mut document, &[update("r1", json!({ "fill": linear }))]);
        // A radial gradient has no "start"; merging would leave it behind.
        let radial = json!({ "type": "radial_gradient", "stops": [
            { "offset": 0, "color": "#FFFFFF" }, { "offset": 1, "color": "#000000" } ] });
        round_trip(
            &mut document,
            &[update("r1", json!({ "fill": radial.clone() }))],
        );
        assert_eq!(document.get(&id("r1")).unwrap().to_value()["fill"], radial);

        // Without a type, a patch reaches inside the gradient.
        round_trip(
            &mut document,
            &[update("r1", json!({ "fill": { "rotation": 45 } }))],
        );
        assert_eq!(
            document.get(&id("r1")).unwrap().to_value()["fill"]["rotation"],
            json!(45)
        );

        round_trip(
            &mut document,
            &[update(
                "fade",
                json!({ "easing": { "type": "bezier", "x1": 0.2, "y1": 0, "x2": 0, "y2": 1 } }),
            )],
        );
        round_trip(
            &mut document,
            &[update("fade", json!({ "easing": { "type": "linear" } }))],
        );
        round_trip(
            &mut document,
            &[update(
                "r2",
                json!({ "stroke": null, "shadow": { "y": 10 } }),
            )],
        );
        let r2 = document.get(&id("r2")).unwrap();
        assert!(r2.data.stroke().is_none());
        assert_eq!(r2.layer().unwrap().shadow.unwrap().y, 10.0);
    }

    #[test]
    fn merge_and_diff_are_inverse_operations() {
        let cases = [
            (
                json!({ "a": 1, "b": { "c": 2, "d": 3 } }),
                json!({ "a": 2, "b": { "c": 2 }, "e": [1] }),
            ),
            (
                json!({ "f": { "type": "x", "p": 1 } }),
                json!({ "f": { "type": "y", "q": 2 } }),
            ),
            (
                json!({ "f": { "type": "x", "p": 1 } }),
                json!({ "f": { "type": "x", "p": 5, "r": 1 } }),
            ),
            (json!({ "f": "#FFFFFF" }), json!({ "f": { "type": "x" } })),
            (json!({ "f": { "type": "x" } }), json!({ "f": "#FFFFFF" })),
            (
                json!({ "f": { "type": "x", "p": 1 } }),
                json!({ "f": { "p": 1 } }),
            ),
            (
                json!({ "f": { "p": 1 } }),
                json!({ "f": { "type": "x", "p": 1 } }),
            ),
            (
                json!({ "s": { "a": { "b": 1 } } }),
                json!({ "s": { "a": { "c": 2 } } }),
            ),
            (json!({}), json!({ "s": { "a": 1 } })),
            (json!({ "s": { "a": 1 } }), json!({})),
        ];
        for (from, to) in cases {
            let patch = diff_patch(from.as_object().unwrap(), to.as_object().unwrap());
            let mut result = from.as_object().unwrap().clone();
            merge_patch(&mut result, &patch);
            assert_eq!(Value::Object(result), to, "{from} -> {to} with {patch:?}");
        }
    }

    #[test]
    fn a_batch_may_list_children_first_and_swap_indices() {
        let mut document = sample();
        round_trip(
            &mut document,
            &[
                add(json!({ "id": "kid", "parent": "box", "index": "a0", "type": "rect" })),
                add(
                    json!({ "id": "anim", "parent": "ab", "index": "a2", "type": "scale",
                            "target": "kid", "start": 0, "end": 300 }),
                ),
                add(json!({ "id": "box", "parent": "ab", "index": "a2", "type": "group" })),
            ],
        );
        assert_eq!(document.children(&id("box"), Family::Layer), [id("kid")]);

        round_trip(
            &mut document,
            &[
                update("r1", json!({ "index": "a1" })),
                update("r2", json!({ "index": "a0" })),
            ],
        );
        assert_eq!(
            document.children(&id("grp"), Family::Layer),
            [id("r2"), id("r1")]
        );
    }

    #[test]
    fn moving_a_node_is_an_update_of_parent_and_index() {
        let mut document = sample();
        round_trip(
            &mut document,
            &[update("r3", json!({ "parent": "grp", "index": "a2" }))],
        );
        assert_eq!(
            document.children(&id("grp"), Family::Layer),
            [id("r1"), id("r2"), id("r3")]
        );
        assert_eq!(document.children(&id("ab"), Family::Layer), [id("grp")]);
    }

    #[test]
    fn a_mixed_batch_undoes_in_one_step() {
        let mut document = sample();
        round_trip(
            &mut document,
            &[
                update("r3", json!({ "x": 5 })),
                add(json!({ "id": "g2", "parent": "ab", "index": "a5", "type": "group" })),
                update("r3", json!({ "parent": "g2", "index": "a0", "x": 9 })),
                Edit::remove("grp"),
                add(
                    json!({ "id": "spin", "parent": "ab", "index": "a9", "type": "spin_in",
                            "target": "g2", "start": 100, "end": 400 }),
                ),
                update("r3", json!({ "x": 5 })),
                add(json!({ "id": "temp", "parent": "g2", "index": "a1", "type": "star" })),
                Edit::remove("temp"),
            ],
        );
        assert!(!document.contains(&id("temp")));
        assert!(!document.contains(&id("grp")));
    }

    #[test]
    fn an_update_that_changes_nothing_has_an_empty_inverse() {
        let mut document = sample();
        let inverse = document
            .apply(&[update("r1", json!({ "fill": "#FF0000", "opacity": 100 }))])
            .unwrap();
        assert!(inverse.is_empty());
        assert!(document.apply(&[]).unwrap().is_empty());
    }

    #[test]
    fn errors_say_which_edit_and_leave_the_document_alone() {
        let mut document = sample();
        let error = refused(
            &mut document,
            &[
                update("r1", json!({ "x": 1 })),
                update("r1", json!({ "opacity": 140 })),
            ],
        );
        assert_eq!((error.index, error.total), (1, 2));
        assert_eq!(error.problem.kind, ProblemKind::OutOfRange);
        assert_eq!(
            error.to_string(),
            "Edit 2 of 2 (update \"r1\") cannot be applied, so none of the edits were applied. \
             rect \"r1\": \"opacity\" is 140, but it must be between 0 and 100."
        );
    }

    #[test]
    fn unknown_and_duplicate_ids_are_refused() {
        let mut document = sample();
        let unknown = refused(&mut document, &[update("nope", json!({ "x": 1 }))]);
        assert_eq!(unknown.problem.kind, ProblemKind::UnknownNode);
        let gone = refused(&mut document, &[Edit::remove("grp"), Edit::remove("r1")]);
        assert_eq!(
            (gone.index, gone.problem.kind),
            (1, ProblemKind::UnknownNode)
        );
        assert!(gone.to_string().contains("earlier edit"), "{gone}");

        let duplicate = refused(
            &mut document,
            &[add(
                json!({ "id": "r1", "parent": "ab", "index": "a7", "type": "ellipse" }),
            )],
        );
        assert_eq!(duplicate.problem.kind, ProblemKind::DuplicateId);
        assert!(duplicate.to_string().contains("rect \"r1\""), "{duplicate}");

        let bad_id = refused(
            &mut document,
            &[add(
                json!({ "id": "no good", "parent": "ab", "index": "a7", "type": "rect" }),
            )],
        );
        assert_eq!(bad_id.problem.kind, ProblemKind::InvalidId);

        let retyped = refused(
            &mut document,
            &[
                Edit::remove("r3"),
                add(json!({ "id": "r3", "parent": "ab", "index": "a1", "type": "star" })),
            ],
        );
        assert!(retyped.to_string().contains("different id"), "{retyped}");
        // The same type may come back under its old id.
        round_trip(
            &mut document,
            &[
                Edit::remove("r3"),
                add(json!({ "id": "r3", "parent": "ab", "index": "a1", "type": "rect", "x": 3 })),
            ],
        );
    }

    #[test]
    fn patches_cannot_change_id_or_type_or_add_unknown_fields() {
        let mut document = sample();
        for (patch, kind, field) in [
            (json!({ "id": "other" }), ProblemKind::Immutable, Some("id")),
            (
                json!({ "type": "ellipse" }),
                ProblemKind::Immutable,
                Some("type"),
            ),
            (
                json!({ "colour": "#FFFFFF" }),
                ProblemKind::Malformed,
                Some("colour"),
            ),
            (json!({ "x": "left" }), ProblemKind::Malformed, None),
            (json!({ "index": null }), ProblemKind::Malformed, None),
            (json!({ "fill": "blue" }), ProblemKind::Malformed, None),
            (
                json!({ "stroke": { "colour": "#000000" } }),
                ProblemKind::Malformed,
                None,
            ),
        ] {
            let error = refused(&mut document, &[update("r1", patch.clone())]);
            assert_eq!(error.problem.kind, kind, "{patch}: {error}");
            assert_eq!(error.problem.field.as_deref(), field, "{patch}: {error}");
            assert!(error.to_string().contains("rect \"r1\""), "{error}");
        }
        // Naming the current id and type is harmless.
        document
            .apply(&[update("r1", json!({ "id": "r1", "type": "rect", "x": 2 }))])
            .unwrap();
    }

    #[test]
    fn parents_must_exist_and_hold_the_family() {
        let mut document = sample();
        let cases = [
            (
                add(json!({ "id": "n", "parent": "zz", "index": "a0", "type": "rect" })),
                "no node has that id",
            ),
            (
                add(json!({ "id": "n", "parent": "r1", "index": "a0", "type": "rect" })),
                "an artboard, a group, a mask_group or a component",
            ),
            (
                add(json!({ "id": "n", "index": "a0", "type": "rect" })),
                "only artboards are roots",
            ),
            (
                add(json!({ "id": "n", "parent": "ab", "index": "a5", "type": "artboard" })),
                "has no parent",
            ),
            (
                add(json!({ "id": "n", "parent": "folder", "index": "a0", "type": "rect" })),
                "a layer can only be placed",
            ),
            (
                add(
                    json!({ "id": "n", "parent": "grp", "index": "a0", "type": "fade_in", "target": "r1", "start": 0, "end": 1 }),
                ),
                "set \"parent\" to \"ab\"",
            ),
            (
                update("r3", json!({ "parent": null })),
                "only artboards are roots",
            ),
        ];
        for (edit, expected) in cases {
            let error = refused(&mut document, std::slice::from_ref(&edit));
            assert_eq!(error.problem.kind, ProblemKind::Parent, "{error}");
            assert_eq!(error.problem.field.as_deref(), Some("parent"));
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn a_node_cannot_end_up_inside_itself() {
        let mut document = sample();
        document
            .apply(&[add(
                json!({ "id": "inner", "parent": "grp", "index": "a5", "type": "group" }),
            )])
            .unwrap();
        for edit in [
            update("grp", json!({ "parent": "inner", "index": "a0" })),
            update("grp", json!({ "parent": "grp" })),
        ] {
            let error = refused(&mut document, &[edit]);
            assert_eq!(error.problem.kind, ProblemKind::Cycle, "{error}");
        }
        // Removing a node in the middle of a loop still ends cleanly.
        refused(
            &mut document,
            &[
                update("grp", json!({ "parent": "inner", "index": "a0" })),
                Edit::remove("r3"),
                update("r1", json!({ "x": 1 })),
            ],
        );
        round_trip(
            &mut document,
            &[
                update("grp", json!({ "parent": "inner", "index": "a0" })),
                Edit::remove("grp"),
            ],
        );
        // Removing a node from inside a loop takes everything below it,
        // however the loop winds: here the artboard is put inside its own
        // group and then removed, and no layer may be left without a parent.
        round_trip(
            &mut document,
            &[
                update("ab", json!({ "parent": "grp", "index": "a1" })),
                Edit::remove("ab"),
            ],
        );
        assert!(document.is_empty());
    }

    #[test]
    fn indices_must_be_valid_and_distinct_among_siblings() {
        let mut document = sample();
        let clash = refused(
            &mut document,
            &[add(
                json!({ "id": "n", "parent": "grp", "index": "a1", "type": "rect" }),
            )],
        );
        assert_eq!(clash.problem.kind, ProblemKind::Index);
        assert!(clash.to_string().contains("rect \"r2\""), "{clash}");
        assert!(
            clash.to_string().contains("\"a2\" would put it in front"),
            "{clash}"
        );

        let invalid = refused(&mut document, &[update("r1", json!({ "index": "front" }))]);
        assert_eq!(invalid.problem.kind, ProblemKind::Index);
        assert!(
            invalid.to_string().contains("not a valid index"),
            "{invalid}"
        );

        // Layers and animations are ordered separately, so "a0" is free twice.
        document
            .apply(&[add(json!({ "id": "n", "parent": "folder", "index": "a1", "type": "hide", "target": "r3", "start": 10 }))])
            .unwrap();
    }

    #[test]
    fn targets_must_be_fitting_layers_in_the_same_artboard() {
        let mut document = sample();
        document
            .apply(&[
                add(json!({ "id": "ab2", "index": "a1", "type": "artboard" })),
                add(json!({ "id": "far", "parent": "ab2", "index": "a0", "type": "rect" })),
                add(json!({ "id": "label", "parent": "ab", "index": "a3", "type": "text", "content": "Hi" })),
            ])
            .unwrap();
        let animation = |ty: &str, target: &str| {
            add(
                json!({ "id": "n", "parent": "ab", "index": "a7", "type": ty, "target": target, "start": 0, "end": 100 }),
            )
        };
        let cases = [
            (animation("fade_in", "zz"), "no node has that id"),
            (animation("fade_in", "fade"), "which is an animation"),
            (animation("fade_in", "far"), "must be in the same artboard"),
            (animation("fade_in", "ab"), "put the layers in a group"),
            (
                animation("corner_radius", "label"),
                "group, rect, image, component or video",
            ),
            (
                animation("text_in", "r1"),
                "only works on these layer types: text",
            ),
            (
                update("fade", json!({ "target": "far" })),
                "must be in the same artboard",
            ),
        ];
        for (edit, expected) in cases {
            let error = refused(&mut document, std::slice::from_ref(&edit));
            assert_eq!(error.problem.kind, ProblemKind::Target, "{error}");
            assert!(error.to_string().contains(expected), "{error}");
        }
        round_trip(
            &mut document,
            &[
                animation("fill", "ab"),
                add(
                    json!({ "id": "n2", "parent": "ab", "index": "a8", "type": "counter",
                            "target": "label", "start": 0, "end": 100 }),
                ),
            ],
        );
    }

    #[test]
    fn moving_a_layer_away_from_its_animations_is_refused() {
        let mut document = sample();
        document
            .apply(&[add(
                json!({ "id": "ab2", "index": "a1", "type": "artboard" }),
            )])
            .unwrap();
        // "grp" is the target of "move" and holds "r1", the target of "fade".
        let error = refused(&mut document, &[update("grp", json!({ "parent": "ab2" }))]);
        assert_eq!(error.problem.kind, ProblemKind::Target);
        assert!(error.to_string().contains("same artboard"), "{error}");
        let error = refused(
            &mut document,
            &[update("folder", json!({ "parent": "ab2" }))],
        );
        assert_eq!(error.problem.kind, ProblemKind::Target);

        // Moving the animations along in the same batch is fine.
        round_trip(
            &mut document,
            &[
                update("grp", json!({ "parent": "ab2" })),
                update("folder", json!({ "parent": "ab2" })),
                update("fade", json!({ "parent": "ab2" })),
            ],
        );
        assert_eq!(document.artboard_of(&id("r1")).unwrap().id, id("ab2"));
    }

    #[test]
    fn check_is_a_dry_run() {
        let document = sample();
        let copy = document.clone();
        document.check(&[Edit::remove("grp")]).unwrap();
        assert!(document.check(&[Edit::remove("nope")]).is_err());
        assert_eq!(document, copy);
    }

    #[test]
    fn from_nodes_checks_every_rule() {
        let nodes: Vec<Node> = sample().nodes().cloned().collect();
        let rebuilt = Document::from_nodes("Sample", nodes.iter().rev().cloned()).unwrap();
        assert_eq!(rebuilt, sample());

        let mut twice = nodes.clone();
        twice.push(nodes[2].clone());
        assert_eq!(
            Document::from_nodes("Sample", twice).unwrap_err().kind,
            ProblemKind::DuplicateId
        );
        let orphans = nodes.iter().filter(|node| node.id != id("folder")).cloned();
        assert_eq!(
            Document::from_nodes("Sample", orphans).unwrap_err().kind,
            ProblemKind::Parent
        );
    }

    mod random_batches {
        use super::*;
        use proptest::prelude::*;

        const IDS: [&str; 10] = [
            "ab", "grp", "r1", "r2", "r3", "fade", "folder", "move", "x1", "x2",
        ];
        const INDICES: [&str; 5] = ["a0", "a1", "a2", "a3", "Zz"];

        /// A random edit over a small pool of ids, so that batches often hit
        /// existing nodes, each other, and every kind of rule.
        fn edit() -> impl Strategy<Value = Edit> {
            let id = || prop::sample::select(&IDS[..]);
            let index = || prop::sample::select(&INDICES[..]);
            prop_oneof![
                id().prop_map(Edit::remove),
                (id(), id(), index(), 0..3usize).prop_map(|(id, parent, index, kind)| {
                    let ty = ["rect", "group", "animation_group"][kind];
                    add(json!({ "id": id, "parent": parent, "index": index, "type": ty }))
                }),
                (id(), id(), id(), index()).prop_map(|(id, parent, target, index)| {
                    add(
                        json!({ "id": id, "parent": parent, "index": index, "type": "fade_in",
                                "target": target, "start": 0, "end": 10 }),
                    )
                }),
                (id(), -500..500i32, 0..150i32)
                    .prop_map(|(id, x, opacity)| update(id, json!({ "x": x, "opacity": opacity }))),
                (id(), id(), index()).prop_map(|(id, parent, index)| update(
                    id,
                    json!({ "parent": parent, "index": index })
                )),
                (id(), id()).prop_map(|(id, target)| update(id, json!({ "target": target }))),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(2000))]

            /// Whatever the batch, it either applies and can be undone and
            /// redone exactly, or it is refused and changes nothing. Either
            /// way the document still satisfies every rule.
            #[test]
            fn batches_are_atomic_and_reversible(edits in prop::collection::vec(edit(), 1..8)) {
                let mut document = sample();
                let before = document.clone();
                match document.apply(&edits) {
                    Err(error) => {
                        prop_assert_eq!(&document, &before);
                        prop_assert!(error.index < edits.len());
                    }
                    Ok(inverse) => {
                        prop_assert!(document.validate().is_ok(), "{:?}", document.validate());
                        let after = document.clone();
                        let redo = document.apply(&inverse);
                        prop_assert!(redo.is_ok(), "the inverse was refused: {:?}", redo);
                        prop_assert_eq!(&document, &before);
                        prop_assert!(document.apply(&redo.unwrap()).is_ok());
                        prop_assert_eq!(&document, &after);
                    }
                }
                // The child lists still agree with a document built afresh.
                let rebuilt = Document::from_nodes_unchecked("Sample", document.nodes_unordered().cloned());
                let order = |document: &Document| -> Vec<NodeId> {
                    document.nodes().map(|node| node.id.clone()).collect()
                };
                prop_assert_eq!(order(&document), order(&rebuilt));
            }
        }
    }

    #[test]
    fn edits_have_a_json_form() {
        let edits = vec![
            add(json!({ "id": "n", "parent": "ab", "index": "a0", "type": "rect", "x": 4 })),
            update("n", json!({ "x": 5, "shadow": null })),
            Edit::remove("n"),
            Edit::set("n", "opacity", 50),
        ];
        let written = serde_json::to_value(&edits).unwrap();
        assert_eq!(written[0]["op"], "add");
        assert_eq!(written[0]["node"]["type"], "rect");
        assert_eq!(
            written[1],
            json!({ "op": "update", "id": "n", "patch": { "x": 5, "shadow": null } })
        );
        assert_eq!(written[2], json!({ "op": "remove", "id": "n" }));
        assert_eq!(written[3]["patch"], json!({ "opacity": 50 }));
        let read: Vec<Edit> = serde_json::from_value(written).unwrap();
        assert_eq!(read, edits);
        assert_eq!(edits[1].summary(), "update \"n\"");
        assert_eq!(edits[0].summary(), "add rect \"n\"");
    }
}

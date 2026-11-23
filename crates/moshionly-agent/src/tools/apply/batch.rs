//! The batch being built: the document as the operations so far leave it,
//! the edits they produced and what to report.

use std::collections::HashMap;

use moshionly_core::{Document, Edit, Family, Node, NodeId, NodeType, Problem, ProblemKind};
use serde_json::{Value, json};

use crate::tool::{EditFailure, ToolError};
use crate::tools::args::Args;
use crate::tools::common;

/// Which part of an operation the fields of a core [`Problem`] are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    /// The `node` of an `add`.
    Node,
    /// The `set` of an `update`.
    Set,
    /// The operation itself: `parent`, `duration` and so on.
    Operation,
}

impl Owner {
    /// Where a field a core rule names is in the operation.
    fn path(self, field: &str) -> String {
        match (self, field) {
            (_, "index") => "position".to_owned(),
            (Owner::Node, "parent") => "parent".to_owned(),
            (Owner::Node, "id") => "temp_id".to_owned(),
            (Owner::Node, field) => format!("node.{field}"),
            (Owner::Set, "id") => "id".to_owned(),
            (Owner::Set, field) => format!("set.{field}"),
            (Owner::Operation, field) => field.to_owned(),
        }
    }
}

/// What one `apply_edits` call has done so far.
pub(super) struct Batch {
    /// The document with every operation so far applied.
    pub scratch: Document,
    /// The edits of every operation so far, in order.
    pub edits: Vec<Edit>,
    /// For each edit, the operation it came from.
    edit_operations: Vec<usize>,
    /// The operation being worked on, and its kind.
    current: (usize, &'static str),
    /// The real id of every node a temporary id was given to.
    temp: HashMap<String, NodeId>,
    /// Each node the batch made, as it is reported.
    pub created: Vec<Value>,
    /// Number of nodes changed by `update` and `move`.
    pub updated: usize,
    /// Number of nodes that went, counting what went along.
    pub removed: usize,
    /// Number of artboards whose duration was set.
    pub durations: usize,
    /// Things the model should know although nothing failed.
    pub warnings: Vec<String>,
}

impl Batch {
    /// A batch that starts from `document`.
    pub fn new(document: Document) -> Self {
        Self {
            scratch: document,
            edits: Vec::new(),
            edit_operations: Vec::new(),
            current: (0, ""),
            temp: HashMap::new(),
            created: Vec::new(),
            updated: 0,
            removed: 0,
            durations: 0,
            warnings: Vec::new(),
        }
    }

    /// Notes which operation the following work belongs to.
    pub fn begin(&mut self, index: usize, op: &'static str) {
        self.current = (index, op);
    }

    /// The operation that produced the edit at `index` of [`Batch::edits`].
    pub fn operation_of_edit(&self, index: usize) -> usize {
        self.edit_operations
            .get(index)
            .copied()
            .unwrap_or(self.current.0)
    }

    /// Applies the edits of the current operation to the scratch document.
    /// A broken rule fails the operation, with the field named as `owner`
    /// sees it.
    pub fn commit(&mut self, edits: Vec<Edit>, owner: Owner) -> Result<(), ToolError> {
        match self.scratch.apply(&edits) {
            Ok(_) => {
                self.edit_operations
                    .extend(std::iter::repeat_n(self.current.0, edits.len()));
                self.edits.extend(edits);
                Ok(())
            }
            Err(error) => Err(self.problem(&error.problem, owner)),
        }
    }

    /// The failure of the current operation for a rule a node breaks.
    pub fn problem(&self, problem: &Problem, owner: Owner) -> ToolError {
        let fix = match problem.kind {
            ProblemKind::Malformed => match &problem.node {
                Some(id) => match self.scratch.get(id) {
                    Some(node) => format!(
                        "get_schema with {{\"type\":\"{}\"}} lists the fields it takes and the \
                         shape of each value.",
                        node.node_type()
                    ),
                    // A node that is being added: the caller knows its type.
                    None => String::new(),
                },
                None => String::new(),
            },
            ProblemKind::UnknownNode => "Find ids with query_nodes. A node removed earlier in \
                                         this batch cannot be used after it."
                .to_owned(),
            _ => String::new(),
        };
        self.failure_at(
            problem.field.as_deref().map(|field| owner.path(field)),
            problem.message.clone(),
            fix,
        )
    }

    /// A failure of the current operation.
    pub fn failure_at(&self, field: Option<String>, message: String, fix: String) -> ToolError {
        ToolError::Edit(Box::new(EditFailure {
            operation: self.current.0,
            op: self.current.1.to_owned(),
            field,
            message: self.with_temp_names(&message),
            fix,
        }))
    }

    /// Turns any error of the current operation into an [`EditFailure`].
    pub fn to_failure(&self, error: ToolError) -> EditFailure {
        let (index, op) = self.current;
        let prefix = format!("operations[{index}]");
        let failure = match error {
            ToolError::Edit(failure) => *failure,
            ToolError::Argument { field, message } => {
                let field = match field.strip_prefix(&prefix) {
                    Some(rest) => rest.strip_prefix('.').unwrap_or(rest).to_owned(),
                    None => field,
                };
                EditFailure {
                    operation: index,
                    op: op.to_owned(),
                    field: (!field.is_empty()).then_some(field),
                    message: self.with_temp_names(&message),
                    fix: String::new(),
                }
            }
            other => EditFailure {
                operation: index,
                op: op.to_owned(),
                field: None,
                message: self.with_temp_names(&other.to_string()),
                fix: String::new(),
            },
        };
        EditFailure {
            operation: index,
            ..failure
        }
    }

    /// Messages from core name nodes by their real ids. The model knows the
    /// ones it created by their temporary ids, so those are put back.
    fn with_temp_names(&self, message: &str) -> String {
        self.temp
            .iter()
            .fold(message.to_owned(), |message, (temp, real)| {
                message.replace(&format!("\"{real}\""), &format!("\"{temp}\""))
            })
    }

    /// Records that `temp` is the name the model gave to the node `real`.
    ///
    /// A temporary id must not be the id of a node that exists, which would
    /// make every later mention ambiguous, nor be given twice.
    pub fn declare(&mut self, temp: &str, real: &NodeId, field: &str) -> Result<(), ToolError> {
        if temp.is_empty() {
            return Err(ToolError::argument(
                field,
                format!(
                    "\"{field}\" is empty. Leave it out, or give a short name such as \"card2\"."
                ),
            ));
        }
        if self.temp.contains_key(temp) {
            return Err(ToolError::argument(
                field,
                format!(
                    "\"{field}\": the temporary id \"{temp}\" was already given to another node \
                     in this batch. Give each new node its own."
                ),
            ));
        }
        if let Some(existing) = self.scratch.get(&NodeId::from(temp)) {
            return Err(ToolError::argument(
                field,
                format!(
                    "\"{field}\": \"{temp}\" is already the id of {}, so it cannot name a new \
                     node. Choose another temporary id; to change that node, use update.",
                    existing.label()
                ),
            ));
        }
        self.temp.insert(temp.to_owned(), real.clone());
        Ok(())
    }

    /// The real id behind `text`, which is the id of a node or a temporary
    /// id given earlier in the batch. `field` is where the text was found.
    pub fn resolve(&self, text: &str, field: &str) -> Result<NodeId, ToolError> {
        if let Some(real) = self.temp.get(text) {
            if self.scratch.contains(real) {
                return Ok(real.clone());
            }
            return Err(ToolError::argument(
                field,
                format!(
                    "\"{field}\": the node \"{text}\" made earlier in this batch has been removed \
                     by an operation since, so it cannot be used here."
                ),
            ));
        }
        common::node(&self.scratch, text, field).map(|node| node.id.clone())
    }

    /// The node behind an id argument, when it is given.
    pub fn id_arg(&self, args: &Args, name: &str) -> Result<Option<NodeId>, ToolError> {
        match args.string(name)? {
            Some(text) => self.resolve(&text, &args.field(name)).map(Some),
            None => Ok(None),
        }
    }

    /// The nodes an operation works on: `id`, or every one of `ids`.
    pub fn targets(&self, args: &Args, what: &str) -> Result<Vec<NodeId>, ToolError> {
        let ids = match (args.string("id")?, args.strings("ids")?) {
            (Some(_), Some(_)) => {
                return Err(ToolError::argument(
                    args.field("ids"),
                    format!(
                        "Give \"{}\" or \"{}\", not both.",
                        args.field("id"),
                        args.field("ids")
                    ),
                ));
            }
            (Some(id), None) => vec![(args.field("id"), id)],
            (None, Some(ids)) if !ids.is_empty() => ids
                .into_iter()
                .enumerate()
                .map(|(at, id)| (format!("{}[{at}]", args.field("ids")), id))
                .collect(),
            _ => return Err(args.missing("id", what)),
        };
        ids.iter()
            .map(|(field, id)| self.resolve(id, field))
            .collect()
    }

    /// The node with this id in the scratch document. Only for ids that
    /// were just resolved or created.
    pub fn node(&self, id: &NodeId) -> Result<&Node, ToolError> {
        self.scratch.get(id).ok_or_else(|| {
            self.failure_at(
                None,
                format!("The node \"{id}\" is gone."),
                "Find ids with query_nodes.".to_owned(),
            )
        })
    }

    /// Reports a node the batch made.
    pub fn created(&mut self, id: &NodeId, temp: Option<&str>, extra: Option<(&str, Value)>) {
        let Some(node) = self.scratch.get(id) else {
            return;
        };
        let mut entry = common::brief(node);
        if let Some(temp) = temp {
            entry["temp_id"] = json!(temp);
        }
        if let Some((name, value)) = extra {
            entry[name] = value;
        }
        entry["operation"] = json!(self.current.0);
        self.created.push(entry);
    }

    /// A few words for the undo menu, saying what the batch did.
    pub fn summary(&self) -> String {
        let count = |n: usize, one: &str, many: &str| match n {
            1 => format!("1 {one}"),
            n => format!("{n} {many}"),
        };
        let mut layers = 0;
        let mut animations = 0;
        let mut clones = 0;
        for entry in &self.created {
            let family = entry["type"]
                .as_str()
                .and_then(NodeType::parse)
                .map(NodeType::family);
            if entry.get("copies").is_some() {
                clones += 1;
            } else if family == Some(Family::Animation) {
                animations += 1;
            } else {
                layers += 1;
            }
        }
        let mut parts = Vec::new();
        match (layers, animations) {
            (0, 0) => {}
            (layers, 0) => parts.push(format!("add {}", count(layers, "layer", "layers"))),
            (0, animations) => parts.push(format!(
                "add {}",
                count(animations, "animation", "animations")
            )),
            (layers, animations) => parts.push(format!(
                "add {} and {}",
                count(layers, "layer", "layers"),
                count(animations, "animation", "animations")
            )),
        }
        if clones > 0 {
            parts.push(format!("copy {}", count(clones, "layer", "layers")));
        }
        if self.updated > 0 {
            parts.push(format!("change {}", count(self.updated, "node", "nodes")));
        }
        if self.removed > 0 {
            parts.push(format!("remove {}", count(self.removed, "node", "nodes")));
        }
        if self.durations > 0 {
            parts.push("set the duration".to_owned());
        }
        let mut summary = match parts.as_slice() {
            [] => "Change nothing".to_owned(),
            [only] => only.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        if let Some(first) = summary.get(..1) {
            summary = first.to_uppercase() + &summary[1..];
        }
        summary
    }
}

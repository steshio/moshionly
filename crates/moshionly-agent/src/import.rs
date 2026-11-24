//! Designs sent from the Figma plugin.
//!
//! The plugin posts a `moshionly-import` envelope to the local HTTP server:
//!
//! ```json
//! { "format": "moshionly-import", "version": 1,
//!   "source": { "app": "figma", "file_name": "Launch", "file_key": "abc" },
//!   "nodes": [ ... layers in Moshionly's node JSON ... ],
//!   "notes": [ { "source_id": "figma:abc:1:2", "layer_name": "Blob", "message": "..." } ] }
//! ```
//!
//! Any page on this machine could send the same, so nothing that arrives
//! is trusted. [`read_envelope`] checks the envelope and every node through
//! core, and accepts images only as embedded `data:` URIs, so a design
//! cannot point at other files on the disk. What passes becomes a
//! [`PendingImport`], which changes the document through
//! [`PendingImport::apply`] as one undo step the person can take back.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, mpsc};

use moshionly_core::import::{self, Destination, ImportError};
use moshionly_core::{Document, Node, NodeData, NodeId, NodeType};
use serde_json::Value;

use crate::session::{Origin, SessionHandle};

/// The `format` of an import envelope.
pub const FORMAT: &str = "moshionly-import";
/// The envelope version this crate reads.
pub const VERSION: u64 = 1;

/// The most notes an envelope may carry.
const MAX_NOTES: usize = 2000;
/// The longest a note's text may be, in characters.
const MAX_NOTE_CHARS: usize = 2000;
/// The longest a name in `source` may be, in characters.
const MAX_NAME_CHARS: usize = 300;
/// The most designs that wait for the app to take them. More are refused
/// until the app takes one, so a page that sends designs in a loop cannot
/// fill memory while the app is busy or slowing it down.
pub const MAX_WAITING: usize = 4;

/// Where a design came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSource {
    /// The app that sent it, such as `figma`.
    pub app: String,
    /// The name of the file it came from.
    pub file_name: Option<String>,
    /// The id of that file in the app, used in the layers' `source_id`.
    pub file_key: Option<String>,
}

/// Something the sender could not bring over exactly, and what it did
/// instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportNote {
    /// The `source_id` of the layer concerned.
    pub source_id: Option<String>,
    /// The layer's name in the other app.
    pub layer_name: Option<String>,
    /// What was approximated or flattened, and why.
    pub message: String,
}

/// What the sender asked for. The person decides in the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedMode {
    /// Add the design as new artboards.
    NewArtboards,
    /// Replace the layers that came from the same place before, keeping
    /// their animations.
    Replace,
}

/// Where an accepted design goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportTarget {
    /// New artboards to the right of the others.
    NewArtboards,
    /// In front of the layers of this artboard or group.
    Into(NodeId),
    /// In place of the layers imported from the same source before, which
    /// keep their animations. Parts of the design that were never imported
    /// become new artboards.
    ReplaceMatching,
}

/// What accepting a design did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    /// The top-level layers that were added or put in place of old ones.
    pub roots: Vec<NodeId>,
    /// How many top-level layers replaced earlier imports.
    pub replaced: usize,
    /// Animations removed because the layer they moved is gone or changed
    /// type.
    pub dropped_animations: usize,
}

/// Why an envelope was refused. Each message says what is wrong in a way
/// the plugin can show.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    /// The body is not JSON.
    #[error("the body is not JSON: {0}")]
    NotJson(String),
    /// The JSON is not an import envelope.
    #[error("this is not a Moshionly import: {0}")]
    NotAnEnvelope(String),
    /// The envelope is of a version this app does not read.
    #[error(
        "this import is version {0}, and this Moshionly reads version {VERSION}; update the \
         plugin or the app so they match"
    )]
    Version(String),
    /// A field is missing or of the wrong kind.
    #[error("\"{field}\" should be {expected}")]
    Field {
        /// The field.
        field: &'static str,
        /// What it should hold.
        expected: &'static str,
    },
    /// An image layer refers to something other than embedded pixels.
    #[error(
        "the image layer {0} does not carry its pixels as a \"data:\" URI; an import may not \
         refer to files"
    )]
    ImageNotEmbedded(String),
    /// The layers are not a design core accepts.
    #[error("the design cannot be used: {0}")]
    Design(#[from] ImportError),
}

/// A design that arrived, checked and ready to apply.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingImport {
    /// Where it came from.
    pub source: ImportSource,
    /// The layers, as checked by core. Their ids are the sender's; fresh
    /// ids are given when the design is applied.
    pub nodes: Vec<Node>,
    /// What the sender could not bring over exactly.
    pub notes: Vec<ImportNote>,
    /// What the sender asked for, when it said.
    pub mode: Option<RequestedMode>,
}

impl fmt::Display for PendingImport {
    /// A line for the person: `12 layers from "Launch" (figma), 2 notes`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let layers = self.nodes.len();
        write!(f, "{layers} layer{}", if layers == 1 { "" } else { "s" })?;
        match &self.source.file_name {
            Some(name) => write!(f, " from \"{name}\" ({})", self.source.app)?,
            None => write!(f, " from {}", self.source.app)?,
        }
        match self.notes.len() {
            0 => Ok(()),
            1 => write!(f, ", 1 note"),
            notes => write!(f, ", {notes} notes"),
        }
    }
}

/// Reads and checks an import envelope.
///
/// Fields this version does not know are ignored, so a newer plugin keeps
/// working; everything it does know is checked, and every node goes
/// through core's import rules.
pub fn read_envelope(body: &[u8]) -> Result<PendingImport, EnvelopeError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| EnvelopeError::NotJson(error.to_string()))?;
    let Some(fields) = value.as_object() else {
        return Err(EnvelopeError::NotAnEnvelope(
            "it is not a JSON object".into(),
        ));
    };
    if fields.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(EnvelopeError::NotAnEnvelope(format!(
            "\"format\" should be \"{FORMAT}\""
        )));
    }
    match fields.get("version") {
        Some(version) if version.as_u64() == Some(VERSION) => {}
        Some(version) => return Err(EnvelopeError::Version(version.to_string())),
        None => {
            return Err(EnvelopeError::Field {
                field: "version",
                expected: "a number",
            });
        }
    }
    let source = read_source(fields.get("source"))?;
    let Some(Value::Array(list)) = fields.get("nodes") else {
        return Err(EnvelopeError::Field {
            field: "nodes",
            expected: "a list of layers",
        });
    };
    let nodes = import::parse_nodes(&Value::Array(list.clone()))?;
    for node in &nodes {
        if let NodeData::Image(image) = &node.data
            && !image
                .source
                .trim_start()
                .get(..5)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
        {
            return Err(EnvelopeError::ImageNotEmbedded(node.label()));
        }
    }
    // A dry run on an empty file catches everything core would refuse.
    let empty = Document::new("");
    let imported = import::insert(&empty, &nodes, &Destination::NewArtboards)?;
    empty
        .check(&imported.edits)
        .map_err(|error| EnvelopeError::Design(ImportError::Invalid(error.problem)))?;

    let notes = read_notes(fields.get("notes"))?;
    let mode = match fields.get("mode").and_then(Value::as_str) {
        Some(mode) if mode.contains("replace") => Some(RequestedMode::Replace),
        Some(mode) if mode.contains("new") => Some(RequestedMode::NewArtboards),
        _ => None,
    };
    Ok(PendingImport {
        source,
        nodes,
        notes,
        mode,
    })
}

fn short(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn read_source(value: Option<&Value>) -> Result<ImportSource, EnvelopeError> {
    let wrong = EnvelopeError::Field {
        field: "source",
        expected: "an object with \"app\"",
    };
    let Some(Value::Object(source)) = value else {
        return Err(wrong);
    };
    let text = |name: &str| {
        source
            .get(name)
            .and_then(Value::as_str)
            .map(|text| short(text, MAX_NAME_CHARS))
    };
    Ok(ImportSource {
        app: text("app").filter(|app| !app.is_empty()).ok_or(wrong)?,
        file_name: text("file_name"),
        file_key: text("file_key"),
    })
}

fn read_notes(value: Option<&Value>) -> Result<Vec<ImportNote>, EnvelopeError> {
    let wrong = EnvelopeError::Field {
        field: "notes",
        expected: "a list of objects with \"message\"",
    };
    let list = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(wrong),
    };
    list.iter()
        .take(MAX_NOTES)
        .map(|note| {
            let text = |name: &str| note.get(name).and_then(Value::as_str);
            Ok(ImportNote {
                message: short(
                    text("message").ok_or(EnvelopeError::Field {
                        field: "notes",
                        expected: "a list of objects with \"message\"",
                    })?,
                    MAX_NOTE_CHARS,
                ),
                source_id: text("source_id").map(|id| short(id, MAX_NAME_CHARS)),
                layer_name: text("layer_name").map(|name| short(name, MAX_NAME_CHARS)),
            })
        })
        .collect()
}

impl PendingImport {
    /// Adds the design to the session's document as one undo step made by
    /// an import, which the person can take back.
    pub fn apply(
        &self,
        session: &SessionHandle,
        target: &ImportTarget,
    ) -> Result<ImportOutcome, ImportError> {
        session.write(|session| {
            let mut scratch = session.document().clone();
            let mut outcome = ImportOutcome::default();
            let mut edits = Vec::new();
            let mut add = |scratch: &mut Document, step: Vec<moshionly_core::Edit>| {
                scratch
                    .apply(&step)
                    .map_err(|error| ImportError::Invalid(error.problem))?;
                edits.extend(step);
                Ok::<(), ImportError>(())
            };
            match target {
                ImportTarget::NewArtboards | ImportTarget::Into(_) => {
                    let destination = match target {
                        ImportTarget::Into(parent) => Destination::Into(parent.clone()),
                        _ => Destination::NewArtboards,
                    };
                    let imported = import::insert(&scratch, &self.nodes, &destination)?;
                    add(&mut scratch, imported.edits)?;
                    outcome.roots = imported.roots;
                }
                ImportTarget::ReplaceMatching => {
                    // Parts that never came over are added together at the
                    // end, so several new frames keep their arrangement.
                    let mut fresh: Vec<Node> = Vec::new();
                    for part in self.parts() {
                        let root = &part[0];
                        let is_artboard = root.node_type() == NodeType::Artboard;
                        let earlier = root
                            .layer()
                            .and_then(|layer| layer.source_id.as_deref())
                            .filter(|source| !source.is_empty())
                            .and_then(|source| layer_from(&scratch, source))
                            // An artboard only takes an artboard's place. A
                            // frame moved into another frame in Figma since,
                            // or out onto the canvas, comes over as new.
                            .filter(|existing| {
                                scratch.get(existing).is_some_and(|node| {
                                    (node.node_type() == NodeType::Artboard) == is_artboard
                                })
                            });
                        match earlier {
                            Some(existing) => {
                                let replaced =
                                    import::replace_keeping_animations(&scratch, &existing, &part)?;
                                add(&mut scratch, replaced.edits)?;
                                outcome.roots.push(replaced.root);
                                outcome.replaced += 1;
                                outcome.dropped_animations += replaced.dropped.len();
                            }
                            None => fresh.extend(part),
                        }
                    }
                    if !fresh.is_empty() {
                        let imported =
                            import::insert(&scratch, &fresh, &Destination::NewArtboards)?;
                        add(&mut scratch, imported.edits)?;
                        outcome.roots.extend(imported.roots);
                    }
                }
            }
            session
                .apply(Origin::Import, format!("Import {self}"), &edits)
                .map_err(|error| ImportError::Invalid(error.problem))?;
            Ok(outcome)
        })
    }

    /// The design cut into its top-level layers, each followed by what is
    /// inside it.
    fn parts(&self) -> Vec<Vec<Node>> {
        let ids: HashSet<&NodeId> = self.nodes.iter().map(|node| &node.id).collect();
        self.nodes
            .iter()
            .filter(|node| {
                node.parent
                    .as_ref()
                    .is_none_or(|parent| !ids.contains(parent))
            })
            .map(|root| {
                let mut part = vec![root.clone()];
                let mut inside: HashSet<&NodeId> = HashSet::from([&root.id]);
                // Parents may come after their children in the list.
                loop {
                    let found: Vec<&Node> = self
                        .nodes
                        .iter()
                        .filter(|node| !inside.contains(&node.id))
                        .filter(|node| {
                            node.parent
                                .as_ref()
                                .is_some_and(|parent| inside.contains(parent))
                        })
                        .collect();
                    if found.is_empty() {
                        break;
                    }
                    for node in found {
                        inside.insert(&node.id);
                        part.push(node.clone());
                    }
                }
                part
            })
            .collect()
    }
}

/// The first layer of the document, in tree order, that was imported from
/// `source`.
fn layer_from(document: &Document, source: &str) -> Option<NodeId> {
    document
        .nodes()
        .find(|node| {
            node.layer()
                .and_then(|layer| layer.source_id.as_deref())
                .is_some_and(|id| id == source)
        })
        .map(|node| node.id.clone())
}

/// What became of a design handed to the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// The app has it.
    Delivered,
    /// [`MAX_WAITING`] designs are already waiting for the app.
    Full,
    /// The app no longer listens.
    Closed,
}

/// Where the HTTP server hands designs on to the app.
#[derive(Clone)]
pub struct ImportSink {
    sender: mpsc::SyncSender<PendingImport>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl fmt::Debug for ImportSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportSink")
            .field("wake", &self.wake.is_some())
            .finish()
    }
}

impl ImportSink {
    /// A sink, and the receiver the app takes pending imports from.
    ///
    /// At most [`MAX_WAITING`] designs wait in the receiver; later ones are
    /// refused until the app takes one. An app that takes designs only at a
    /// pace it chooses makes the limit slow down a sender too.
    pub fn new() -> (ImportSink, mpsc::Receiver<PendingImport>) {
        let (sender, receiver) = mpsc::sync_channel(MAX_WAITING);
        (ImportSink { sender, wake: None }, receiver)
    }

    /// The same sink, calling `wake` after each design is handed on, for
    /// example to ask the window to repaint.
    pub fn with_wake(mut self, wake: impl Fn() + Send + Sync + 'static) -> Self {
        self.wake = Some(Arc::new(wake));
        self
    }

    /// Hands a design on, unless too many are waiting or the app no longer
    /// listens.
    pub(crate) fn deliver(&self, import: PendingImport) -> Delivery {
        match self.sender.try_send(import) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => return Delivery::Full,
            Err(mpsc::TrySendError::Disconnected(_)) => return Delivery::Closed,
        }
        if let Some(wake) = &self.wake {
            wake();
        }
        Delivery::Delivered
    }
}

#[cfg(test)]
mod tests;

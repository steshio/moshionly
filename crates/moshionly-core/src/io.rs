//! Reading and writing `.moshi` project files, and finding the assets they
//! refer to.
//!
//! A project file is JSON:
//!
//! ```json
//! {
//!   "format": "moshionly",
//!   "version": 1,
//!   "name": "Launch teaser",
//!   "nodes": [
//!     {"id":"ab12","parent":null,"index":"a0","type":"artboard","name":"Scene 1"},
//!     {"id":"cd34","parent":"ab12","index":"a0","type":"rect","fill":"#0D99FF"}
//!   ]
//! }
//! ```
//!
//! Files are written with one node per line, nodes in tree order and fields
//! in a fixed order, so that two saves of the same document are identical
//! byte for byte and a change shows up as a small diff.
//!
//! The full reference is `docs/file-format.md`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::document::Document;
use crate::id::NodeId;
use crate::index;
use crate::model::{Family, Node, NodeType};
use crate::validate::Problem;

/// The value of the `format` field in every project file.
pub const FORMAT: &str = "moshionly";

/// The format version this build reads and writes.
pub const VERSION: u32 = 1;

/// The file extension of project files, without the dot.
pub const EXTENSION: &str = "moshi";

/// Why a project file could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be read from disk.
    #[error("could not read \"{}\": {source}", .path.display())]
    Io {
        /// The file that was to be read.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The text is not JSON.
    #[error("this is not a Moshionly project: it is not valid JSON ({0})")]
    NotJson(String),
    /// The JSON is not a project: `format` is missing or says something else.
    #[error("this is not a Moshionly project: \"format\" should be \"moshionly\" but is {found}")]
    WrongFormat {
        /// What `format` held, as JSON, or `missing`.
        found: String,
    },
    /// A top-level field is missing or has the wrong shape.
    #[error("the project file is damaged: \"{field}\" should be {expected}")]
    BadField {
        /// The field.
        field: &'static str,
        /// What it should hold.
        expected: &'static str,
    },
    /// The file was written by a newer version of the format.
    #[error(
        "this project was saved in format version {found}, and this version of Moshionly \
         reads up to version {supported}. Update Moshionly to open it."
    )]
    NewerVersion {
        /// The version in the file.
        found: u64,
        /// The newest version this build reads.
        supported: u32,
    },
    /// The file is of an old version that cannot be brought up to date.
    #[error("this project is in format version {found}, which can no longer be read: {reason}")]
    Migration {
        /// The version that could not be upgraded.
        found: u32,
        /// Why.
        reason: String,
    },
    /// One of the nodes cannot be read.
    #[error(
        "node {} of the project file{} cannot be read: {message}",
        .position + 1,
        .id.as_ref().map(|id| format!(" (\"{id}\")")).unwrap_or_default()
    )]
    BadNode {
        /// Position of the node in the `nodes` array, counting from 0.
        position: usize,
        /// The node's id, when it has one.
        id: Option<String>,
        /// What is wrong with it.
        message: String,
    },
    /// The nodes do not form a valid document.
    #[error("the project file breaks a rule: {0}")]
    Invalid(#[from] Problem),
}

/// Why a project file could not be saved.
#[derive(Debug, thiserror::Error)]
#[error("could not save \"{}\": {source}", .path.display())]
pub struct SaveError {
    /// The file that was to be written.
    pub path: PathBuf,
    /// What the operating system said.
    pub source: std::io::Error,
}

/// Upgrades a file's JSON by one format version, in place.
type Migration = fn(&mut Map<String, Value>) -> Result<(), String>;

/// The step that upgrades a file from `version` to `version + 1`.
///
/// When the format changes, raise [`VERSION`] and add the step here. A step
/// rewrites the raw JSON, so it keeps working however the Rust types change
/// afterwards.
fn migration_from(version: u32) -> Option<Migration> {
    // Version 1 is the first version: nothing to upgrade from yet.
    let _ = version;
    None
}

/// Brings a file's JSON from version `from` up to `to`, one step at a time.
fn migrate(
    file: &mut Map<String, Value>,
    from: u32,
    to: u32,
    step_from: impl Fn(u32) -> Option<Migration>,
) -> Result<(), LoadError> {
    for version in from..to {
        let failed = |reason: String| LoadError::Migration {
            found: version,
            reason,
        };
        let step = step_from(version)
            .ok_or_else(|| failed("this version of Moshionly has no upgrade for it".into()))?;
        step(file).map_err(failed)?;
        file.insert("version".into(), Value::from(version + 1));
    }
    Ok(())
}

/// Reads a document from the text of a project file.
pub fn parse(text: &str) -> Result<Document, LoadError> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| LoadError::NotJson(error.to_string()))?;
    from_value(value)
}

/// Reads a document from the JSON of a project file.
///
/// Older format versions are upgraded on the way. Unknown node types and
/// unknown fields are errors that name the node. Sibling indices that are
/// not valid or not distinct are regenerated in the order they sort in,
/// because hand-written files rarely get them right and nothing else depends
/// on their exact values.
pub fn from_value(value: Value) -> Result<Document, LoadError> {
    let Value::Object(mut file) = value else {
        return Err(LoadError::WrongFormat {
            found: "missing (the file is not a JSON object)".into(),
        });
    };
    match file.get("format") {
        Some(Value::String(format)) if format == FORMAT => {}
        other => {
            return Err(LoadError::WrongFormat {
                found: other.map_or("missing".into(), Value::to_string),
            });
        }
    }
    let version = file
        .get("version")
        .and_then(Value::as_u64)
        .ok_or(LoadError::BadField {
            field: "version",
            expected: "a whole number",
        })?;
    if version > u64::from(VERSION) {
        return Err(LoadError::NewerVersion {
            found: version,
            supported: VERSION,
        });
    }
    migrate(&mut file, version as u32, VERSION, migration_from)?;

    let name = match file.remove("name") {
        None => String::new(),
        Some(Value::String(name)) => name,
        Some(_) => {
            return Err(LoadError::BadField {
                field: "name",
                expected: "text",
            });
        }
    };
    let Some(Value::Array(values)) = file.remove("nodes") else {
        return Err(LoadError::BadField {
            field: "nodes",
            expected: "a list of nodes",
        });
    };
    let mut nodes = values
        .into_iter()
        .enumerate()
        .map(|(position, value)| read_node(position, value))
        .collect::<Result<Vec<Node>, LoadError>>()?;
    repair_indices(&mut nodes);
    Ok(Document::from_nodes(name, nodes)?)
}

fn read_node(position: usize, value: Value) -> Result<Node, LoadError> {
    let id = value.get("id").and_then(Value::as_str).map(str::to_owned);
    let unknown_type = match value.get("type") {
        Some(Value::String(ty)) if NodeType::parse(ty).is_none() => Some(ty.clone()),
        _ => None,
    };
    Node::from_value(value).map_err(|error| LoadError::BadNode {
        position,
        id,
        message: match unknown_type {
            Some(ty) => format!(
                "its type is \"{ty}\", which this version of Moshionly does not know. The \
                 file may come from a newer version, or the type is misspelt. Known types: {}",
                NodeType::ALL
                    .iter()
                    .map(|ty| ty.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => error.to_string(),
        },
    })
}

/// Gives fresh indices to every group of siblings whose indices are not
/// valid and distinct, keeping the order they sort in (and, for equal
/// indices, the order of the file).
fn repair_indices(nodes: &mut [Node]) {
    let mut groups: HashMap<(Option<NodeId>, Family), Vec<usize>> = HashMap::new();
    for (position, node) in nodes.iter().enumerate() {
        groups
            .entry((node.parent.clone(), node.family()))
            .or_default()
            .push(position);
    }
    for mut members in groups.into_values() {
        let mut seen = HashSet::new();
        let sound = members.iter().all(|position| {
            let index = nodes[*position].index.as_str();
            index::is_valid(index) && seen.insert(index)
        });
        if sound {
            continue;
        }
        members.sort_by(|a, b| nodes[*a].index.cmp(&nodes[*b].index).then(a.cmp(b)));
        if let Ok(fresh) = index::keys_between(None, None, members.len()) {
            for (position, index) in members.into_iter().zip(fresh) {
                nodes[position].index = index;
            }
        }
    }
}

/// The JSON of a project file for this document.
pub fn to_value(document: &Document) -> Value {
    let mut file = Map::new();
    file.insert("format".into(), Value::from(FORMAT));
    file.insert("version".into(), Value::from(VERSION));
    file.insert("name".into(), Value::from(document.name.as_str()));
    file.insert(
        "nodes".into(),
        Value::Array(document.nodes().map(Node::to_value).collect()),
    );
    Value::Object(file)
}

/// The text of a project file for this document.
///
/// The header fields come first, then one node per line in tree order, and
/// properties at their default are left out. Saving the same document twice
/// gives the same text.
pub fn to_string(document: &Document) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{{");
    let _ = writeln!(text, "  \"format\": {},", Value::from(FORMAT));
    let _ = writeln!(text, "  \"version\": {VERSION},");
    let _ = writeln!(text, "  \"name\": {},", Value::from(document.name.as_str()));
    let _ = write!(text, "  \"nodes\": [");
    for (position, node) in document.nodes().enumerate() {
        let separator = if position == 0 { "" } else { "," };
        let _ = write!(text, "{separator}\n    {}", node.to_value());
    }
    if !document.is_empty() {
        let _ = write!(text, "\n  ");
    }
    let _ = writeln!(text, "]");
    let _ = writeln!(text, "}}");
    text
}

/// Loads a project file from disk.
pub fn load(path: impl AsRef<Path>) -> Result<Document, LoadError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_owned(),
        source,
    })?;
    parse(&text)
}

/// Saves a document to disk.
///
/// The text is written to a temporary file next to the target and then
/// moved over it, so a crash or a full disk never leaves half a project.
pub fn save(document: &Document, path: impl AsRef<Path>) -> Result<(), SaveError> {
    let path = path.as_ref();
    let failed = |source: std::io::Error| SaveError {
        path: path.to_owned(),
        source,
    };
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, to_string(document)).map_err(failed)?;
    std::fs::rename(&temporary, path).map_err(|source| {
        let _ = std::fs::remove_file(&temporary);
        failed(source)
    })
}

/// Where the content of an image layer's `source` is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asset {
    /// A file on disk, as an absolute path whenever the project file's
    /// location is known.
    File(PathBuf),
    /// Bytes embedded in the document as a `data:` URI.
    Data {
        /// The media type from the URI, for example `image/png`. Empty when
        /// the URI does not say.
        media_type: String,
        /// The decoded content.
        bytes: Vec<u8>,
    },
}

/// Why an asset reference cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssetError {
    /// The source is empty.
    #[error("the source is empty; it should be a file path or a \"data:\" URI")]
    Empty,
    /// The `data:` URI is malformed.
    #[error("the \"data:\" URI cannot be read: {0}")]
    BadDataUri(String),
    /// The source is a web address or another kind of link.
    #[error(
        "\"{0}\" is a link, and Moshionly only loads images from files and \"data:\" URIs; \
         download the image and use its path"
    )]
    Unsupported(String),
}

/// Finds the content an image layer's `source` refers to.
///
/// A source is one of:
///
/// - a `data:` URI, which carries the bytes itself;
/// - an absolute path;
/// - a path relative to the folder of the project file, written with `/`.
///   When `project_file` is `None` (the project has not been saved yet) a
///   relative path is returned as it is.
///
/// A `file://` prefix is accepted on paths. This does not touch the disk:
/// whether the file exists is for the caller to find out.
pub fn resolve_asset(source: &str, project_file: Option<&Path>) -> Result<Asset, AssetError> {
    let source = source.trim();
    if source.is_empty() {
        return Err(AssetError::Empty);
    }
    if let Some(rest) = strip_prefix_ignoring_case(source, "data:") {
        return decode_data_uri(rest);
    }
    let path = strip_prefix_ignoring_case(source, "file://").unwrap_or(source);
    if path == source && has_scheme(source) {
        return Err(AssetError::Unsupported(source.to_owned()));
    }
    let path = Path::new(path);
    Ok(Asset::File(match project_file.and_then(Path::parent) {
        Some(folder) if path.is_relative() => folder.join(path),
        _ => path.to_owned(),
    }))
}

/// The `source` to store for an image file: relative to the project's folder
/// with `/` separators when the image is inside it, absolute otherwise.
///
/// Keeping images in or below the project's folder makes the project
/// portable: the folder can be moved or shared as a whole.
pub fn asset_source_for(image: &Path, project_file: Option<&Path>) -> String {
    let inside = project_file
        .and_then(Path::parent)
        .and_then(|folder| image.strip_prefix(folder).ok());
    match inside {
        Some(relative) => relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        None => image.to_string_lossy().into_owned(),
    }
}

/// A `data:` URI that embeds `bytes` in the document.
pub fn data_uri(media_type: &str, bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut uri = format!("data:{media_type};base64,");
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().enumerate().fold(0u32, |group, (i, byte)| {
            group | (u32::from(*byte) << (16 - 8 * i))
        });
        for position in 0..4 {
            if position <= chunk.len() {
                uri.push(ALPHABET[((group >> (18 - 6 * position)) & 63) as usize] as char);
            } else {
                uri.push('=');
            }
        }
    }
    uri
}

fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Whether the text starts like `https://` or `mailto:`, as opposed to a
/// path such as `photos/a.png` or `C:\photos\a.png`.
fn has_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once(':') else {
        return false;
    };
    scheme.len() > 1
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
}

/// Decodes what follows `data:` in a data URI.
fn decode_data_uri(rest: &str) -> Result<Asset, AssetError> {
    let (header, payload) = rest.split_once(',').ok_or_else(|| {
        AssetError::BadDataUri("there is no comma between the header and the data".into())
    })?;
    let mut parts = header.split(';');
    let media_type = parts.next().unwrap_or_default().trim().to_owned();
    let base64 = parts.any(|part| part.trim().eq_ignore_ascii_case("base64"));
    let bytes = if base64 {
        decode_base64(payload)?
    } else {
        decode_percent(payload)
    };
    Ok(Asset::Data { media_type, bytes })
}

fn decode_base64(text: &str) -> Result<Vec<u8>, AssetError> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => {
                return Err(AssetError::BadDataUri(format!(
                    "\"{}\" is not a base64 character",
                    c as char
                )));
            }
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Ok(bytes)
}

fn decode_percent(text: &str) -> Vec<u8> {
    let raw = text.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let escaped = (raw[i] == b'%')
            .then(|| text.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                bytes.push(byte);
                i += 3;
            }
            None => {
                bytes.push(raw[i]);
                i += 1;
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;
    use serde_json::json;

    /// The example from the architecture document.
    const EXAMPLE: &str = r##"{
  "format": "moshionly",
  "version": 1,
  "name": "Launch teaser",
  "nodes": [
    { "id": "ab12", "parent": null,   "index": "a0", "type": "artboard", "name": "Scene 1",
      "x": 0, "y": 0, "width": 1080, "height": 1080, "duration": 4000, "fill": "#FFFFFF" },
    { "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "name": "Card",
      "x": 240, "y": 340, "width": 600, "height": 400, "fill": "#0D99FF", "corner_radius": 32 },
    { "id": "ef56", "parent": "ab12", "index": "a0", "type": "fade_in",
      "target": "cd34", "start": 0, "end": 600, "easing": { "type": "smooth", "intensity": 50 } }
  ]
}"##;

    fn file(nodes: Value) -> Value {
        json!({ "format": "moshionly", "version": 1, "name": "Test", "nodes": nodes })
    }

    #[test]
    fn reads_the_architecture_example_and_writes_it_compactly() {
        let document = parse(EXAMPLE).unwrap();
        assert_eq!(document.name, "Launch teaser");
        assert_eq!(document.len(), 3);
        let text = to_string(&document);
        assert_eq!(
            text,
            r##"{
  "format": "moshionly",
  "version": 1,
  "name": "Launch teaser",
  "nodes": [
    {"id":"ab12","parent":null,"index":"a0","type":"artboard","name":"Scene 1","width":1080,"height":1080,"fill":"#FFFFFF"},
    {"id":"cd34","parent":"ab12","index":"a0","type":"rect","name":"Card","x":240,"y":340,"width":600,"height":400,"fill":"#0D99FF","corner_radius":32},
    {"id":"ef56","parent":"ab12","index":"a0","type":"fade_in","target":"cd34","start":0,"end":600}
  ]
}
"##
        );
        assert_eq!(parse(&text).unwrap(), document);
        assert_eq!(from_value(to_value(&document)).unwrap(), document);
    }

    #[test]
    fn saving_does_not_depend_on_the_order_of_edits() {
        let nodes: Vec<Node> = parse(EXAMPLE).unwrap().nodes().cloned().collect();
        let mut forward = Document::new("Launch teaser");
        forward
            .apply(&nodes.iter().cloned().map(Edit::add).collect::<Vec<_>>())
            .unwrap();
        let mut backward = Document::new("Launch teaser");
        backward
            .apply(
                &nodes
                    .iter()
                    .rev()
                    .cloned()
                    .map(Edit::add)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(to_string(&forward), to_string(&backward));
    }

    #[test]
    fn an_empty_document_round_trips() {
        let empty = Document::new("Nothing \"yet\"");
        let text = to_string(&empty);
        assert_eq!(
            text,
            "{\n  \"format\": \"moshionly\",\n  \"version\": 1,\n  \"name\": \"Nothing \\\"yet\\\"\",\n  \"nodes\": []\n}\n"
        );
        assert_eq!(parse(&text).unwrap(), empty);
    }

    #[test]
    fn other_files_are_told_apart_from_projects() {
        assert!(matches!(
            parse("not json at all"),
            Err(LoadError::NotJson(_))
        ));
        assert!(matches!(
            parse("[1, 2]"),
            Err(LoadError::WrongFormat { .. })
        ));
        let lottie = parse(r#"{ "v": "5.7", "layers": [] }"#).unwrap_err();
        assert!(matches!(lottie, LoadError::WrongFormat { .. }));
        assert!(
            lottie.to_string().contains("not a Moshionly project"),
            "{lottie}"
        );
        let other = parse(r#"{ "format": "sketch", "version": 1, "nodes": [] }"#).unwrap_err();
        assert!(other.to_string().contains("\"sketch\""), "{other}");
    }

    #[test]
    fn versions_are_checked() {
        let newer =
            from_value(json!({ "format": "moshionly", "version": 7, "nodes": [] })).unwrap_err();
        assert!(matches!(
            newer,
            LoadError::NewerVersion {
                found: 7,
                supported: VERSION
            }
        ));
        assert!(newer.to_string().contains("Update Moshionly"), "{newer}");

        let missing = from_value(json!({ "format": "moshionly", "nodes": [] })).unwrap_err();
        assert!(matches!(
            missing,
            LoadError::BadField {
                field: "version",
                ..
            }
        ));
        let older =
            from_value(json!({ "format": "moshionly", "version": 0, "nodes": [] })).unwrap_err();
        assert!(matches!(older, LoadError::Migration { found: 0, .. }));
    }

    #[test]
    fn migrations_run_one_version_at_a_time() {
        // A made-up history: version 1 called the name "title", version 2
        // kept nodes under "layers".
        fn rename_title(file: &mut Map<String, Value>) -> Result<(), String> {
            let title = file.remove("title").ok_or("there is no title")?;
            file.insert("name".into(), title);
            Ok(())
        }
        fn rename_layers(file: &mut Map<String, Value>) -> Result<(), String> {
            let layers = file.remove("layers").unwrap_or_else(|| json!([]));
            file.insert("nodes".into(), layers);
            Ok(())
        }
        let steps = |version: u32| -> Option<Migration> {
            match version {
                1 => Some(rename_title),
                2 => Some(rename_layers),
                _ => None,
            }
        };
        let old = json!({ "format": "moshionly", "version": 1, "title": "Old", "layers": [1] });
        let mut file = old.as_object().unwrap().clone();
        migrate(&mut file, 1, 3, steps).unwrap();
        assert_eq!(
            Value::Object(file),
            json!({ "format": "moshionly", "version": 3, "name": "Old", "nodes": [1] })
        );

        let mut broken = json!({ "version": 1 }).as_object().unwrap().clone();
        let error = migrate(&mut broken, 1, 3, steps).unwrap_err();
        assert!(matches!(error, LoadError::Migration { found: 1, .. }));
        assert!(error.to_string().contains("there is no title"), "{error}");
        let mut ancient = Map::new();
        assert!(matches!(
            migrate(&mut ancient, 0, 3, steps),
            Err(LoadError::Migration { found: 0, .. })
        ));
    }

    #[test]
    fn bad_nodes_are_named() {
        let unknown_type = from_value(file(json!([
            { "id": "ab", "index": "a0", "type": "artboard" },
            { "id": "hx", "parent": "ab", "index": "a0", "type": "hexagon" }
        ])))
        .unwrap_err();
        let message = unknown_type.to_string();
        assert!(matches!(
            unknown_type,
            LoadError::BadNode { position: 1, .. }
        ));
        assert!(
            message.starts_with("node 2 of the project file (\"hx\") cannot be read"),
            "{message}"
        );
        assert!(
            message.contains("\"hexagon\"") && message.contains("mask_group"),
            "{message}"
        );

        let unknown_field = from_value(file(json!([
            { "id": "ab", "index": "a0", "type": "artboard", "colour": "#FFFFFF" }
        ])))
        .unwrap_err();
        assert!(
            unknown_field.to_string().contains("unknown field `colour`"),
            "{unknown_field}"
        );

        let no_id = from_value(file(json!([{ "index": "a0", "type": "artboard" }]))).unwrap_err();
        assert!(
            no_id
                .to_string()
                .starts_with("node 1 of the project file cannot be read"),
            "{no_id}"
        );

        let not_a_list =
            from_value(json!({ "format": "moshionly", "version": 1, "nodes": {} })).unwrap_err();
        assert!(matches!(
            not_a_list,
            LoadError::BadField { field: "nodes", .. }
        ));
    }

    #[test]
    fn structure_and_values_are_validated() {
        let orphan = from_value(file(json!([
            { "id": "r1", "parent": "nowhere", "index": "a0", "type": "rect" }
        ])))
        .unwrap_err();
        assert!(matches!(orphan, LoadError::Invalid(_)));
        assert!(
            orphan.to_string().contains("no node has that id"),
            "{orphan}"
        );

        let out_of_range = from_value(file(json!([
            { "id": "ab", "index": "a0", "type": "artboard", "duration": 0 }
        ])))
        .unwrap_err();
        assert!(
            out_of_range.to_string().contains("\"duration\""),
            "{out_of_range}"
        );

        let twice = from_value(file(json!([
            { "id": "ab", "index": "a0", "type": "artboard" },
            { "id": "ab", "index": "a1", "type": "artboard" }
        ])))
        .unwrap_err();
        assert!(twice.to_string().contains("already used"), "{twice}");
    }

    #[test]
    fn sloppy_indices_are_repaired_in_order() {
        let document = from_value(file(json!([
            { "id": "ab", "index": "0", "type": "artboard" },
            { "id": "c", "parent": "ab", "index": "2", "type": "rect" },
            { "id": "a", "parent": "ab", "index": "1", "type": "rect" },
            { "id": "b", "parent": "ab", "index": "1", "type": "rect" },
            { "id": "f2", "parent": "ab", "index": "a1", "type": "show", "target": "a", "start": 0 },
            { "id": "f1", "parent": "ab", "index": "a0", "type": "show", "target": "a", "start": 0 }
        ])))
        .unwrap();
        let layers = document.child_nodes(&NodeId::from("ab"), Family::Layer);
        let layers: Vec<(&str, &str)> = layers
            .map(|node| (node.id.as_str(), node.index.as_str()))
            .collect();
        assert_eq!(layers, [("a", "a0"), ("b", "a1"), ("c", "a2")]);
        // Groups that were fine keep their indices.
        let rows = document.child_nodes(&NodeId::from("ab"), Family::Animation);
        let rows: Vec<(&str, &str)> = rows
            .map(|node| (node.id.as_str(), node.index.as_str()))
            .collect();
        assert_eq!(rows, [("f1", "a0"), ("f2", "a1")]);
        assert_eq!(document.get(&NodeId::from("ab")).unwrap().index, "a0");
    }

    #[test]
    fn save_and_load_through_a_file() {
        let folder = std::env::temp_dir().join(format!("moshionly-io-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("project.moshi");
        let document = parse(EXAMPLE).unwrap();
        save(&document, &path).unwrap();
        save(&document, &path).unwrap();
        assert_eq!(load(&path).unwrap(), document);
        let leftovers: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temporary file is left behind");

        let missing = load(folder.join("missing.moshi")).unwrap_err();
        assert!(matches!(missing, LoadError::Io { .. }));
        assert!(missing.to_string().contains("missing.moshi"), "{missing}");
        let nowhere = save(&document, folder.join("no/such/folder/p.moshi")).unwrap_err();
        assert!(nowhere.to_string().contains("could not save"), "{nowhere}");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn assets_resolve_against_the_project_folder() {
        let project = Path::new("/work/teaser/teaser.moshi");
        assert_eq!(
            resolve_asset("assets/photo.png", Some(project)).unwrap(),
            Asset::File(PathBuf::from("/work/teaser/assets/photo.png"))
        );
        assert_eq!(
            resolve_asset("/shared/logo.svg", Some(project)).unwrap(),
            Asset::File(PathBuf::from("/shared/logo.svg"))
        );
        assert_eq!(
            resolve_asset("file:///shared/logo.svg", Some(project)).unwrap(),
            Asset::File(PathBuf::from("/shared/logo.svg"))
        );
        assert_eq!(
            resolve_asset(" photo.png ", None).unwrap(),
            Asset::File(PathBuf::from("photo.png"))
        );
        assert_eq!(resolve_asset("  ", Some(project)), Err(AssetError::Empty));
        let link = resolve_asset("https://example.com/a.png", Some(project)).unwrap_err();
        assert!(matches!(link, AssetError::Unsupported(_)));
        assert!(link.to_string().contains("download the image"), "{link}");
    }

    #[test]
    fn stored_sources_are_relative_inside_the_project_folder() {
        let project = Path::new("/work/teaser/teaser.moshi");
        assert_eq!(
            asset_source_for(Path::new("/work/teaser/assets/photo.png"), Some(project)),
            "assets/photo.png"
        );
        assert_eq!(
            asset_source_for(Path::new("/elsewhere/photo.png"), Some(project)),
            "/elsewhere/photo.png"
        );
        assert_eq!(asset_source_for(Path::new("/a/b.png"), None), "/a/b.png");
        // And back again.
        let stored = asset_source_for(Path::new("/work/teaser/assets/photo.png"), Some(project));
        assert_eq!(
            resolve_asset(&stored, Some(project)).unwrap(),
            Asset::File(PathBuf::from("/work/teaser/assets/photo.png"))
        );
    }

    #[test]
    fn data_uris_carry_their_bytes() {
        for bytes in [&b""[..], b"M", b"Mo", b"Mos", b"Moshionly \x00\xff\x10"] {
            let uri = data_uri("image/png", bytes);
            assert_eq!(
                resolve_asset(&uri, None).unwrap(),
                Asset::Data {
                    media_type: "image/png".into(),
                    bytes: bytes.to_vec()
                },
                "{uri}"
            );
        }
        assert_eq!(data_uri("text/plain", b"Mo"), "data:text/plain;base64,TW8=");
        assert_eq!(
            resolve_asset("data:text/plain;charset=utf-8,Hello%2C%20world%21", None).unwrap(),
            Asset::Data {
                media_type: "text/plain".into(),
                bytes: b"Hello, world!".to_vec()
            }
        );
        assert_eq!(
            resolve_asset("DATA:;base64,TW9z\n", None).unwrap(),
            Asset::Data {
                media_type: String::new(),
                bytes: b"Mos".to_vec()
            }
        );
        assert!(matches!(
            resolve_asset("data:image/png;base64", None),
            Err(AssetError::BadDataUri(_))
        ));
        assert!(matches!(
            resolve_asset("data:image/png;base64,***", None),
            Err(AssetError::BadDataUri(_))
        ));
        // The placeholder image of new image layers is a real PNG.
        let Asset::Data { media_type, bytes } =
            resolve_asset(crate::build::PLACEHOLDER_IMAGE, None).unwrap()
        else {
            panic!("expected embedded data");
        };
        assert_eq!(media_type, "image/png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}

//! Checks that what the Figma plugin produces is what Moshionly accepts.
//!
//! ```text
//! cargo run --manifest-path plugins/figma/acceptance/Cargo.toml -- check <envelope.json>...
//! cargo run --manifest-path plugins/figma/acceptance/Cargo.toml -- fields [--check] <core-fields.json>
//! ```
//!
//! `check` runs each envelope through the code behind the app's `/import`
//! endpoint, then imports it into an empty project three ways, validating
//! the project after each: as new artboards, into an existing artboard, and
//! once more over itself after every imported layer got an animation, which
//! must keep every animation (what "Replace the matching layers" does when a
//! design is sent again).
//!
//! `fields` writes the fields of every layer type as core defines them, or
//! with `--check` says whether the file is up to date. The plugin's Node
//! tests check their output against that file, so they need no Rust.
//!
//! This crate is not part of the Moshionly workspace and CI does not build
//! it; `plugins/figma/acceptance/run.sh` runs both checks on the fixtures.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::process::ExitCode;

use moshionly_agent::import::{ImportTarget, PendingImport, read_envelope};
use moshionly_agent::{Origin, SessionHandle};
use moshionly_core::schema::{field_names, read_node};
use moshionly_core::{Document, Edit, Family, NodeId, NodeType, index};
use serde_json::json;

const USAGE: &str = "usage: acceptance check <envelope.json>...\n       acceptance fields [--check] <core-fields.json>";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.first().map(String::as_str) {
        Some("check") if arguments.len() > 1 => check_all(&arguments[1..]),
        Some("fields") => fields(&arguments[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn check_all(files: &[String]) -> Result<(), String> {
    let mut failures = 0;
    for file in files {
        match check(Path::new(file)) {
            Ok(summary) => println!("ok    {file}: {summary}"),
            Err(message) => {
                failures += 1;
                println!("FAIL  {file}: {message}");
            }
        }
    }
    match failures {
        0 => Ok(()),
        _ => Err(format!(
            "{failures} of {} designs were not accepted",
            files.len()
        )),
    }
}

/// Imports one envelope every way the app can, and says what happened.
fn check(path: &Path) -> Result<String, String> {
    let body = std::fs::read(path).map_err(|error| format!("cannot read it: {error}"))?;
    // Exactly what `POST /import` does with the body before the person sees it.
    let pending = read_envelope(&body).map_err(|error| format!("/import refuses it: {error}"))?;

    let session = SessionHandle::from_document(Document::new("Acceptance"));
    pending
        .apply(&session, &ImportTarget::NewArtboards)
        .map_err(|error| format!("as new artboards: {error}"))?;
    validate(&session, "as new artboards")?;

    let into = SessionHandle::from_document((*session.document()).clone());
    let artboard = into.document().artboards()[0].clone();
    pending
        .apply(&into, &ImportTarget::Into(artboard))
        .map_err(|error| format!("into an artboard: {error}"))?;
    validate(&into, "into an artboard")?;

    let animated = animate_everything(&session)?;
    let again = pending
        .apply(&session, &ImportTarget::ReplaceMatching)
        .map_err(|error| format!("sent again: {error}"))?;
    validate(&session, "sent again")?;
    let roots = top_level(&pending);
    if again.replaced != roots {
        return Err(format!(
            "sent again, {} of {roots} top-level layers replaced their earlier version",
            again.replaced
        ));
    }
    let document = session.document();
    let kept = document
        .nodes()
        .filter(|node| node.family() == Family::Animation)
        .count();
    if again.dropped_animations != 0 || kept != animated {
        return Err(format!(
            "sent again, {kept} of {animated} animations were kept ({} dropped)",
            again.dropped_animations
        ));
    }
    Ok(format!(
        "{} layers, {} notes; new artboards, into an artboard, and sent again keeping all {animated} animations",
        pending.nodes.len(),
        pending.notes.len()
    ))
}

fn validate(session: &SessionHandle, step: &str) -> Result<(), String> {
    session
        .document()
        .validate()
        .map_err(|problem| format!("{step}, the project is invalid: {problem}"))
}

/// How many layers of the design are not inside another of its layers.
fn top_level(pending: &PendingImport) -> usize {
    let ids: HashSet<&NodeId> = pending.nodes.iter().map(|node| &node.id).collect();
    pending
        .nodes
        .iter()
        .filter(|node| {
            node.parent
                .as_ref()
                .is_none_or(|parent| !ids.contains(parent))
        })
        .count()
}

/// Gives every imported layer except artboards a fade-in, as a person
/// animating the design would. Returns how many were added.
fn animate_everything(session: &SessionHandle) -> Result<usize, String> {
    let document = session.document();
    let mut targets: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for node in document.nodes() {
        let imported = node.layer().is_some_and(|layer| layer.source_id.is_some());
        if !imported || node.node_type() == NodeType::Artboard {
            continue;
        }
        let artboard = document
            .artboard_of(&node.id)
            .ok_or_else(|| format!("{} is in no artboard", node.label()))?;
        targets
            .entry(artboard.id.clone())
            .or_default()
            .push(node.id.clone());
    }
    let mut edits = Vec::new();
    for (artboard, layers) in &targets {
        let indices =
            index::keys_between(None, None, layers.len()).map_err(|error| error.to_string())?;
        for (layer, position) in layers.iter().zip(indices) {
            let fade = read_node(json!({
                "id": format!("fade{}", edits.len()), "parent": artboard, "index": position,
                "type": "fade_in", "target": layer, "start": 0, "end": 500
            }))
            .map_err(|problem| problem.to_string())?;
            edits.push(Edit::add(fade));
        }
    }
    session
        .apply(Origin::User, "Animate every layer", &edits)
        .map_err(|error| format!("animating the layers: {error}"))?;
    Ok(edits.len())
}

/// Writes or checks the fields of every layer type.
fn fields(arguments: &[String]) -> Result<(), String> {
    let (only_check, path) = match arguments {
        [flag, path] if flag == "--check" => (true, path),
        [path] => (false, path),
        _ => return Err(USAGE.to_owned()),
    };
    let layers: BTreeMap<&str, &[String]> = NodeType::ALL
        .iter()
        .filter(|ty| ty.family() == Family::Layer)
        .map(|ty| (ty.as_str(), field_names(*ty)))
        .collect();
    let text = serde_json::to_string_pretty(&layers).map_err(|error| error.to_string())? + "\n";
    if !only_check {
        std::fs::write(path, text).map_err(|error| format!("cannot write {path}: {error}"))?;
        println!("wrote {path}");
        return Ok(());
    }
    let current = std::fs::read_to_string(path).unwrap_or_default();
    if current == text {
        println!("ok    {path} matches core");
        return Ok(());
    }
    let written: HashMap<String, Vec<String>> = serde_json::from_str(&current).unwrap_or_default();
    let changed: Vec<&str> = layers
        .iter()
        .filter(|(ty, names)| written.get(**ty).map(Vec::as_slice) != Some(**names))
        .map(|(ty, _)| *ty)
        .collect();
    Err(format!(
        "{path} is out of date for {}; write it again with `fields {path}`",
        changed.join(", ")
    ))
}

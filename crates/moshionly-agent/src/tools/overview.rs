//! `get_overview`: what is in the file, in a few lines.

use std::collections::{BTreeMap, BTreeSet};

use moshionly_core::{Document, Family, NodeData};
use serde_json::{Value, json};

use super::args::Args;
use super::common::number;
use super::{Context, ToolDef, input_schema};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static GET_OVERVIEW: ToolDef = ToolDef {
    name: "get_overview",
    title: "File overview",
    description: "Start here. Returns the file name, every artboard with its id, size, \
        duration in milliseconds and number of layers and animations, the fonts in use, and \
        whether this host can take screenshots and export. Ids from this result are what the \
        other tools take. The person may be editing at the same time, so call this again when \
        you come back to a file instead of trusting ids you read earlier.",
    read_only: true,
    destructive: false,
    schema: || input_schema(json!({}), &[]),
    run,
};

fn run(context: &Context<'_>, _args: &Args) -> Result<ToolOutput, ToolError> {
    let (document, file, revision) = context
        .session
        .read(|session| (session.snapshot(), session.file_name(), session.revision()));
    let artboards: Vec<Value> = document
        .artboards()
        .iter()
        .filter_map(|id| {
            let node = document.get(id)?;
            let NodeData::Artboard(artboard) = &node.data else {
                return None;
            };
            let animations = document
                .walk(id, Family::Animation)
                .filter(|(_, animation)| animation.target().is_some())
                .count();
            Some(json!({
                "id": id,
                "name": artboard.layer.name,
                "width": number(artboard.layer.width),
                "height": number(artboard.layer.height),
                "duration": number(artboard.duration),
                "layers": document.walk(id, Family::Layer).count(),
                "animations": animations,
            }))
        })
        .collect();

    let mut overview = json!({
        "file": file,
        "name": document.name,
        "revision": revision,
        "artboards": artboards,
        "fonts": fonts_in_use(&document),
        "host": {
            "screenshot": context.host.frames.is_some(),
            "export": context.host.exporter.as_ref().map_or(Vec::new(), |exporter| {
                exporter.formats().iter().map(|format| format.as_str()).collect()
            }),
        },
    });
    if document.artboards().is_empty() {
        overview["hint"] = json!(
            "The file is empty. Everything lives in an artboard, so add one first with \
             apply_edits: {\"op\":\"add\",\"node\":{\"type\":\"artboard\",\"name\":\"Scene\",\
             \"width\":1080,\"height\":1080}}."
        );
    }
    Ok(ToolOutput::json(&overview))
}

/// The font families the text layers of a document use, with the weights
/// used of each.
pub(crate) fn fonts_in_use(document: &Document) -> Value {
    let mut families: BTreeMap<&str, (BTreeSet<u16>, bool)> = BTreeMap::new();
    for node in document.nodes_unordered() {
        if let NodeData::Text(text) = &node.data {
            let entry = families.entry(text.font.family.as_str()).or_default();
            entry.0.insert(text.font.weight);
            entry.1 |= text.font.italic;
        }
    }
    families
        .into_iter()
        .map(|(family, (weights, italic))| {
            json!({ "family": family, "weights": weights, "italic": italic })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::tools::testing::{call, demo_tools};
    use crate::{Host, SessionHandle, Tools};
    use moshionly_core::Document;
    use serde_json::json;

    #[test]
    fn lists_artboards_counts_and_fonts() {
        let overview = call(&demo_tools(), "get_overview", json!({}));
        assert_eq!(overview["file"], "Untitled.moshi");
        assert_eq!(overview["name"], "Launch day");
        assert_eq!(
            overview["artboards"],
            json!([
                { "id": "scene", "name": "Launch", "width": 1080, "height": 1080,
                  "duration": 4000, "layers": 5, "animations": 3 },
                { "id": "second", "name": "Second", "width": 800, "height": 600,
                  "duration": 4000, "layers": 0, "animations": 0 },
            ])
        );
        assert_eq!(
            overview["fonts"],
            json!([{ "family": "Inter", "weights": [700], "italic": false }])
        );
        assert_eq!(
            overview["host"],
            json!({ "screenshot": false, "export": [] })
        );
        assert!(overview.get("hint").is_none());
    }

    #[test]
    fn an_empty_file_says_how_to_begin() {
        let tools = Tools::new(
            SessionHandle::from_document(Document::new("New")),
            Host::new(),
        );
        let overview = call(&tools, "get_overview", json!({}));
        assert_eq!(overview["artboards"], json!([]));
        assert!(
            overview["hint"]
                .as_str()
                .unwrap()
                .contains("add one first with apply_edits")
        );
    }

    #[test]
    fn takes_no_arguments() {
        let output = demo_tools().call("get_overview", json!({ "artboard": "scene" }));
        assert!(output.is_error);
        assert!(
            output
                .joined_text()
                .contains("is not an argument of this tool")
        );
    }
}

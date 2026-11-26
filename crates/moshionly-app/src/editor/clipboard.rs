//! Copying and pasting layers, within a document and between documents.
//!
//! The clipboard carries Moshionly's own JSON: a small envelope around the
//! copied nodes as they are written to files. Copying a layer takes
//! everything inside it and every animation that targets it or something
//! inside it; copying an artboard takes all of it. Pasting gives every node
//! a fresh id, so the same clipboard can be pasted again and again.
//!
//! ```json
//! { "moshionly": "clipboard", "version": 1, "nodes": [ ... ] }
//! ```
//!
//! A main component pasted with its instances is a new main that the
//! pasted instances show. An instance copied without its main keeps
//! showing it, which works wherever that main is. For a document that does
//! not have it, the clipboard also carries `"detached"`: the same nodes
//! with those instances detached into plain groups that look the same,
//! which are pasted instead, so that nothing is left pointing nowhere.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use moshionly_core::build::{self, ARTBOARD_GAP, Place};
use moshionly_core::{Document, Edit, Family, Node, NodeId, NodeType};
use serde_json::{Value, json};

use super::assets;

/// The marker that says a text is Moshionly's clipboard.
pub const MARKER: &str = "clipboard";

/// The version of the clipboard format.
pub const VERSION: u64 = 1;

/// Why a clipboard could not be pasted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasteError {
    /// The text is not Moshionly's clipboard.
    #[error("The clipboard holds no Moshionly layers.")]
    NotOurs,
    /// The clipboard is Moshionly's but cannot be read.
    #[error("The copied layers cannot be read: {0}")]
    Broken(String),
    /// There is nowhere to paste layers.
    #[error("Layers are pasted into an artboard; add an artboard first.")]
    NoArtboard,
    /// The document refused the pasted nodes.
    #[error("{0}")]
    Refused(String),
}

/// The nodes `ids` with everything that belongs to them, in tree order:
/// their insides, and the animations that target them or their insides.
/// Nodes inside another listed node are taken once, with it.
pub fn copied_nodes(document: &Document, ids: &[NodeId]) -> Vec<Node> {
    let roots: Vec<&NodeId> = ids
        .iter()
        .filter(|id| document.contains(id))
        .filter(|id| !ids.iter().any(|other| document.is_ancestor(other, id)))
        .collect();
    let mut taken: HashSet<NodeId> = HashSet::new();
    let mut nodes = Vec::new();
    let mut take = |node: &Node, nodes: &mut Vec<Node>| {
        if taken.insert(node.id.clone()) {
            nodes.push(node.clone());
        }
    };
    for root in &roots {
        let Some(node) = document.get(root) else {
            continue;
        };
        take(node, &mut nodes);
        for (_, child) in document.walk(root, Family::Layer) {
            take(child, &mut nodes);
        }
        if node.node_type() == NodeType::Artboard {
            for (_, animation) in document.walk(root, Family::Animation) {
                take(animation, &mut nodes);
            }
        }
    }
    // Animations of copied layers, from their artboards' timelines.
    let layers: HashSet<NodeId> = nodes
        .iter()
        .filter(|node| node.is_layer())
        .map(|node| node.id.clone())
        .collect();
    for artboard in document.artboards() {
        for (_, animation) in document.walk(artboard, Family::Animation) {
            if animation
                .target()
                .is_some_and(|target| layers.contains(target))
            {
                take(animation, &mut nodes);
            }
        }
    }
    nodes
}

/// The main components that instances among `nodes` show and that are
/// not among `nodes` themselves.
fn outside_mains(nodes: &[Node]) -> HashSet<&NodeId> {
    let ids: HashSet<&NodeId> = nodes.iter().map(|node| &node.id).collect();
    nodes
        .iter()
        .filter_map(Node::component)
        .filter(|main| !ids.contains(main))
        .collect()
}

/// The nodes [`copied_nodes`] takes for `ids`, with every instance whose
/// main is not taken along detached into a plain group, in a scratch copy
/// of `document`. `None` when no instance needs that.
fn detached_nodes(document: &Document, ids: &[NodeId]) -> Option<Vec<Node>> {
    let mut nodes = copied_nodes(document, ids);
    if outside_mains(&nodes).is_empty() {
        return None;
    }
    let mut scratch = document.clone();
    let mut roots = ids.to_vec();
    // Detaching copies the main's layers, which may hold instances of
    // other mains; those are detached in turn.
    for _ in 0..document.len() * 4 + 16 {
        let outside = outside_mains(&nodes);
        let Some(next) = nodes
            .iter()
            .find(|node| node.component().is_some_and(|main| outside.contains(main)))
            .map(|node| node.id.clone())
        else {
            return Some(nodes);
        };
        let (group, edits) = build::detach(&scratch, &next).ok()?;
        scratch.apply(&edits).ok()?;
        for root in &mut roots {
            if *root == next {
                *root = group.clone();
            }
        }
        nodes = copied_nodes(&scratch, &roots);
    }
    None
}

/// The clipboard text for `ids` of `document`, saved as `project_file`
/// (or never saved). Image sources are written as full paths, so that the
/// images can be found from whatever project they are pasted into.
pub fn copy(document: &Document, ids: &[NodeId], project_file: Option<&Path>) -> Option<String> {
    let mut nodes = copied_nodes(document, ids);
    if nodes.is_empty() {
        return None;
    }
    assets::absolute_sources(&mut nodes, project_file);
    let values: Vec<Value> = nodes.iter().map(Node::to_value).collect();
    let mut text = json!({ "moshionly": MARKER, "version": VERSION, "nodes": values });
    if let Some(mut detached) = detached_nodes(document, ids) {
        assets::absolute_sources(&mut detached, project_file);
        let values: Vec<Value> = detached.iter().map(Node::to_value).collect();
        text["detached"] = Value::from(values);
    }
    Some(text.to_string())
}

/// What a clipboard text holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard {
    /// The copied nodes.
    pub nodes: Vec<Node>,
    /// The same nodes with the instances whose main was not copied along
    /// detached into plain groups, when there are such instances.
    pub detached: Option<Vec<Node>>,
}

impl Clipboard {
    /// The nodes to paste into `document`: the copied nodes when the
    /// document has every main component their instances show, and
    /// otherwise the ones with those instances detached.
    pub fn nodes_for(self, document: &Document) -> Vec<Node> {
        let missing = outside_mains(&self.nodes).into_iter().any(|main| {
            document
                .get(main)
                .is_none_or(|node| node.node_type() != NodeType::Component)
        });
        match self.detached {
            Some(detached) if missing => detached,
            _ => self.nodes,
        }
    }
}

/// The nodes in a clipboard text.
pub fn read(text: &str) -> Result<Clipboard, PasteError> {
    let value: Value = serde_json::from_str(text.trim()).map_err(|_| PasteError::NotOurs)?;
    if value.get("moshionly").and_then(Value::as_str) != Some(MARKER) {
        return Err(PasteError::NotOurs);
    }
    let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version != VERSION {
        return Err(PasteError::Broken(format!(
            "they were copied by a version of Moshionly that writes clipboard version {version}"
        )));
    }
    let list = |values: &Value| -> Result<Vec<Node>, PasteError> {
        values
            .as_array()
            .ok_or_else(|| PasteError::Broken("there are no nodes in it".to_owned()))?
            .iter()
            .map(|node| {
                Node::from_value(node.clone())
                    .map_err(|error| PasteError::Broken(error.to_string()))
            })
            .collect()
    };
    let nodes = list(value.get("nodes").unwrap_or(&Value::Null))?;
    let detached = value.get("detached").map(list).transpose()?;
    Ok(Clipboard { nodes, detached })
}

/// The edits that paste `nodes` into `document`.
///
/// Layers go into `parent` (an artboard or a group) at `place`, keeping
/// their positions; their animations go to the end of the timeline of
/// `parent`'s artboard. Artboards are pasted whole to the right of the
/// others. Pasted instances show the pasted copy of their main when it is
/// pasted with them, and otherwise the main they showed. Returns the edits
/// and the ids of the pasted top-level nodes.
pub fn paste(
    document: &Document,
    nodes: &[Node],
    parent: Option<&NodeId>,
    place: &Place,
) -> Result<(Vec<Edit>, Vec<NodeId>), PasteError> {
    let ids: HashSet<&NodeId> = nodes.iter().map(|node| &node.id).collect();
    let fresh: HashMap<NodeId, NodeId> = nodes
        .iter()
        .map(|node| (node.id.clone(), NodeId::generate()))
        .collect();
    let renamed = |id: &NodeId| fresh.get(id).cloned().unwrap_or_else(|| id.clone());
    // Roots: nodes whose parent was not copied.
    let is_root = |node: &&Node| node.parent.as_ref().is_none_or(|p| !ids.contains(p));
    let layer_roots: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.is_layer() && n.node_type() != NodeType::Artboard)
        .filter(is_root)
        .collect();
    let artboard_roots: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.node_type() == NodeType::Artboard)
        .collect();
    let loose_animations: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.is_animation())
        .filter(is_root)
        .collect();

    let mut edits = Vec::new();
    let mut roots = Vec::new();

    // Artboards go to the right of the others, in a row.
    if !artboard_roots.is_empty() {
        let indices = build::indices_at(
            document,
            None,
            Family::Layer,
            &Place::Last,
            artboard_roots.len(),
            &[],
        )
        .map_err(|error| PasteError::Refused(error.to_string()))?;
        let mut x = document
            .artboards()
            .iter()
            .filter_map(|id| document.get(id)?.layer())
            .map(|layer| layer.x + layer.width + ARTBOARD_GAP)
            .fold(0.0_f64, f64::max);
        let y = document
            .artboards()
            .first()
            .and_then(|id| document.get(id)?.layer())
            .map_or(0.0, |layer| layer.y);
        for (node, index) in artboard_roots.iter().zip(indices) {
            let mut copy = (*node).clone();
            copy.id = renamed(&node.id);
            copy.index = index;
            if let Some(layer) = copy.data.layer_mut() {
                layer.x = x;
                layer.y = y;
                x += layer.width + ARTBOARD_GAP;
            }
            roots.push(copy.id.clone());
            edits.push(Edit::add(copy));
        }
    }

    if !layer_roots.is_empty() || !loose_animations.is_empty() {
        let parent = parent.ok_or(PasteError::NoArtboard)?;
        let artboard = document
            .artboard_of(parent)
            .map(|node| node.id.clone())
            .ok_or(PasteError::NoArtboard)?;
        if !layer_roots.is_empty() {
            let indices = build::indices_at(
                document,
                Some(parent),
                Family::Layer,
                place,
                layer_roots.len(),
                &[],
            )
            .map_err(|error| PasteError::Refused(error.to_string()))?;
            for (node, index) in layer_roots.iter().zip(indices) {
                let mut copy = (*node).clone();
                copy.id = renamed(&node.id);
                copy.parent = Some(parent.clone());
                copy.index = index;
                roots.push(copy.id.clone());
                edits.push(Edit::add(copy));
            }
        }
        // Animations whose folder was not copied go to the end of the
        // artboard's timeline, in order.
        if !loose_animations.is_empty() {
            let indices = build::indices_at(
                document,
                Some(&artboard),
                Family::Animation,
                &Place::Last,
                loose_animations.len(),
                &[],
            )
            .map_err(|error| PasteError::Refused(error.to_string()))?;
            for (node, index) in loose_animations.iter().zip(indices) {
                let mut copy = (*node).clone();
                copy.id = renamed(&node.id);
                copy.parent = Some(artboard.clone());
                copy.index = index;
                if let Some(target) = node.target() {
                    copy.data.set_target(renamed(target));
                }
                edits.push(Edit::add(copy));
            }
        }
    }

    // Everything else keeps its place under its copied parent.
    for node in nodes.iter().filter(|node| !is_root(node)) {
        let mut copy = node.clone();
        copy.id = renamed(&node.id);
        copy.parent = node.parent.as_ref().map(renamed);
        if let Some(target) = node.target() {
            copy.data.set_target(renamed(target));
        }
        edits.push(Edit::add(copy));
    }
    // A pasted main is a new main, which the instances pasted with it show.
    for edit in &mut edits {
        if let Edit::Add { node } = edit
            && let Some(main) = node.component().map(renamed)
        {
            node.data.set_component(main);
        }
    }
    Ok((edits, roots))
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","width":500,"height":500},
            {"id":"card","parent":"ab","index":"a0","type":"group","x":10,"y":10,"width":200,"height":200},
            {"id":"dot","parent":"card","index":"a0","type":"ellipse","x":5,"y":5,"width":20,"height":20},
            {"id":"loose","parent":"ab","index":"a1","type":"rect","x":300,"y":300},
            {"id":"fade","parent":"ab","index":"a0","type":"fade_in","target":"card","start":0,"end":500},
            {"id":"folder","parent":"ab","index":"a1","type":"animation_group","name":"Dots"},
            {"id":"pop","parent":"folder","index":"a0","type":"grow_in","target":"dot","start":100,"end":600},
            {"id":"other","parent":"ab","index":"a2","type":"fade_in","target":"loose","start":0,"end":500}
            ]}"##,
        )
        .unwrap()
    }

    fn ids(nodes: &[Node]) -> Vec<String> {
        nodes.iter().map(|node| node.id.to_string()).collect()
    }

    #[test]
    fn a_copied_layer_brings_its_insides_and_animations() {
        let doc = document();
        let nodes = copied_nodes(&doc, &[NodeId::from("card"), NodeId::from("dot")]);
        assert_eq!(ids(&nodes), ["card", "dot", "fade", "pop"]);
        let whole = copied_nodes(&doc, &[NodeId::from("ab")]);
        assert_eq!(whole.len(), 8);
    }

    #[test]
    fn the_clipboard_text_round_trips() {
        let doc = document();
        let text = copy(&doc, &[NodeId::from("loose")], None).unwrap();
        assert!(text.contains("\"moshionly\":\"clipboard\""));
        let nodes = read(&text).unwrap().nodes;
        assert_eq!(ids(&nodes), ["loose", "other"]);
        assert_eq!(read("hello"), Err(PasteError::NotOurs));
        assert_eq!(read("{\"nodes\":[]}"), Err(PasteError::NotOurs));
        assert!(matches!(
            read("{\"moshionly\":\"clipboard\",\"version\":99,\"nodes\":[]}"),
            Err(PasteError::Broken(_))
        ));
        assert!(copy(&doc, &[NodeId::from("missing")], None).is_none());
    }

    #[test]
    fn pasting_makes_fresh_copies_with_their_animations() {
        let mut doc = document();
        let nodes = read(&copy(&doc, &[NodeId::from("card")], None).unwrap())
            .unwrap()
            .nodes;
        let (edits, roots) = paste(&doc, &nodes, Some(&NodeId::from("ab")), &Place::Last).unwrap();
        doc.apply(&edits).unwrap();
        assert_eq!(roots.len(), 1);
        let copy = doc.get(&roots[0]).unwrap();
        assert_ne!(copy.id, NodeId::from("card"));
        assert_eq!(copy.layer().unwrap().x, 10.0, "same place");
        // In front of everything, with its dot and both animations.
        assert_eq!(
            doc.children(&NodeId::from("ab"), Family::Layer).last(),
            Some(&roots[0])
        );
        assert_eq!(doc.children(&roots[0], Family::Layer).len(), 1);
        assert_eq!(doc.animations_targeting(&roots[0]).len(), 1);
        let dot = doc.children(&roots[0], Family::Layer)[0].clone();
        assert_eq!(doc.animations_targeting(&dot).len(), 1);
        // Pasting again makes another copy.
        let (edits, again) = paste(&doc, &nodes, Some(&NodeId::from("ab")), &Place::Last).unwrap();
        doc.apply(&edits).unwrap();
        assert_ne!(again, roots);
    }

    #[test]
    fn layers_paste_into_another_document_and_artboards_beside_the_others() {
        let source = document();
        let mut target = io::parse(
            r##"{"format":"moshionly","version":1,"name":"Other","nodes":[
            {"id":"main","parent":null,"index":"a0","type":"artboard","width":800,"height":600}
            ]}"##,
        )
        .unwrap();
        let layers = read(&copy(&source, &[NodeId::from("loose")], None).unwrap())
            .unwrap()
            .nodes;
        let (edits, roots) =
            paste(&target, &layers, Some(&NodeId::from("main")), &Place::Last).unwrap();
        target.apply(&edits).unwrap();
        assert_eq!(
            target.get(&roots[0]).unwrap().parent,
            Some(NodeId::from("main"))
        );
        assert_eq!(target.animations_targeting(&roots[0]).len(), 1);

        let artboard = read(&copy(&source, &[NodeId::from("ab")], None).unwrap())
            .unwrap()
            .nodes;
        let (edits, roots) = paste(&target, &artboard, None, &Place::Last).unwrap();
        target.apply(&edits).unwrap();
        let pasted = target.get(&roots[0]).unwrap().layer().unwrap();
        assert_eq!(pasted.x, 800.0 + ARTBOARD_GAP);
        assert_eq!(target.artboards().len(), 2);
        assert_eq!(target.descendants(&roots[0], Family::Animation).len(), 4);
        // Layers with nowhere to go are refused.
        assert_eq!(
            paste(&Document::new("Empty"), &layers, None, &Place::Last),
            Err(PasteError::NoArtboard)
        );
    }

    /// A card component with a dot that fades in, and two instances of it
    /// on another artboard, one in a group.
    fn components() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":2,"name":"Test","nodes":[
            {"id":"home","parent":null,"index":"a0","type":"artboard","width":500,"height":500},
            {"id":"card","parent":"home","index":"a0","type":"component","name":"Card","width":200,"height":100,"fill":"#FF0000"},
            {"id":"dot","parent":"card","index":"a0","type":"ellipse","width":20,"height":20},
            {"id":"pop","parent":"home","index":"a0","type":"fade_in","target":"dot","start":0,"end":500},
            {"id":"away","parent":null,"index":"a1","type":"artboard","x":600,"width":500,"height":500},
            {"id":"copy","parent":"away","index":"a0","type":"instance","component":"card","x":10,"width":200,"height":100},
            {"id":"bag","parent":"away","index":"a1","type":"group","width":300,"height":300},
            {"id":"inner","parent":"bag","index":"a0","type":"instance","component":"card","width":200,"height":100}
            ]}"##,
        )
        .unwrap()
    }

    fn pasted_into(document: &mut Document, text: &str, parent: &str) -> Vec<NodeId> {
        let nodes = read(text).unwrap().nodes_for(document);
        let (edits, roots) =
            paste(document, &nodes, Some(&NodeId::from(parent)), &Place::Last).unwrap();
        document.apply(&edits).unwrap();
        roots
    }

    #[test]
    fn an_instance_pasted_in_its_file_keeps_its_main() {
        let mut doc = components();
        let text = copy(&doc, &[NodeId::from("copy")], None).unwrap();
        let roots = pasted_into(&mut doc, &text, "home");
        let pasted = doc.get(&roots[0]).unwrap();
        assert_eq!(pasted.node_type(), NodeType::Instance);
        assert_eq!(pasted.component(), Some(&NodeId::from("card")));
        assert_eq!(doc.instances_of(&NodeId::from("card")).len(), 3);
    }

    #[test]
    fn an_instance_pasted_where_its_main_is_missing_is_detached() {
        let source = components();
        let text = copy(&source, &[NodeId::from("bag")], None).unwrap();
        let mut other = io::parse(
            r##"{"format":"moshionly","version":2,"name":"Other","nodes":[
            {"id":"main","parent":null,"index":"a0","type":"artboard","width":800,"height":600}
            ]}"##,
        )
        .unwrap();
        let roots = pasted_into(&mut other, &text, "main");
        // The group came with a group in the place of the instance, which
        // has the main's look, its dot and the dot's fade.
        let bag = &roots[0];
        assert_eq!(other.get(bag).unwrap().node_type(), NodeType::Group);
        let inside = other.children(bag, Family::Layer).to_vec();
        assert_eq!(inside.len(), 1);
        let detached = other.get(&inside[0]).unwrap();
        assert_eq!(detached.node_type(), NodeType::Group);
        assert_eq!(
            detached.data.fill().and_then(|fill| fill.as_solid()),
            Some(moshionly_core::Color::rgb(0xFF, 0, 0))
        );
        let dot = other.children(&inside[0], Family::Layer)[0].clone();
        assert_eq!(other.animations_targeting(&dot).len(), 1);
        assert!(other.nodes().all(|node| node.component().is_none()));
        // Copied without instances, nothing is detached.
        assert!(
            !copy(&source, &[NodeId::from("dot")], None)
                .unwrap()
                .contains("detached")
        );
    }

    #[test]
    fn a_main_pasted_with_its_instances_is_a_new_main_they_show() {
        let mut doc = components();
        let text = copy(&doc, &[NodeId::from("home"), NodeId::from("away")], None).unwrap();
        assert!(!text.contains("detached"), "every main came along");
        let roots = paste(
            &doc,
            &read(&text).unwrap().nodes_for(&doc),
            None,
            &Place::Last,
        )
        .map(|(edits, roots)| {
            doc.apply(&edits).unwrap();
            roots
        })
        .unwrap();
        let new_main = doc
            .walk(&roots[0], Family::Layer)
            .find(|(_, node)| node.node_type() == NodeType::Component)
            .map(|(_, node)| node.id.clone())
            .unwrap();
        assert_ne!(new_main, NodeId::from("card"));
        assert_eq!(doc.instances_of(&new_main).len(), 2);
        assert_eq!(doc.instances_of(&NodeId::from("card")).len(), 2);
    }
}

//! Bringing a design in from another tool.
//!
//! The Figma plugin sends a design as a list of layer nodes in this crate's
//! node JSON. The list is *foreign*: its ids mean nothing here, its indices
//! are only good for ordering, and it holds no animations. The helpers in
//! this module turn such a list into edits:
//!
//! - [`insert`] adds the design with fresh ids, inside an existing artboard
//!   or group, or as new artboards;
//! - [`replace_keeping_animations`] swaps a design that was imported before
//!   for a new version of itself, and moves the animations over to the
//!   layers that are still there, recognised by their `source_id`.
//!
//! Like the helpers in [`crate::build`], nothing here changes a document.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::build::{BuildError, Place, free_canvas_spot, indices_at};
use crate::document::Document;
use crate::edit::Edit;
use crate::id::NodeId;
use crate::index::{self, IndexError};
use crate::model::{Artboard, Family, Group, LayerProps, Node, NodeData, NodeType};
use crate::schema::read_node;
use crate::validate::{self, Problem, animation_applies_to};

/// Why a design could not be imported.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// The JSON is not a list of nodes.
    #[error(
        "the design should be a list of nodes, or an object with a \"nodes\" list, but it is {0}"
    )]
    NotAList(String),
    /// There is nothing to import.
    #[error("the design has no layers")]
    Empty,
    /// A node is not a layer.
    #[error("{0} is an animation; a design to import holds layers only")]
    NotALayer(String),
    /// Two nodes of the design have the same id.
    #[error("the design has two nodes with the id \"{0}\"")]
    DuplicateId(NodeId),
    /// A node sits inside a node that cannot hold layers, or inside itself.
    #[error(
        "{child} cannot be inside {parent}: layers go in an artboard, a group, a mask_group or a component"
    )]
    BadParent {
        /// The node.
        child: String,
        /// The node it names as its parent.
        parent: String,
    },
    /// An artboard is inside another node of the design.
    #[error(
        "{0} is an artboard inside another node; artboards cannot be nested, so send inner frames as groups"
    )]
    NestedArtboard(String),
    /// Parents form a loop.
    #[error("{0} is inside itself; the parents in the design form a loop")]
    Loop(String),
    /// A node of the design breaks a rule.
    #[error("the design cannot be used as it is. {0}")]
    Invalid(#[from] Problem),
    /// The place to import to, or the design to replace, does not exist.
    #[error("there is no node with the id \"{0}\"")]
    UnknownNode(NodeId),
    /// The place to import to cannot hold layers.
    #[error(
        "{0} cannot hold layers; import into an artboard, a group, a mask_group or a component"
    )]
    BadDestination(String),
    /// A replacement needs one top-level layer to take the place of the old one.
    #[error(
        "the new design has {0} top-level layers, but a replacement needs exactly one to take the place of the old one"
    )]
    NotOneRoot(usize),
    /// An artboard can only be replaced by an artboard.
    #[error(
        "{old} is an artboard and can only be replaced by an artboard, but the new design is {new}"
    )]
    NotAnArtboard {
        /// The artboard to replace.
        old: String,
        /// The top-level layer of the new design.
        new: String,
    },
    /// No fractional index could be made.
    #[error(transparent)]
    Index(#[from] IndexError),
}

impl From<BuildError> for ImportError {
    fn from(error: BuildError) -> Self {
        match error {
            BuildError::UnknownNode(id) | BuildError::NotASibling(id) => {
                ImportError::UnknownNode(id)
            }
            BuildError::Index(error) => ImportError::Index(error),
            other => ImportError::BadDestination(other.to_string()),
        }
    }
}

/// Where an imported design goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// In front of the other layers in this artboard, group or mask group.
    /// A top-level artboard of the design becomes a clipping group.
    Into(NodeId),
    /// Onto the canvas as new artboards, to the right of the existing ones.
    /// A top-level layer of the design that is not an artboard gets an
    /// artboard of its own size around it.
    NewArtboards,
}

/// The result of [`insert`].
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The edits that add the design.
    pub edits: Vec<Edit>,
    /// The new id of every node of the design, by its id in the design.
    pub ids: HashMap<NodeId, NodeId>,
    /// The new ids of the top-level layers, in order. With
    /// [`Destination::NewArtboards`] these are the artboards.
    pub roots: Vec<NodeId>,
}

/// The result of [`replace_keeping_animations`].
#[derive(Debug, Clone, PartialEq)]
pub struct Replaced {
    /// The edits that swap the old design for the new one.
    pub edits: Vec<Edit>,
    /// The id of the layer that took the old one's place.
    pub root: NodeId,
    /// The new id of every node of the new design, by its id in the design.
    pub ids: HashMap<NodeId, NodeId>,
    /// For every old layer that was recognised in the new design, the id of
    /// the new layer that continues it.
    pub matched: HashMap<NodeId, NodeId>,
    /// The animations that are removed because their layer is gone, or
    /// became a type they do not work on.
    pub dropped: Vec<NodeId>,
}

/// Reads a design from JSON: a list of nodes, or an object with a `nodes`
/// list (a whole project file works too).
///
/// This is more forgiving than loading a project. `index` may be left out,
/// in which case nodes keep the order of the list, and `parent` may be left
/// out for top-level layers. Everything else is checked as usual, with
/// errors that name the node and the field.
pub fn parse_nodes(design: &Value) -> Result<Vec<Node>, ImportError> {
    let list = match design {
        Value::Array(list) => list,
        Value::Object(fields) => match fields.get("nodes") {
            Some(Value::Array(list)) => list,
            _ => return Err(ImportError::NotAList("an object without one".into())),
        },
        other => {
            let shown: String = other.to_string().chars().take(40).collect();
            return Err(ImportError::NotAList(shown));
        }
    };
    let order = index::keys_between(None, None, list.len())?;
    list.iter()
        .zip(order)
        .map(|(node, position)| {
            let mut node = node.clone();
            if let Value::Object(fields) = &mut node {
                fields.entry("index").or_insert_with(|| json!(position));
            }
            Ok(read_node(node)?)
        })
        .collect()
}

/// A foreign design, checked and put in order.
struct Design<'a> {
    nodes: HashMap<&'a NodeId, &'a Node>,
    /// The layers inside each node, back to front.
    children: HashMap<&'a NodeId, Vec<&'a Node>>,
    /// The top-level layers, back to front.
    roots: Vec<&'a Node>,
}

impl<'a> Design<'a> {
    fn new(list: &'a [Node]) -> Result<Self, ImportError> {
        if list.is_empty() {
            return Err(ImportError::Empty);
        }
        let mut nodes: HashMap<&NodeId, &Node> = HashMap::new();
        for node in list {
            if !node.is_layer() {
                return Err(ImportError::NotALayer(node.label()));
            }
            if nodes.insert(&node.id, node).is_some() {
                return Err(ImportError::DuplicateId(node.id.clone()));
            }
        }
        // Siblings sort by index and, where that does not decide, stay in
        // the order of the list.
        let mut ordered: Vec<(usize, &Node)> = list.iter().enumerate().collect();
        ordered.sort_by(|a, b| a.1.index.cmp(&b.1.index).then(a.0.cmp(&b.0)));

        let mut children: HashMap<&NodeId, Vec<&Node>> = HashMap::new();
        let mut roots = Vec::new();
        for (_, node) in ordered {
            match node.parent.as_ref().and_then(|parent| nodes.get(parent)) {
                None => roots.push(node),
                Some(parent) if node.node_type() == NodeType::Artboard => {
                    let _ = parent;
                    return Err(ImportError::NestedArtboard(node.label()));
                }
                Some(parent) if !parent.node_type().holds_layers() => {
                    return Err(ImportError::BadParent {
                        child: node.label(),
                        parent: parent.label(),
                    });
                }
                Some(parent) => children.entry(&parent.id).or_default().push(node),
            }
        }
        let design = Self {
            nodes,
            children,
            roots,
        };
        // Whatever cannot be reached from a root is caught in a loop.
        let reached: HashSet<&NodeId> = design.in_order().iter().map(|node| &node.id).collect();
        match list.iter().find(|node| !reached.contains(&node.id)) {
            Some(stray) => Err(ImportError::Loop(stray.label())),
            None => Ok(design),
        }
    }

    fn children_of(&self, id: &NodeId) -> &[&'a Node] {
        self.children.get(id).map_or(&[], Vec::as_slice)
    }

    /// Every node, parents before children, siblings back to front.
    fn in_order(&self) -> Vec<&'a Node> {
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut pending: Vec<&Node> = self.roots.iter().rev().copied().collect();
        while let Some(node) = pending.pop() {
            order.push(node);
            pending.extend(self.children_of(&node.id).iter().rev());
        }
        order
    }

    /// Copies of every node below the roots under fresh ids, with fresh
    /// indices, pointing at the copies of their parents. The roots
    /// themselves are left to the caller, which decides where they go.
    fn copies_below_roots(&self, ids: &HashMap<NodeId, NodeId>) -> Result<Vec<Node>, ImportError> {
        let mut copies = Vec::new();
        for node in self.in_order() {
            let children = self.children_of(&node.id);
            let indices = index::keys_between(None, None, children.len())?;
            for (child, index) in children.iter().zip(indices) {
                let mut copy = (*child).clone().with_id(ids[&child.id].clone());
                copy.parent = Some(ids[&node.id].clone());
                copy.index = index;
                copies.push(copy);
            }
        }
        Ok(copies)
    }

    fn fresh_ids(&self) -> HashMap<NodeId, NodeId> {
        self.nodes
            .keys()
            .map(|id| ((*id).clone(), NodeId::generate()))
            .collect()
    }
}

/// An artboard of a design turned into a group, for use inside another
/// artboard. It keeps its name, size and background, and clips like an
/// artboard does.
fn as_group(node: &Node) -> Node {
    let NodeData::Artboard(artboard) = &node.data else {
        return node.clone();
    };
    Node {
        id: node.id.clone(),
        parent: node.parent.clone(),
        index: node.index.clone(),
        data: Group {
            layer: LayerProps {
                x: 0.0,
                y: 0.0,
                ..artboard.layer.clone()
            },
            fill: artboard.fill.clone(),
            clip: true,
            ..Group::default()
        }
        .into(),
    }
}

fn checked(nodes: Vec<Node>) -> Result<Vec<Edit>, ImportError> {
    for node in &nodes {
        validate::check_values(node)?;
    }
    Ok(nodes.into_iter().map(Edit::add).collect())
}

/// Edits that add a foreign design to the document.
///
/// Every node gets a fresh id and fresh indices; the order of the design is
/// kept. `source_id` and all other properties are carried over as they are.
pub fn insert(
    document: &Document,
    nodes: &[Node],
    destination: &Destination,
) -> Result<Imported, ImportError> {
    let design = Design::new(nodes)?;
    let ids = design.fresh_ids();
    let mut added = Vec::new();
    let mut roots = Vec::new();
    match destination {
        Destination::Into(parent) => {
            let holder = document
                .get(parent)
                .ok_or_else(|| ImportError::UnknownNode(parent.clone()))?;
            if !holder.node_type().holds_layers() {
                return Err(ImportError::BadDestination(holder.label()));
            }
            let indices = indices_at(
                document,
                Some(parent),
                Family::Layer,
                &Place::Last,
                design.roots.len(),
                &[],
            )?;
            for (root, index) in design.roots.iter().zip(indices) {
                let mut copy = as_group(root).with_id(ids[&root.id].clone());
                copy.parent = Some(parent.clone());
                copy.index = index;
                roots.push(copy.id.clone());
                added.push(copy);
            }
        }
        Destination::NewArtboards => {
            let indices = indices_at(
                document,
                None,
                Family::Layer,
                &Place::Last,
                design.roots.len(),
                &[],
            )?;
            // The design's artboards keep their arrangement, moved as a
            // whole to free canvas. Loose layers get an artboard each, in a
            // row after them.
            let boards: Vec<&LayerProps> = design
                .roots
                .iter()
                .filter(|root| root.node_type() == NodeType::Artboard)
                .filter_map(|root| root.layer())
                .collect();
            let least = |values: Vec<f64>| values.into_iter().reduce(f64::min).unwrap_or(0.0);
            let left = least(boards.iter().map(|board| board.x).collect());
            let top = least(boards.iter().map(|board| board.y).collect());
            let (free_x, free_y) = free_canvas_spot(document);
            let mut next_x = boards
                .iter()
                .map(|board| board.x - left + board.width + crate::build::ARTBOARD_GAP)
                .fold(0.0, f64::max);
            for (root, index) in design.roots.iter().zip(indices) {
                let mut copy = (*root).clone().with_id(ids[&root.id].clone());
                if let NodeData::Artboard(artboard) = &mut copy.data {
                    artboard.layer.x += free_x - left;
                    artboard.layer.y += free_y - top;
                    copy.parent = None;
                    copy.index = index;
                    roots.push(copy.id.clone());
                    added.push(copy);
                    continue;
                }
                let (name, width, height) =
                    root.layer().map_or((String::new(), 1.0, 1.0), |layer| {
                        (
                            layer.name.clone(),
                            layer.width.max(1.0),
                            layer.height.max(1.0),
                        )
                    });
                let artboard = Node::new(Artboard {
                    layer: LayerProps::new(name, free_x + next_x, free_y, width, height),
                    duration: 4000.0,
                    fill: None,
                })
                .with_index(index);
                next_x += width + crate::build::ARTBOARD_GAP;
                if let Some(layer) = copy.data.layer_mut() {
                    (layer.x, layer.y) = (0.0, 0.0);
                }
                copy.parent = Some(artboard.id.clone());
                copy.index = index::FIRST.to_owned();
                roots.push(artboard.id.clone());
                added.push(artboard);
                added.push(copy);
            }
        }
    }
    added.extend(design.copies_below_roots(&ids)?);
    Ok(Imported {
        edits: checked(added)?,
        ids,
        roots,
    })
}

/// Edits that replace a design that is in the document with a new version
/// of it, keeping its animations.
///
/// `existing` is the top layer of the old design (an artboard, or a layer in
/// one). `nodes` is the new design, with exactly one top-level layer, which
/// takes the old one's place: same parent, same position among its siblings,
/// same `x` and `y`, and for an artboard the same timeline length.
///
/// Old and new layers are paired up by `source_id` (the two top layers are
/// a pair whatever theirs say). Where several old layers share a
/// `source_id`, the first of them in tree order goes with the first new
/// layer with that id, the second with the second, and so on. Every
/// animation on an old layer of a pair is pointed at the new layer. The
/// other animations go: those on layers that have no counterpart any more,
/// and those of a type that does not work on what their layer has become.
pub fn replace_keeping_animations(
    document: &Document,
    existing: &NodeId,
    nodes: &[Node],
) -> Result<Replaced, ImportError> {
    let old_root = document
        .get(existing)
        .filter(|node| node.is_layer())
        .ok_or_else(|| ImportError::UnknownNode(existing.clone()))?;
    let design = Design::new(nodes)?;
    let [new_root] = design.roots[..] else {
        return Err(ImportError::NotOneRoot(design.roots.len()));
    };
    let replaces_artboard = old_root.node_type() == NodeType::Artboard;
    if replaces_artboard && new_root.node_type() != NodeType::Artboard {
        return Err(ImportError::NotAnArtboard {
            old: old_root.label(),
            new: new_root.label(),
        });
    }
    let ids = design.fresh_ids();

    // The new top layer takes over the place of the old one.
    let mut root = match replaces_artboard {
        true => (*new_root).clone(),
        false => as_group(new_root),
    }
    .with_id(ids[&new_root.id].clone());
    root.parent = old_root.parent.clone();
    root.index = old_root.index.clone();
    if let (Some(new), Some(old)) = (root.data.layer_mut(), old_root.layer()) {
        (new.x, new.y) = (old.x, old.y);
    }
    if let (NodeData::Artboard(new), NodeData::Artboard(old)) = (&mut root.data, &old_root.data) {
        new.duration = old.duration;
    }
    let root_type = root.node_type();
    let root_id = root.id.clone();
    let mut added = vec![root];
    added.extend(design.copies_below_roots(&ids)?);
    let mut edits = checked(added)?;

    // Pair old layers with new ones.
    let mut matched: HashMap<NodeId, NodeId> = HashMap::from([(existing.clone(), root_id.clone())]);
    let mut new_types: HashMap<NodeId, NodeType> = HashMap::from([(root_id.clone(), root_type)]);
    let mut by_source: HashMap<&str, Vec<&Node>> = HashMap::new();
    for node in design.in_order().into_iter().skip(1) {
        new_types.insert(ids[&node.id].clone(), node.node_type());
        if let Some(source) = node.layer().and_then(|layer| layer.source_id.as_deref()) {
            by_source.entry(source).or_default().push(node);
        }
    }
    let mut taken: HashMap<&str, usize> = HashMap::new();
    for (_, old) in document.walk(existing, Family::Layer) {
        let Some(source) = old.layer().and_then(|layer| layer.source_id.as_deref()) else {
            continue;
        };
        let position = taken.entry(source).or_default();
        if let Some(new) = by_source.get(source).and_then(|list| list.get(*position)) {
            matched.insert(old.id.clone(), ids[&new.id].clone());
        }
        *position += 1;
    }

    // Move the timeline over.
    let old_layers: HashSet<&NodeId> = std::iter::once(existing)
        .chain(
            document
                .walk(existing, Family::Layer)
                .map(|(_, node)| &node.id),
        )
        .collect();
    let artboard = document
        .artboard_of(existing)
        .ok_or_else(|| ImportError::UnknownNode(existing.clone()))?;
    let mut dropped = Vec::new();
    for (depth, animation) in document.walk(&artboard.id, Family::Animation) {
        let mut patch = serde_json::Map::new();
        if replaces_artboard && depth == 1 {
            // Rows directly in the old artboard's timeline move to the new
            // one; rows in folders follow their folder.
            patch.insert("parent".into(), json!(root_id));
        }
        if let Some(target) = animation
            .target()
            .filter(|target| old_layers.contains(target))
        {
            let successor = matched.get(target).filter(|new| {
                new_types
                    .get(*new)
                    .is_some_and(|ty| animation_applies_to(animation.node_type(), *ty))
            });
            match successor {
                Some(new) => {
                    patch.insert("target".into(), json!(new));
                }
                None => {
                    // It goes with the old design when that is removed.
                    dropped.push(animation.id.clone());
                }
            }
        }
        if !patch.is_empty() {
            edits.push(Edit::update(&animation.id, patch));
        }
    }
    edits.push(Edit::remove(existing));
    Ok(Replaced {
        edits,
        root: root_id,
        ids,
        matched,
        dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    fn design(nodes: Value) -> Vec<Node> {
        parse_nodes(&nodes).unwrap()
    }

    /// A document with one artboard holding a rect.
    fn base() -> Document {
        let mut document = Document::new("Base");
        document
            .apply(&[
                Edit::add(read_node(json!({ "id": "ab", "index": "a0", "type": "artboard", "width": 800, "height": 600 })).unwrap()),
                Edit::add(read_node(json!({ "id": "old", "parent": "ab", "index": "a0", "type": "rect" })).unwrap()),
            ])
            .unwrap();
        document
    }

    /// A card as the plugin would send it: a frame with a background rect,
    /// a title and an icon group.
    fn card() -> Value {
        json!([
            { "id": "1:1", "type": "artboard", "name": "Card", "x": 4000, "y": -200, "width": 300, "height": 200,
              "fill": "#FFFFFF", "source_id": "figma:f:1:1" },
            { "id": "1:2", "parent": "1:1", "type": "rect", "name": "Background", "width": 300, "height": 200,
              "source_id": "figma:f:1:2" },
            { "id": "1:3", "parent": "1:1", "type": "text", "name": "Title", "content": "Hello", "x": 20, "y": 20,
              "source_id": "figma:f:1:3" },
            { "id": "1:4", "parent": "1:1", "type": "group", "name": "Icon", "x": 240, "y": 20, "width": 40, "height": 40,
              "source_id": "figma:f:1:4" },
            { "id": "1:5", "parent": "1:4", "type": "ellipse", "name": "Dot", "width": 40, "height": 40,
              "source_id": "figma:f:1:5" }
        ])
    }

    fn applied(document: &Document, edits: &[Edit]) -> Document {
        let mut result = document.clone();
        result
            .apply(edits)
            .unwrap_or_else(|error| panic!("{error}"));
        result.validate().unwrap();
        result
    }

    fn names(document: &Document, parent: &NodeId) -> Vec<String> {
        document
            .child_nodes(parent, Family::Layer)
            .map(|node| node.name().to_owned())
            .collect()
    }

    #[test]
    fn designs_are_read_from_a_list_or_a_file() {
        let nodes = design(card());
        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes[0].parent, None);
        // Without indices, the order of the list is the order.
        assert!(nodes.windows(2).all(|pair| pair[0].index < pair[1].index));
        assert_eq!(parse_nodes(&json!({ "nodes": card() })).unwrap(), nodes);

        assert!(matches!(
            parse_nodes(&json!("nope")),
            Err(ImportError::NotAList(_))
        ));
        assert!(matches!(
            parse_nodes(&json!({ "layers": [] })),
            Err(ImportError::NotAList(_))
        ));
        let error =
            parse_nodes(&json!([{ "id": "x", "type": "rect", "colour": "#FFFFFF" }])).unwrap_err();
        assert!(
            error.to_string().contains("Did you mean \"fill\"?"),
            "{error}"
        );
    }

    #[test]
    fn a_design_goes_into_an_artboard_with_fresh_ids() {
        let document = base();
        let imported = insert(&document, &design(card()), &Destination::Into(id("ab"))).unwrap();
        let result = applied(&document, &imported.edits);
        assert_eq!(result.len(), 2 + 5);
        assert_eq!(imported.ids.len(), 5);
        assert!(imported.ids.values().all(|new| new.as_str().len() == 12));
        assert!(
            !result.contains(&id("1:1")),
            "the design's own ids are not used"
        );

        // The frame became a clipping group in front of what was there.
        let [root] = &imported.roots[..] else {
            panic!()
        };
        assert_eq!(
            result.children(&id("ab"), Family::Layer),
            [id("old"), root.clone()]
        );
        let frame = result.get(root).unwrap();
        assert_eq!(frame.node_type(), NodeType::Group);
        let NodeData::Group(group) = &frame.data else {
            panic!()
        };
        assert!(group.clip && group.fill.is_some());
        assert_eq!(group.layer.frame(), Rect::new(0.0, 0.0, 300.0, 200.0));
        assert_eq!(group.layer.source_id.as_deref(), Some("figma:f:1:1"));
        // The tree and its order are kept.
        assert_eq!(names(&result, root), ["Background", "Title", "Icon"]);
        let icon = &imported.ids[&id("1:4")];
        assert_eq!(names(&result, icon), ["Dot"]);
        assert_eq!(
            result
                .get(&imported.ids[&id("1:5")])
                .unwrap()
                .parent
                .as_ref(),
            Some(icon)
        );
    }

    #[test]
    fn a_design_becomes_new_artboards_on_free_canvas() {
        let document = base();
        let mut nodes = card().as_array().unwrap().clone();
        nodes.push(json!({ "id": "2:1", "type": "artboard", "name": "Second", "x": 4400, "y": -200, "width": 100, "height": 100 }));
        nodes.push(json!({ "id": "3:1", "type": "star", "name": "Loose", "x": 9, "y": 9, "width": 50, "height": 60 }));
        let imported = insert(
            &document,
            &design(Value::Array(nodes)),
            &Destination::NewArtboards,
        )
        .unwrap();
        let result = applied(&document, &imported.edits);

        let [card, second, wrapper] = &imported.roots[..] else {
            panic!()
        };
        assert_eq!(
            result.artboards(),
            [id("ab"), card.clone(), second.clone(), wrapper.clone()]
        );
        let frame = |id: &NodeId| result.get(id).unwrap().layer().unwrap().frame();
        // Right of the existing artboard (800 wide), keeping their spacing.
        assert_eq!(frame(card), Rect::new(900.0, 0.0, 300.0, 200.0));
        assert_eq!(frame(second), Rect::new(1300.0, 0.0, 100.0, 100.0));
        // The loose star got an artboard of its own size, after the others.
        assert_eq!(frame(wrapper), Rect::new(1500.0, 0.0, 50.0, 60.0));
        assert_eq!(result.get(wrapper).unwrap().name(), "Loose");
        let star = &imported.ids[&id("3:1")];
        assert_eq!(
            result.children(wrapper, Family::Layer),
            std::slice::from_ref(star)
        );
        assert_eq!(frame(star), Rect::new(0.0, 0.0, 50.0, 60.0));
        assert_eq!(names(&result, card), ["Background", "Title", "Icon"]);
    }

    #[test]
    fn sibling_order_follows_indices_then_the_list() {
        let document = base();
        let imported = insert(
            &document,
            &design(json!([
                { "id": "c", "type": "rect", "name": "third", "index": "a2" },
                { "id": "a", "type": "rect", "name": "first", "index": "a0" },
                { "id": "b1", "type": "rect", "name": "second", "index": "a1" },
                { "id": "b2", "type": "rect", "name": "second too", "index": "a1" }
            ])),
            &Destination::Into(id("ab")),
        )
        .unwrap();
        let result = applied(&document, &imported.edits);
        assert_eq!(
            names(&result, &id("ab")),
            ["", "first", "second", "second too", "third"]
        );
    }

    #[test]
    fn broken_designs_are_refused_with_the_reason() {
        let document = base();
        let attempt = |nodes: Value| {
            insert(&document, &design(nodes), &Destination::Into(id("ab"))).unwrap_err()
        };
        assert_eq!(attempt(json!([])), ImportError::Empty);
        assert!(matches!(
            attempt(json!([{ "id": "f", "type": "fade_in", "target": "x", "start": 0, "end": 1 }])),
            ImportError::NotALayer(_)
        ));
        assert_eq!(
            attempt(json!([{ "id": "a", "type": "rect" }, { "id": "a", "type": "rect" }])),
            ImportError::DuplicateId(id("a"))
        );
        assert!(matches!(
            attempt(
                json!([{ "id": "a", "type": "rect" }, { "id": "b", "type": "rect", "parent": "a" }])
            ),
            ImportError::BadParent { .. }
        ));
        assert!(matches!(
            attempt(
                json!([{ "id": "a", "type": "group" }, { "id": "b", "type": "artboard", "parent": "a" }])
            ),
            ImportError::NestedArtboard(_)
        ));
        assert!(matches!(
            attempt(
                json!([{ "id": "a", "type": "group", "parent": "b" }, { "id": "b", "type": "group", "parent": "a" }])
            ),
            ImportError::Loop(_)
        ));
        let invalid = attempt(json!([{ "id": "a", "type": "rect", "opacity": 300 }]));
        assert!(
            invalid.to_string().contains("\"opacity\" is 300"),
            "{invalid}"
        );

        let nodes = design(card());
        assert_eq!(
            insert(&document, &nodes, &Destination::Into(id("nope"))),
            Err(ImportError::UnknownNode(id("nope")))
        );
        assert!(matches!(
            insert(&document, &nodes, &Destination::Into(id("old"))),
            Err(ImportError::BadDestination(_))
        ));
    }

    /// The card imported into "ab", with animations on its layers, and the
    /// ids it got.
    fn animated_card() -> (Document, Imported) {
        let document = base();
        let imported = insert(&document, &design(card()), &Destination::Into(id("ab"))).unwrap();
        let mut document = applied(&document, &imported.edits);
        let on = |name: &str| imported.ids[&id(name)].to_string();
        let animation = |name: &str, ty: &str, target: String, index: &str| {
            Edit::add(read_node(json!({
                "id": name, "parent": "ab", "index": index, "type": ty, "target": target, "start": 0, "end": 500
            })).unwrap())
        };
        document
            .apply(&[
                animation("on-card", "slide_in", on("1:1"), "a0"),
                animation("on-bg", "corner_radius", on("1:2"), "a1"),
                animation("on-title", "text_in", on("1:3"), "a2"),
                animation("on-dot", "fade_in", on("1:5"), "a3"),
                animation("on-old", "fade_in", "old".into(), "a4"),
            ])
            .unwrap();
        (document, imported)
    }

    #[test]
    fn a_new_version_keeps_the_animations_of_layers_that_are_still_there() {
        let (document, first) = animated_card();
        let old_root = first.roots[0].clone();
        // The new version: the title moved, the icon group is gone, the dot
        // is now directly in the frame, and there is a new badge.
        let version_two = design(json!([
            { "id": "9:1", "type": "artboard", "name": "Card v2", "width": 320, "height": 220, "source_id": "figma:f:1:1" },
            { "id": "9:2", "parent": "9:1", "type": "rect", "name": "Background", "width": 320, "height": 220, "source_id": "figma:f:1:2" },
            { "id": "9:3", "parent": "9:1", "type": "text", "name": "Title", "content": "Hello again", "x": 40, "y": 40, "source_id": "figma:f:1:3" },
            { "id": "9:5", "parent": "9:1", "type": "ellipse", "name": "Dot", "source_id": "figma:f:1:5" },
            { "id": "9:6", "parent": "9:1", "type": "rect", "name": "Badge", "source_id": "figma:f:1:6" }
        ]));
        let mut moved = document.clone();
        moved.apply(&[Edit::set(&old_root, "x", 55)]).unwrap();
        let replaced = replace_keeping_animations(&moved, &old_root, &version_two).unwrap();
        let result = applied(&moved, &replaced.edits);

        // The new frame sits where the old one was.
        assert!(!result.contains(&old_root));
        assert_eq!(
            result.children(&id("ab"), Family::Layer),
            [id("old"), replaced.root.clone()]
        );
        let frame = result.get(&replaced.root).unwrap();
        assert_eq!(
            (frame.name(), frame.layer().unwrap().frame()),
            ("Card v2", Rect::new(55.0, 0.0, 320.0, 220.0))
        );
        assert_eq!(
            names(&result, &replaced.root),
            ["Background", "Title", "Dot", "Badge"]
        );

        // Animations followed their layers.
        let target = |animation: &str| {
            result
                .get(&id(animation))
                .unwrap()
                .target()
                .unwrap()
                .clone()
        };
        assert_eq!(target("on-card"), replaced.root);
        assert_eq!(target("on-bg"), replaced.ids[&id("9:2")]);
        assert_eq!(target("on-title"), replaced.ids[&id("9:3")]);
        assert_eq!(
            target("on-dot"),
            replaced.ids[&id("9:5")],
            "matched although it moved out of its group"
        );
        assert_eq!(
            target("on-old"),
            id("old"),
            "animations outside the design are untouched"
        );
        assert!(replaced.dropped.is_empty());
        assert_eq!(replaced.matched.len(), 4);
        assert_eq!(
            replaced.matched[&first.ids[&id("1:5")]],
            replaced.ids[&id("9:5")]
        );
    }

    #[test]
    fn animations_without_a_counterpart_are_dropped() {
        let (document, first) = animated_card();
        let old_root = first.roots[0].clone();
        // The background is now an ellipse (no corners to animate), and the
        // title is gone.
        let version_two = design(json!([
            { "id": "9:1", "type": "group", "name": "Card", "width": 300, "height": 200, "source_id": "figma:f:1:1" },
            { "id": "9:2", "parent": "9:1", "type": "ellipse", "name": "Background", "source_id": "figma:f:1:2" },
            { "id": "9:4", "parent": "9:1", "type": "group", "name": "Icon", "source_id": "figma:f:1:4" },
            { "id": "9:5", "parent": "9:4", "type": "ellipse", "name": "Dot", "source_id": "figma:f:1:5" }
        ]));
        let replaced = replace_keeping_animations(&document, &old_root, &version_two).unwrap();
        let result = applied(&document, &replaced.edits);
        let mut dropped = replaced.dropped.clone();
        dropped.sort();
        assert_eq!(dropped, [id("on-bg"), id("on-title")]);
        assert!(!result.contains(&id("on-bg")) && !result.contains(&id("on-title")));
        assert!(result.contains(&id("on-card")) && result.contains(&id("on-dot")));
        assert_eq!(
            result.animations_targeting(&replaced.ids[&id("9:5")]).len(),
            1
        );
    }

    #[test]
    fn layers_sharing_a_source_id_are_matched_in_order() {
        let document = base();
        // Three copies of one component instance share a source id.
        let row = |ids: [&str; 3]| {
            let mut nodes = vec![
                json!({ "id": "row", "type": "group", "name": "Row", "source_id": "figma:f:7:0" }),
            ];
            for (position, id) in ids.into_iter().enumerate() {
                nodes.push(json!({ "id": id, "parent": "row", "type": "rect", "name": format!("Item {position}"),
                                   "x": position * 50, "source_id": "figma:f:7:1" }));
            }
            design(Value::Array(nodes))
        };
        let first = insert(
            &document,
            &row(["a", "b", "c"]),
            &Destination::Into(id("ab")),
        )
        .unwrap();
        let mut document = applied(&document, &first.edits);
        let fades: Vec<Edit> = ["a", "b", "c"]
            .iter()
            .enumerate()
            .map(|(position, name)| {
                Edit::add(read_node(json!({
                    "id": format!("fade-{name}"), "parent": "ab", "index": format!("a{position}"), "type": "fade_in",
                    "target": first.ids[&id(name)], "start": position * 100, "end": 500 + position * 100
                })).unwrap())
            })
            .collect();
        document.apply(&fades).unwrap();

        // The new version has only two of them.
        let second = design(json!([
            { "id": "row", "type": "group", "name": "Row", "source_id": "figma:f:7:0" },
            { "id": "x", "parent": "row", "type": "rect", "name": "Item 0", "source_id": "figma:f:7:1" },
            { "id": "y", "parent": "row", "type": "rect", "name": "Item 1", "source_id": "figma:f:7:1" }
        ]));
        let replaced = replace_keeping_animations(&document, &first.roots[0], &second).unwrap();
        let result = applied(&document, &replaced.edits);
        let target = |animation: &str| {
            result
                .get(&id(animation))
                .unwrap()
                .target()
                .unwrap()
                .clone()
        };
        assert_eq!(target("fade-a"), replaced.ids[&id("x")]);
        assert_eq!(
            target("fade-b"),
            replaced.ids[&id("y")],
            "each copy keeps its own animation"
        );
        assert!(
            !result.contains(&id("fade-c")),
            "the third copy is gone, and its animation with it"
        );
        assert_eq!(replaced.dropped, [id("fade-c")]);
    }

    #[test]
    fn a_whole_artboard_is_replaced_with_its_timeline() {
        let document = base();
        let first = insert(&document, &design(card()), &Destination::NewArtboards).unwrap();
        let mut document = applied(&document, &first.edits);
        let board = first.roots[0].clone();
        let title = first.ids[&id("1:3")].clone();
        document
            .apply(&[
                Edit::set(&board, "duration", 9000),
                Edit::add(read_node(json!({ "id": "folder", "parent": board, "index": "a0", "type": "animation_group", "name": "Intro" })).unwrap()),
                Edit::add(read_node(json!({ "id": "type", "parent": "folder", "index": "a0", "type": "text_in", "target": title, "start": 0, "end": 800 })).unwrap()),
                Edit::add(read_node(json!({ "id": "tint", "parent": board, "index": "a1", "type": "fill", "target": board, "start": 0, "end": 800, "to": "#000000" })).unwrap()),
            ])
            .unwrap();
        let before = document.get(&board).unwrap().layer().unwrap().frame();

        let replaced = replace_keeping_animations(&document, &board, &design(card())).unwrap();
        let result = applied(&document, &replaced.edits);
        assert!(!result.contains(&board));
        let new_board = result.get(&replaced.root).unwrap();
        assert_eq!(new_board.node_type(), NodeType::Artboard);
        assert_eq!(
            crate::eval::duration_of(&result, &replaced.root),
            Some(9000.0),
            "the timeline keeps its length"
        );
        assert_eq!(
            new_board.layer().unwrap().frame(),
            before,
            "and the artboard its place on the canvas"
        );
        // The timeline moved over, folder and all.
        assert_eq!(
            result.children(&replaced.root, Family::Animation),
            [id("folder"), id("tint")]
        );
        assert_eq!(result.get(&id("type")).unwrap().parent, Some(id("folder")));
        assert_eq!(
            result.get(&id("type")).unwrap().target(),
            Some(&replaced.ids[&id("1:3")])
        );
        assert_eq!(
            result.get(&id("tint")).unwrap().target(),
            Some(&replaced.root)
        );
    }

    #[test]
    fn replacements_that_cannot_work_are_refused() {
        let (document, first) = animated_card();
        let root = first.roots[0].clone();
        assert_eq!(
            replace_keeping_animations(&document, &id("nope"), &design(card())),
            Err(ImportError::UnknownNode(id("nope")))
        );
        assert_eq!(
            replace_keeping_animations(&document, &id("on-card"), &design(card())),
            Err(ImportError::UnknownNode(id("on-card")))
        );
        let two_roots =
            design(json!([{ "id": "a", "type": "rect" }, { "id": "b", "type": "rect" }]));
        assert_eq!(
            replace_keeping_animations(&document, &root, &two_roots),
            Err(ImportError::NotOneRoot(2))
        );
        let not_an_artboard = replace_keeping_animations(
            &document,
            &id("ab"),
            &design(json!([{ "id": "a", "type": "rect" }])),
        );
        assert!(matches!(
            not_an_artboard,
            Err(ImportError::NotAnArtboard { .. })
        ));
    }
}

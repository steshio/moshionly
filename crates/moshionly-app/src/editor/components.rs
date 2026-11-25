//! Components: the main components of a file, where new instances of them
//! go, and what deleting a main does to its instances.
//!
//! A main component (`component`) is a group whose look, layers and
//! animations every instance of it (`instance`) shows. The engine draws
//! instances and keeps them up to date; the rules here are about the
//! editor: which components the components list shows, where an inserted
//! instance lands, how instances follow a resized main, and which commands
//! apply to a selection.

use std::collections::{HashMap, HashSet};

use moshionly_core::build::{self, BuildError, Place};
use moshionly_core::{Document, Edit, Family, Node, NodeId, NodeType, Point};
use serde_json::{Map, Value};

use super::describe::display_name;

/// Space left between a main component and an instance inserted beside it.
pub const INSTANCE_GAP: f64 = 40.0;

/// A main component as the components list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentEntry {
    /// The main component.
    pub id: NodeId,
    /// Its name.
    pub name: String,
    /// The name of the artboard it is in.
    pub artboard: String,
    /// How many instances show it directly.
    pub instances: usize,
}

/// Every main component of `document`, in the order of the layer tree.
pub fn components(document: &Document) -> Vec<ComponentEntry> {
    let mut counts: HashMap<&NodeId, usize> = HashMap::new();
    for node in document.nodes_unordered() {
        if let Some(main) = node.component() {
            *counts.entry(main).or_default() += 1;
        }
    }
    document
        .nodes()
        .filter(|node| node.node_type() == NodeType::Component)
        .map(|node| ComponentEntry {
            id: node.id.clone(),
            name: display_name(node),
            artboard: document
                .artboard_of(&node.id)
                .map(display_name)
                .unwrap_or_default(),
            instances: counts.get(&node.id).copied().unwrap_or(0),
        })
        .collect()
}

/// Whether `id` is a node of type `ty` in `document`.
fn is(document: &Document, id: &NodeId, ty: NodeType) -> bool {
    document.get(id).is_some_and(|node| node.node_type() == ty)
}

/// The main components among `ids`, in the order given.
pub fn mains_in(document: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter()
        .filter(|id| is(document, id, NodeType::Component))
        .cloned()
        .collect()
}

/// The instances among `ids`, in the order given.
pub fn instances_in(document: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter()
        .filter(|id| is(document, id, NodeType::Instance))
        .cloned()
        .collect()
}

/// The main component that the instances among `ids` show, when there are
/// instances and they all show the same one, and it exists.
pub fn shown_main(document: &Document, ids: &[NodeId]) -> Option<NodeId> {
    let mut mains = ids
        .iter()
        .filter_map(|id| document.get(id)?.component())
        .collect::<HashSet<_>>()
        .into_iter();
    match (mains.next(), mains.next()) {
        (Some(main), None) if is(document, main, NodeType::Component) => Some(main.clone()),
        _ => None,
    }
}

/// Edits that add an instance of `main` beside it: in the same parent, just
/// in front of it, to its right with a gap, and further right past the
/// instances of it already there, so that none covers another.
///
/// Returns the new instance with the edits.
pub fn instance_beside(
    document: &Document,
    main: &NodeId,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let node = document
        .get(main)
        .ok_or_else(|| BuildError::UnknownNode(main.clone()))?;
    let (Some(layer), Some(parent)) = (node.layer(), node.parent.as_ref()) else {
        return Err(BuildError::NotAComponent(node.label()));
    };
    let taken: Vec<(f64, f64)> = document
        .child_nodes(parent, Family::Layer)
        .filter(|sibling| sibling.component() == Some(main))
        .filter_map(|sibling| sibling.layer().map(|layer| (layer.x, layer.y)))
        .collect();
    let step = layer.width.max(1.0) + INSTANCE_GAP;
    let near = |a: f64, b: f64| (a - b).abs() < 0.5;
    let mut x = layer.x + step;
    for _ in 0..=taken.len() {
        if !taken
            .iter()
            .any(|(tx, ty)| near(*tx, x) && near(*ty, layer.y))
        {
            break;
        }
        x += step;
    }
    build::add_instance(
        document,
        main,
        parent,
        &Place::After(main.clone()),
        Point::new(x.round(), layer.y),
    )
}

/// Edits that add an instance of `main` to `artboard`, in front of its
/// other layers, centred on `center` in the artboard's coordinates.
///
/// Returns the new instance with the edits.
pub fn instance_at(
    document: &Document,
    main: &NodeId,
    artboard: &NodeId,
    center: Point,
) -> Result<(NodeId, Vec<Edit>), BuildError> {
    let node = document
        .get(main)
        .ok_or_else(|| BuildError::UnknownNode(main.clone()))?;
    let layer = node
        .layer()
        .ok_or_else(|| BuildError::NotAComponent(node.label()))?;
    let corner = Point::new(
        (center.x - layer.width / 2.0).round(),
        (center.y - layer.height / 2.0).round(),
    );
    build::add_instance(document, main, artboard, &Place::Last, corner)
}

/// What deleting `ids` does to the instances of the main components that
/// go with them (listed, or inside a listed layer): those instances are
/// detached into plain groups. Returns the message that says so, such as
/// "Detached 3 instances of Card", or `None` when no instance is left
/// without its main.
pub fn detach_message(document: &Document, ids: &[NodeId]) -> Option<String> {
    let mut gone: HashSet<&NodeId> = HashSet::new();
    for id in ids {
        if let Some(node) = document.get(id) {
            gone.insert(&node.id);
            gone.extend(document.walk(id, Family::Layer).map(|(_, node)| &node.id));
        }
    }
    let goes = |node: &Node| {
        gone.contains(&node.id)
            || document
                .ancestors(&node.id)
                .any(|ancestor| gone.contains(&ancestor.id))
    };
    let mut detached: Vec<(String, usize)> = Vec::new();
    for main in document
        .nodes()
        .filter(|node| node.node_type() == NodeType::Component && gone.contains(&node.id))
    {
        let left = document
            .instances_of(&main.id)
            .into_iter()
            .filter(|instance| !goes(instance))
            .count();
        if left > 0 {
            detached.push((display_name(main), left));
        }
    }
    let total: usize = detached.iter().map(|(_, count)| count).sum();
    let instances = if total == 1 { "instance" } else { "instances" };
    match detached.as_slice() {
        [] => None,
        [(name, _)] => Some(format!("Detached {total} {instances} of {name}")),
        several => Some(format!(
            "Detached {total} {instances} of {} components",
            several.len()
        )),
    }
}

/// Edits that keep instances the size of their main when `edits` resize
/// the main, as Figma's do: an instance as wide as its main was takes the
/// main's new width, and the same for the height, each side on its own.
/// An instance resized by hand keeps its size, and a side that `edits` set
/// on the instance itself keeps that. Without this, the engine would
/// squeeze or stretch the main's content into the old box. `document` is
/// the document before `edits`.
pub fn instances_follow_mains(document: &Document, edits: &[Edit]) -> Vec<Edit> {
    const SIDES: [&str; 2] = ["width", "height"];
    // The width and height `edits` give each node; a later edit wins.
    let mut sizes: HashMap<&NodeId, [Option<f64>; 2]> = HashMap::new();
    for edit in edits {
        if let Edit::Update { id, patch } = edit {
            let size = sizes.entry(id).or_default();
            for (side, field) in SIDES.iter().enumerate() {
                if let Some(value) = patch.get(*field).and_then(Value::as_f64) {
                    size[side] = Some(value);
                }
            }
        }
    }
    // Sizes are tidied to a hundredth.
    let same = |a: f64, b: f64| (a - b).abs() < 0.005;
    let mut follow = Vec::new();
    let mut done: HashSet<&NodeId> = HashSet::new();
    for edit in edits {
        let Edit::Update { id, .. } = edit else {
            continue;
        };
        let Some(main) = document
            .get(id)
            .filter(|node| node.node_type() == NodeType::Component && done.insert(&node.id))
        else {
            continue;
        };
        let (Some(layer), Some(new)) = (main.layer(), sizes.get(id)) else {
            continue;
        };
        let was = [layer.width, layer.height];
        for instance in document.instances_of(id) {
            let Some(own) = instance.layer() else {
                continue;
            };
            let set = sizes.get(&instance.id).copied().unwrap_or_default();
            let mut patch = Map::new();
            for (side, now) in [own.width, own.height].into_iter().enumerate() {
                if let Some(to) = new[side]
                    && set[side].is_none()
                    && same(now, was[side])
                    && !same(to, was[side])
                {
                    patch.insert(SIDES[side].to_owned(), Value::from(to));
                }
            }
            if !patch.is_empty() {
                follow.push(Edit::update(instance.id.clone(), patch));
            }
        }
    }
    follow
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    /// Two artboards: a card component with a dot in the first, with an
    /// instance beside it, and two more instances in the second.
    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":2,"name":"Test","nodes":[
            {"id":"home","parent":null,"index":"a0","type":"artboard","name":"Home","width":1000,"height":1000},
            {"id":"card","parent":"home","index":"a0","type":"component","name":"Card","x":100,"y":50,"width":200,"height":100},
            {"id":"dot","parent":"card","index":"a0","type":"ellipse","width":20,"height":20},
            {"id":"near","parent":"home","index":"a1","type":"instance","component":"card","x":340,"y":50,"width":200,"height":100},
            {"id":"badge","parent":"home","index":"a2","type":"component","name":"Badge","width":40,"height":40},
            {"id":"away","parent":null,"index":"a1","type":"artboard","name":"Away","x":1100,"width":1000,"height":1000},
            {"id":"one","parent":"away","index":"a0","type":"instance","component":"card","width":200,"height":100},
            {"id":"two","parent":"away","index":"a1","type":"instance","component":"card","y":200,"width":200,"height":100},
            {"id":"tag","parent":"away","index":"a2","type":"instance","component":"badge","width":40,"height":40}
            ]}"##,
        )
        .unwrap()
    }

    #[test]
    fn the_list_has_every_main_with_its_artboard_and_instances() {
        let list = components(&document());
        let shown: Vec<(&str, &str, usize)> = list
            .iter()
            .map(|entry| {
                (
                    entry.name.as_str(),
                    entry.artboard.as_str(),
                    entry.instances,
                )
            })
            .collect();
        assert_eq!(shown, [("Card", "Home", 3), ("Badge", "Home", 1)]);
        assert!(components(&Document::new("Empty")).is_empty());
    }

    #[test]
    fn the_selection_names_its_mains_and_instances() {
        let doc = document();
        let ids = [id("card"), id("one"), id("dot"), id("tag")];
        assert_eq!(mains_in(&doc, &ids), [id("card")]);
        assert_eq!(instances_in(&doc, &ids), [id("one"), id("tag")]);
        // Instances of one main lead to it; of two, nowhere.
        assert_eq!(shown_main(&doc, &[id("one"), id("two")]), Some(id("card")));
        assert_eq!(shown_main(&doc, &[id("one"), id("card")]), Some(id("card")));
        assert_eq!(shown_main(&doc, &[id("one"), id("tag")]), None);
        assert_eq!(shown_main(&doc, &[id("card")]), None);
        assert_eq!(shown_main(&doc, &[]), None);
    }

    #[test]
    fn an_instance_goes_beside_its_main_past_the_ones_there() {
        let mut doc = document();
        // An instance already sits at 340, one step to the right.
        let (first, edits) = instance_beside(&doc, &id("card")).unwrap();
        doc.apply(&edits).unwrap();
        let node = doc.get(&first).unwrap();
        assert_eq!(node.parent, Some(id("home")));
        assert_eq!(node.component(), Some(&id("card")));
        let layer = node.layer().unwrap();
        assert_eq!((layer.x, layer.y), (580.0, 50.0));
        assert_eq!((layer.width, layer.height), (200.0, 100.0));
        // Just in front of the main.
        let order = doc.children(&id("home"), Family::Layer);
        let at = |id: &NodeId| order.iter().position(|other| other == id).unwrap();
        assert_eq!(at(&first), at(&id("card")) + 1);
        // The next one goes further right.
        let (second, edits) = instance_beside(&doc, &id("card")).unwrap();
        doc.apply(&edits).unwrap();
        assert_eq!(doc.get(&second).unwrap().layer().unwrap().x, 820.0);
        assert!(instance_beside(&doc, &id("missing")).is_err());
    }

    #[test]
    fn an_instance_dropped_on_an_artboard_is_centred_where_it_landed() {
        let mut doc = document();
        let (instance, edits) =
            instance_at(&doc, &id("card"), &id("away"), Point::new(500.0, 500.0)).unwrap();
        doc.apply(&edits).unwrap();
        let node = doc.get(&instance).unwrap();
        assert_eq!(node.parent, Some(id("away")));
        let layer = node.layer().unwrap();
        assert_eq!((layer.x, layer.y), (400.0, 450.0));
        assert_eq!(
            doc.children(&id("away"), Family::Layer).last(),
            Some(&instance),
            "on top"
        );
        // Only components have instances.
        assert!(instance_at(&doc, &id("dot"), &id("away"), Point::ZERO).is_err());
    }

    #[test]
    fn deleting_a_main_says_how_many_instances_are_detached() {
        let doc = document();
        assert_eq!(
            detach_message(&doc, &[id("card")]).as_deref(),
            Some("Detached 3 instances of Card")
        );
        // Instances that go too are not counted.
        assert_eq!(
            detach_message(&doc, &[id("card"), id("one"), id("near")]).as_deref(),
            Some("Detached 1 instance of Card")
        );
        assert_eq!(
            detach_message(&doc, &[id("card"), id("away"), id("near")]),
            None
        );
        // A main inside a deleted artboard goes with it.
        assert_eq!(
            detach_message(&doc, &[id("home")]).as_deref(),
            Some("Detached 3 instances of 2 components")
        );
        assert_eq!(detach_message(&doc, &[id("dot"), id("one")]), None);
    }
}

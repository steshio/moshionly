//! The document: a flat set of nodes, with the lookups that make it a tree.

use std::collections::HashMap;

use crate::id::NodeId;
use crate::model::{Family, Node};

/// A Moshionly document.
///
/// A document owns a flat set of [`Node`]s. The tree is assembled from each
/// node's `parent`, and siblings of one family are ordered by `index`. The
/// document keeps those child lists up to date, so tree queries (see the
/// methods defined in [`crate::tree`]) are cheap.
///
/// A document is only ever changed through [`Document::apply`], which keeps
/// these promises:
///
/// - ids are unique and well formed;
/// - artboards are the roots, and every other node sits under a parent that
///   may hold it;
/// - every animation targets a layer in its own artboard;
/// - siblings of one family have distinct, valid fractional indices;
/// - every property is inside its documented range.
#[derive(Debug, Clone, Default)]
pub struct Document {
    /// The name of the project. It is stored in the file and is not a node,
    /// so changing it is not an edit and is not undone.
    pub name: String,
    nodes: HashMap<NodeId, Node>,
    /// Nodes without a parent, sorted by index: the artboards.
    roots: Vec<NodeId>,
    /// Layer children of each node, sorted by index (back to front).
    layers: HashMap<NodeId, Vec<NodeId>>,
    /// Animation children of each node, sorted by index (timeline rows).
    animations: HashMap<NodeId, Vec<NodeId>>,
}

impl PartialEq for Document {
    /// Two documents are equal when they have the same name and the same
    /// nodes. The child lists are derived from the nodes.
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.nodes == other.nodes
    }
}

impl Document {
    /// An empty document.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Number of nodes of both families.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the document has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node with this id.
    pub fn get(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Whether a node with this id exists.
    pub fn contains(&self, id: &NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    /// The ids of the artboards, sorted by index.
    pub fn artboards(&self) -> &[NodeId] {
        &self.roots
    }

    /// Every node in tree order: each artboard, followed by its layers depth
    /// first (back to front) and then its animations depth first (timeline
    /// rows top to bottom).
    ///
    /// This is the order nodes are written to a file in, so it does not
    /// depend on the order in which the document was edited.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.roots.iter().flat_map(move |root| {
            self.nodes
                .get(root)
                .into_iter()
                .chain(self.walk(root, Family::Layer).map(|(_, node)| node))
                .chain(self.walk(root, Family::Animation).map(|(_, node)| node))
        })
    }

    /// Every node in no particular order. Cheaper than [`Document::nodes`].
    pub fn nodes_unordered(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// The sorted child list a node with this parent and family belongs to.
    pub(crate) fn child_list(&self, parent: Option<&NodeId>, family: Family) -> &[NodeId] {
        match (parent, family) {
            (None, _) => &self.roots,
            (Some(parent), Family::Layer) => self.layers.get(parent).map_or(&[], Vec::as_slice),
            (Some(parent), Family::Animation) => {
                self.animations.get(parent).map_or(&[], Vec::as_slice)
            }
        }
    }

    /// Adds a node without checking any rule. The id must not be in use.
    pub(crate) fn insert_raw(&mut self, node: Node) {
        debug_assert!(!self.nodes.contains_key(&node.id), "id already in use");
        let nodes = &self.nodes;
        let list = match (&node.parent, node.family()) {
            (None, _) => &mut self.roots,
            (Some(parent), Family::Layer) => self.layers.entry(parent.clone()).or_default(),
            (Some(parent), Family::Animation) => self.animations.entry(parent.clone()).or_default(),
        };
        // Ties on the index are broken by id so the order is always defined,
        // also in the middle of a batch that will be refused for the tie.
        let position = list.partition_point(|sibling| {
            nodes.get(sibling).is_some_and(|sibling| {
                (sibling.index.as_str(), &sibling.id) < (node.index.as_str(), &node.id)
            })
        });
        list.insert(position, node.id.clone());
        self.nodes.insert(node.id.clone(), node);
    }

    /// Removes one node, and only that node, without checking any rule.
    pub(crate) fn remove_raw(&mut self, id: &NodeId) -> Option<Node> {
        let node = self.nodes.remove(id)?;
        let forget = |list: &mut Vec<NodeId>| list.retain(|sibling| sibling != id);
        match (&node.parent, node.family()) {
            (None, _) => forget(&mut self.roots),
            (Some(parent), family) => {
                let lists = match family {
                    Family::Layer => &mut self.layers,
                    Family::Animation => &mut self.animations,
                };
                if let Some(list) = lists.get_mut(parent) {
                    forget(list);
                    if list.is_empty() {
                        lists.remove(parent);
                    }
                }
            }
        }
        Some(node)
    }

    /// Builds a document from nodes without checking any rule except that
    /// ids are distinct (a repeated id keeps the first node).
    #[cfg(test)]
    pub(crate) fn from_nodes_unchecked(
        name: impl Into<String>,
        nodes: impl IntoIterator<Item = Node>,
    ) -> Self {
        let mut document = Self::new(name);
        for node in nodes {
            if !document.contains(&node.id) {
                document.insert_raw(node);
            }
        }
        document
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Artboard, FadePreset, LayerProps, NodeData, RectLayer, Timing};

    fn artboard(id: &str, index: &str) -> Node {
        Node::new(Artboard {
            layer: LayerProps::new(id, 0.0, 0.0, 100.0, 100.0),
            duration: 1000.0,
            fill: None,
        })
        .with_id(id)
        .with_index(index)
    }

    fn rect(id: &str, parent: &str, index: &str) -> Node {
        Node::new(RectLayer::default())
            .with_id(id)
            .with_parent(parent)
            .with_index(index)
    }

    fn fade(id: &str, parent: &str, target: &str, index: &str) -> Node {
        Node::new(NodeData::FadeIn(FadePreset {
            timing: Timing::new(target, 0.0, 500.0),
        }))
        .with_id(id)
        .with_parent(parent)
        .with_index(index)
    }

    #[test]
    fn child_lists_follow_inserts_and_removals() {
        let mut document = Document::new("Test");
        document.insert_raw(artboard("b", "a1"));
        document.insert_raw(artboard("a", "a0"));
        document.insert_raw(rect("r2", "a", "a1"));
        document.insert_raw(rect("r1", "a", "a0"));
        document.insert_raw(fade("f1", "a", "r1", "a0"));
        assert_eq!(document.len(), 5);
        assert_eq!(document.artboards(), [NodeId::from("a"), NodeId::from("b")]);
        assert_eq!(
            document.child_list(Some(&NodeId::from("a")), Family::Layer),
            [NodeId::from("r1"), NodeId::from("r2")]
        );
        assert_eq!(
            document.child_list(Some(&NodeId::from("a")), Family::Animation),
            [NodeId::from("f1")]
        );

        let removed = document.remove_raw(&NodeId::from("r1")).unwrap();
        assert_eq!(removed.id, NodeId::from("r1"));
        assert_eq!(
            document.child_list(Some(&NodeId::from("a")), Family::Layer),
            [NodeId::from("r2")]
        );
        assert!(document.remove_raw(&NodeId::from("r1")).is_none());
        document.remove_raw(&NodeId::from("r2"));
        assert!(document.layers.is_empty(), "empty lists are dropped");
    }

    #[test]
    fn nodes_come_out_in_tree_order_whatever_the_insert_order() {
        let nodes = [
            fade("f1", "a", "r1", "a0"),
            rect("r2", "a", "a1"),
            artboard("b", "a1"),
            rect("r1", "a", "a0"),
            artboard("a", "a0"),
        ];
        let forward = Document::from_nodes_unchecked("Test", nodes.clone());
        let backward = Document::from_nodes_unchecked("Test", nodes.into_iter().rev());
        assert_eq!(forward, backward);
        let order = |document: &Document| -> Vec<String> {
            document.nodes().map(|node| node.id.to_string()).collect()
        };
        assert_eq!(order(&forward), ["a", "r1", "r2", "f1", "b"]);
        assert_eq!(order(&backward), order(&forward));
        assert_eq!(forward.nodes_unordered().count(), 5);
    }

    #[test]
    fn equal_indices_are_ordered_by_id() {
        let document = Document::from_nodes_unchecked(
            "Test",
            [
                artboard("a", "a0"),
                rect("z", "a", "a0"),
                rect("m", "a", "a0"),
            ],
        );
        assert_eq!(
            document.child_list(Some(&NodeId::from("a")), Family::Layer),
            [NodeId::from("m"), NodeId::from("z")]
        );
    }
}

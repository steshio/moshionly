//! Tree queries on a [`Document`]: children, ancestors, walks, lookups and
//! resting geometry.
//!
//! All of them are read-only and answer with `None` or an empty result for an
//! id that does not exist.

use crate::document::Document;
use crate::geometry::{Affine, Rect};
use crate::id::NodeId;
use crate::model::{Family, Node, NodeType};

impl Document {
    /// The ids of a node's children of one family, sorted by index.
    ///
    /// Layers come back to front: the last one is drawn on top. Animations
    /// come in timeline row order.
    pub fn children(&self, parent: &NodeId, family: Family) -> &[NodeId] {
        self.child_list(Some(parent), family)
    }

    /// A node's children of one family, sorted by index.
    pub fn child_nodes<'a>(
        &'a self,
        parent: &NodeId,
        family: Family,
    ) -> impl Iterator<Item = &'a Node> + use<'a> {
        self.children(parent, family)
            .iter()
            .filter_map(move |id| self.get(id))
    }

    /// The parent of a node. `None` for an artboard or an unknown id.
    pub fn parent_of(&self, id: &NodeId) -> Option<&Node> {
        self.get(self.get(id)?.parent.as_ref()?)
    }

    /// The ancestors of a node, nearest first, ending with its artboard.
    pub fn ancestors(&self, id: &NodeId) -> Ancestors<'_> {
        Ancestors {
            document: self,
            next: self.get(id).and_then(|node| node.parent.as_ref()),
            budget: self.len(),
        }
    }

    /// The artboard a node belongs to; an artboard belongs to itself.
    pub fn artboard_of(&self, id: &NodeId) -> Option<&Node> {
        let node = self.get(id)?;
        let root = self.ancestors(id).last().unwrap_or(node);
        (root.node_type() == NodeType::Artboard).then_some(root)
    }

    /// Whether `ancestor` is somewhere above `id` in the tree. A node is not
    /// its own ancestor.
    pub fn is_ancestor(&self, ancestor: &NodeId, id: &NodeId) -> bool {
        self.ancestors(id).any(|node| &node.id == ancestor)
    }

    /// How many ancestors a node has: 0 for an artboard.
    pub fn depth(&self, id: &NodeId) -> usize {
        self.ancestors(id).count()
    }

    /// A depth-first walk below `root` through children of one family,
    /// parents before children and siblings in index order.
    ///
    /// The walk yields each node with its depth below `root`, starting at 1.
    /// `root` itself is not included. Walking an artboard with
    /// [`Family::Layer`] visits every layer in it back to front; with
    /// [`Family::Animation`] it visits every timeline row top to bottom.
    pub fn walk(&self, root: &NodeId, family: Family) -> Walk<'_> {
        Walk {
            document: self,
            family,
            stack: vec![self.children(root, family).iter()],
            budget: self.len(),
        }
    }

    /// Every node below `root` in one family, in [`Document::walk`] order.
    pub fn descendants(&self, root: &NodeId, family: Family) -> Vec<&Node> {
        self.walk(root, family).map(|(_, node)| node).collect()
    }

    /// The animations that target a layer, in the order they take effect:
    /// by start time, and for equal start times by timeline row.
    ///
    /// Animation groups are folders and never appear here.
    pub fn animations_targeting(&self, layer: &NodeId) -> Vec<&Node> {
        let Some(artboard) = self.artboard_of(layer) else {
            return Vec::new();
        };
        let mut animations: Vec<&Node> = self
            .walk(&artboard.id, Family::Animation)
            .map(|(_, node)| node)
            .filter(|node| node.target() == Some(layer))
            .collect();
        animations.sort_by(|a, b| start_of(a).total_cmp(&start_of(b)));
        animations
    }

    /// Every node with exactly this name, in tree order.
    pub fn find_by_name(&self, name: &str) -> Vec<&Node> {
        self.nodes().filter(|node| node.name() == name).collect()
    }

    /// The same-family siblings of a node, itself included, sorted by index.
    pub fn siblings(&self, id: &NodeId) -> &[NodeId] {
        match self.get(id) {
            Some(node) => self.child_list(node.parent.as_ref(), node.family()),
            None => &[],
        }
    }

    /// The sibling just before a node in index order: the layer behind it,
    /// or the timeline row above it.
    pub fn previous_sibling(&self, id: &NodeId) -> Option<&NodeId> {
        let siblings = self.siblings(id);
        let position = siblings.iter().position(|sibling| sibling == id)?;
        siblings.get(position.checked_sub(1)?)
    }

    /// The sibling just after a node in index order: the layer in front of
    /// it, or the timeline row below it.
    pub fn next_sibling(&self, id: &NodeId) -> Option<&NodeId> {
        let siblings = self.siblings(id);
        let position = siblings.iter().position(|sibling| sibling == id)?;
        siblings.get(position + 1)
    }

    /// The transform from a layer's own coordinates (origin at the top-left
    /// corner of its box) to the coordinates of its artboard, at rest.
    ///
    /// An artboard's position on the editor canvas is not part of it: for an
    /// artboard this is the identity. `None` when the id is not a layer.
    pub fn world_transform(&self, layer: &NodeId) -> Option<Affine> {
        let node = self.get(layer)?;
        let props = node.layer()?;
        if node.node_type() == NodeType::Artboard {
            return Some(Affine::IDENTITY);
        }
        let mut transform = props.transform();
        for ancestor in self.ancestors(layer) {
            if ancestor.node_type() == NodeType::Artboard {
                break;
            }
            transform = ancestor.layer()?.transform() * transform;
        }
        Some(transform)
    }

    /// The axis-aligned box around a layer in the coordinates of its
    /// artboard, at rest, with the rotation and scale of the layer and of its
    /// ancestors applied. `None` when the id is not a layer.
    ///
    /// The box is the layer's own `width` by `height`; it does not grow for
    /// strokes, shadows, blur or children that stick out of a group.
    pub fn world_bounds(&self, layer: &NodeId) -> Option<Rect> {
        let props = self.get(layer)?.layer()?;
        let own = Rect::new(0.0, 0.0, props.width, props.height);
        Some(self.world_transform(layer)?.bounds_of(&own))
    }
}

/// When an animation starts; animation groups sort first.
fn start_of(node: &Node) -> f64 {
    node.data.span().map_or(0.0, |(start, _)| start)
}

/// Iterator over the ancestors of a node. See [`Document::ancestors`].
#[derive(Debug, Clone)]
pub struct Ancestors<'a> {
    document: &'a Document,
    next: Option<&'a NodeId>,
    /// Steps left; guards against a parent loop in a half-applied batch.
    budget: usize,
}

impl<'a> Iterator for Ancestors<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<&'a Node> {
        self.budget = self.budget.checked_sub(1)?;
        let node = self.document.get(self.next?)?;
        self.next = node.parent.as_ref();
        Some(node)
    }
}

/// Depth-first iterator below a node. See [`Document::walk`].
#[derive(Debug, Clone)]
pub struct Walk<'a> {
    document: &'a Document,
    family: Family,
    stack: Vec<std::slice::Iter<'a, NodeId>>,
    /// Nodes left; guards against a parent loop in a half-applied batch.
    budget: usize,
}

impl<'a> Iterator for Walk<'a> {
    type Item = (usize, &'a Node);

    fn next(&mut self) -> Option<(usize, &'a Node)> {
        loop {
            let Some(id) = self.stack.last_mut()?.next() else {
                self.stack.pop();
                continue;
            };
            let Some(node) = self.document.get(id) else {
                continue;
            };
            self.budget = self.budget.checked_sub(1)?;
            let depth = self.stack.len();
            self.stack
                .push(self.document.children(id, self.family).iter());
            return Some((depth, node));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;
    use crate::model::{
        AnimationGroup, Artboard, FadePreset, Group, InstantAnimation, LayerProps, MoveAnimation,
        NodeData, RectLayer, Timing,
    };

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    fn layer(name: &str, x: f64, y: f64, width: f64, height: f64) -> LayerProps {
        LayerProps::new(name, x, y, width, height)
    }

    /// Two artboards. The first holds a card group with two rects and a
    /// loose rect, and animations in and out of a folder.
    fn sample() -> Document {
        let place = |node: Node, id: &str, parent: &str, index: &str| {
            node.with_id(id).with_parent(parent).with_index(index)
        };
        let artboard = |name: &str, index: &str| {
            Node::new(Artboard {
                layer: layer(name, 500.0, 0.0, 800.0, 600.0),
                duration: 3000.0,
                fill: None,
            })
            .with_id(name)
            .with_index(index)
        };
        let rect = |name: &str, x: f64, y: f64| {
            Node::new(RectLayer {
                layer: layer(name, x, y, 100.0, 50.0),
                ..RectLayer::default()
            })
        };
        let fade = |target: &str, start: f64| {
            Node::new(NodeData::FadeIn(FadePreset {
                timing: Timing::new(target, start, start + 500.0),
            }))
        };
        Document::from_nodes_unchecked(
            "Sample",
            [
                artboard("one", "a0"),
                artboard("two", "a1"),
                place(
                    Node::new(Group {
                        layer: LayerProps {
                            rotation: 90.0,
                            ..layer("Card", 100.0, 100.0, 200.0, 200.0)
                        },
                        ..Group::default()
                    }),
                    "card",
                    "one",
                    "a0",
                ),
                place(rect("Back", 0.0, 0.0), "back", "card", "a0"),
                place(rect("Front", 50.0, 75.0), "front", "card", "a1"),
                place(rect("Loose", 10.0, 20.0), "loose", "one", "a1"),
                place(rect("Front", 0.0, 0.0), "other", "two", "a0"),
                place(fade("front", 800.0), "late", "one", "a0"),
                place(
                    Node::new(AnimationGroup {
                        name: "Intro".into(),
                    }),
                    "intro",
                    "one",
                    "a1",
                ),
                place(fade("front", 200.0), "early", "intro", "a0"),
                place(
                    Node::new(MoveAnimation {
                        timing: Timing::new("front", 200.0, 900.0),
                        from: None,
                        to: None,
                    }),
                    "slide",
                    "intro",
                    "a1",
                ),
                place(
                    Node::new(NodeData::Show(InstantAnimation::new("card", 100.0))),
                    "show",
                    "one",
                    "a2",
                ),
            ],
        )
    }

    fn ids(nodes: impl IntoIterator<Item = impl std::borrow::Borrow<Node>>) -> Vec<String> {
        nodes
            .into_iter()
            .map(|node| node.borrow().id.to_string())
            .collect()
    }

    #[test]
    fn children_are_listed_by_family_in_index_order() {
        let document = sample();
        assert_eq!(
            document.children(&id("one"), Family::Layer),
            [id("card"), id("loose")]
        );
        assert_eq!(
            document.children(&id("one"), Family::Animation),
            [id("late"), id("intro"), id("show")]
        );
        assert_eq!(
            ids(document.child_nodes(&id("card"), Family::Layer)),
            ["back", "front"]
        );
        assert!(document.children(&id("front"), Family::Layer).is_empty());
        assert!(document.children(&id("missing"), Family::Layer).is_empty());
    }

    #[test]
    fn ancestors_run_from_the_parent_up_to_the_artboard() {
        let document = sample();
        assert_eq!(ids(document.ancestors(&id("front"))), ["card", "one"]);
        assert_eq!(ids(document.ancestors(&id("early"))), ["intro", "one"]);
        assert!(document.ancestors(&id("one")).next().is_none());
        assert!(document.ancestors(&id("missing")).next().is_none());
        assert_eq!(document.parent_of(&id("front")).unwrap().id, id("card"));
        assert!(document.parent_of(&id("one")).is_none());
        assert_eq!(document.depth(&id("front")), 2);
        assert!(document.is_ancestor(&id("one"), &id("front")));
        assert!(!document.is_ancestor(&id("front"), &id("front")));
        assert!(!document.is_ancestor(&id("two"), &id("front")));
    }

    #[test]
    fn every_node_knows_its_artboard() {
        let document = sample();
        for (node, artboard) in [
            ("front", "one"),
            ("early", "one"),
            ("one", "one"),
            ("other", "two"),
        ] {
            assert_eq!(document.artboard_of(&id(node)).unwrap().id, id(artboard));
        }
        assert!(document.artboard_of(&id("missing")).is_none());
    }

    #[test]
    fn walks_are_depth_first_with_depths() {
        let document = sample();
        let layers: Vec<(usize, String)> = document
            .walk(&id("one"), Family::Layer)
            .map(|(depth, node)| (depth, node.id.to_string()))
            .collect();
        assert_eq!(
            layers,
            [
                (1, "card".to_owned()),
                (2, "back".to_owned()),
                (2, "front".to_owned()),
                (1, "loose".to_owned())
            ]
        );
        assert_eq!(
            ids(document.descendants(&id("one"), Family::Animation)),
            ["late", "intro", "early", "slide", "show"]
        );
        assert!(
            document
                .descendants(&id("card"), Family::Animation)
                .is_empty()
        );
        assert!(
            document
                .descendants(&id("missing"), Family::Layer)
                .is_empty()
        );
    }

    #[test]
    fn animations_targeting_a_layer_come_in_playback_order() {
        let document = sample();
        // "early" and "slide" both start at 200 and keep their row order;
        // "late" is the first row but starts last.
        assert_eq!(
            ids(document.animations_targeting(&id("front"))),
            ["early", "slide", "late"]
        );
        assert_eq!(ids(document.animations_targeting(&id("card"))), ["show"]);
        assert!(document.animations_targeting(&id("back")).is_empty());
        assert!(document.animations_targeting(&id("missing")).is_empty());
    }

    #[test]
    fn names_are_looked_up_across_the_document() {
        let document = sample();
        assert_eq!(ids(document.find_by_name("Front")), ["front", "other"]);
        assert_eq!(ids(document.find_by_name("Intro")), ["intro"]);
        assert!(document.find_by_name("front").is_empty(), "names are exact");
    }

    #[test]
    fn siblings_and_neighbours() {
        let document = sample();
        assert_eq!(document.siblings(&id("front")), [id("back"), id("front")]);
        assert_eq!(document.siblings(&id("two")), [id("one"), id("two")]);
        assert_eq!(document.previous_sibling(&id("front")), Some(&id("back")));
        assert_eq!(document.next_sibling(&id("front")), None);
        assert_eq!(document.next_sibling(&id("late")), Some(&id("intro")));
        assert_eq!(document.previous_sibling(&id("late")), None);
        assert!(document.siblings(&id("missing")).is_empty());
    }

    #[test]
    fn world_transform_stacks_ancestors_but_not_the_artboard() {
        let document = sample();
        assert_eq!(document.world_transform(&id("one")), Some(Affine::IDENTITY));
        assert_eq!(
            document.world_transform(&id("loose")),
            Some(Affine::translate(10.0, 20.0))
        );
        // The card is turned a quarter about its centre (200, 200), so the
        // front rect's corner at (50, 75) inside it lands at (225, 150).
        let front = document.world_transform(&id("front")).unwrap();
        let corner = front.apply(Point::ZERO);
        assert!(
            (corner.x - 225.0).abs() < 1e-9 && (corner.y - 150.0).abs() < 1e-9,
            "{corner:?}"
        );
        assert!(document.world_transform(&id("late")).is_none());
        assert!(document.world_transform(&id("missing")).is_none());
    }

    #[test]
    fn world_bounds_wrap_the_turned_box() {
        let document = sample();
        assert_eq!(
            document.world_bounds(&id("loose")),
            Some(Rect::new(10.0, 20.0, 100.0, 50.0))
        );
        // 100 x 50 inside the quarter-turned card becomes 50 x 100.
        let front = document.world_bounds(&id("front")).unwrap();
        let expected = Rect::new(175.0, 150.0, 50.0, 100.0);
        for (got, want) in [
            (front.x, expected.x),
            (front.y, expected.y),
            (front.width, expected.width),
            (front.height, expected.height),
        ] {
            assert!((got - want).abs() < 1e-9, "{front:?}");
        }
        assert_eq!(
            document.world_bounds(&id("one")),
            Some(Rect::new(0.0, 0.0, 800.0, 600.0))
        );
    }
}

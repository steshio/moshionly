//! The layer tree as the layer list shows it.
//!
//! The list shows every artboard and, under it, its layers front to back:
//! the layer drawn on top is the first row, as in Figma. Artboards start
//! expanded and groups folded; [`TreeState`] remembers the rows the person
//! opened or closed.

use std::collections::HashSet;

use moshionly_core::build::{self, Place};
use moshionly_core::{Document, Edit, Family, NodeId, NodeType};

use super::describe::display_name;

/// One row of the layer list.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerRow {
    /// The layer.
    pub id: NodeId,
    /// Its type.
    pub node_type: NodeType,
    /// The name to show.
    pub name: String,
    /// How deep it is: 0 for artboards.
    pub depth: usize,
    /// `Some` when the layer has layers inside it: whether they show.
    pub expanded: Option<bool>,
    /// The layer is hidden.
    pub hidden: bool,
    /// The layer is locked.
    pub locked: bool,
}

/// Which rows are open: the exceptions to "artboards open, groups closed".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeState {
    flipped: HashSet<NodeId>,
}

impl TreeState {
    /// Whether the children of `id` show.
    pub fn is_expanded(&self, document: &Document, id: &NodeId) -> bool {
        let artboard = document
            .get(id)
            .is_some_and(|node| node.node_type() == NodeType::Artboard);
        artboard != self.flipped.contains(id)
    }

    /// Opens or closes a row.
    pub fn set_expanded(&mut self, document: &Document, id: &NodeId, expanded: bool) {
        if self.is_expanded(document, id) != expanded && !self.flipped.remove(id) {
            self.flipped.insert(id.clone());
        }
    }

    /// Opens every row above `id`, so that its row shows.
    pub fn reveal(&mut self, document: &Document, id: &NodeId) {
        let ancestors: Vec<NodeId> = document.ancestors(id).map(|node| node.id.clone()).collect();
        for ancestor in ancestors {
            self.set_expanded(document, &ancestor, true);
        }
    }

    /// Forgets rows of nodes that are gone.
    pub fn retain(&mut self, document: &Document) {
        self.flipped.retain(|id| document.contains(id));
    }
}

/// The rows of the layer list. With a non-empty `query`, only the layers
/// whose name contains it, ignoring case, flat and in tree order.
pub fn layer_rows(document: &Document, state: &TreeState, query: &str) -> Vec<LayerRow> {
    let query = query.trim().to_lowercase();
    let mut rows = Vec::new();
    for artboard in document.artboards() {
        push_rows(document, state, artboard, 0, &query, &mut rows);
    }
    rows
}

fn push_rows(
    document: &Document,
    state: &TreeState,
    id: &NodeId,
    depth: usize,
    query: &str,
    rows: &mut Vec<LayerRow>,
) {
    let Some(node) = document.get(id) else {
        return;
    };
    let children = document.children(id, Family::Layer);
    let name = display_name(node);
    let searching = !query.is_empty();
    let expanded = state.is_expanded(document, id);
    if !searching || name.to_lowercase().contains(query) {
        let layer = node.layer();
        rows.push(LayerRow {
            id: id.clone(),
            node_type: node.node_type(),
            name,
            depth: if searching { 0 } else { depth },
            expanded: (!children.is_empty() && !searching).then_some(expanded),
            hidden: layer.is_some_and(|layer| layer.hidden),
            locked: layer.is_some_and(|layer| layer.locked),
        });
    }
    if searching || expanded {
        // Front to back: the last child is drawn on top and listed first.
        for child in children.iter().rev() {
            push_rows(document, state, child, depth + 1, query, rows);
        }
    }
}

/// Where a row being dragged in the layer list would land, relative to the
/// row under the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drop {
    /// Just above the row: in front of that layer.
    Above(NodeId),
    /// Just below the row: behind that layer.
    Below(NodeId),
    /// Into the row's layer, on top of what is in it.
    Inside(NodeId),
}

/// Where a drop over `row` lands, from how far down the row the pointer is
/// (0 at its top, 1 at its bottom). The top and bottom quarters drop beside
/// it; the middle drops into it when it can hold layers.
pub fn drop_at(row: &LayerRow, fraction: f32) -> Drop {
    let holds = row.node_type.holds_layers();
    let id = row.id.clone();
    if holds && (0.25..=0.75).contains(&fraction) {
        Drop::Inside(id)
    } else if fraction < 0.5 {
        Drop::Above(id)
    } else if holds && row.expanded == Some(true) {
        // Below an open group is its first row, on top of its layers.
        Drop::Inside(id)
    } else {
        Drop::Below(id)
    }
}

/// The parent and place a drop puts layers at. Layers dropped beside an
/// artboard go into it; artboards only move among artboards.
pub fn drop_target(
    document: &Document,
    drop: &Drop,
    moving_artboards: bool,
) -> Option<(Option<NodeId>, Place)> {
    let (row, place) = match drop {
        Drop::Inside(id) => {
            if moving_artboards {
                return None;
            }
            return Some((Some(id.clone()), Place::Last));
        }
        Drop::Above(id) => (id, Place::After(id.clone())),
        Drop::Below(id) => (id, Place::Before(id.clone())),
    };
    let node = document.get(row)?;
    let row_is_artboard = node.node_type() == NodeType::Artboard;
    match (moving_artboards, row_is_artboard) {
        // Artboards are listed first to last, not front to back.
        (true, true) => Some((
            None,
            match drop {
                Drop::Above(id) => Place::Before(id.clone()),
                _ => Place::After(row.clone()),
            },
        )),
        (true, false) => None,
        (false, true) => {
            // Above an artboard is on top of it, below it at its back.
            let into = match place {
                Place::After(_) => Place::Last,
                _ => Place::First,
            };
            Some((Some(row.clone()), into))
        }
        (false, false) => Some((node.parent.clone(), place)),
    }
}

/// The edits that move `ids` to where `drop` lands, keeping each layer
/// where it is on the artboard. Layers dropped into themselves, and drops
/// that make no sense, give no edits.
pub fn drop_edits(document: &Document, ids: &[NodeId], drop: &Drop) -> Vec<Edit> {
    let moving_artboards = ids.iter().any(|id| {
        document
            .get(id)
            .is_some_and(|n| n.node_type() == NodeType::Artboard)
    });
    let Some((parent, place)) = drop_target(document, drop, moving_artboards) else {
        return Vec::new();
    };
    // Nothing moves next to or into one of the moving layers.
    let anchor = match &place {
        Place::Before(id) | Place::After(id) => Some(id),
        _ => None,
    };
    if anchor.is_some_and(|a| ids.contains(a)) || parent.as_ref().is_some_and(|p| ids.contains(p)) {
        return Vec::new();
    }
    let mut working = document.clone();
    let mut edits = Vec::new();
    // Back to front, so each lands in front of the one before it.
    let mut ordered: Vec<&NodeId> = ids.iter().collect();
    ordered.sort_by_key(|id| document.walk_index(id));
    let mut place = place;
    for id in ordered {
        let result = match &parent {
            Some(parent) => build::move_node(&working, id, parent, &place, true),
            None => build::index_at(&working, None, Family::Layer, &place, Some(id))
                .map(|index| vec![Edit::set(id.clone(), "index", index)]),
        };
        let Ok(more) = result else {
            continue;
        };
        if working.apply(&more).is_ok() {
            edits.extend(more);
            place = Place::After(id.clone());
        }
    }
    edits
}

/// Where a node is in the tree, for sorting nodes back to front.
trait WalkIndex {
    fn walk_index(&self, id: &NodeId) -> usize;
}

impl WalkIndex for Document {
    fn walk_index(&self, id: &NodeId) -> usize {
        self.nodes()
            .position(|node| node.id == *id)
            .unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","name":"Scene"},
            {"id":"card","parent":"ab","index":"a0","type":"group","name":"Card"},
            {"id":"bg","parent":"card","index":"a0","type":"rect","name":"Background"},
            {"id":"title","parent":"card","index":"a1","type":"text","content":"Hello"},
            {"id":"top","parent":"ab","index":"a1","type":"ellipse","hidden":true,"locked":true},
            {"id":"ab2","parent":null,"index":"a1","type":"artboard","name":"Empty"}
            ]}"##,
        )
        .unwrap()
    }

    fn names(rows: &[LayerRow]) -> Vec<&str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[test]
    fn rows_list_layers_front_to_back_with_groups_folded() {
        let doc = document();
        let rows = layer_rows(&doc, &TreeState::default(), "");
        assert_eq!(names(&rows), ["Scene", "Ellipse", "Card", "Empty"]);
        assert_eq!(rows[0].expanded, Some(true));
        assert_eq!(rows[2].expanded, Some(false));
        assert_eq!(
            rows[3].expanded, None,
            "an empty artboard has nothing to open"
        );
        assert!(rows[1].hidden && rows[1].locked);
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn opening_and_closing_rows() {
        let doc = document();
        let mut state = TreeState::default();
        state.set_expanded(&doc, &NodeId::from("card"), true);
        let rows = layer_rows(&doc, &state, "");
        assert_eq!(
            names(&rows),
            ["Scene", "Ellipse", "Card", "Hello", "Background", "Empty"]
        );
        assert_eq!(rows[3].depth, 2);
        state.set_expanded(&doc, &NodeId::from("ab"), false);
        assert_eq!(names(&layer_rows(&doc, &state, "")), ["Scene", "Empty"]);
        // Setting what is already so changes nothing.
        state.set_expanded(&doc, &NodeId::from("ab"), false);
        assert!(!state.is_expanded(&doc, &NodeId::from("ab")));
    }

    #[test]
    fn revealing_a_layer_opens_its_ancestors() {
        let doc = document();
        let mut state = TreeState::default();
        state.set_expanded(&doc, &NodeId::from("ab"), false);
        state.reveal(&doc, &NodeId::from("bg"));
        assert!(state.is_expanded(&doc, &NodeId::from("ab")));
        assert!(state.is_expanded(&doc, &NodeId::from("card")));
        assert!(
            layer_rows(&doc, &state, "")
                .iter()
                .any(|row| row.name == "Background")
        );
    }

    fn row(doc: &Document, id: &str, state: &TreeState) -> LayerRow {
        layer_rows(doc, state, "")
            .into_iter()
            .find(|row| row.id == NodeId::from(id))
            .unwrap()
    }

    #[test]
    fn the_pointer_picks_above_below_or_inside() {
        let doc = document();
        let state = TreeState::default();
        let ellipse = row(&doc, "top", &state);
        assert_eq!(drop_at(&ellipse, 0.1), Drop::Above(NodeId::from("top")));
        assert_eq!(drop_at(&ellipse, 0.5), Drop::Below(NodeId::from("top")));
        let card = row(&doc, "card", &state);
        assert_eq!(drop_at(&card, 0.5), Drop::Inside(NodeId::from("card")));
        assert_eq!(drop_at(&card, 0.9), Drop::Below(NodeId::from("card")));
        let mut open = TreeState::default();
        open.set_expanded(&doc, &NodeId::from("card"), true);
        let card = row(&doc, "card", &open);
        assert_eq!(drop_at(&card, 0.9), Drop::Inside(NodeId::from("card")));
    }

    #[test]
    fn dropping_reorders_and_moves_between_groups() {
        let doc = document();
        let id = |text: &str| NodeId::from(text);
        // The ellipse below the card: behind it.
        let mut moved = doc.clone();
        moved
            .apply(&drop_edits(&doc, &[id("top")], &Drop::Below(id("card"))))
            .unwrap();
        assert_eq!(
            moved.children(&id("ab"), Family::Layer),
            [id("top"), id("card")]
        );
        // Into the card, on top of what is in it.
        let mut moved = doc.clone();
        moved
            .apply(&drop_edits(&doc, &[id("top")], &Drop::Inside(id("card"))))
            .unwrap();
        assert_eq!(
            moved.children(&id("card"), Family::Layer).last(),
            Some(&id("top"))
        );
        // Out of the card, above the ellipse.
        let mut moved = doc.clone();
        moved
            .apply(&drop_edits(&doc, &[id("bg")], &Drop::Above(id("top"))))
            .unwrap();
        assert_eq!(
            moved.children(&id("ab"), Family::Layer).last(),
            Some(&id("bg"))
        );
        // Beside an artboard row: into that artboard.
        let mut moved = doc.clone();
        moved
            .apply(&drop_edits(&doc, &[id("top")], &Drop::Below(id("ab2"))))
            .unwrap();
        assert_eq!(moved.children(&id("ab2"), Family::Layer), [id("top")]);
        // Not into itself, and artboards only among artboards.
        assert!(drop_edits(&doc, &[id("card")], &Drop::Inside(id("card"))).is_empty());
        assert!(drop_edits(&doc, &[id("ab")], &Drop::Inside(id("card"))).is_empty());
        let mut moved = doc.clone();
        moved
            .apply(&drop_edits(&doc, &[id("ab")], &Drop::Below(id("ab2"))))
            .unwrap();
        assert_eq!(moved.artboards(), [id("ab2"), id("ab")]);
    }

    #[test]
    fn a_query_lists_matches_flat() {
        let doc = document();
        let rows = layer_rows(&doc, &TreeState::default(), "  BACK ");
        assert_eq!(names(&rows), ["Background"]);
        assert_eq!(rows[0].depth, 0);
        assert!(layer_rows(&doc, &TreeState::default(), "nothing").is_empty());
    }
}

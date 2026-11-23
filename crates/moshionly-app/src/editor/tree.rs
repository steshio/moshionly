//! The layer tree as the layer list shows it.
//!
//! The list shows every artboard and, under it, its layers front to back:
//! the layer drawn on top is the first row, as in Figma. Artboards start
//! expanded and groups folded; [`TreeState`] remembers the rows the person
//! opened or closed.

use std::collections::HashSet;

use moshionly_core::{Document, Family, NodeId, NodeType};

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

    #[test]
    fn a_query_lists_matches_flat() {
        let doc = document();
        let rows = layer_rows(&doc, &TreeState::default(), "  BACK ");
        assert_eq!(names(&rows), ["Background"]);
        assert_eq!(rows[0].depth, 0);
        assert!(layer_rows(&doc, &TreeState::default(), "nothing").is_empty());
    }
}

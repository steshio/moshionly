//! The selection, and the rules that decide what a click selects.
//!
//! Groups behave as in Figma. A click selects the outermost group below the
//! artboard; once something inside a group is selected, clicks pick among
//! that group's children. A double-click goes one level into the selected
//! group, and ⌘-click picks the deepest layer under the pointer at once.
//! Locked layers, and everything inside them, cannot be picked on the
//! canvas: a click passes through them to what is behind.

use moshionly_core::{Document, Family, NodeId};

/// The selected nodes, in the order they were selected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<NodeId>,
}

impl Selection {
    /// Nothing selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// The selected ids, oldest first.
    pub fn ids(&self) -> &[NodeId] {
        &self.ids
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// How many nodes are selected.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether `id` is selected.
    pub fn contains(&self, id: &NodeId) -> bool {
        self.ids.contains(id)
    }

    /// The one selected node, when exactly one is.
    pub fn single(&self) -> Option<&NodeId> {
        match self.ids.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }

    /// Selects exactly these nodes. Repeats are dropped.
    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.ids.clear();
        for id in ids {
            if !self.ids.contains(&id) {
                self.ids.push(id);
            }
        }
    }

    /// Selects only `id`.
    pub fn select(&mut self, id: NodeId) {
        self.set([id]);
    }

    /// Adds `id` when it is not selected and removes it when it is: what
    /// shift-click does.
    pub fn toggle(&mut self, id: NodeId) {
        if let Some(at) = self.ids.iter().position(|selected| *selected == id) {
            self.ids.remove(at);
        } else {
            self.ids.push(id);
        }
    }

    /// Adds nodes that are not selected yet.
    pub fn extend(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        for id in ids {
            if !self.ids.contains(&id) {
                self.ids.push(id);
            }
        }
    }

    /// Selects nothing.
    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Drops nodes that are not in `document` any more, or are not layers.
    /// Returns whether anything was dropped.
    pub fn retain_existing(&mut self, document: &Document) -> bool {
        let before = self.ids.len();
        self.ids
            .retain(|id| document.get(id).is_some_and(|node| node.is_layer()));
        self.ids.len() != before
    }
}

/// How a click on a layer chooses what to select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A plain click: the outermost group, or a sibling of the selection
    /// inside a group that is already open.
    Normal,
    /// A ⌘-click: the deepest layer under the pointer.
    Deep,
    /// A double-click: one level into the selected group.
    EnterGroup,
}

/// The nodes from the artboard down to `leaf`: artboard first, `leaf` last.
pub fn path_to(document: &Document, leaf: &NodeId) -> Vec<NodeId> {
    let mut path: Vec<NodeId> = document
        .ancestors(leaf)
        .map(|node| node.id.clone())
        .collect();
    path.reverse();
    path.push(leaf.clone());
    path
}

/// Whether `id` or anything above it below the artboard is locked, which
/// keeps it from being picked on the canvas.
pub fn is_locked(document: &Document, id: &NodeId) -> bool {
    let locked = |id: &NodeId| {
        document
            .get(id)
            .and_then(|node| node.layer())
            .is_some_and(|layer| layer.locked)
    };
    path_to(document, id)
        .iter()
        .skip(1) // An artboard's lock does not stop picking its layers.
        .any(locked)
}

/// Whether `id` or anything above it below the artboard is hidden: it is
/// not drawn, so nothing on the canvas picks it.
pub fn is_hidden(document: &Document, id: &NodeId) -> bool {
    let hidden = |id: &NodeId| {
        document
            .get(id)
            .and_then(|node| node.layer())
            .is_some_and(|layer| layer.hidden)
    };
    path_to(document, id).iter().skip(1).any(hidden)
}

/// Whether the canvas may pick `id` (by a marquee, or with select all):
/// it is neither locked nor hidden.
pub fn is_pickable(document: &Document, id: &NodeId) -> bool {
    !is_locked(document, id) && !is_hidden(document, id)
}

/// What a click on the layer `leaf` selects, given what is selected now.
///
/// `leaf` is the frontmost layer drawn under the pointer, as hit testing
/// finds it. The result is never the artboard itself: artboards are
/// selected by their name or in the layer list. `None` when `leaf` is not a
/// layer in the document or cannot be picked because it is locked.
pub fn pick(
    document: &Document,
    leaf: &NodeId,
    selection: &[NodeId],
    mode: Pick,
) -> Option<NodeId> {
    let node = document.get(leaf)?;
    if !node.is_layer() || node.parent.is_none() || is_locked(document, leaf) {
        return None;
    }
    let path = path_to(document, leaf);
    // `path[0]` is the artboard, `path[1]` the outermost layer.
    let after = |at: usize| path.get(at + 1).cloned();
    match mode {
        Pick::Deep => Some(leaf.clone()),
        Pick::EnterGroup => {
            // The selected node on the path that is nearest the leaf.
            let entered = path
                .iter()
                .enumerate()
                .skip(1)
                .rev()
                .find(|(_, id)| selection.contains(id) && *id != leaf)
                .map(|(at, _)| at);
            match entered {
                Some(at) => after(at),
                None => pick(document, leaf, selection, Pick::Normal),
            }
        }
        Pick::Normal => {
            // The containers the selection is in are open: a click picks
            // the child of the deepest open container on the path.
            let open = |id: &NodeId| {
                selection
                    .iter()
                    .any(|selected| document.is_ancestor(id, selected))
            };
            let deepest = path
                .iter()
                .enumerate()
                .take(path.len() - 1)
                .rev()
                .find(|(at, id)| *at == 0 || open(id))
                .map_or(0, |(at, _)| at);
            after(deepest)
        }
    }
}

/// The layers a marquee picks among: the children of the artboard, front
/// to back. Locked and hidden layers are left out.
pub fn marquee_candidates(document: &Document, artboard: &NodeId) -> Vec<NodeId> {
    document
        .children(artboard, Family::Layer)
        .iter()
        .rev()
        .filter(|id| is_pickable(document, id))
        .cloned()
        .collect()
}

/// The layers directly in `artboard` that can be picked, back to front.
pub fn top_level(document: &Document, artboard: &NodeId) -> Vec<NodeId> {
    document
        .children(artboard, Family::Layer)
        .iter()
        .filter(|id| is_pickable(document, id))
        .cloned()
        .collect()
}

/// What "select all" selects: the siblings of the selection, or the top
/// level of `artboard` when nothing below an artboard is selected.
pub fn select_all(
    document: &Document,
    selection: &[NodeId],
    artboard: Option<&NodeId>,
) -> Vec<NodeId> {
    let parent = selection
        .first()
        .and_then(|id| document.get(id))
        .and_then(|node| node.parent.clone());
    match parent {
        Some(parent) => document
            .children(&parent, Family::Layer)
            .iter()
            .filter(|id| is_pickable(document, id))
            .cloned()
            .collect(),
        None => match (selection.is_empty(), artboard) {
            // Artboards selected: select every artboard.
            (false, _) => document.artboards().to_vec(),
            (true, Some(artboard)) => top_level(document, artboard),
            (true, None) => Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    /// An artboard holding a card group (with a nested badge group) and a
    /// loose rect; a second artboard with one rect.
    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard"},
            {"id":"card","parent":"ab","index":"a0","type":"group"},
            {"id":"bg","parent":"card","index":"a0","type":"rect"},
            {"id":"badge","parent":"card","index":"a1","type":"group"},
            {"id":"dot","parent":"badge","index":"a0","type":"ellipse"},
            {"id":"loose","parent":"ab","index":"a1","type":"rect"},
            {"id":"ab2","parent":null,"index":"a1","type":"artboard","x":2000},
            {"id":"other","parent":"ab2","index":"a0","type":"rect"}
            ]}"##,
        )
        .unwrap()
    }

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    fn ids(texts: &[&str]) -> Vec<NodeId> {
        texts.iter().map(|text| id(text)).collect()
    }

    #[test]
    fn a_click_picks_the_outermost_group() {
        let doc = document();
        assert_eq!(pick(&doc, &id("dot"), &[], Pick::Normal), Some(id("card")));
        assert_eq!(pick(&doc, &id("bg"), &[], Pick::Normal), Some(id("card")));
        assert_eq!(
            pick(&doc, &id("loose"), &[], Pick::Normal),
            Some(id("loose"))
        );
        // Selecting the group itself does not open it.
        assert_eq!(
            pick(&doc, &id("dot"), &ids(&["card"]), Pick::Normal),
            Some(id("card"))
        );
    }

    #[test]
    fn inside_an_open_group_clicks_pick_siblings() {
        let doc = document();
        // With the background of the card selected, the card is open.
        assert_eq!(
            pick(&doc, &id("dot"), &ids(&["bg"]), Pick::Normal),
            Some(id("badge"))
        );
        // With the dot selected, both the card and the badge are open.
        assert_eq!(
            pick(&doc, &id("dot"), &ids(&["dot"]), Pick::Normal),
            Some(id("dot"))
        );
        assert_eq!(
            pick(&doc, &id("bg"), &ids(&["dot"]), Pick::Normal),
            Some(id("bg"))
        );
        // Something outside the open group is picked from the top again.
        assert_eq!(
            pick(&doc, &id("loose"), &ids(&["dot"]), Pick::Normal),
            Some(id("loose"))
        );
        assert_eq!(
            pick(&doc, &id("other"), &ids(&["dot"]), Pick::Normal),
            Some(id("other"))
        );
    }

    #[test]
    fn a_double_click_goes_one_level_in() {
        let doc = document();
        assert_eq!(
            pick(&doc, &id("dot"), &ids(&["card"]), Pick::EnterGroup),
            Some(id("badge"))
        );
        assert_eq!(
            pick(&doc, &id("dot"), &ids(&["badge"]), Pick::EnterGroup),
            Some(id("dot"))
        );
        // At the bottom it stays on the leaf.
        assert_eq!(
            pick(&doc, &id("dot"), &ids(&["dot"]), Pick::EnterGroup),
            Some(id("dot"))
        );
        // With nothing selected it acts as a click.
        assert_eq!(
            pick(&doc, &id("dot"), &[], Pick::EnterGroup),
            Some(id("card"))
        );
    }

    #[test]
    fn command_click_picks_the_deepest_layer() {
        let doc = document();
        assert_eq!(pick(&doc, &id("dot"), &[], Pick::Deep), Some(id("dot")));
        assert_eq!(pick(&doc, &id("ab"), &[], Pick::Normal), None);
        assert_eq!(pick(&doc, &id("nothing"), &[], Pick::Normal), None);
    }

    #[test]
    fn locked_layers_and_their_contents_cannot_be_picked() {
        let mut doc = document();
        doc.apply(&[moshionly_core::Edit::set("badge", "locked", true)])
            .unwrap();
        assert!(is_locked(&doc, &id("dot")));
        assert!(!is_locked(&doc, &id("bg")));
        assert_eq!(pick(&doc, &id("dot"), &[], Pick::Normal), None);
        assert_eq!(pick(&doc, &id("dot"), &[], Pick::Deep), None);
        assert_eq!(pick(&doc, &id("bg"), &[], Pick::Normal), Some(id("card")));
        // A locked artboard still lets its layers be picked.
        doc.apply(&[moshionly_core::Edit::set("ab2", "locked", true)])
            .unwrap();
        assert_eq!(
            pick(&doc, &id("other"), &[], Pick::Normal),
            Some(id("other"))
        );
    }

    #[test]
    fn hidden_layers_are_not_picked_by_select_all_or_a_marquee() {
        let mut doc = document();
        doc.apply(&[moshionly_core::Edit::set("loose", "hidden", true)])
            .unwrap();
        assert!(is_hidden(&doc, &id("loose")));
        assert!(!is_pickable(&doc, &id("loose")));
        assert_eq!(select_all(&doc, &[], Some(&id("ab"))), ids(&["card"]));
        assert_eq!(select_all(&doc, &ids(&["card"]), None), ids(&["card"]));
        assert_eq!(marquee_candidates(&doc, &id("ab")), ids(&["card"]));
        // Inside a hidden group, too.
        doc.apply(&[moshionly_core::Edit::set("badge", "hidden", true)])
            .unwrap();
        assert!(is_hidden(&doc, &id("dot")));
        assert_eq!(select_all(&doc, &ids(&["bg"]), None), ids(&["bg"]));
    }

    #[test]
    fn the_selection_toggles_and_forgets_removed_nodes() {
        let mut selection = Selection::new();
        selection.set(ids(&["card", "loose", "card"]));
        assert_eq!(selection.ids(), ids(&["card", "loose"]));
        selection.toggle(id("card"));
        assert_eq!(selection.single(), Some(&id("loose")));
        selection.toggle(id("bg"));
        assert_eq!(selection.len(), 2);

        let mut doc = document();
        doc.apply(&[moshionly_core::Edit::remove("loose")]).unwrap();
        assert!(selection.retain_existing(&doc));
        assert_eq!(selection.ids(), ids(&["bg"]));
        assert!(!selection.retain_existing(&doc));
        selection.clear();
        assert!(selection.is_empty());
    }

    #[test]
    fn select_all_takes_the_siblings_of_the_selection() {
        let doc = document();
        assert_eq!(select_all(&doc, &ids(&["dot"]), None), ids(&["dot"]));
        assert_eq!(select_all(&doc, &ids(&["bg"]), None), ids(&["bg", "badge"]));
        assert_eq!(
            select_all(&doc, &[], Some(&id("ab"))),
            ids(&["card", "loose"])
        );
        assert_eq!(select_all(&doc, &ids(&["ab"]), None), ids(&["ab", "ab2"]));
        assert!(select_all(&doc, &[], None).is_empty());
        assert_eq!(marquee_candidates(&doc, &id("ab")), ids(&["loose", "card"]));
    }
}

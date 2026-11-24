//! Actions: the animations of layers, as the Animate tab and the timeline
//! work on them.
//!
//! An action is one animation node. This module says which actions a layer
//! can take and how the add menu lists them, where a new action starts, and
//! writes the edits that add, retime, stagger, duplicate, group and remove
//! actions. The rules are plain functions of a document; the [`Editor`]
//! methods at the bottom apply each change as one undo step and keep the
//! selected actions in step with what they did.

use std::collections::HashSet;

use moshionly_core::build::{self, BuildError};
use moshionly_core::validate::animation_applies_to;
use moshionly_core::{Document, Edit, EditError, Family, Node, NodeId, NodeType};
use serde_json::json;

use super::Editor;
use super::describe::{next_name, type_label};

/// Why actions could not be changed.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// Nothing is selected that the change could work on.
    #[error("select a layer first")]
    NothingSelected,
    /// The action type does not work on one of the layers.
    #[error("{action} does not work on {layer}")]
    WrongLayer {
        /// The action's name.
        action: &'static str,
        /// The layer's name.
        layer: String,
    },
    /// The edits could not be written.
    #[error(transparent)]
    Build(#[from] BuildError),
    /// The document refused the edits.
    #[error("{}", .0.problem.message)]
    Edit(#[from] EditError),
}

/// The parts of the add menu, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuSection {
    /// Ready-made entrances.
    In,
    /// Ready-made exits.
    Out,
    /// One property from one value to another.
    Change,
    /// Showing or hiding at one moment.
    Visibility,
    /// Text that comes in piece by piece, and counting numbers.
    Text,
}

impl MenuSection {
    /// The heading shown above the section.
    pub fn label(self) -> &'static str {
        match self {
            Self::In => "In",
            Self::Out => "Out",
            Self::Change => "Change",
            Self::Visibility => "Show and hide",
            Self::Text => "Text",
        }
    }
}

/// Every action type the add menu offers, with its section, in menu order.
pub const MENU: [(MenuSection, NodeType); 29] = {
    use MenuSection as S;
    use NodeType as T;
    [
        (S::In, T::FadeIn),
        (S::In, T::SlideIn),
        (S::In, T::GrowIn),
        (S::In, T::ShrinkIn),
        (S::In, T::SpinIn),
        (S::In, T::BlurIn),
        (S::In, T::RevealIn),
        (S::Out, T::FadeOut),
        (S::Out, T::SlideOut),
        (S::Out, T::GrowOut),
        (S::Out, T::ShrinkOut),
        (S::Out, T::SpinOut),
        (S::Out, T::BlurOut),
        (S::Out, T::RevealOut),
        (S::Change, T::Move),
        (S::Change, T::Rotate),
        (S::Change, T::Scale),
        (S::Change, T::Opacity),
        (S::Change, T::Resize),
        (S::Change, T::Fill),
        (S::Change, T::Stroke),
        (S::Change, T::CornerRadius),
        (S::Change, T::Blur),
        (S::Change, T::Shadow),
        (S::Visibility, T::Show),
        (S::Visibility, T::Hide),
        (S::Text, T::TextIn),
        (S::Text, T::TextOut),
        (S::Text, T::Counter),
    ]
};

/// The menu entries that work on every one of `targets`, in menu order,
/// with the text section first when there is one. Empty when there are no
/// targets or one of them is not a layer.
pub fn menu_for(document: &Document, targets: &[NodeId]) -> Vec<(MenuSection, NodeType)> {
    let types: Option<Vec<NodeType>> = targets
        .iter()
        .map(|id| {
            document
                .get(id)
                .filter(|n| n.is_layer())
                .map(Node::node_type)
        })
        .collect();
    let Some(types) = types.filter(|types| !types.is_empty()) else {
        return Vec::new();
    };
    let mut menu: Vec<(MenuSection, NodeType)> = MENU
        .into_iter()
        .filter(|(_, action)| {
            types
                .iter()
                .all(|layer| animation_applies_to(*action, *layer))
        })
        .collect();
    // What only text can do comes first for text.
    menu.sort_by_key(|(section, _)| *section != MenuSection::Text);
    menu
}

/// Where a new action of `length` milliseconds starts: at the playhead, in
/// whole milliseconds. With the playhead at or past the end of a scene
/// `scene` long, it ends at the end of the scene instead, so it can be seen.
pub fn new_start(playhead: f64, length: f64, scene: f64) -> f64 {
    let playhead = if playhead.is_finite() {
        playhead.max(0.0)
    } else {
        0.0
    };
    let start = if scene > 0.0 && playhead >= scene {
        scene - length
    } else {
        playhead
    };
    start.max(0.0).round()
}

/// `ids` in the order the layer list shows them, top to bottom: the front
/// layer first. Ids that are not layers go last, in their given order.
pub fn in_list_order(document: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut listed: Vec<NodeId> = Vec::with_capacity(ids.len());
    let wanted: HashSet<&NodeId> = ids.iter().collect();
    for artboard in document.artboards() {
        let mut layers: Vec<&NodeId> = document
            .walk(artboard, Family::Layer)
            .map(|(_, node)| &node.id)
            .filter(|id| wanted.contains(id))
            .collect();
        layers.reverse();
        listed.extend(layers.into_iter().cloned());
        if wanted.contains(artboard) {
            listed.push(artboard.clone());
        }
    }
    for id in ids {
        if !listed.contains(id) {
            listed.push(id.clone());
        }
    }
    listed
}

/// Edits that add an action of type `ty` to each of `targets`, the first
/// starting at `start` and each next one `stagger` milliseconds after the
/// one before. Each action gets its type's length, easing and settings.
///
/// Returns the new actions' ids, in the order of `targets`, with the edits.
pub fn add_edits(
    document: &Document,
    ty: NodeType,
    targets: &[NodeId],
    start: f64,
    stagger: f64,
) -> Result<(Vec<NodeId>, Vec<Edit>), ActionError> {
    if targets.is_empty() {
        return Err(ActionError::NothingSelected);
    }
    // Work on a scratch copy so that each new row sees the ones before it.
    let mut scratch = document.clone();
    let mut ids = Vec::with_capacity(targets.len());
    let mut edits = Vec::with_capacity(targets.len());
    for (n, target) in targets.iter().enumerate() {
        let at = (start + n as f64 * stagger.max(0.0)).round();
        let data =
            build::default_animation(ty, target, at).ok_or(BuildError::WrongType(ty, "action"))?;
        let (id, step) = build::add_animation(&scratch, data, None).map_err(|error| {
            match (&error, scratch.get(target)) {
                (BuildError::WrongTarget { .. }, Some(layer)) => ActionError::WrongLayer {
                    action: type_label(ty),
                    layer: super::describe::display_name(layer),
                },
                _ => ActionError::Build(error),
            }
        })?;
        scratch.apply(&step)?;
        ids.push(id);
        edits.extend(step);
    }
    Ok((ids, edits))
}

/// The edit that gives an action new times, or `None` when it already has
/// them or is not an action. `show` and `hide` take only the start.
pub fn retime(document: &Document, id: &NodeId, start: f64, end: f64) -> Option<Edit> {
    let node = document.get(id)?;
    let (now_start, now_end) = node.data.span()?;
    let start = start.max(0.0);
    match node.node_type() {
        NodeType::Show | NodeType::Hide => {
            (start != now_start).then(|| Edit::set(id.clone(), "start", start))
        }
        _ => {
            let end = end.max(start);
            ((start, end) != (now_start, now_end))
                .then(|| Edit::update(id.clone(), [("start", json!(start)), ("end", json!(end))]))
        }
    }
}

/// The actions among `ids`, by start time: earliest first, ties in timeline
/// row order.
pub fn by_start(document: &Document, ids: &[NodeId]) -> Vec<(NodeId, f64, f64)> {
    let order: Vec<&NodeId> = document
        .artboards()
        .iter()
        .flat_map(|artboard| document.walk(artboard, Family::Animation))
        .map(|(_, node)| &node.id)
        .collect();
    let mut spans: Vec<(usize, NodeId, f64, f64)> = ids
        .iter()
        .filter_map(|id| {
            let (start, end) = document.get(id)?.data.span()?;
            let row = order
                .iter()
                .position(|row| *row == id)
                .unwrap_or(usize::MAX);
            Some((row, id.clone(), start, end))
        })
        .collect();
    spans.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
    spans
        .into_iter()
        .map(|(_, id, start, end)| (id, start, end))
        .collect()
}

/// The times of actions spaced out evenly: the earliest keeps its start and
/// each next one starts `delay` milliseconds after the one before. Lengths
/// are kept. `spans` must be in the order to play them, as [`by_start`]
/// gives them.
pub fn staggered(spans: &[(NodeId, f64, f64)], delay: f64) -> Vec<(NodeId, f64, f64)> {
    let Some((_, first, _)) = spans.first() else {
        return Vec::new();
    };
    let delay = delay.max(0.0);
    spans
        .iter()
        .enumerate()
        .map(|(n, (id, start, end))| {
            let at = (first + n as f64 * delay).round();
            (id.clone(), at, at + (end - start))
        })
        .collect()
}

/// Edits that put new times on actions, leaving out those that keep theirs.
pub fn retime_all(document: &Document, times: &[(NodeId, f64, f64)]) -> Vec<Edit> {
    times
        .iter()
        .filter_map(|(id, start, end)| retime(document, id, *start, *end))
        .collect()
}

/// The folders (`animation_group`) among `ids`, and the folders holding
/// any of them, each once.
pub fn folders_of(document: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut folders: Vec<NodeId> = Vec::new();
    for id in ids {
        let Some(node) = document.get(id) else {
            continue;
        };
        let folder = if node.node_type() == NodeType::AnimationGroup {
            Some(node)
        } else {
            document
                .parent_of(id)
                .filter(|parent| parent.node_type() == NodeType::AnimationGroup)
        };
        if let Some(folder) = folder
            && !folders.contains(&folder.id)
        {
            folders.push(folder.id.clone());
        }
    }
    folders
}

/// Whether `id` is a timeline folder (`animation_group`).
fn is_folder_id(document: &Document, id: &NodeId) -> bool {
    document
        .get(id)
        .is_some_and(|node| node.node_type() == NodeType::AnimationGroup)
}

/// The actions among `ids`, with the folders among them standing for the
/// actions in them, each once. Folders themselves are left out.
pub fn leaf_actions(document: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut leaves: Vec<NodeId> = Vec::new();
    let mut push = |node: &Node| {
        if node.node_type() != NodeType::AnimationGroup && !leaves.contains(&node.id) {
            leaves.push(node.id.clone());
        }
    };
    for id in ids {
        let Some(node) = document.get(id).filter(|node| node.is_animation()) else {
            continue;
        };
        push(node);
        for (_, inner) in document.walk(id, Family::Animation) {
            push(inner);
        }
    }
    leaves
}

/// The layers the actions among `ids` animate, each once; a folder stands
/// for the actions in it.
pub fn targets_of(document: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut targets: Vec<NodeId> = Vec::new();
    for action in leaf_actions(document, ids) {
        if let Some(target) = document.get(&action).and_then(Node::target)
            && !targets.contains(target)
        {
            targets.push(target.clone());
        }
    }
    targets
}

/// `edits`, followed by the removal of the folders they empty: those that
/// held actions before and hold none after. A folder with nothing in it
/// has no bars to click on the timeline, so it goes with its last action,
/// in the same undo step.
pub fn with_emptied_folders(document: &Document, edits: Vec<Edit>) -> Result<Vec<Edit>, EditError> {
    let mut after = document.clone();
    after.apply(&edits)?;
    let holds_actions = |document: &Document, folder: &NodeId| {
        !leaf_actions(document, std::slice::from_ref(folder)).is_empty()
    };
    let emptied: Vec<NodeId> = after
        .artboards()
        .iter()
        .flat_map(|artboard| after.walk(artboard, Family::Animation))
        .map(|(_, node)| &node.id)
        .filter(|id| is_folder_id(&after, id))
        .filter(|id| holds_actions(document, id) && !holds_actions(&after, id))
        .cloned()
        .collect();
    let mut edits = edits;
    // A folder inside another emptied one goes with it.
    edits.extend(build::remove(&after, &emptied));
    Ok(edits)
}

/// Edits that dissolve the folders among or around `ids`, one after the
/// other. Returns the edits with the actions that were in them.
pub fn ungroup_edits(
    document: &Document,
    ids: &[NodeId],
) -> Result<(Vec<Edit>, Vec<NodeId>), ActionError> {
    let folders = folders_of(document, ids);
    let mut scratch = document.clone();
    let mut edits = Vec::new();
    let mut freed = Vec::new();
    for folder in folders {
        freed.extend(
            scratch
                .child_nodes(&folder, Family::Animation)
                .map(|node| node.id.clone()),
        );
        let step = build::ungroup(&scratch, &folder)?;
        scratch.apply(&step)?;
        edits.extend(step);
    }
    Ok((edits, freed))
}

/// The name for a new folder: "Folder 1", "Folder 2" and so on.
pub fn folder_name(document: &Document) -> String {
    let names: Vec<&str> = document
        .artboards()
        .iter()
        .flat_map(|artboard| document.walk(artboard, Family::Animation))
        .filter(|(_, node)| node.node_type() == NodeType::AnimationGroup)
        .map(|(_, node)| node.name())
        .collect();
    next_name("Folder", names)
}

/// The actions that target `layers`, by start time.
pub fn actions_of(document: &Document, layers: &[NodeId]) -> Vec<NodeId> {
    let ids: Vec<NodeId> = layers
        .iter()
        .flat_map(|layer| document.animations_targeting(layer))
        .map(|node| node.id.clone())
        .collect();
    by_start(document, &ids)
        .into_iter()
        .map(|(id, ..)| id)
        .collect()
}

impl Editor {
    /// The selected actions, oldest selection first.
    pub fn actions(&self) -> &[NodeId] {
        &self.actions
    }

    /// Selects exactly these actions. Ids that are not actions are left out,
    /// and so is a folder without every action in it: a folder is selected
    /// as a whole or not at all, so that Delete never takes away an action
    /// the person let go of.
    pub fn set_actions(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        let document = std::sync::Arc::clone(&self.document);
        self.actions.clear();
        for id in ids {
            let action = document.get(&id).is_some_and(|node| node.is_animation());
            if action && !self.actions.contains(&id) {
                self.actions.push(id);
            }
        }
        let partial: Vec<NodeId> = self
            .actions
            .iter()
            .filter(|id| is_folder_id(&document, id))
            .filter(|folder| {
                leaf_actions(&document, std::slice::from_ref(*folder))
                    .iter()
                    .any(|inside| !self.actions.contains(inside))
            })
            .cloned()
            .collect();
        self.actions.retain(|id| !partial.contains(id));
    }

    /// The one selected action, when exactly one is selected.
    pub fn single_action(&self) -> Option<&NodeId> {
        match self.actions.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }

    /// Selects a timeline folder: the folder itself, so that commands work
    /// on it as a whole, and the actions in it.
    pub fn select_folder(&mut self, folder: &NodeId) {
        let document = std::sync::Arc::clone(&self.document);
        if !is_folder_id(&document, folder) {
            return;
        }
        let inside = document
            .walk(folder, Family::Animation)
            .map(|(_, node)| node.id.clone());
        self.set_actions(std::iter::once(folder.clone()).chain(inside));
    }

    /// Selects the layers the selected actions target, each once.
    pub fn select_targets(&mut self) {
        let targets = targets_of(&self.document, &self.actions);
        if !targets.is_empty() {
            self.selection.set(targets);
        }
    }

    /// Adds an action of type `ty` to each selected layer at the playhead,
    /// one `stagger` milliseconds after the other from the top of the layer
    /// list down, as one undo step. The new actions become the selected
    /// actions.
    pub fn add_actions(&mut self, ty: NodeType, stagger: f64) -> Result<Vec<NodeId>, ActionError> {
        let document = std::sync::Arc::clone(&self.document);
        let targets = in_list_order(&document, self.selection.ids());
        let scene = targets
            .first()
            .and_then(|id| document.artboard_of(id))
            .and_then(|artboard| moshionly_core::eval::duration_of(&document, &artboard.id))
            .unwrap_or(0.0);
        let start = new_start(self.playhead, build::default_duration(ty), scene);
        let (ids, edits) = add_edits(&document, ty, &targets, start, stagger)?;
        self.apply(&format!("Add {}", type_label(ty)), &edits)?;
        self.actions = ids.clone();
        Ok(ids)
    }

    /// Removes the selected actions, as one undo step. A selected folder
    /// goes with everything in it, and a folder left empty goes too.
    pub fn delete_actions(&mut self) -> Result<bool, ActionError> {
        let ids = std::mem::take(&mut self.actions);
        let document = std::sync::Arc::clone(&self.document);
        let edits = with_emptied_folders(&document, build::remove(&document, &ids))?;
        // What is selected besides what is in a selected folder.
        let outermost: Vec<&NodeId> = ids
            .iter()
            .filter(|id| {
                !document
                    .ancestors(id)
                    .any(|ancestor| ids.contains(&ancestor.id))
            })
            .collect();
        let summary = match outermost.as_slice() {
            [one] if is_folder_id(&document, one) => "Delete folder",
            [_] => "Delete action",
            _ => "Delete actions",
        };
        Ok(self.apply(summary, &edits)?)
    }

    /// Copies the selected actions onto the rows below them, and selects
    /// the copies.
    pub fn duplicate_actions(&mut self) -> Result<Vec<NodeId>, ActionError> {
        if self.actions.is_empty() {
            return Ok(Vec::new());
        }
        let (edits, copies) = build::duplicate(&self.document, &self.actions)?;
        self.apply("Duplicate action", &edits)?;
        let ids: Vec<NodeId> = self
            .actions
            .iter()
            .filter_map(|id| copies.get(id).cloned())
            .collect();
        self.actions.clone_from(&ids);
        Ok(ids)
    }

    /// Puts the selected actions in a new timeline folder. Returns the
    /// folder.
    pub fn group_actions(&mut self) -> Result<NodeId, ActionError> {
        let name = folder_name(&self.document);
        let (folder, edits) = build::group_animations(&self.document, &self.actions, &name)?;
        self.apply("Group actions", &edits)?;
        Ok(folder)
    }

    /// Dissolves the folders the selected actions are in; what was in them
    /// stays selected.
    pub fn ungroup_actions(&mut self) -> Result<bool, ActionError> {
        let (edits, freed) = ungroup_edits(&self.document, &self.actions)?;
        let changed = self.apply("Ungroup actions", &edits)?;
        let kept: Vec<NodeId> = self
            .actions
            .iter()
            .filter(|id| self.document.contains(id))
            .cloned()
            .chain(freed)
            .collect();
        self.set_actions(kept);
        Ok(changed)
    }

    /// Spaces the selected actions out so each starts `delay` milliseconds
    /// after the one before, as one undo step.
    pub fn stagger_actions(&mut self, delay: f64) -> Result<bool, ActionError> {
        let spans = by_start(&self.document, &self.actions);
        let edits = retime_all(&self.document, &staggered(&spans, delay));
        Ok(self.apply("Stagger actions", &edits)?)
    }

    /// Gives actions new times. Inside a gesture this is one more step of
    /// it; otherwise it is an undo step of its own.
    pub fn retime_actions(
        &mut self,
        summary: &str,
        times: &[(NodeId, f64, f64)],
    ) -> Result<bool, ActionError> {
        let edits = retime_all(&self.document, times);
        Ok(self.apply(summary, &edits)?)
    }

    /// Changes the length of `artboard`'s scene, in milliseconds.
    pub fn set_scene_length(
        &mut self,
        artboard: &NodeId,
        duration: f64,
    ) -> Result<bool, EditError> {
        let now = moshionly_core::eval::duration_of(&self.document, artboard);
        if now.is_none_or(|now| now == duration) {
            return Ok(false);
        }
        self.apply(
            "Change scene length",
            &[Edit::set(artboard.clone(), "duration", duration)],
        )
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::model::NodeData;

    use super::super::view::tests::editor;
    use super::*;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    #[test]
    fn the_menu_offers_what_works_on_every_selected_layer() {
        let editor = editor();
        let document = editor.document();
        let rect = menu_for(document, &[id("loose")]);
        assert!(rect.contains(&(MenuSection::In, NodeType::FadeIn)));
        assert!(rect.contains(&(MenuSection::Change, NodeType::CornerRadius)));
        assert!(
            !rect
                .iter()
                .any(|(section, _)| *section == MenuSection::Text)
        );
        // An ellipse has no corners, so a rect and an ellipse together
        // cannot take a corner radius.
        let both = menu_for(document, &[id("loose"), id("dot")]);
        assert!(!both.iter().any(|(_, ty)| *ty == NodeType::CornerRadius));
        assert!(menu_for(document, &[]).is_empty());
        // An artboard can only change its background.
        let artboard = menu_for(document, &[id("ab")]);
        assert_eq!(artboard, [(MenuSection::Change, NodeType::Fill)]);
        // Every animation type except folders is somewhere in the menu.
        for ty in NodeType::ALL {
            let listed = MENU.iter().any(|(_, entry)| entry == ty);
            let action = ty.family() == Family::Animation && *ty != NodeType::AnimationGroup;
            assert_eq!(listed, action, "{ty}");
        }
    }

    #[test]
    fn text_layers_also_offer_text_actions() {
        let document = moshionly_core::io::parse(
            r#"{"format":"moshionly","version":1,"name":"T","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard"},
            {"id":"t","parent":"ab","index":"a0","type":"text","content":"Hi 42"}]}"#,
        )
        .unwrap();
        let menu = menu_for(&document, &[id("t")]);
        // First, as they are what only text can do.
        assert_eq!(
            menu[..3],
            [
                (MenuSection::Text, NodeType::TextIn),
                (MenuSection::Text, NodeType::TextOut),
                (MenuSection::Text, NodeType::Counter)
            ]
        );
        assert_eq!(menu[3], (MenuSection::In, NodeType::FadeIn));
    }

    #[test]
    fn a_new_action_starts_at_the_playhead() {
        assert_eq!(new_start(1234.4, 500.0, 4000.0), 1234.0);
        assert_eq!(new_start(-20.0, 500.0, 4000.0), 0.0);
        assert_eq!(new_start(f64::NAN, 500.0, 4000.0), 0.0);
        // At the end of the scene it ends there instead, to be seen.
        assert_eq!(new_start(4000.0, 500.0, 4000.0), 3500.0);
        assert_eq!(new_start(9000.0, 500.0, 4000.0), 3500.0);
        // A scene shorter than the action: it starts at zero.
        assert_eq!(new_start(300.0, 500.0, 300.0), 0.0);
    }

    #[test]
    fn every_action_type_is_added_with_its_defaults() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        editor.set_playhead(250.0);
        for (_, ty) in MENU {
            if !animation_applies_to(ty, NodeType::Rect) {
                continue;
            }
            let ids = editor.add_actions(ty, 0.0).unwrap();
            assert_eq!(ids.len(), 1);
            let node = editor.document().get(&ids[0]).unwrap();
            assert_eq!(node.node_type(), ty);
            assert_eq!(node.target(), Some(&id("loose")));
            let (start, end) = node.data.span().unwrap();
            assert_eq!(start, 250.0, "{ty}");
            assert_eq!(end - start, build::default_duration(ty).max(0.0), "{ty}");
            if let Some(easing) = node.data.easing() {
                assert_eq!(*easing, build::default_easing(ty), "{ty}");
            }
            assert_eq!(editor.actions(), ids.as_slice());
            assert_eq!(
                editor.undo_label(),
                Some(format!("Undo Add {}", type_label(ty)))
            );
        }
        // A slide comes up from 100 pixels below, fading.
        let slide = editor.add_actions(NodeType::SlideIn, 0.0).unwrap();
        match &editor.document().get(&slide[0]).unwrap().data {
            NodeData::SlideIn(preset) => {
                assert_eq!(preset.distance, 100.0);
                assert!(preset.fade);
            }
            other => panic!("not a slide: {other:?}"),
        }
    }

    #[test]
    fn a_text_action_on_a_rect_is_refused_in_words() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let error = editor.add_actions(NodeType::TextIn, 0.0).unwrap_err();
        assert_eq!(error.to_string(), "Text in does not work on Rectangle");
        editor.selection.clear();
        assert!(matches!(
            editor.add_actions(NodeType::FadeIn, 0.0),
            Err(ActionError::NothingSelected)
        ));
    }

    #[test]
    fn several_layers_get_one_action_each_staggered_down_the_list() {
        let mut editor = editor();
        // Selected back to front; the list shows the front layer first.
        editor
            .selection
            .set([id("card"), id("turned"), id("loose")]);
        editor.set_playhead(100.0);
        let ids = editor.add_actions(NodeType::FadeIn, 150.0).unwrap();
        let document = editor.document();
        let starts: Vec<(NodeId, f64)> = ids
            .iter()
            .map(|action| {
                let node = document.get(action).unwrap();
                (node.target().unwrap().clone(), node.data.span().unwrap().0)
            })
            .collect();
        assert_eq!(
            starts,
            [
                (id("loose"), 100.0),
                (id("turned"), 250.0),
                (id("card"), 400.0)
            ]
        );
        // One undo takes them all away.
        editor.undo().unwrap();
        assert!(ids.iter().all(|action| !editor.document().contains(action)));
    }

    #[test]
    fn staggering_spaces_actions_evenly_and_keeps_their_lengths() {
        let spans = vec![
            (id("a"), 200.0, 700.0),
            (id("b"), 200.0, 400.0),
            (id("c"), 900.0, 1000.0),
        ];
        assert_eq!(
            staggered(&spans, 120.0),
            [
                (id("a"), 200.0, 700.0),
                (id("b"), 320.0, 520.0),
                (id("c"), 440.0, 540.0)
            ]
        );
        assert!(staggered(&[], 100.0).is_empty());
        // A negative delay counts as none.
        assert_eq!(staggered(&spans, -5.0)[2], (id("c"), 200.0, 300.0));
    }

    #[test]
    fn stagger_works_on_the_selected_actions_in_time_order() {
        let mut editor = editor();
        editor.selection.set([id("loose"), id("turned")]);
        editor.set_playhead(0.0);
        let ids = editor.add_actions(NodeType::FadeIn, 0.0).unwrap();
        editor.set_actions(ids.clone());
        assert!(editor.stagger_actions(300.0).unwrap());
        let span = |editor: &Editor, n: usize| editor.document().get(&ids[n]).unwrap().data.span();
        assert_eq!(span(&editor, 0), Some((0.0, 500.0)));
        assert_eq!(span(&editor, 1), Some((300.0, 800.0)));
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Stagger actions"));
    }

    #[test]
    fn retiming_writes_only_what_changes() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let fade = editor.add_actions(NodeType::FadeIn, 0.0).unwrap()[0].clone();
        let show = editor.add_actions(NodeType::Show, 0.0).unwrap()[0].clone();
        let document = editor.document();
        assert!(retime(document, &fade, 0.0, 500.0).is_none());
        assert_eq!(
            retime(document, &fade, 100.0, 650.0),
            Some(Edit::update(
                fade.clone(),
                [("start", json!(100.0)), ("end", json!(650.0))]
            ))
        );
        // An instant only has a start.
        assert_eq!(
            retime(document, &show, 300.0, 900.0),
            Some(Edit::set(show.clone(), "start", 300.0))
        );
        assert!(retime(document, &id("loose"), 0.0, 1.0).is_none());
        // Times are never negative or inside out.
        assert_eq!(
            retime(document, &fade, -40.0, -60.0),
            Some(Edit::update(
                fade,
                [("start", json!(0.0)), ("end", json!(0.0))]
            ))
        );
    }

    #[test]
    fn actions_are_duplicated_grouped_ungrouped_and_deleted() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let fade = editor.add_actions(NodeType::FadeIn, 0.0).unwrap()[0].clone();
        let copies = editor.duplicate_actions().unwrap();
        assert_eq!(copies.len(), 1);
        assert_ne!(copies[0], fade);
        assert_eq!(editor.actions(), copies.as_slice());

        editor.set_actions([fade.clone(), copies[0].clone()]);
        let folder = editor.group_actions().unwrap();
        let node = editor.document().get(&folder).unwrap();
        assert_eq!(node.name(), "Folder 1");
        assert_eq!(editor.document().parent_of(&fade).unwrap().id, folder);
        let folders = folders_of(editor.document(), std::slice::from_ref(&fade));
        assert_eq!(folders.as_slice(), std::slice::from_ref(&folder));

        editor.set_actions([fade.clone()]);
        assert!(editor.ungroup_actions().unwrap());
        assert!(!editor.document().contains(&folder));
        assert!(editor.actions().contains(&fade));
        assert!(editor.actions().contains(&copies[0]), "what was in it");

        assert!(editor.delete_actions().unwrap());
        assert!(!editor.document().contains(&fade));
        assert!(editor.actions().is_empty());
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Delete actions"));
    }

    /// The editor with two fades on the loose rect, grouped in a folder.
    fn grouped() -> (Editor, NodeId, [NodeId; 2]) {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let fade = editor.add_actions(NodeType::FadeIn, 0.0).unwrap()[0].clone();
        let copy = editor.duplicate_actions().unwrap()[0].clone();
        editor.set_actions([fade.clone(), copy.clone()]);
        let folder = editor.group_actions().unwrap();
        (editor, folder, [fade, copy])
    }

    #[test]
    fn deleting_the_last_actions_of_a_folder_takes_the_folder_too() {
        let (mut editor, folder, [fade, copy]) = grouped();
        // One of two: the folder stays.
        editor.set_actions([fade.clone()]);
        editor.delete_actions().unwrap();
        assert!(editor.document().contains(&folder));
        editor.undo().unwrap();
        // Both: it goes, in the same step.
        editor.set_actions([fade.clone(), copy.clone()]);
        editor.delete_actions().unwrap();
        assert!(!editor.document().contains(&folder));
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Delete actions"));
        editor.undo().unwrap();
        for back in [&folder, &fade, &copy] {
            assert!(editor.document().contains(back), "{back}");
        }
        assert_eq!(editor.document().parent_of(&fade).unwrap().id, folder);
    }

    #[test]
    fn a_selected_folder_is_deleted_and_copied_whole() {
        let (mut editor, folder, [fade, copy]) = grouped();
        editor.select_folder(&folder);
        assert_eq!(
            editor.actions(),
            [folder.clone(), fade.clone(), copy.clone()]
        );
        // Its layer, through its actions.
        editor.selection.clear();
        editor.select_targets();
        assert_eq!(editor.selection.ids(), [id("loose")]);

        let copies = editor.duplicate_actions().unwrap();
        assert_eq!(copies.len(), 3, "the folder and both actions");
        let new_folder = copies[0].clone();
        let node = editor.document().get(&new_folder).unwrap();
        assert_eq!(node.node_type(), NodeType::AnimationGroup);
        assert_eq!(
            leaf_actions(editor.document(), std::slice::from_ref(&new_folder)).len(),
            2
        );
        editor.undo().unwrap();

        editor.select_folder(&folder);
        editor.delete_actions().unwrap();
        for gone in [&folder, &fade, &copy] {
            assert!(!editor.document().contains(gone), "{gone}");
        }
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Delete folder"));
        // Not a folder: nothing is selected.
        editor.undo().unwrap();
        editor.set_actions([]);
        editor.select_folder(&fade);
        assert!(editor.actions().is_empty());
    }

    #[test]
    fn a_folder_stays_selected_only_with_everything_in_it() {
        let (mut editor, folder, [fade, copy]) = grouped();
        editor.select_folder(&folder);
        // Shift-clicking one of its actions away lets go of the folder,
        // so Delete takes only what is still selected.
        editor.set_actions([folder.clone(), copy.clone()]);
        assert_eq!(editor.actions(), std::slice::from_ref(&copy));
        editor.delete_actions().unwrap();
        assert!(editor.document().contains(&fade));
        assert!(editor.document().contains(&folder));
        assert!(!editor.document().contains(&copy));
    }

    #[test]
    fn an_empty_folder_can_be_selected_ungrouped_and_deleted() {
        let mut editor = editor();
        // As a file or an agent might leave one.
        let folder = Node::new(moshionly_core::model::AnimationGroup {
            name: "Empty".to_owned(),
        })
        .with_parent(id("ab"))
        .with_index(moshionly_core::index::keys_between(None, None, 1).unwrap()[0].clone());
        let folder_id = folder.id.clone();
        editor.apply("Add folder", &[Edit::add(folder)]).unwrap();
        editor.select_folder(&folder_id);
        assert_eq!(editor.actions(), std::slice::from_ref(&folder_id));
        assert!(editor.ungroup_actions().unwrap());
        assert!(!editor.document().contains(&folder_id));
        assert!(editor.actions().is_empty());
        editor.undo().unwrap();
        editor.select_folder(&folder_id);
        assert!(editor.delete_actions().unwrap());
        assert!(!editor.document().contains(&folder_id));
    }

    #[test]
    fn emptied_folders_are_found_inside_others() {
        let (mut editor, folder, [fade, copy]) = grouped();
        // Put the folder in another one.
        let (outer, edits) =
            build::group_animations(editor.document(), std::slice::from_ref(&folder), "Outer")
                .unwrap();
        editor.apply("Nest", &edits).unwrap();
        let removed = build::remove(editor.document(), &[fade, copy]);
        let edits = with_emptied_folders(editor.document(), removed).unwrap();
        editor.apply("Delete", &edits).unwrap();
        assert!(!editor.document().contains(&folder));
        assert!(!editor.document().contains(&outer));
    }

    #[test]
    fn the_scene_length_changes_once() {
        let mut editor = editor();
        let artboard = id("ab");
        assert!(editor.set_scene_length(&artboard, 6000.0).unwrap());
        assert!(!editor.set_scene_length(&artboard, 6000.0).unwrap());
        assert_eq!(
            moshionly_core::eval::duration_of(editor.document(), &artboard),
            Some(6000.0)
        );
    }

    #[test]
    fn the_layer_list_order_is_front_first() {
        let editor = editor();
        let ordered = in_list_order(
            editor.document(),
            &[id("card"), id("ab"), id("loose"), id("ghost")],
        );
        assert_eq!(ordered, [id("loose"), id("card"), id("ab"), id("ghost")]);
    }
}

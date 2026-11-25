//! The timeline's rows, made from an artboard's animations.
//!
//! The timeline groups actions under the layer they target, one group per
//! layer, in the order of the layer list (front to back) with the artboard
//! last. Within a group, actions keep their timeline row order. Folders
//! (`animation_group`) come first, as groups of their own: their actions
//! are listed in them, named with the layer each animates.

use std::collections::{HashMap, HashSet};

use moshionly_core::{Document, Family, Node, NodeId, NodeType};

use super::describe::{ActionFamily, action_family, display_name, type_label};
use crate::{
    key::Key,
    theme::{Tone, icons},
    timeline::{Bar, Group},
};

/// The icon of a layer type.
pub fn layer_icon(ty: NodeType) -> &'static str {
    match ty {
        NodeType::Artboard => icons::FRAME_CORNERS,
        NodeType::Group => icons::FOLDER_SIMPLE,
        NodeType::MaskGroup => icons::INTERSECT,
        NodeType::Rect => icons::RECTANGLE,
        NodeType::Ellipse => icons::CIRCLE,
        NodeType::Star => icons::STAR,
        NodeType::Path => icons::PEN_NIB,
        NodeType::Text => icons::TEXT_T,
        NodeType::Image => icons::IMAGE,
        NodeType::Component => icons::DIAMONDS_FOUR,
        NodeType::Instance => icons::DIAMOND,
        _ => action_icon(ty),
    }
}

/// The colour family a layer type is marked with in lists, if any: main
/// components and their instances are violet, as in Figma.
pub fn layer_tone(ty: NodeType) -> Option<Tone> {
    matches!(ty, NodeType::Component | NodeType::Instance).then_some(Tone::Violet)
}

/// The icon of an animation type.
pub fn action_icon(ty: NodeType) -> &'static str {
    use NodeType as T;
    match ty {
        T::FadeIn | T::FadeOut | T::Opacity => icons::DROP_HALF,
        T::SlideIn => icons::ARROW_LINE_RIGHT,
        T::SlideOut => icons::ARROW_LINE_LEFT,
        T::GrowIn | T::ShrinkOut => icons::ARROWS_OUT_SIMPLE,
        T::GrowOut | T::ShrinkIn => icons::ARROWS_IN_SIMPLE,
        T::SpinIn | T::SpinOut | T::Rotate => icons::ARROW_CLOCKWISE,
        T::BlurIn | T::BlurOut | T::Blur => icons::DROP,
        T::RevealIn | T::RevealOut => icons::SELECTION,
        T::Move => icons::ARROWS_OUT_CARDINAL,
        T::Scale => icons::RESIZE,
        T::Resize => icons::CORNERS_OUT,
        T::Fill => icons::PAINT_BUCKET,
        T::Stroke => icons::LINE_SEGMENT,
        T::CornerRadius => icons::CORNERS_OUT,
        T::Shadow => icons::SQUARE,
        T::Show => icons::EYE,
        T::Hide => icons::EYE_SLASH,
        T::TextIn | T::TextOut => icons::TEXT_AA,
        T::Counter => icons::HASH,
        T::AnimationGroup => icons::FOLDER_SIMPLE,
        _ => icons::LIGHTNING,
    }
}

/// The colour family of an animation type: teal brings in, rose takes away,
/// the accent changes a property, amber works on text.
pub fn action_tone(ty: NodeType) -> Tone {
    match action_family(ty) {
        Some(ActionFamily::Entrance) => Tone::Teal,
        Some(ActionFamily::Exit) => Tone::Rose,
        Some(ActionFamily::Change) => Tone::Accent,
        Some(ActionFamily::Text) => Tone::Amber,
        Some(ActionFamily::Instant) | None => Tone::Neutral,
    }
}

/// The bar of an action, named after it (and, with `with_layer`, after the
/// layer it animates too), with the layer it targets.
fn bar_of(document: &Document, node: &Node, with_layer: bool) -> Option<(NodeId, Bar)> {
    let (Some(target), Some((start, end))) = (node.target(), node.data.span()) else {
        return None;
    };
    let ty = node.node_type();
    let mut label = match node.name().trim() {
        "" => type_label(ty).to_owned(),
        name => name.to_owned(),
    };
    if with_layer && let Some(layer) = document.get(target) {
        label = format!("{} · {label}", display_name(layer));
    }
    let bar = Bar {
        key: Key::new(node.id.as_str()),
        label,
        icon: Some(action_icon(ty)),
        tone: action_tone(ty),
        start,
        end: end.max(start),
        instant: action_family(ty) == Some(ActionFamily::Instant),
    };
    Some((target.clone(), bar))
}

/// The timeline groups of `artboard`: first its folders, each holding the
/// actions put in it, then one group per layer that has other actions.
/// `collapsed` names the groups whose rows are folded away.
pub fn timeline_groups(
    document: &Document,
    artboard: &NodeId,
    collapsed: &HashSet<NodeId>,
) -> Vec<Group> {
    // Actions by target, in timeline row order, and the folders.
    let mut bars: HashMap<NodeId, Vec<Bar>> = HashMap::new();
    let mut folders: Vec<Group> = Vec::new();
    for node in document.child_nodes(artboard, Family::Animation) {
        if node.node_type() == NodeType::AnimationGroup {
            let inside = document
                .walk(&node.id, Family::Animation)
                .filter_map(|(_, inner)| bar_of(document, inner, true))
                .map(|(_, bar)| bar)
                .collect();
            folders.push(Group {
                key: Key::new(node.id.as_str()),
                label: display_name(node),
                icon: Some(icons::FOLDER_SIMPLE),
                collapsed: collapsed.contains(&node.id),
                hidden: false,
                locked: false,
                folder: true,
                bars: inside,
            });
        } else if let Some((target, bar)) = bar_of(document, node, false) {
            bars.entry(target).or_default().push(bar);
        }
    }
    // Layers front to back, as the layer list reads, then the artboard.
    let mut layers: Vec<NodeId> = document
        .walk(artboard, Family::Layer)
        .map(|(_, node)| node.id.clone())
        .collect();
    layers.reverse();
    layers.push(artboard.clone());
    let layer_groups = layers.into_iter().filter_map(|id| {
        let bars = bars.remove(&id)?;
        let node = document.get(&id)?;
        let layer = node.layer();
        Some(Group {
            key: Key::new(id.as_str()),
            label: display_name(node),
            icon: Some(layer_icon(node.node_type())),
            collapsed: collapsed.contains(&id),
            hidden: layer.is_some_and(|layer| layer.hidden),
            locked: layer.is_some_and(|layer| layer.locked),
            folder: false,
            bars,
        })
    });
    folders.into_iter().chain(layer_groups).collect()
}

/// Whether a timeline group is a folder rather than a layer.
pub fn is_folder(document: &Document, key: &Key) -> bool {
    document
        .get(&NodeId::from(key.as_str()))
        .is_some_and(|node| node.node_type() == NodeType::AnimationGroup)
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","name":"Scene"},
            {"id":"back","parent":"ab","index":"a0","type":"rect","name":"Back"},
            {"id":"front","parent":"ab","index":"a1","type":"ellipse","name":"Front","hidden":true},
            {"id":"quiet","parent":"ab","index":"a2","type":"star"},
            {"id":"f1","parent":"ab","index":"a0","type":"fade_in","target":"back","start":0,"end":500},
            {"id":"m1","parent":"ab","index":"a1","type":"move","name":"Drift","target":"front","start":200,"end":900,"to":{"x":10,"y":0}},
            {"id":"folder","parent":"ab","index":"a2","type":"animation_group","name":"Outro"},
            {"id":"o1","parent":"folder","index":"a0","type":"fade_out","target":"back","start":3000,"end":3500},
            {"id":"bg","parent":"ab","index":"a3","type":"fill","target":"ab","start":0,"end":1000,"to":"#000000"},
            {"id":"off","parent":"ab","index":"a4","type":"hide","target":"back","start":2000}
            ]}"##,
        )
        .unwrap()
    }

    #[test]
    fn actions_group_under_their_layers_front_to_back() {
        let doc = document();
        let groups = timeline_groups(&doc, &NodeId::from("ab"), &HashSet::new());
        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["Outro", "Front", "Back", "Scene"]);
        // The folder holds its action, named with its layer.
        let outro = &groups[0];
        assert!(outro.folder);
        assert_eq!(outro.bars[0].label, "Back · Fade out");
        assert_eq!(outro.bars[0].tone, Tone::Rose);
        assert_eq!((outro.bars[0].start, outro.bars[0].end), (3000.0, 3500.0));
        let back: Vec<&str> = groups[2].bars.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(back, ["Fade in", "Hide"]);
        // A hide is a marker with no length; a fade has ends.
        let hide = &groups[2].bars[1];
        assert!(hide.instant);
        assert_eq!((hide.start, hide.end), (2000.0, 2000.0));
        assert!(!groups[2].bars[0].instant);
        assert_eq!(groups[1].bars[0].label, "Drift");
        assert_eq!(groups[1].bars[0].tone, Tone::Accent);
        assert_eq!(groups[2].bars[0].tone, Tone::Teal);
        assert!(groups[1].hidden && !groups[1].folder);
        assert!(is_folder(&doc, &groups[0].key));
        assert!(!is_folder(&doc, &groups[1].key));
    }

    #[test]
    fn collapsed_groups_are_marked() {
        let doc = document();
        let collapsed = HashSet::from([NodeId::from("back")]);
        let groups = timeline_groups(&doc, &NodeId::from("ab"), &collapsed);
        assert!(groups[2].collapsed);
        assert!(!groups[1].collapsed);
        assert!(timeline_groups(&doc, &NodeId::from("missing"), &collapsed).is_empty());
    }

    #[test]
    fn every_type_has_an_icon() {
        for ty in NodeType::ALL {
            assert!(!layer_icon(*ty).is_empty());
        }
        assert_eq!(action_tone(NodeType::Show), Tone::Neutral);
        assert_eq!(layer_tone(NodeType::Component), Some(Tone::Violet));
        assert_eq!(layer_tone(NodeType::Instance), Some(Tone::Violet));
        assert_eq!(layer_tone(NodeType::Group), None);
        assert_ne!(
            layer_icon(NodeType::Component),
            layer_icon(NodeType::Instance)
        );
        assert_eq!(action_tone(NodeType::TextIn), Tone::Amber);
    }
}

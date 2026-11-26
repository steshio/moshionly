//! The timeline's rows, made from an artboard's animations.
//!
//! The timeline groups actions under the layer they target, one group per
//! layer, in the order of the layer list (front to back) with the artboard
//! last. Within a group, actions keep their timeline row order. Folders
//! (`animation_group`) come first, as groups of their own: their actions
//! are listed in them, named with the layer each animates.
//!
//! An instance plays the animations of the layers inside its main
//! component, which live on the main's own timeline. Its group starts with
//! one bar that cannot be edited, saying which component it plays and
//! when.
//!
//! Video and sound are clips (see [`clips`](super::clips)): a video
//! layer's group starts with the bar of its clip, and the scene's audio
//! items have a group of their own at the bottom, "Sound", one row each.

use std::collections::{HashMap, HashSet};

use moshionly_core::model::NodeData;
use moshionly_core::{Document, Family, Node, NodeId, NodeType};

use super::clips::clip_key;
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
        NodeType::Video => icons::VIDEO_CAMERA,
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
        T::Glow => icons::SUN_DIM,
        T::Noise => icons::DOTS_NINE,
        T::DirectionalBlur => icons::WIND,
        T::Show => icons::EYE,
        T::Hide => icons::EYE_SLASH,
        T::TextIn | T::TextOut => icons::TEXT_AA,
        T::Counter => icons::HASH,
        T::AnimationGroup => icons::FOLDER_SIMPLE,
        T::Audio => icons::SPEAKER_HIGH,
        _ => icons::LIGHTNING,
    }
}

/// The colour family of an animation type: teal brings in, rose takes away,
/// the accent changes a property, amber works on text, and sound is sky
/// blue like video clips.
pub fn action_tone(ty: NodeType) -> Tone {
    if ty == NodeType::Audio {
        return Tone::Sky;
    }
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
        fixed: false,
    };
    Some((target.clone(), bar))
}

/// The key of the timeline group that holds the scene's sound. Ids never
/// hold a `/`, so it is no node's.
pub const SOUND_GROUP: &str = "/sound";

/// The name a media file is shown with: the last part of its source.
pub fn file_label(source: &str) -> String {
    let name = source.trim().rsplit(['/', '\\']).next().unwrap_or_default();
    if name.is_empty() {
        source.trim().to_owned()
    } else {
        name.to_owned()
    }
}

/// What an audio item is called: its name, or else its file's.
pub fn sound_name(node: &Node) -> String {
    match (node.name().trim(), node.data.source()) {
        ("", Some(source)) => file_label(source),
        ("", None) => type_label(node.node_type()).to_owned(),
        (name, _) => name.to_owned(),
    }
}

/// The bar of the clip of a video layer or an audio item: the stretch of
/// the timeline it plays, named after its file (or the audio item).
pub fn clip_bar(node: &Node) -> Option<Bar> {
    let clip = node.data.clip()?;
    let ty = node.node_type();
    let (label, icon) = match ty {
        NodeType::Video => (file_label(node.data.source()?), icons::FILM_STRIP),
        _ => (sound_name(node), icons::SPEAKER_HIGH),
    };
    Some(Bar {
        key: clip_key(&node.id, ty),
        label,
        icon: Some(icon),
        tone: Tone::Sky,
        start: clip.start,
        end: clip.end.max(clip.start),
        instant: false,
        fixed: false,
    })
}

/// How deep instances inside components are followed to find what an
/// instance plays. The rules forbid a component that shows itself; this
/// only guards against a broken document.
const MAX_DEPTH: usize = 16;

/// When the animations a main component's instances play run, on the
/// main's own timeline: from the earliest start to the latest end of the
/// animations of the layers inside it, and of what instances inside it
/// play. `None` when nothing inside it is animated.
pub fn main_span(document: &Document, main: &NodeId) -> Option<(f64, f64)> {
    span_within(document, main, 0)
}

fn span_within(document: &Document, main: &NodeId, depth: usize) -> Option<(f64, f64)> {
    if depth > MAX_DEPTH {
        return None;
    }
    let artboard = document.artboard_of(main)?;
    let inside: HashSet<&NodeId> = document
        .walk(main, Family::Layer)
        .map(|(_, node)| &node.id)
        .collect();
    let own = document
        .walk(&artboard.id, Family::Animation)
        .filter(|(_, node)| node.target().is_some_and(|target| inside.contains(target)))
        .filter_map(|(_, node)| node.data.span());
    let nested = document
        .walk(main, Family::Layer)
        .filter_map(|(_, node)| match &node.data {
            NodeData::Instance(instance) => {
                let (start, end) = span_within(document, &instance.component, depth + 1)?;
                Some((start + instance.time_offset, end + instance.time_offset))
            }
            _ => None,
        });
    own.chain(nested)
        .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

/// The label of the bar of an instance: "Plays Card · 300 ms later".
pub fn plays_label(main: &str, time_offset: f64) -> String {
    let offset = time_offset.round();
    if offset > 0.0 {
        format!("Plays {main} · {offset:.0} ms later")
    } else if offset < 0.0 {
        format!("Plays {main} · {:.0} ms earlier", -offset)
    } else {
        format!("Plays {main}")
    }
}

/// The bar that says which main component an instance plays and when, or
/// `None` when it plays no animation within the scene. `spans` remembers
/// [`main_span`] for each main, as many instances often show one.
fn plays_bar(
    document: &Document,
    node: &Node,
    spans: &mut HashMap<NodeId, Option<(f64, f64)>>,
) -> Option<Bar> {
    let NodeData::Instance(instance) = &node.data else {
        return None;
    };
    let main = document.get(&instance.component)?;
    let (start, end) = (*spans
        .entry(main.id.clone())
        .or_insert_with(|| main_span(document, &main.id)))?;
    let (start, end) = (start + instance.time_offset, end + instance.time_offset);
    if end <= 0.0 && start < 0.0 {
        // All played before the scene starts.
        return None;
    }
    let start = start.max(0.0);
    Some(Bar {
        key: Key::new(format!("{}/plays", node.id)),
        label: plays_label(&display_name(main), instance.time_offset),
        icon: Some(icons::DIAMONDS_FOUR),
        tone: Tone::Violet,
        start,
        end: end.max(start),
        instant: false,
        fixed: true,
    })
}

/// The timeline groups of `artboard`: first its folders, each holding the
/// actions put in it, then one group per layer that has other actions or
/// is a video, then the scene's sound. `collapsed` names the groups whose
/// rows are folded away.
pub fn timeline_groups(
    document: &Document,
    artboard: &NodeId,
    collapsed: &HashSet<NodeId>,
) -> Vec<Group> {
    // Actions by target, in timeline row order, and the folders.
    let mut bars: HashMap<NodeId, Vec<Bar>> = HashMap::new();
    let mut folders: Vec<Group> = Vec::new();
    let mut sound: Vec<Bar> = Vec::new();
    for node in document.child_nodes(artboard, Family::Animation) {
        if node.node_type() == NodeType::AnimationGroup {
            let inside = document
                .walk(&node.id, Family::Animation)
                .filter_map(|(_, inner)| {
                    bar_of(document, inner, true)
                        .map(|(_, bar)| bar)
                        .or_else(|| clip_bar(inner))
                })
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
        } else if let Some(bar) = clip_bar(node) {
            sound.push(bar);
        }
    }
    // Instances first say what they play, and videos when they play.
    let mut spans = HashMap::new();
    for (_, node) in document.walk(artboard, Family::Layer) {
        let first = match node.node_type() {
            NodeType::Video => clip_bar(node),
            _ => plays_bar(document, node, &mut spans),
        };
        if let Some(bar) = first {
            bars.entry(node.id.clone()).or_default().insert(0, bar);
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
    let sound_group = (!sound.is_empty()).then(|| Group {
        key: Key::new(SOUND_GROUP),
        label: "Sound".to_owned(),
        icon: Some(icons::SPEAKER_HIGH),
        collapsed: collapsed.contains(&NodeId::from(SOUND_GROUP)),
        hidden: false,
        locked: false,
        // Not a layer: no hide and lock toggles.
        folder: true,
        bars: sound,
    });
    folders
        .into_iter()
        .chain(layer_groups)
        .chain(sound_group)
        .collect()
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
    fn an_instance_says_what_it_plays_and_when() {
        let doc = io::parse(
            r##"{"format":"moshionly","version":2,"name":"Test","nodes":[
            {"id":"home","parent":null,"index":"a0","type":"artboard","name":"Home"},
            {"id":"card","parent":"home","index":"a0","type":"component","name":"Card"},
            {"id":"dot","parent":"card","index":"a0","type":"ellipse"},
            {"id":"still","parent":"home","index":"a1","type":"component","name":"Still"},
            {"id":"pop","parent":"home","index":"a0","type":"grow_in","target":"dot","start":200,"end":700},
            {"id":"own","parent":"home","index":"a1","type":"fade_in","target":"card","start":0,"end":2000},
            {"id":"away","parent":null,"index":"a1","type":"artboard","name":"Away","x":2000},
            {"id":"late","parent":"away","index":"a0","type":"instance","component":"card","name":"Late","time_offset":300},
            {"id":"early","parent":"away","index":"a1","type":"instance","component":"card","name":"Early","time_offset":-1000},
            {"id":"calm","parent":"away","index":"a2","type":"instance","component":"still","name":"Calm"},
            {"id":"fade","parent":"away","index":"a0","type":"fade_in","target":"late","start":0,"end":100}
            ]}"##,
        )
        .unwrap();
        // The animation of the card itself is not part of what it plays.
        assert_eq!(main_span(&doc, &NodeId::from("card")), Some((200.0, 700.0)));
        assert_eq!(main_span(&doc, &NodeId::from("still")), None);
        let groups = timeline_groups(&doc, &NodeId::from("away"), &HashSet::new());
        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        // Played before the scene starts, or nothing to play: no bar.
        assert_eq!(labels, ["Late"]);
        let bars = &groups[0].bars;
        assert_eq!(bars.len(), 2);
        assert!(bars[0].fixed && !bars[1].fixed);
        assert_eq!(bars[0].label, "Plays Card · 300 ms later");
        assert_eq!(bars[0].tone, Tone::Violet);
        assert_eq!((bars[0].start, bars[0].end), (500.0, 1000.0));
        assert_eq!(plays_label("Card", -250.0), "Plays Card · 250 ms earlier");
        assert_eq!(plays_label("Card", 0.0), "Plays Card");
    }

    #[test]
    fn videos_and_sound_show_their_clips() {
        let doc = io::parse(
            r##"{"format":"moshionly","version":3,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","name":"Scene"},
            {"id":"film","parent":"ab","index":"a0","type":"video","source":"assets/beach.mp4",
             "start":500,"end":2500},
            {"id":"still","parent":"ab","index":"a1","type":"rect"},
            {"id":"fade","parent":"ab","index":"a0","type":"fade_in","target":"film","start":0,"end":300},
            {"id":"song","parent":"ab","index":"a1","type":"audio","source":"/music/Song.mp3",
             "start":0,"end":4000},
            {"id":"voice","parent":"ab","index":"a2","type":"audio","name":"Voice over",
             "source":"assets/voice.wav","start":1000,"end":2000},
            {"id":"folder","parent":"ab","index":"a3","type":"animation_group","name":"Outro"},
            {"id":"sting","parent":"folder","index":"a0","type":"audio","source":"assets/sting.wav",
             "start":3000,"end":3500}
            ]}"##,
        )
        .unwrap();
        let groups = timeline_groups(&doc, &NodeId::from("ab"), &HashSet::new());
        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["Outro", "Video", "Sound"]);
        // The folder holds its sound.
        assert_eq!(groups[0].bars[0].label, "sting.wav");
        // The video's clip comes before its actions, keyed apart from them.
        let video = &groups[1];
        assert_eq!(video.icon, Some(icons::VIDEO_CAMERA));
        assert_eq!(video.bars.len(), 2);
        let clip = &video.bars[0];
        assert_eq!(clip.key.as_str(), "film/clip");
        assert_eq!(clip.label, "beach.mp4");
        assert_eq!((clip.start, clip.end), (500.0, 2500.0));
        assert_eq!(clip.tone, Tone::Sky);
        assert!(!clip.fixed && !clip.instant);
        assert_eq!(video.bars[1].key.as_str(), "fade");
        // The sound has a group of its own, one row per audio item.
        let sound = &groups[2];
        assert_eq!(sound.key.as_str(), SOUND_GROUP);
        assert!(sound.folder);
        let rows: Vec<(&str, &str)> = sound
            .bars
            .iter()
            .map(|bar| (bar.key.as_str(), bar.label.as_str()))
            .collect();
        assert_eq!(rows, [("song", "Song.mp3"), ("voice", "Voice over")]);
        assert_eq!(action_tone(NodeType::Audio), Tone::Sky);
        assert_eq!(file_label("C:\\clips\\a.mov"), "a.mov");
        let collapsed = HashSet::from([NodeId::from(SOUND_GROUP)]);
        let groups = timeline_groups(&doc, &NodeId::from("ab"), &collapsed);
        assert!(groups[2].collapsed);
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

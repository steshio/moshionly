//! Words for nodes: the names the layer list and the timeline show.

use moshionly_core::model::{AnimationKind, NodeData};
use moshionly_core::{Node, NodeType};

/// The name of a node type in the interface: "Rectangle", "Fade in".
pub fn type_label(ty: NodeType) -> &'static str {
    use NodeType as T;
    match ty {
        T::Artboard => "Artboard",
        T::Group => "Group",
        T::MaskGroup => "Mask group",
        T::Rect => "Rectangle",
        T::Ellipse => "Ellipse",
        T::Star => "Star",
        T::Path => "Path",
        T::Text => "Text",
        T::Image => "Image",
        T::Component => "Component",
        T::Instance => "Instance",
        T::Video => "Video",
        T::Move => "Move",
        T::Rotate => "Rotate",
        T::Scale => "Scale",
        T::Opacity => "Opacity",
        T::Resize => "Resize",
        T::Fill => "Fill",
        T::Stroke => "Stroke",
        T::CornerRadius => "Corner radius",
        T::Blur => "Blur",
        T::Shadow => "Shadow",
        T::Glow => "Glow",
        T::Noise => "Noise",
        T::DirectionalBlur => "Directional blur",
        T::Show => "Show",
        T::Hide => "Hide",
        T::FadeIn => "Fade in",
        T::FadeOut => "Fade out",
        T::SlideIn => "Slide in",
        T::SlideOut => "Slide out",
        T::GrowIn => "Grow in",
        T::GrowOut => "Grow out",
        T::ShrinkIn => "Shrink in",
        T::ShrinkOut => "Shrink out",
        T::SpinIn => "Spin in",
        T::SpinOut => "Spin out",
        T::BlurIn => "Blur in",
        T::BlurOut => "Blur out",
        T::RevealIn => "Reveal in",
        T::RevealOut => "Reveal out",
        T::TextIn => "Text in",
        T::TextOut => "Text out",
        T::Counter => "Counter",
        T::Audio => "Sound",
        T::AnimationGroup => "Folder",
    }
}

/// The longest name made from a text layer's content.
const TEXT_NAME_LENGTH: usize = 40;

/// The name shown for a node: its own name, or for an unnamed text layer
/// the start of its text, or else its type.
pub fn display_name(node: &Node) -> String {
    let name = node.name().trim();
    if !name.is_empty() {
        return name.to_owned();
    }
    if let NodeData::Text(text) = &node.data {
        let line = text.content.lines().next().unwrap_or("").trim();
        if !line.is_empty() {
            let mut short: String = line.chars().take(TEXT_NAME_LENGTH).collect();
            if line.chars().count() > TEXT_NAME_LENGTH {
                short.push('…');
            }
            return short;
        }
    }
    type_label(node.node_type()).to_owned()
}

/// The family of an animation for the timeline's colours: what it does to
/// the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionFamily {
    /// Brings a layer in.
    Entrance,
    /// Takes a layer away.
    Exit,
    /// Changes a property.
    Change,
    /// Animates text.
    Text,
    /// Happens at one moment.
    Instant,
}

/// Which family an animation type belongs to; `None` for layers, folders
/// and sound.
pub fn action_family(ty: NodeType) -> Option<ActionFamily> {
    let kind = ty.animation_kind()?;
    Some(match kind {
        AnimationKind::Instant => ActionFamily::Instant,
        AnimationKind::Text => ActionFamily::Text,
        // Sound and folders change no layer.
        AnimationKind::Media | AnimationKind::Structure => return None,
        _ if ty.is_entrance() => ActionFamily::Entrance,
        _ if ty.is_exit() => ActionFamily::Exit,
        _ => ActionFamily::Change,
    })
}

/// The next free name of the form `"<base> <n>"`: one more than the highest
/// number already used with that base.
pub fn next_name<'a>(base: &str, existing: impl IntoIterator<Item = &'a str>) -> String {
    let highest = existing
        .into_iter()
        .filter_map(|name| {
            name.strip_prefix(base)?
                .strip_prefix(' ')?
                .parse::<u32>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    format!("{base} {}", highest + 1)
}

#[cfg(test)]
mod tests {
    use moshionly_core::model::{LayerProps, RectLayer, TextLayer};

    use super::*;

    #[test]
    fn unnamed_layers_are_named_by_their_type_or_text() {
        let rect = Node::new(RectLayer::default());
        assert_eq!(display_name(&rect), "Rectangle");
        let named = Node::new(RectLayer {
            layer: LayerProps::new("  Card ", 0.0, 0.0, 1.0, 1.0),
            ..RectLayer::default()
        });
        assert_eq!(display_name(&named), "Card");
        let text = Node::new(TextLayer {
            content: "Hello there\nsecond line".to_owned(),
            ..TextLayer::default()
        });
        assert_eq!(display_name(&text), "Hello there");
        let long = Node::new(TextLayer {
            content: "x".repeat(60),
            ..TextLayer::default()
        });
        assert_eq!(display_name(&long).chars().count(), TEXT_NAME_LENGTH + 1);
        let empty = Node::new(TextLayer::default());
        assert_eq!(display_name(&empty), "Text");
    }

    #[test]
    fn animations_fall_into_families() {
        assert_eq!(
            action_family(NodeType::FadeIn),
            Some(ActionFamily::Entrance)
        );
        assert_eq!(action_family(NodeType::SlideOut), Some(ActionFamily::Exit));
        assert_eq!(action_family(NodeType::Move), Some(ActionFamily::Change));
        assert_eq!(action_family(NodeType::TextIn), Some(ActionFamily::Text));
        assert_eq!(action_family(NodeType::Show), Some(ActionFamily::Instant));
        assert_eq!(action_family(NodeType::AnimationGroup), None);
        assert_eq!(action_family(NodeType::Rect), None);
    }

    #[test]
    fn new_names_count_up() {
        assert_eq!(next_name("Rectangle", []), "Rectangle 1");
        assert_eq!(
            next_name(
                "Rectangle",
                ["Rectangle 1", "Rectangle 7", "Rectangle x", "Ellipse 9"]
            ),
            "Rectangle 8"
        );
        assert_eq!(next_name("Scene", ["Scene", "Scene 2"]), "Scene 3");
    }

    #[test]
    fn every_type_has_a_label() {
        for ty in NodeType::ALL {
            assert!(!type_label(*ty).is_empty());
        }
    }
}

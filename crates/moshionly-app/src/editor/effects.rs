//! A layer's effects, as the Design tab's Effects section lists them: a
//! drop shadow, a blur, an outer glow, film grain and a directional blur.
//!
//! Each effect is a property of its own on a layer (artboards have none),
//! absent until it is added. This module names the effects, says which
//! ones the add menu offers for a selection, and writes the edits that add,
//! change and remove one.
//!
//! Edits work on each layer's own value. Changing one part of an effect,
//! such as a glow's radius, merges that part into the effect of every
//! selected layer that has one and keeps the rest of each: two layers with
//! glows of different colours keep their own colours when their radius is
//! set. Adding an effect gives it only to the layers that lack it.

use moshionly_core::model::{DirectionalBlur, Glow, LayerProps, Noise, Shadow};
use moshionly_core::{Edit, Node, NodeId, NodeType};
use serde_json::{Value, json};

use super::inspect::tidy;
use super::timeline::action_icon;

/// The blur a new blur effect starts with, in pixels.
pub const NEW_BLUR: f64 = 4.0;

/// The smallest grain the Effects section sets, in pixels: the file needs
/// more than 0, and finer grain than this cannot be seen.
pub const MIN_GRAIN: f64 = 0.1;

/// One kind of effect a layer can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    /// A drop shadow (`shadow`).
    Shadow,
    /// A blur of the whole layer (`blur`).
    Blur,
    /// An outer glow (`glow`).
    Glow,
    /// Film grain over the layer (`noise`).
    Noise,
    /// A blur along one direction (`directional_blur`).
    DirectionalBlur,
}

impl Effect {
    /// Every effect, in the order the Effects section and its add menu
    /// list them.
    pub const ALL: [Effect; 5] = [
        Effect::Shadow,
        Effect::Blur,
        Effect::Glow,
        Effect::Noise,
        Effect::DirectionalBlur,
    ];

    /// The effect's name, as its row and the add menu show it.
    pub fn label(self) -> &'static str {
        match self {
            Effect::Shadow => "Shadow",
            Effect::Blur => "Blur",
            Effect::Glow => "Glow",
            Effect::Noise => "Noise",
            Effect::DirectionalBlur => "Directional blur",
        }
    }

    /// The name in the middle of a sentence: "Add directional blur".
    pub fn lowercase(self) -> String {
        self.label().to_lowercase()
    }

    /// The layer property the effect is kept in.
    pub fn field(self) -> &'static str {
        match self {
            Effect::Shadow => "shadow",
            Effect::Blur => "blur",
            Effect::Glow => "glow",
            Effect::Noise => "noise",
            Effect::DirectionalBlur => "directional_blur",
        }
    }

    /// The action type that animates the effect.
    pub fn action(self) -> NodeType {
        match self {
            Effect::Shadow => NodeType::Shadow,
            Effect::Blur => NodeType::Blur,
            Effect::Glow => NodeType::Glow,
            Effect::Noise => NodeType::Noise,
            Effect::DirectionalBlur => NodeType::DirectionalBlur,
        }
    }

    /// The effect's icon: the same as the action that animates it.
    pub fn icon(self) -> &'static str {
        action_icon(self.action())
    }

    /// Whether `layer` has the effect. A blur of 0 is no blur.
    pub fn is_on(self, layer: &LayerProps) -> bool {
        match self {
            Effect::Shadow => layer.shadow.is_some(),
            Effect::Blur => layer.blur > 0.0,
            Effect::Glow => layer.glow.is_some(),
            Effect::Noise => layer.noise.is_some(),
            Effect::DirectionalBlur => layer.directional_blur.is_some(),
        }
    }

    /// The value a newly added effect starts with: the file's default.
    fn new_value(self) -> Value {
        let value = match self {
            Effect::Shadow => serde_json::to_value(Shadow::default()),
            Effect::Blur => return json!(NEW_BLUR),
            Effect::Glow => serde_json::to_value(Glow::default()),
            Effect::Noise => serde_json::to_value(Noise::default()),
            Effect::DirectionalBlur => serde_json::to_value(DirectionalBlur::default()),
        };
        value.unwrap_or_else(|_| json!({}))
    }

    /// The value with the effect taken away.
    fn none(self) -> Value {
        match self {
            Effect::Blur => json!(0.0),
            _ => Value::Null,
        }
    }
}

/// The layers among `nodes` that can have effects, with their properties:
/// every layer but an artboard.
pub fn layers<'a>(nodes: &[&'a Node]) -> Vec<(&'a NodeId, &'a LayerProps)> {
    nodes
        .iter()
        .filter(|node| node.node_type() != NodeType::Artboard)
        .filter_map(|node| Some((&node.id, node.layer()?)))
        .collect()
}

/// The effects at least one of `layers` has, in list order: the rows the
/// Effects section shows.
pub fn present(layers: &[(&NodeId, &LayerProps)]) -> Vec<Effect> {
    Effect::ALL
        .into_iter()
        .filter(|effect| layers.iter().any(|(_, layer)| effect.is_on(layer)))
        .collect()
}

/// The effects the add menu offers: each one that at least one of
/// `layers` does not have yet, in list order. Empty without layers.
pub fn addable(layers: &[(&NodeId, &LayerProps)]) -> Vec<Effect> {
    Effect::ALL
        .into_iter()
        .filter(|effect| layers.iter().any(|(_, layer)| !effect.is_on(layer)))
        .collect()
}

/// The ids of the layers that have `effect`.
pub fn with(effect: Effect, layers: &[(&NodeId, &LayerProps)]) -> Vec<NodeId> {
    layers
        .iter()
        .filter(|(_, layer)| effect.is_on(layer))
        .map(|(id, _)| (*id).clone())
        .collect()
}

/// Edits that give `effect`, at its defaults, to each of `layers` that
/// does not have it. The layers that have it keep theirs.
pub fn add(effect: Effect, layers: &[(&NodeId, &LayerProps)]) -> Vec<Edit> {
    let value = effect.new_value();
    layers
        .iter()
        .filter(|(_, layer)| !effect.is_on(layer))
        .map(|(id, _)| Edit::set((*id).clone(), effect.field(), value.clone()))
        .collect()
}

/// Edits that take `effect` away from each of `layers` that has it.
pub fn remove(effect: Effect, layers: &[(&NodeId, &LayerProps)]) -> Vec<Edit> {
    with(effect, layers)
        .into_iter()
        .map(|id| Edit::set(id, effect.field(), effect.none()))
        .collect()
}

/// Edits that set the part `part` of `effect` to `value` on each of
/// `layers` that has the effect. Each layer keeps its other parts: the
/// part is merged into its own value. The blur is one number, so for it
/// `part` is `None` and `value` the whole blur.
pub fn set_part(
    effect: Effect,
    layers: &[(&NodeId, &LayerProps)],
    part: Option<&str>,
    value: Value,
) -> Vec<Edit> {
    let value = match part {
        Some(part) => json!({ part: value }),
        None => value,
    };
    with(effect, layers)
        .into_iter()
        .map(|id| Edit::set(id, effect.field(), value.clone()))
        .collect()
}

/// A part's value as the file keeps it, from the number its field shows:
/// within the part's range and with the noise of arithmetic rounded away.
/// Noise's amount is shown in percent and kept as a fraction.
pub fn stored_part(effect: Effect, part: Option<&str>, shown: f64) -> f64 {
    match (effect, part) {
        (Effect::Noise, Some("amount")) => (shown.clamp(0.0, 100.0) * 10.0).round() / 1000.0,
        (Effect::Noise, Some("size")) => tidy(shown.max(MIN_GRAIN)),
        // Offsets and angles may be negative.
        (Effect::Shadow, Some("x" | "y")) | (Effect::DirectionalBlur, Some("angle")) => tidy(shown),
        // Blurs, radii, spreads and amounts are 0 or more.
        _ => tidy(shown.max(0.0)),
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::{Color, Document, io};

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":4,"name":"T","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard"},
            {"id":"plain","parent":"ab","index":"a0","type":"rect"},
            {"id":"lit","parent":"ab","index":"a1","type":"rect","shadow":{"y":8},
             "glow":{"color":"#FF0000","radius":12,"spread":2}},
            {"id":"warm","parent":"ab","index":"a2","type":"ellipse",
             "glow":{"color":"#FFAA00","radius":30}},
            {"id":"all","parent":"ab","index":"a3","type":"text","content":"Hi","blur":3,
             "shadow":{},"glow":{},"noise":{"amount":0.4,"animated":false},
             "directional_blur":{"amount":8,"angle":90}}
            ]}"##,
        )
        .unwrap()
    }

    fn layers_of<'a>(document: &'a Document, ids: &[&str]) -> Vec<(&'a NodeId, &'a LayerProps)> {
        let nodes: Vec<&Node> = ids
            .iter()
            .map(|id| document.get(&NodeId::from(*id)).unwrap())
            .collect();
        layers(&nodes)
    }

    #[test]
    fn the_menu_offers_what_some_layer_lacks() {
        let document = document();
        assert_eq!(
            addable(&layers_of(&document, &["plain"])),
            Effect::ALL,
            "a plain rect can take every effect"
        );
        assert_eq!(
            addable(&layers_of(&document, &["lit"])),
            [Effect::Blur, Effect::Noise, Effect::DirectionalBlur]
        );
        assert!(addable(&layers_of(&document, &["all"])).is_empty());
        // One layer that lacks an effect is enough for it to be offered.
        assert_eq!(
            addable(&layers_of(&document, &["all", "warm"])),
            [
                Effect::Shadow,
                Effect::Blur,
                Effect::Noise,
                Effect::DirectionalBlur
            ]
        );
        // Artboards take no effects: nothing is offered for one.
        assert!(layers_of(&document, &["ab"]).is_empty());
        assert!(addable(&layers_of(&document, &["ab"])).is_empty());
        assert_eq!(layers_of(&document, &["ab", "plain"]).len(), 1);
    }

    #[test]
    fn the_section_lists_what_some_layer_has() {
        let document = document();
        assert!(present(&layers_of(&document, &["plain"])).is_empty());
        assert_eq!(
            present(&layers_of(&document, &["lit", "warm"])),
            [Effect::Shadow, Effect::Glow]
        );
        assert_eq!(present(&layers_of(&document, &["all"])), Effect::ALL);
    }

    #[test]
    fn effects_have_names_fields_and_actions() {
        for effect in Effect::ALL {
            assert!(!effect.label().is_empty());
            assert!(!effect.icon().is_empty());
            assert_eq!(effect.action().as_str(), effect.field(), "{effect:?}");
        }
        assert_eq!(Effect::DirectionalBlur.lowercase(), "directional blur");
    }

    #[test]
    fn adding_gives_the_default_to_the_layers_without() {
        let mut document = document();
        let edits = add(Effect::Glow, &layers_of(&document, &["plain", "lit"]));
        assert_eq!(edits.len(), 1, "only the layer without a glow");
        document.apply(&edits).unwrap();
        let glow = |document: &Document, id: &str| {
            document
                .get(&NodeId::from(id))
                .unwrap()
                .layer()
                .unwrap()
                .glow
        };
        assert_eq!(glow(&document, "plain"), Some(Glow::default()));
        assert_eq!(glow(&document, "lit").unwrap().color, Color::rgb(255, 0, 0));
        let edits = add(Effect::Blur, &layers_of(&document, &["plain"]));
        document.apply(&edits).unwrap();
        let plain = document
            .get(&NodeId::from("plain"))
            .unwrap()
            .layer()
            .unwrap();
        assert_eq!(plain.blur, NEW_BLUR);
        for effect in [Effect::Shadow, Effect::Noise, Effect::DirectionalBlur] {
            let edits = add(effect, &layers_of(&document, &["plain"]));
            document.apply(&edits).unwrap();
        }
        let plain = document
            .get(&NodeId::from("plain"))
            .unwrap()
            .layer()
            .unwrap();
        assert_eq!(plain.noise, Some(Noise::default()));
        assert_eq!(plain.directional_blur, Some(DirectionalBlur::default()));
        assert_eq!(plain.shadow, Some(Shadow::default()));
    }

    #[test]
    fn removing_takes_it_from_the_layers_with() {
        let mut document = document();
        let edits = remove(
            Effect::Glow,
            &layers_of(&document, &["plain", "lit", "warm"]),
        );
        assert_eq!(edits.len(), 2);
        document.apply(&edits).unwrap();
        let edits = remove(Effect::Blur, &layers_of(&document, &["all"]));
        document.apply(&edits).unwrap();
        let layers = layers_of(&document, &["plain", "lit", "warm", "all"]);
        assert!(with(Effect::Glow, &layers) == [NodeId::from("all")]);
        assert!(with(Effect::Blur, &layers).is_empty());
    }

    #[test]
    fn a_part_set_on_several_layers_keeps_each_ones_other_parts() {
        let mut document = document();
        let layers = layers_of(&document, &["lit", "warm", "plain"]);
        let edits = set_part(Effect::Glow, &layers, Some("radius"), json!(40.0));
        assert_eq!(edits.len(), 2, "not the plain rect, which has no glow");
        document.apply(&edits).unwrap();
        let glow = |id: &str| {
            document
                .get(&NodeId::from(id))
                .unwrap()
                .layer()
                .unwrap()
                .glow
                .unwrap()
        };
        assert_eq!(
            glow("lit"),
            Glow {
                color: Color::rgb(255, 0, 0),
                radius: 40.0,
                spread: 2.0
            }
        );
        assert_eq!(
            glow("warm"),
            Glow {
                color: Color::rgb(255, 0xAA, 0),
                radius: 40.0,
                spread: 0.0
            }
        );
        // The blur is one number.
        let edits = set_part(
            Effect::Blur,
            &layers_of(&document, &["all"]),
            None,
            json!(9),
        );
        document.apply(&edits).unwrap();
        let all = document.get(&NodeId::from("all")).unwrap().layer().unwrap();
        assert_eq!(all.blur, 9.0);
        // Whether grain moves is a part like any other.
        let edits = set_part(
            Effect::Noise,
            &layers_of(&document, &["all"]),
            Some("animated"),
            json!(true),
        );
        document.apply(&edits).unwrap();
        let noise = document
            .get(&NodeId::from("all"))
            .unwrap()
            .layer()
            .unwrap()
            .noise
            .unwrap();
        assert_eq!((noise.amount, noise.animated), (0.4, true));
    }

    #[test]
    fn parts_are_kept_within_their_ranges() {
        assert_eq!(stored_part(Effect::Noise, Some("amount"), 33.3), 0.333);
        assert_eq!(stored_part(Effect::Noise, Some("amount"), 140.0), 1.0);
        assert_eq!(stored_part(Effect::Noise, Some("amount"), -5.0), 0.0);
        assert_eq!(stored_part(Effect::Noise, Some("size"), 0.0), MIN_GRAIN);
        assert_eq!(stored_part(Effect::Glow, Some("radius"), -3.0), 0.0);
        assert_eq!(stored_part(Effect::Glow, Some("spread"), 2.004), 2.0);
        assert_eq!(stored_part(Effect::Blur, None, -1.0), 0.0);
        assert_eq!(stored_part(Effect::Shadow, Some("x"), -6.0), -6.0);
        assert_eq!(
            stored_part(Effect::DirectionalBlur, Some("angle"), -45.0),
            -45.0
        );
        assert_eq!(
            stored_part(Effect::DirectionalBlur, Some("amount"), -45.0),
            0.0
        );
    }
}

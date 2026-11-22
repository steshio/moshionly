//! Placeholder content for the shell.
//!
//! The shell is built before the document engine is connected to it. So that
//! the layout can be judged and every widget exercised, the panels show the
//! sample scene defined here. It is plain view data in the shapes the widgets
//! take (rows, numbers, colours), not a document model: when the shell is
//! bound to a real document this module is deleted and the same widgets are
//! fed from the document instead.

use egui::{Color32, Context, Rect, TextureHandle, TextureOptions, pos2, vec2};

use super::sample_art;
use crate::{
    key::Key,
    theme::{Tone, icons},
    viewport::Artboard,
    widgets::Rgba,
};

/// One artboard of the sample scene.
#[derive(Clone)]
pub struct SampleArtboard {
    /// The artboard's key; it is also the key of its row in the layer list.
    pub key: Key,
    /// The artboard's name.
    pub name: &'static str,
    /// Position and size on the canvas, in world coordinates.
    pub rect: Rect,
    /// Background colour, shown until the picture is loaded.
    pub fill: Color32,
    /// The picture of the artboard, loaded on first use.
    pub texture: Option<TextureHandle>,
}

/// One row of the sample layer list.
#[derive(Clone, Debug)]
pub struct SampleLayer {
    /// The row's key.
    pub key: Key,
    /// The layer's name.
    pub name: &'static str,
    /// The icon for the layer's type.
    pub icon: &'static str,
    /// Nesting level.
    pub depth: usize,
    /// `Some` for layers with children: whether they are showing.
    pub expanded: Option<bool>,
    /// The layer is hidden.
    pub hidden: bool,
    /// The layer is locked.
    pub locked: bool,
}

/// One sample action: something a layer does between two moments.
#[derive(Clone, Debug)]
pub struct SampleAction {
    /// The action's key.
    pub key: Key,
    /// The key of the layer it animates.
    pub layer: Key,
    /// The action's name.
    pub name: &'static str,
    /// The icon for the action's type.
    pub icon: &'static str,
    /// The colour family of the action's type.
    pub tone: Tone,
    /// Start, in milliseconds.
    pub start: f64,
    /// End, in milliseconds.
    pub end: f64,
}

/// A blend mode, to give a dropdown something to choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SampleBlend {
    /// Normal.
    #[default]
    Normal,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
    /// Overlay.
    Overlay,
}

impl SampleBlend {
    /// Every mode with its label, in menu order.
    pub const OPTIONS: [(Self, &'static str); 4] = [
        (Self::Normal, "Normal"),
        (Self::Multiply, "Multiply"),
        (Self::Screen, "Screen"),
        (Self::Overlay, "Overlay"),
    ];
}

/// An easing, to give a dropdown something to choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SampleEasing {
    /// Linear.
    Linear,
    /// Smooth.
    #[default]
    Smooth,
    /// Natural.
    Natural,
    /// Overshoot.
    Overshoot,
    /// Bounce.
    Bounce,
}

impl SampleEasing {
    /// Every easing with its label, in menu order.
    pub const OPTIONS: [(Self, &'static str); 5] = [
        (Self::Linear, "Linear"),
        (Self::Smooth, "Smooth"),
        (Self::Natural, "Natural"),
        (Self::Overshoot, "Overshoot"),
        (Self::Bounce, "Bounce"),
    ];
}

/// The numbers the Design tab edits.
#[derive(Clone, Debug)]
pub struct SampleDesign {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
    /// Rotation in degrees.
    pub rotation: f64,
    /// Corner radius.
    pub corner_radius: f64,
    /// Opacity, 0 to 100.
    pub opacity: f64,
    /// Blend mode.
    pub blend: SampleBlend,
    /// Fill colour.
    pub fill: Rgba,
    /// Stroke colour.
    pub stroke: Rgba,
    /// Stroke width.
    pub stroke_width: f64,
}

/// The sample scene and the bits of state the placeholder panels keep.
#[derive(Clone)]
pub struct Sample {
    /// The project's name, shown in the top bar.
    pub project_name: String,
    /// The artboards on the canvas.
    pub artboards: Vec<SampleArtboard>,
    /// The layer list, top to bottom.
    pub layers: Vec<SampleLayer>,
    /// The selected layer.
    pub selected_layer: Option<Key>,
    /// The actions of the scene.
    pub actions: Vec<SampleAction>,
    /// The selected actions.
    pub selected_actions: Vec<Key>,
    /// The Design tab's values.
    pub design: SampleDesign,
    /// The easing chosen in the Animate tab.
    pub easing: SampleEasing,
    /// The easing's intensity, 0 to 100.
    pub intensity: f64,
    /// Length of the scene in milliseconds.
    pub duration: f64,
    /// What is typed into the layer search.
    pub layer_query: String,
    /// What is typed into the chat composer.
    pub chat_draft: String,
}

fn layer(
    key: &str,
    name: &'static str,
    icon: &'static str,
    depth: usize,
    expanded: Option<bool>,
) -> SampleLayer {
    SampleLayer {
        key: Key::new(key),
        name,
        icon,
        depth,
        expanded,
        hidden: false,
        locked: false,
    }
}

fn action(
    key: &str,
    layer: &str,
    name: &'static str,
    icon: &'static str,
    tone: Tone,
    start: f64,
    end: f64,
) -> SampleAction {
    SampleAction {
        key: Key::new(key),
        layer: Key::new(layer),
        name,
        icon,
        tone,
        start,
        end,
    }
}

impl Default for Sample {
    fn default() -> Self {
        let mut layers = vec![
            layer("scene-1", "Scene 1", icons::FRAME_CORNERS, 0, Some(true)),
            layer("headline", "Headline", icons::TEXT_T, 1, None),
            layer("badge", "Badge", icons::CIRCLE, 1, None),
            layer("card", "Card", icons::RECTANGLE, 1, None),
            layer("photo", "Photo", icons::IMAGE, 1, None),
            layer("confetti", "Confetti", icons::FOLDER_SIMPLE, 1, Some(false)),
            layer("spark-1", "Spark", icons::STAR, 2, None),
            layer("spark-2", "Spark", icons::STAR, 2, None),
            layer("backdrop", "Backdrop", icons::RECTANGLE, 1, None),
            layer("scene-2", "Scene 2", icons::FRAME_CORNERS, 0, Some(false)),
            layer("outro", "Outro title", icons::TEXT_T, 1, None),
        ];
        layers[8].locked = true;

        // Tones tell the families of actions apart: teal appears, rose
        // disappears, the accent moves or changes a property, amber is text.
        let actions = vec![
            action(
                "a-headline-in",
                "headline",
                "Text in",
                icons::TEXT_AA,
                Tone::Amber,
                300.0,
                1300.0,
            ),
            action(
                "a-headline-out",
                "headline",
                "Fade out",
                icons::DROP_HALF,
                Tone::Rose,
                3400.0,
                4000.0,
            ),
            action(
                "a-badge-in",
                "badge",
                "Grow in",
                icons::ARROWS_OUT_SIMPLE,
                Tone::Teal,
                900.0,
                1500.0,
            ),
            action(
                "a-badge-spin",
                "badge",
                "Rotate",
                icons::ARROW_CLOCKWISE,
                Tone::Accent,
                1500.0,
                3200.0,
            ),
            action(
                "a-card-in",
                "card",
                "Fade in",
                icons::DROP_HALF,
                Tone::Teal,
                0.0,
                600.0,
            ),
            action(
                "a-card-move",
                "card",
                "Move",
                icons::ARROWS_OUT_CARDINAL,
                Tone::Accent,
                200.0,
                1100.0,
            ),
            action(
                "a-card-scale",
                "card",
                "Scale",
                icons::RESIZE,
                Tone::Accent,
                2600.0,
                3400.0,
            ),
            action(
                "a-photo-in",
                "photo",
                "Slide in",
                icons::ARROW_LINE_RIGHT,
                Tone::Teal,
                600.0,
                1400.0,
            ),
        ];

        let side = 1080.0;
        let artboards = vec![
            SampleArtboard {
                key: Key::new("scene-1"),
                name: "Scene 1",
                rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(side, side)),
                fill: Color32::from_rgb(0xF6, 0xF5, 0xFC),
                texture: None,
            },
            SampleArtboard {
                key: Key::new("scene-2"),
                name: "Scene 2",
                rect: Rect::from_min_size(pos2(side + 160.0, 0.0), vec2(side, side)),
                fill: Color32::from_rgb(0x18, 0x18, 0x1E),
                texture: None,
            },
        ];

        Self {
            project_name: "Launch teaser".to_owned(),
            artboards,
            layers,
            selected_layer: Some(Key::new("card")),
            actions,
            selected_actions: vec![Key::new("a-card-move")],
            design: SampleDesign {
                x: 240.0,
                y: 340.0,
                width: 600.0,
                height: 400.0,
                rotation: 0.0,
                corner_radius: 32.0,
                opacity: 100.0,
                blend: SampleBlend::Normal,
                fill: Rgba::rgb(0x5B, 0x5F, 0xEF),
                stroke: Rgba::new(0x1C, 0x1C, 0x20, 0x33),
                stroke_width: 1.0,
            },
            easing: SampleEasing::Smooth,
            intensity: 60.0,
            duration: 4000.0,
            layer_query: String::new(),
            chat_draft: String::new(),
        }
    }
}

impl Sample {
    /// Loads the artboards' pictures as textures, once.
    pub fn load_textures(&mut self, ctx: &Context) {
        let pictures: [fn() -> crate::raster::Canvas; 2] =
            [sample_art::scene_one, sample_art::scene_two];
        for (artboard, picture) in self.artboards.iter_mut().zip(pictures) {
            if artboard.texture.is_none() {
                artboard.texture = Some(ctx.load_texture(
                    format!("sample-{}", artboard.key),
                    picture().to_color_image(),
                    TextureOptions::LINEAR,
                ));
            }
        }
    }

    /// The artboards in the form the viewport takes.
    pub fn viewport_artboards(&self) -> Vec<Artboard<'static>> {
        self.artboards
            .iter()
            .map(|artboard| Artboard {
                key: artboard.key.clone(),
                name: artboard.name,
                rect: artboard.rect,
                texture: artboard.texture.as_ref().map(TextureHandle::id),
                fill: artboard.fill,
                selected: self.selected_layer.as_ref() == Some(&artboard.key),
            })
            .collect()
    }

    /// The rectangle around all artboards, in world coordinates.
    pub fn bounds(&self) -> Rect {
        self.artboards
            .iter()
            .fold(Rect::NOTHING, |bounds, artboard| {
                bounds.union(artboard.rect)
            })
    }

    /// The layer with `key`.
    pub fn layer(&self, key: &Key) -> Option<&SampleLayer> {
        self.layers.iter().find(|layer| layer.key == *key)
    }

    /// The selected layer.
    pub fn selected(&self) -> Option<&SampleLayer> {
        self.selected_layer.as_ref().and_then(|key| self.layer(key))
    }

    /// Indices of the layers the list shows: children of collapsed layers
    /// are left out, and a search query keeps only the matching names.
    pub fn visible_layers(&self) -> Vec<usize> {
        let query = self.layer_query.trim().to_lowercase();
        if !query.is_empty() {
            return self
                .layers
                .iter()
                .enumerate()
                .filter(|(_, layer)| layer.name.to_lowercase().contains(&query))
                .map(|(index, _)| index)
                .collect();
        }
        let mut visible = Vec::new();
        // Rows deeper than this are inside a collapsed layer.
        let mut collapsed_below: Option<usize> = None;
        for (index, layer) in self.layers.iter().enumerate() {
            if let Some(depth) = collapsed_below {
                if layer.depth > depth {
                    continue;
                }
                collapsed_below = None;
            }
            visible.push(index);
            if layer.expanded == Some(false) {
                collapsed_below = Some(layer.depth);
            }
        }
        visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sample: &Sample) -> Vec<&'static str> {
        sample
            .visible_layers()
            .into_iter()
            .map(|index| sample.layers[index].name)
            .collect()
    }

    #[test]
    fn collapsed_layers_hide_their_children() {
        let mut sample = Sample::default();
        assert_eq!(
            names(&sample),
            [
                "Scene 1", "Headline", "Badge", "Card", "Photo", "Confetti", "Backdrop", "Scene 2"
            ]
        );
        sample.layers[0].expanded = Some(false);
        assert_eq!(names(&sample), ["Scene 1", "Scene 2"]);
        sample.layers[0].expanded = Some(true);
        sample.layers[5].expanded = Some(true);
        assert_eq!(names(&sample).len(), 10);
    }

    #[test]
    fn a_query_lists_matches_flat() {
        let mut sample = Sample {
            layer_query: "  SPARK ".to_owned(),
            ..Sample::default()
        };
        assert_eq!(names(&sample), ["Spark", "Spark"]);
        sample.layer_query = "nothing".to_owned();
        assert!(names(&sample).is_empty());
    }

    #[test]
    fn every_action_targets_a_layer() {
        let sample = Sample::default();
        for action in &sample.actions {
            assert!(sample.layer(&action.layer).is_some(), "{}", action.name);
            assert!(action.end > action.start);
            assert!(action.end <= sample.duration);
        }
        assert!(sample.selected().is_some());
    }
}

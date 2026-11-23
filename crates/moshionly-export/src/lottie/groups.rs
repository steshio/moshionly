//! Groups, clipping and masks: layers inside other layers.
//!
//! A group that only moves its children becomes a null layer, an invisible
//! layer the children are parented to, which is exact: Lottie passes a
//! parent's transform on to its children. A group that does more (fades
//! as a whole, blends, blurs, casts a shadow, clips) becomes a
//! precomposition: its children are drawn into a composition of their own,
//! which the group's layer shows, faded, blended and clipped as one.
//!
//! Players clip a precomposition to its size, so each is made large enough
//! for everything of it that can ever be seen: the part of the group's
//! space that falls on the artboard at any frame. The children are moved
//! by the same amount, and the anchor with them.
//!
//! A mask group becomes a precomposition holding a track matte: the mask
//! layer, with the rest of the children in a composition of their own just
//! below it, shown where the mask is opaque.

use moshionly_core::eval::{Property, PropertyValue};
use moshionly_core::model::BlendMode;
use moshionly_core::{Family, Node, NodeData, NodeId, Point};
use serde_json::{Map, Value};

use crate::error::ExportError;

use super::fit::{Animated, Hints};
use super::json::{num, object, places};
use super::layers::{Comp, Converter, Parts};
use super::props::{Position, Transform, map};
use super::shapes::{self, Contour, Outline};
use super::timeline::{places_for, tolerance};

/// How far the sides of a reveal that are not wiping reach past the box,
/// as the scene has them.
const REACH: f64 = 100_000.0;

/// The most a precomposition reaches from its group's origin, in pixels.
/// A group scaled almost to nothing would otherwise need an endless one.
const LIMIT: f64 = 100_000.0;

/// A rectangle in a group's own space: left, top, right, bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Area {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Area {
    /// A `width` by `height` box at the origin.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            left: 0.0,
            top: 0.0,
            right: width,
            bottom: height,
        }
    }

    /// Grows the area to hold `point`.
    pub fn include(&mut self, point: Point) {
        self.left = self.left.min(point.x);
        self.top = self.top.min(point.y);
        self.right = self.right.max(point.x);
        self.bottom = self.bottom.max(point.y);
    }

    /// The area with whole-pixel edges, within the limit.
    pub fn rounded(self) -> Self {
        let edge = |value: f64, round: fn(f64) -> f64| round(value.clamp(-LIMIT, LIMIT));
        Self {
            left: edge(self.left, f64::floor),
            top: edge(self.top, f64::floor),
            right: edge(self.right, f64::ceil),
            bottom: edge(self.bottom, f64::ceil),
        }
    }

    /// How far the group's space moves to put the area's corner at the
    /// precomposition's origin.
    pub fn origin(&self) -> [f64; 2] {
        [-self.left, -self.top]
    }

    fn width(&self) -> f64 {
        (self.right - self.left).max(1.0)
    }

    fn height(&self) -> f64 {
        (self.bottom - self.top).max(1.0)
    }
}

/// How a layer's own space sits in the space its masks are drawn in: a
/// point `p` of the layer is at `p * scale + offset` there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Place {
    /// Scale each way.
    pub scale: [f64; 2],
    /// Shift after scaling.
    pub offset: [f64; 2],
}

impl Default for Place {
    fn default() -> Self {
        Self {
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
        }
    }
}

impl Place {
    /// The layer moved by `offset`.
    pub fn moved(offset: [f64; 2]) -> Self {
        Self {
            offset,
            ..Self::default()
        }
    }

    /// `contour` placed: points scaled and shifted, handles scaled.
    fn apply(&self, mut contour: Contour) -> Contour {
        for vertex in &mut contour.vertices {
            *vertex = [
                vertex[0] * self.scale[0] + self.offset[0],
                vertex[1] * self.scale[1] + self.offset[1],
            ];
        }
        for handle in contour.ins.iter_mut().chain(&mut contour.outs) {
            *handle = [handle[0] * self.scale[0], handle[1] * self.scale[1]];
        }
        contour
    }
}

/// A rectangle from its edges, as a contour of four points, also when it
/// has no size.
fn rectangle([left, top, right, bottom]: [f64; 4]) -> Contour {
    let mut contour = Contour::starting_at([left, top]);
    contour.line_to([right, top]);
    contour.line_to([right, bottom]);
    contour.line_to([left, bottom]);
    contour.closed = true;
    contour
}

impl Converter<'_> {
    /// Adds a group: a null layer its children are parented to, or a
    /// precomposition when it does more than move them.
    pub fn add_group(
        &mut self,
        node: &Node,
        clip: bool,
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) -> Result<(), ExportError> {
        let hints = self.timeline.hints(&node.id);
        let transform = self.transform(node, &hints, offset);
        if !self.isolated(node, clip, &transform.opacity) {
            let index = comp.index();
            // The outline in front of the children, the background behind.
            self.add_paint_layer(node, &hints, Parts::STROKE, comp, Some(index), [0.0, 0.0]);
            self.add_children(&node.id, comp, Some(index), [0.0, 0.0])?;
            self.add_paint_layer(node, &hints, Parts::FILL, comp, Some(index), [0.0, 0.0]);
            let null = self.layer(node, 3, index, parent_index, &transform);
            comp.layers.push(Value::Object(null));
            return Ok(());
        }

        let area = self.area(&node.id);
        let origin = area.origin();
        let has_stroke = self
            .survey
            .iter()
            .any(|frame| self.timeline.stroke(&node.id, *frame).is_some());
        // The clip cuts the children, not the group's own outline or its
        // shadow: with an outline or effects, the children get a clipped
        // composition of their own. (Players apply a layer's masks after
        // its effects, which would cut the shadow.)
        let clip_children_apart = clip && (has_stroke || self.has_effects(&node.id));
        let asset = self.composition(node, |this, inner| {
            this.add_paint_layer(node, &hints, Parts::STROKE, inner, None, origin);
            if clip_children_apart {
                let children = this.composition(node, |this, innermost| {
                    this.add_children(&node.id, innermost, None, origin)
                })?;
                let index = inner.index();
                let mut layer = this.composition_layer(
                    node,
                    &children,
                    area,
                    &Transform::default(),
                    index,
                    None,
                );
                layer.remove("bm");
                this.add_masks(node, &mut layer, &|_| Place::moved(origin), Masks::CLIP);
                inner.layers.push(Value::Object(layer));
            } else {
                this.add_children(&node.id, inner, None, origin)?;
            }
            this.add_paint_layer(node, &hints, Parts::FILL, inner, None, origin);
            Ok(())
        })?;
        let transform = anchored(transform, origin);
        let index = comp.index();
        let mut layer = self.composition_layer(node, &asset, area, &transform, index, parent_index);
        let masks = Masks {
            clip: clip && !clip_children_apart,
            reveal: true,
        };
        self.add_masks(node, &mut layer, &|_| Place::moved(origin), masks);
        self.add_effects(node, &mut layer);
        comp.layers.push(Value::Object(layer));
        Ok(())
    }

    /// Adds a mask group: a precomposition with its first child as a track
    /// matte over the rest. A mask group whose mask is never seen shows
    /// nothing, and is left out.
    pub fn add_mask_group(
        &mut self,
        node: &Node,
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) -> Result<(), ExportError> {
        let document = self.timeline.document;
        let children: Vec<&Node> = document.child_nodes(&node.id, Family::Layer).collect();
        let Some((mask, rest)) = children.split_first() else {
            return Ok(());
        };
        if !self.is_ever_visible(&mask.id) {
            return Ok(());
        }
        let hints = self.timeline.hints(&node.id);
        let transform = self.transform(node, &hints, offset);
        let area = self.area(&node.id);
        let origin = area.origin();
        let asset = self.composition(node, |this, inner| {
            let content = this.composition(node, |this, content| {
                for child in rest.iter().rev() {
                    this.add_layer(child, content, None, origin)?;
                }
                Ok(())
            })?;
            let mut matte = this.single_layer(mask, area, inner)?;
            matte.insert("td".into(), Value::from(1));
            inner.layers.push(Value::Object(matte));
            let index = inner.index();
            let mut layer =
                this.composition_layer(node, &content, area, &Transform::default(), index, None);
            layer.remove("bm");
            layer.insert(
                "nm".into(),
                Value::from(format!("{} content", node_name(node))),
            );
            layer.insert("tt".into(), Value::from(1));
            inner.layers.push(Value::Object(layer));
            Ok(())
        })?;
        let transform = anchored(transform, origin);
        let index = comp.index();
        let mut layer = self.composition_layer(node, &asset, area, &transform, index, parent_index);
        self.add_masks(node, &mut layer, &|_| Place::moved(origin), Masks::REVEAL);
        self.add_effects(node, &mut layer);
        comp.layers.push(Value::Object(layer));
        Ok(())
    }

    /// `node` as exactly one layer of `comp`, as a track matte must be: its
    /// own layer when it makes one, or a precomposition of its layers.
    fn single_layer(
        &mut self,
        node: &Node,
        area: Area,
        comp: &mut Comp,
    ) -> Result<Map<String, Value>, ExportError> {
        let origin = area.origin();
        let mut alone = Comp::default();
        self.add_layer(node, &mut alone, None, origin)?;
        if let [Value::Object(layer)] = alone.layers.as_slice() {
            let mut layer = layer.clone();
            layer.insert("ind".into(), Value::from(comp.index()));
            return Ok(layer);
        }
        let asset = self.composition(node, |_, inner| {
            inner.layers = std::mem::take(&mut alone.layers);
            Ok(())
        })?;
        let index = comp.index();
        let mut layer =
            self.composition_layer(node, &asset, area, &Transform::default(), index, None);
        layer.remove("bm");
        Ok(layer)
    }

    /// Whether a group needs a composition of its own: when it fades,
    /// blends, blurs, casts a shadow or clips, which apply to its children
    /// drawn together.
    fn isolated(&self, node: &Node, clip: bool, opacity: &Animated) -> bool {
        let blends = node
            .layer()
            .is_some_and(|layer| layer.blend_mode != BlendMode::Normal);
        clip || blends
            || self.has_reveal(&node.id)
            || *opacity != Animated::Static(vec![100.0])
            || self.has_effects(&node.id)
    }

    /// Whether a layer is ever blurred or casts a shadow.
    pub fn has_effects(&self, id: &NodeId) -> bool {
        let timeline = self.timeline;
        self.survey.iter().any(|frame| {
            let ms = timeline.clock.ms(*frame);
            let blurred = timeline.number(id, Property::Blur, *frame) > 0.0;
            let shadowed = matches!(
                timeline.value(id, Property::Shadow, ms),
                Some(PropertyValue::Shadow(Some(_)))
            );
            blurred || shadowed
        })
    }

    /// Whether a reveal animation targets the layer.
    pub fn has_reveal(&self, id: &NodeId) -> bool {
        self.timeline
            .document
            .animations_targeting(id)
            .iter()
            .any(|animation| {
                matches!(
                    animation.data,
                    NodeData::RevealIn(_) | NodeData::RevealOut(_)
                )
            })
    }

    /// The part of a group's own space that can ever be seen: its box, and
    /// the artboard as seen from the group at every frame it is drawn.
    pub fn area(&self, id: &NodeId) -> Area {
        let (mut width, mut height) = (0.0_f64, 0.0_f64);
        for frame in &self.survey {
            let (w, h) = self.size(id, *frame);
            (width, height) = (width.max(w), height.max(h));
        }
        let mut area = Area::new(width, height);
        for frame in &self.survey {
            let Some(scene) = self.timeline.scenes.at(*frame) else {
                continue;
            };
            let Some(inverse) = scene.transform_of(id).and_then(|t| t.invert()) else {
                continue;
            };
            for corner in [
                Point::new(0.0, 0.0),
                Point::new(scene.width, 0.0),
                Point::new(scene.width, scene.height),
                Point::new(0.0, scene.height),
            ] {
                area.include(inverse.apply(corner));
            }
        }
        area.rounded()
    }

    /// A precomposition whose layers `build` adds; returns its id.
    pub fn composition(
        &mut self,
        node: &Node,
        build: impl FnOnce(&mut Self, &mut Comp) -> Result<(), ExportError>,
    ) -> Result<String, ExportError> {
        let id = format!("comp_{}", self.assets.len());
        // Hold the place, so compositions made while building get their
        // own numbers.
        self.assets.push(Value::Null);
        let place = self.assets.len() - 1;
        let mut inner = Comp::default();
        build(self, &mut inner)?;
        self.assets[place] = object([
            ("id", Value::from(id.as_str())),
            ("nm", Value::from(node_name(node))),
            ("layers", Value::Array(inner.layers)),
        ]);
        Ok(id)
    }

    /// A layer showing the precomposition `asset`, which covers `area`.
    pub fn composition_layer(
        &self,
        node: &Node,
        asset: &str,
        area: Area,
        transform: &Transform,
        index: i64,
        parent_index: Option<i64>,
    ) -> Map<String, Value> {
        let mut layer = self.layer(node, 0, index, parent_index, transform);
        layer.insert("refId".into(), Value::from(asset));
        layer.insert("w".into(), num(area.width(), 0));
        layer.insert("h".into(), num(area.height(), 0));
        layer
    }

    /// A layer drawing a group's own background or outline, in the group's
    /// space moved by `origin`.
    fn add_paint_layer(
        &mut self,
        node: &Node,
        hints: &Hints,
        parts: Parts,
        comp: &mut Comp,
        parent_index: Option<i64>,
        origin: [f64; 2],
    ) {
        let items = self.paint_items(node, hints, parts);
        if items.is_empty() {
            return;
        }
        let transform = Transform {
            position: Position::Joined(Animated::Static(origin.to_vec())),
            ..Transform::default()
        };
        let index = comp.index();
        let mut layer = self.layer(node, 4, index, parent_index, &transform);
        layer.remove("bm");
        let role = if parts.fill { "background" } else { "outline" };
        layer.insert(
            "nm".into(),
            Value::from(format!("{} {role}", node_name(node))),
        );
        layer.insert("shapes".into(), Value::Array(items));
        comp.layers.push(Value::Object(layer));
    }

    /// Adds the masks a layer needs: its box's clip, and what reveal
    /// animations leave showing. Masks are drawn in the node's own space
    /// put in its Lottie layer's space by `place` at each frame.
    pub fn add_masks(
        &self,
        node: &Node,
        layer: &mut Map<String, Value>,
        place: &dyn Fn(f64) -> Place,
        masks: Masks,
    ) {
        let mut list = Vec::new();
        if masks.clip {
            list.push(self.clip_mask(node, place));
        }
        if masks.reveal && self.has_reveal(&node.id) {
            list.push(self.reveal_mask(node, place));
        }
        if list.is_empty() {
            return;
        }
        // The first mask adds what it covers; later ones keep only what
        // they cover too.
        for (position, mask) in list.iter_mut().enumerate() {
            if let Value::Object(mask) = mask {
                let mode = if position == 0 { "a" } else { "i" };
                mask.insert("mode".into(), Value::from(mode));
            }
        }
        layer.insert("hasMask".into(), Value::Bool(true));
        layer.insert("masksProperties".into(), Value::Array(list));
    }

    /// A mask of a layer's rounded box.
    fn clip_mask(&self, node: &Node, place: &dyn Fn(f64) -> Place) -> Value {
        let id = &node.id;
        let timeline = self.timeline;
        let contour = |frame: f64| {
            let (width, height) = self.size(id, frame);
            place(frame).apply(shapes::rounded_rect(
                width,
                height,
                timeline.radii(id, frame),
            ))
        };
        let numbers = |frame: f64| shapes::flatten(&[contour(frame)]);
        let hints = timeline.hints(id);
        let outline = Outline {
            template: vec![contour(0.0)],
            numbers: timeline.fit(
                &numbers,
                &hints,
                tolerance::LENGTH,
                places_for(tolerance::LENGTH),
            ),
        };
        mask("Clip", &outline)
    }

    /// A mask of the part of a layer that reveal animations leave showing.
    fn reveal_mask(&self, node: &Node, place: &dyn Fn(f64) -> Place) -> Value {
        let id = &node.id;
        let timeline = self.timeline;
        let edges = |frame: f64| -> [f64; 4] {
            let (width, height) = self.size(id, frame);
            let scene = timeline.scenes.at(frame);
            match scene.as_deref().and_then(|scene| scene.find(id)) {
                Some(drawn) => match drawn.clip {
                    Some(clip) => [
                        clip.rect.x,
                        clip.rect.y,
                        clip.rect.right(),
                        clip.rect.bottom(),
                    ],
                    None => [-REACH, -REACH, width + REACH, height + REACH],
                },
                // Not drawn although shown: the reveal leaves nothing.
                None => [0.0, 0.0, 0.0, 0.0],
            }
        };
        let contour = |frame: f64| place(frame).apply(rectangle(edges(frame)));
        let numbers = |frame: f64| shapes::flatten(&[contour(frame)]);
        let hints = timeline.hints(id);
        let outline = Outline {
            template: vec![contour(0.0)],
            numbers: timeline.fit(
                &numbers,
                &hints,
                tolerance::LENGTH,
                places_for(tolerance::LENGTH),
            ),
        };
        mask("Reveal", &outline)
    }
}

/// Which masks a layer gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Masks {
    /// The clip of a group to its box.
    pub clip: bool,
    /// What reveal animations leave showing.
    pub reveal: bool,
}

impl Masks {
    /// The reveal only.
    pub const REVEAL: Masks = Masks {
        clip: false,
        reveal: true,
    };
    /// The clip only.
    pub const CLIP: Masks = Masks {
        clip: true,
        reveal: false,
    };
}

/// A Lottie mask with the outline's first contour. Its expansion `x` is
/// written although it is 0: players read it without a default.
fn mask(name: &str, outline: &Outline) -> Value {
    object([
        ("nm", Value::from(name)),
        ("mode", Value::from("a")),
        ("inv", Value::Bool(false)),
        ("pt", outline.path_property(0, places::LENGTH)),
        ("o", object([("k", Value::from(100))])),
        ("x", object([("k", Value::from(0))])),
    ])
}

/// A transform whose anchor moves with its layer's content by `origin`.
pub(crate) fn anchored(mut transform: Transform, origin: [f64; 2]) -> Transform {
    transform.anchor = map(&transform.anchor, |anchor| {
        vec![anchor[0] + origin[0], anchor[1] + origin[1]]
    });
    transform
}

/// The name of a layer for the layers made from it.
fn node_name(node: &Node) -> String {
    match node.name() {
        "" => node.node_type().to_string(),
        name => name.to_owned(),
    }
}

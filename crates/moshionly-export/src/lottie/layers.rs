//! Layers: the artboard's tree of layers as Lottie layers.
//!
//! Every Moshionly layer becomes a Lottie layer with the same transform:
//! Lottie turns and scales a layer about its anchor point, which is set to
//! the centre of the layer's box, exactly as Moshionly does. Shapes are
//! drawn in the layer's own coordinates, with the origin at the top-left
//! corner of its box. Lottie lists layers front to back, the reverse of
//! Moshionly's order.

use moshionly_core::eval::Property;
use moshionly_core::model::{FillRule, StrokeJoin, StrokePosition};
use moshionly_core::{Color, Document, Node, NodeData, NodeId};
use moshionly_render::Renderer;
use serde_json::{Map, Value};

use crate::error::ExportError;

use super::fit::{Animated, Hints};
use super::groups::{Masks, Place};
use super::images::Images;
use super::json::{num, object, places};
use super::paint::{self, Notes, Role, Source};
use super::props::{Position, Transform, array, fixed_property, property};
use super::shapes::{self, Contour, Outline};
use super::timeline::{Timeline, places_for, tolerance};
use super::warnings::Warnings;

/// The layers of one Lottie composition, front first, and the numbers
/// that identify them.
#[derive(Debug, Default)]
pub(crate) struct Comp {
    /// The layers, front first.
    pub layers: Vec<Value>,
    /// The last layer number handed out.
    last_index: i64,
}

impl Comp {
    /// A number for a new layer, unique in this composition.
    pub fn index(&mut self) -> i64 {
        self.last_index += 1;
        self.last_index
    }
}

/// How far the conversion has come.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Step {
    /// A layer is converted.
    Layer,
    /// This part of the layer being converted is done, from 0 to 1.
    Part(f64),
}

/// Called as the conversion goes on; fails when the export is cancelled.
pub(crate) type Progress<'a> = Box<dyn FnMut(Step) -> Result<(), ExportError> + 'a>;

/// Turns the layers of an artboard into Lottie layers.
pub(crate) struct Converter<'a> {
    /// Values over time.
    pub timeline: Timeline<'a>,
    /// What could not be exported exactly.
    pub warnings: Warnings,
    /// The frames a value is looked at to decide how to write it: every
    /// frame of the export, and the end.
    pub survey: Vec<f64>,
    /// Precompositions and images, for the file's `assets`.
    pub assets: Vec<Value>,
    /// Draws text, for its outlines.
    pub renderer: &'a Renderer,
    /// The pictures of image layers.
    pub images: Images,
    /// Reports each converted layer.
    progress: Progress<'a>,
}

impl<'a> Converter<'a> {
    /// A converter reading `timeline`, drawing text with `renderer`, and
    /// calling `progress` after each layer.
    pub fn new(
        timeline: Timeline<'a>,
        renderer: &'a Renderer,
        images: Images,
        progress: Progress<'a>,
    ) -> Self {
        let frames = timeline.clock.frames;
        Self {
            timeline,
            warnings: Warnings::default(),
            survey: (0..=frames).map(|frame| frame as f64).collect(),
            assets: Vec::new(),
            renderer,
            images,
            progress,
        }
    }

    /// What could not be exported exactly, and the pictures written.
    pub fn finish(self) -> (Warnings, Images) {
        (self.warnings, self.images)
    }

    fn document(&self) -> &'a Document {
        self.timeline.document
    }

    /// Adds the layers inside `parent` to `comp`, front first. Each is
    /// placed `offset` further than its own position, and is parented to
    /// the layer numbered `parent_index` when there is one.
    pub fn add_children(
        &mut self,
        parent: &NodeId,
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) -> Result<(), ExportError> {
        let document = self.document();
        let children: Vec<&Node> = document
            .child_nodes(parent, moshionly_core::Family::Layer)
            .collect();
        for node in children.into_iter().rev() {
            self.add_layer(node, comp, parent_index, offset)?;
        }
        Ok(())
    }

    /// Adds the Lottie layers of one Moshionly layer to `comp`.
    pub fn add_layer(
        &mut self,
        node: &Node,
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) -> Result<(), ExportError> {
        if !self.is_ever_visible(&node.id) {
            return (self.progress)(Step::Layer);
        }
        match &node.data {
            NodeData::Rect(_) | NodeData::Ellipse(_) | NodeData::Star(_) | NodeData::Path(_) => {
                let hints = self.timeline.hints(&node.id);
                let transform = self.transform(node, &hints, offset);
                let shapes = self.shape_items(node, &hints);
                let index = comp.index();
                let mut layer = self.layer(node, 4, index, parent_index, &transform);
                layer.insert("shapes".into(), Value::Array(shapes));
                self.add_masks(node, &mut layer, &|_| Place::default(), Masks::REVEAL);
                self.add_effects(node, &mut layer);
                comp.layers.push(Value::Object(layer));
            }
            NodeData::Group(group) => {
                self.add_group(node, group.clip, comp, parent_index, offset)?;
            }
            // A component is drawn where it is like a group.
            NodeData::Component(component) => {
                self.add_group(node, component.clip, comp, parent_index, offset)?;
            }
            // Instances are detached into groups before converting.
            NodeData::Instance(_) => self.warnings.layer(
                node,
                "is an instance that could not be converted, and is left out.",
            ),
            NodeData::MaskGroup(_) => self.add_mask_group(node, comp, parent_index, offset)?,
            NodeData::Text(text) => self.add_text(node, text, comp, parent_index, offset)?,
            NodeData::Image(image) => self.add_image(node, image, comp, parent_index, offset)?,
            NodeData::Video(_) => self.warnings.layer(
                node,
                "is a video, which Lottie files cannot play, and is left out with its sound.",
            ),
            // Artboards are never inside other layers.
            NodeData::Artboard(_) => {}
            // Animations and sound are not layers.
            _ => {}
        }
        (self.progress)(Step::Layer)
    }

    /// Reports that `part` of the layer being converted is done, from 0 to
    /// 1, for layers that take long; fails when the export is cancelled.
    pub fn step(&mut self, part: f64) -> Result<(), ExportError> {
        (self.progress)(Step::Part(part))
    }

    /// Whether `layer` is shown at any moment of the export. Layers hidden
    /// throughout are left out.
    pub fn is_ever_visible(&self, layer: &NodeId) -> bool {
        self.survey
            .iter()
            .any(|frame| self.timeline.visible(layer, *frame))
    }

    /// The parts every Lottie layer has: its kind `ty`, its number, its
    /// name, its transform and its time.
    pub fn layer(
        &self,
        node: &Node,
        ty: u8,
        index: i64,
        parent_index: Option<i64>,
        transform: &Transform,
    ) -> Map<String, Value> {
        let mut layer = layer_start(ty);
        layer.insert("ind".into(), Value::from(index));
        if let Some(parent) = parent_index {
            layer.insert("parent".into(), Value::from(parent));
        }
        let name = match node.name() {
            "" => node.node_type().to_string(),
            name => name.to_owned(),
        };
        layer.insert("nm".into(), Value::from(name));
        layer.insert("ks".into(), transform.to_json());
        if let Some(mode) = node.layer().map(|layer| blend_code(layer.blend_mode))
            && mode != 0
        {
            layer.insert("bm".into(), Value::from(mode));
        }
        layer.insert("ip".into(), Value::from(0));
        layer.insert(
            "op".into(),
            num(self.timeline.clock.out_point(), places::TIME),
        );
        layer.insert("st".into(), Value::from(0));
        layer
    }

    /// The box of `layer` at a frame: width and height.
    pub fn size(&self, layer: &NodeId, frame: f64) -> (f64, f64) {
        (
            self.timeline.number(layer, Property::Width, frame).max(0.0),
            self.timeline
                .number(layer, Property::Height, frame)
                .max(0.0),
        )
    }

    /// The transform of a layer: anchored at the centre of its box, at the
    /// box's centre in the parent, plus `offset`.
    pub fn transform(&self, node: &Node, hints: &Hints, offset: [f64; 2]) -> Transform {
        let id = &node.id;
        let timeline = self.timeline;
        let length = places_for(tolerance::LENGTH);
        let anchor = |frame: f64| {
            let (width, height) = self.size(id, frame);
            vec![width / 2.0, height / 2.0]
        };
        let center = |property: Property, size: Property, offset: f64| {
            move |frame: f64| {
                vec![
                    timeline.number(id, property, frame)
                        + timeline.number(id, size, frame) / 2.0
                        + offset,
                ]
            }
        };
        let x = timeline.fit(
            &center(Property::X, Property::Width, offset[0]),
            hints,
            tolerance::LENGTH,
            length,
        );
        let y = timeline.fit(
            &center(Property::Y, Property::Height, offset[1]),
            hints,
            tolerance::LENGTH,
            length,
        );
        Transform {
            anchor: timeline.fit(&anchor, hints, tolerance::LENGTH, length),
            position: Position::from_parts(x, y),
            scale: timeline.fit_number(id, Property::Scale, hints, 100.0, tolerance::SCALE),
            rotation: timeline.fit_number(id, Property::Rotation, hints, 1.0, tolerance::ANGLE),
            opacity: self.opacity(id, hints),
        }
    }

    /// The opacity of a layer in percent, 0 while `show` and `hide` keep it
    /// hidden.
    pub fn opacity(&self, layer: &NodeId, hints: &Hints) -> Animated {
        let timeline = self.timeline;
        let at = |frame: f64| {
            let shown = timeline.visible(layer, frame);
            let opacity = timeline.number(layer, Property::Opacity, frame);
            vec![if shown { opacity } else { 0.0 }]
        };
        timeline.fit(
            &at,
            hints,
            tolerance::OPACITY,
            places_for(tolerance::OPACITY),
        )
    }

    /// The shape items of a rectangle, ellipse, star or path layer: its
    /// outline, filled and stroked.
    fn shape_items(&mut self, node: &Node, hints: &Hints) -> Vec<Value> {
        self.paint_items(node, hints, Parts::BOTH)
    }

    /// The outline of a layer, painted with the `parts` of its paint that
    /// are asked for. Notes what cannot be painted exactly.
    pub fn paint_items(&mut self, node: &Node, hints: &Hints, parts: Parts) -> Vec<Value> {
        let this = &*self;
        let Some(geometry) = this.geometry(node) else {
            return Vec::new();
        };
        let rule = match &node.data {
            NodeData::Path(path) => path.path.fill_rule,
            _ => FillRule::NonZero,
        };
        let (items, notes) = this.painted(node, hints, &geometry, rule, parts);
        drop(geometry);
        self.note_paints(node, &notes);
        items
    }

    /// What the outline of a layer is drawn as: a group's box is a
    /// rectangle like a rectangle layer's.
    fn geometry<'g>(&'g self, node: &'g Node) -> Option<Geometry<'g>> {
        let id = &node.id;
        let timeline = self.timeline;
        Some(match &node.data {
            NodeData::Rect(_)
            | NodeData::Group(_)
            | NodeData::Component(_)
            | NodeData::Image(_) => {
                let uniform = self.survey.iter().all(|frame| {
                    let radii = timeline.radii(id, *frame);
                    radii.iter().all(|radius| *radius == radii[0])
                });
                if uniform {
                    Geometry::Rect
                } else {
                    Geometry::Contours(Box::new(move |frame| {
                        let (width, height) = self.size(id, frame);
                        vec![shapes::rounded_rect(
                            width,
                            height,
                            timeline.radii(id, frame),
                        )]
                    }))
                }
            }
            NodeData::Ellipse(ellipse) => {
                let (start, sweep, inner) =
                    (ellipse.start_angle, ellipse.sweep, ellipse.inner_radius);
                if shapes::ellipse(1.0, 1.0, start, sweep, inner).is_none() {
                    Geometry::Ellipse
                } else {
                    Geometry::Contours(Box::new(move |frame| {
                        let (width, height) = self.size(id, frame);
                        shapes::ellipse(width, height, start, sweep, inner).unwrap_or_default()
                    }))
                }
            }
            NodeData::Star(star) => {
                let (points, inner) = (star.points, star.inner_radius);
                Geometry::Contours(Box::new(move |frame| {
                    let (width, height) = self.size(id, frame);
                    shapes::star(width, height, points, inner)
                }))
            }
            NodeData::Path(path) => Geometry::Contours(Box::new(move |frame| {
                let (width, height) = self.size(id, frame);
                shapes::pen(&path.path.scaled(width, height))
            })),
            _ => return None,
        })
    }

    /// Geometry painted with the layer's fill and stroke.
    fn painted(
        &self,
        node: &Node,
        hints: &Hints,
        geometry: &Geometry<'_>,
        rule: FillRule,
        parts: Parts,
    ) -> (Vec<Value>, Vec<Notes>) {
        let id = &node.id;
        let timeline = self.timeline;
        let size = |frame: f64| self.size(id, frame);
        let fill_paint = |frame: f64| timeline.fill(id, frame).filter(|_| parts.fill);
        let fill = paint::item(
            &Source {
                timeline,
                hints,
                paint: &fill_paint,
                size: &size,
                survey: &self.survey,
            },
            &Role::Fill(rule),
        );
        let mut notes = vec![fill.1];

        let first_stroke = self
            .survey
            .iter()
            .filter(|_| parts.stroke)
            .find_map(|frame| timeline.stroke(id, *frame));
        let mut stroke_item = None;
        let mut inset: Box<dyn Fn(f64) -> f64> = Box::new(|_| 0.0);
        let mut position = StrokePosition::Center;
        if let Some(first) = &first_stroke {
            let width_at = |frame: f64| {
                vec![
                    timeline
                        .stroke(id, frame)
                        .map_or(0.0, |stroke| stroke.width),
                ]
            };
            let width = timeline.fit(
                &width_at,
                hints,
                tolerance::LENGTH,
                places_for(tolerance::LENGTH),
            );
            let stroke_paint = |frame: f64| timeline.stroke(id, frame).map(|stroke| stroke.paint);
            let (item, stroke_notes) = paint::item(
                &Source {
                    timeline,
                    hints,
                    paint: &stroke_paint,
                    size: &size,
                    survey: &self.survey,
                },
                &Role::Stroke {
                    width: &width,
                    cap: first.cap,
                    join: first.join,
                },
            );
            notes.push(stroke_notes);
            stroke_item = item;
            position = first.position;
            if matches!(geometry, Geometry::Rect | Geometry::Ellipse) {
                let sign = match position {
                    StrokePosition::Inside => 0.5,
                    StrokePosition::Outside => -0.5,
                    StrokePosition::Center => 0.0,
                };
                inset = Box::new(move |frame| {
                    sign * timeline
                        .stroke(id, frame)
                        .map_or(0.0, |stroke| stroke.width)
                });
            }
        }

        if fill.0.is_none() && stroke_item.is_none() {
            // Nothing is ever painted.
            return (Vec::new(), notes);
        }
        let join = first_stroke.as_ref().map_or(StrokeJoin::Miter, |s| s.join);
        let offset = position != StrokePosition::Center
            && matches!(geometry, Geometry::Rect | Geometry::Ellipse)
            && stroke_item.is_some();
        let mut items = Vec::new();
        if offset {
            // The stroke follows an outline moved half its width in or out,
            // so that it lies on its side of the edge.
            let stroke_shape = self.geometry_items(node, hints, geometry, &*inset, join);
            let mut group = stroke_shape;
            group.extend(stroke_item);
            items.push(group_item(group));
            if let Some(fill) = fill.0 {
                let mut group = self.geometry_items(node, hints, geometry, &|_| 0.0, join);
                group.push(fill);
                items.push(group_item(group));
            }
        } else {
            items.extend(self.geometry_items(node, hints, geometry, &|_| 0.0, join));
            items.extend(stroke_item);
            items.extend(fill.0);
        }
        if position != StrokePosition::Center && !offset && first_stroke.is_some() {
            notes.push(Notes {
                centred_stroke: true,
                ..Notes::default()
            });
        }
        (items, notes)
    }

    /// The shape items of `geometry`, moved `inset(frame)` pixels inward
    /// (outward when negative) for strokes inside or outside the edge.
    fn geometry_items(
        &self,
        node: &Node,
        hints: &Hints,
        geometry: &Geometry<'_>,
        inset: &dyn Fn(f64) -> f64,
        join: StrokeJoin,
    ) -> Vec<Value> {
        let id = &node.id;
        let timeline = self.timeline;
        let length = places_for(tolerance::LENGTH);
        let fit = |at: &dyn Fn(f64) -> Vec<f64>| timeline.fit(at, hints, tolerance::LENGTH, length);
        let center = |frame: f64| {
            let (width, height) = self.size(id, frame);
            vec![width / 2.0, height / 2.0]
        };
        let size = |frame: f64| {
            let (width, height) = self.size(id, frame);
            let inset = inset(frame);
            vec![
                (width - 2.0 * inset).max(0.0),
                (height - 2.0 * inset).max(0.0),
            ]
        };
        match geometry {
            Geometry::Rect => {
                let radius = |frame: f64| {
                    let (width, height) = self.size(id, frame);
                    let limit = (width.min(height) / 2.0).max(0.0);
                    let radius = timeline.radii(id, frame)[0].clamp(0.0, limit);
                    let inset = inset(frame);
                    vec![if inset >= 0.0 {
                        (radius - inset).max(0.0)
                    } else if radius > 0.0 || join == StrokeJoin::Round {
                        radius - inset
                    } else {
                        0.0
                    }]
                };
                let mut item = Map::new();
                item.insert("ty".into(), Value::from("rc"));
                item.insert("p".into(), property(&fit(&center), places::LENGTH));
                item.insert("s".into(), property(&fit(&size), places::LENGTH));
                item.insert("r".into(), property(&fit(&radius), places::LENGTH));
                vec![Value::Object(item)]
            }
            Geometry::Ellipse => vec![object([
                ("ty", Value::from("el")),
                ("p", property(&fit(&center), places::LENGTH)),
                ("s", property(&fit(&size), places::LENGTH)),
            ])],
            Geometry::Contours(contours) => {
                let template = contours(0.0);
                let numbers = |frame: f64| shapes::flatten(&contours(frame));
                let outline = Outline {
                    numbers: fit(&numbers),
                    template,
                };
                outline.items(places::LENGTH)
            }
        }
    }

    /// Turns what painting noted into warnings about `node`.
    pub fn note_paints(&mut self, node: &Node, notes: &[Notes]) {
        for note in notes {
            if note.mixed {
                self.warnings.layer(
                    node,
                    "changes between a linear and a radial gradient; Lottie cannot, so it keeps \
                     the kind it shows longest.",
                );
            }
            if note.stretched {
                self.warnings.layer(
                    node,
                    "has a stretched radial gradient; Lottie's radial gradients are round, so it \
                     is drawn as a circle.",
                );
            }
            if note.centred_stroke {
                self.warnings.layer(
                    node,
                    "has a stroke inside or outside its edge; Lottie draws it centred on the edge.",
                );
            }
        }
    }

    /// A shape layer filling the artboard with `paint`, for the background.
    pub fn background(
        &self,
        artboard: &Node,
        comp: &mut Comp,
        parent_index: Option<i64>,
        over: Option<Color>,
    ) {
        let id = &artboard.id;
        let timeline = self.timeline;
        let hints = timeline.hints(id);
        let size = |frame: f64| self.size(id, frame);
        let rect = |frame: f64| {
            let (width, height) = size(frame);
            vec![width / 2.0, height / 2.0, width, height]
        };
        let geometry = timeline.fit(
            &rect,
            &hints,
            tolerance::LENGTH,
            places_for(tolerance::LENGTH),
        );
        let rect_item = |geometry: &Animated| {
            let half = super::props::map(geometry, |v| v[..2].to_vec());
            let whole = super::props::map(geometry, |v| v[2..].to_vec());
            object([
                ("ty", Value::from("rc")),
                ("p", property(&half, places::LENGTH)),
                ("s", property(&whole, places::LENGTH)),
                ("r", fixed_property(Value::from(0))),
            ])
        };
        let add = |name: &str, fill: Value, comp: &mut Comp| {
            let index = comp.index();
            let mut layer = layer_start(4);
            layer.insert("ind".into(), Value::from(index));
            if let Some(parent) = parent_index {
                layer.insert("parent".into(), Value::from(parent));
            }
            layer.insert("nm".into(), Value::from(name));
            layer.insert("ks".into(), object([]));
            layer.insert("ip".into(), Value::from(0));
            layer.insert("op".into(), num(timeline.clock.out_point(), places::TIME));
            layer.insert("st".into(), Value::from(0));
            layer.insert(
                "shapes".into(),
                Value::Array(vec![rect_item(&geometry), fill]),
            );
            comp.layers.push(Value::Object(layer));
        };
        let fill_paint = |frame: f64| timeline.fill(id, frame);
        let (fill, _) = paint::item(
            &Source {
                timeline,
                hints: &hints,
                paint: &fill_paint,
                size: &size,
                survey: &self.survey,
            },
            &Role::Fill(FillRule::NonZero),
        );
        if let Some(fill) = fill {
            add("Background", fill, comp);
        }
        if let Some(color) = over {
            let [r, g, b, a] = [color.r, color.g, color.b, color.a].map(|c| f64::from(c) / 255.0);
            let fill = object([
                ("ty", Value::from("fl")),
                ("c", fixed_property(array(&[r, g, b], places::COLOR))),
                ("o", fixed_property(num(a * 100.0, places::PERCENT))),
            ]);
            add("Background colour", fill, comp);
        }
    }
}

/// Which parts of a layer's paint to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Parts {
    /// The fill.
    pub fill: bool,
    /// The stroke.
    pub stroke: bool,
}

impl Parts {
    /// Fill and stroke.
    pub const BOTH: Parts = Parts {
        fill: true,
        stroke: true,
    };
    /// The fill only.
    pub const FILL: Parts = Parts {
        fill: true,
        stroke: false,
    };
    /// The stroke only.
    pub const STROKE: Parts = Parts {
        fill: false,
        stroke: true,
    };
}

/// What a shape is drawn as.
enum Geometry<'g> {
    /// A Lottie rectangle with one corner radius.
    Rect,
    /// A Lottie ellipse.
    Ellipse,
    /// Paths, worked out at each frame.
    Contours(Box<dyn Fn(f64) -> Vec<Contour> + 'g>),
}

/// The first keys of every Lottie layer: its kind `ty`, and `ddd` 0 for a
/// flat layer. rlottie reads a layer without `ddd` as a 3D one, which turns
/// by `rz` and ignores `r`, and reads the keys in order, so `ddd` comes
/// before the transform.
pub(crate) fn layer_start(ty: u8) -> Map<String, Value> {
    let mut layer = Map::new();
    layer.insert("ty".into(), Value::from(ty));
    layer.insert("ddd".into(), Value::from(0));
    layer
}

/// A shape group holding `items`, with the transform it must end with.
pub(crate) fn group_item(mut items: Vec<Value>) -> Value {
    items.push(Transform::default().to_group_item());
    object([("ty", Value::from("gr")), ("it", Value::Array(items))])
}

/// Lottie's number for a blend mode.
fn blend_code(mode: moshionly_core::model::BlendMode) -> u8 {
    use moshionly_core::model::BlendMode as B;
    match mode {
        B::Normal => 0,
        B::Multiply => 1,
        B::Screen => 2,
        B::Overlay => 3,
        B::Darken => 4,
        B::Lighten => 5,
        B::ColorDodge => 6,
        B::ColorBurn => 7,
        B::HardLight => 8,
        B::SoftLight => 9,
        B::Difference => 10,
        B::Exclusion => 11,
        B::Hue => 12,
        B::Saturation => 13,
        B::Color => 14,
        B::Luminosity => 15,
    }
}

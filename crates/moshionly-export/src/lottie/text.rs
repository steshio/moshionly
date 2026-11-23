//! Text: outlines of the letters, made with the renderer's own fonts, so
//! the text looks the same in every player without any font files.
//!
//! The outlines come from the renderer itself: the text is drawn onto
//! Skia's SVG canvas, which turns glyphs into paths, and the paths are read
//! back. Layout, fonts, fallback fonts and letter spacing are therefore
//! exactly what the renderer draws.
//!
//! Text animations (`text_in`, `text_out`) move pieces of the text on their
//! own. Each piece (a letter, word or line, as finely as the animations
//! cut) becomes a shape group whose transform and opacity follow the piece
//! frame by frame, fitted with keyframes like any other value.
//!
//! A counter changes the text itself. Each value it shows is a different
//! set of outlines, shown for its stretch of time. The outlines of each
//! letter are made once and reused: every letter is a small composition,
//! placed where it appears in each value.

use std::collections::HashMap;
use std::ops::Range;

use moshionly_core::eval::Property;
use moshionly_core::model::{
    BlendMode, Paint, StrokePosition, TextAnimation, TextLayer, TextOrder, TextSplit,
};
use moshionly_core::scene::{Content, SceneNode, ScenePaint, TextContent, TextUnit};
use moshionly_core::text::{TextMeasurer, TextRequest, split};
use moshionly_core::{Affine, Color, Node, NodeData, NodeId, Point, Scene};
use moshionly_render::skia_safe as sk;
use serde_json::{Map, Value};

use crate::error::ExportError;

use super::fit::{Animated, Hints};
use super::json::{num, object, places};
use super::layers::{Comp, Converter};
use super::paint::{self, Role, Source};
use super::props::{Position, Transform};
use super::shapes::{Contour, Outline};
use super::svg;
use super::timeline::{places_for, tolerance};

/// How far glyphs are moved into the SVG canvas while they are read, so
/// that letters above or left of the text box are not cut off.
const SHIFT: f64 = 20_000.0;

/// Room left around each letter's outline in its composition, in pixels.
const PAD: f64 = 2.0;

/// The text a layer shows over a stretch of time.
struct State {
    /// The first frame it shows.
    start: f64,
    /// The frame after the last one it shows.
    end: f64,
    /// A moment of the stretch, in frames, where the text is drawn.
    frame: f64,
    /// The text node as a scene has it at that moment.
    node: SceneNode,
    /// Its text and layout.
    content: TextContent,
}

/// One `text_in` or `text_out`.
struct Effect<'n> {
    animation: &'n TextAnimation,
    start: f64,
    end: f64,
}

impl<'a> Converter<'a> {
    /// Adds a text layer: its letters as outlines.
    pub fn add_text(
        &mut self,
        node: &Node,
        text: &TextLayer,
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) -> Result<(), ExportError> {
        let states = self.states(node, text);
        match states.as_slice() {
            [] => Ok(()),
            [state] => {
                self.add_still_text(node, state, comp, parent_index, offset);
                Ok(())
            }
            _ => self.add_counting_text(node, &states, comp, parent_index, offset),
        }
    }

    /// The texts a layer shows over the export, each with its stretch of
    /// time. Usually one; a counter shows a new text whenever its number
    /// changes.
    fn states(&self, node: &Node, text: &TextLayer) -> Vec<State> {
        let timeline = self.timeline;
        let content = |frame: f64| match timeline.value(
            &node.id,
            Property::Content,
            timeline.clock.ms(frame),
        ) {
            Some(moshionly_core::eval::PropertyValue::Text(typed)) => text.case.apply(&typed),
            _ => text.case.apply(&text.content),
        };
        // Where the text changes: between two frames, found to a
        // thousandth of a frame.
        let mut stretches: Vec<(f64, String)> = Vec::new();
        for frame in &self.survey {
            let now = content(*frame);
            match stretches.last() {
                Some((_, last)) if *last == now => {}
                Some((start, last)) => {
                    let (mut low, mut high) = (frame - 1.0, *frame);
                    low = low.max(*start);
                    while high - low > 1e-3 {
                        let middle = (low + high) / 2.0;
                        if content(middle) == *last {
                            low = middle;
                        } else {
                            high = middle;
                        }
                    }
                    stretches.push((json_round(high), now));
                }
                None => stretches.push((0.0, now)),
            }
        }
        let out = timeline.clock.out_point();
        let mut states = Vec::new();
        for (position, (start, _)) in stretches.iter().enumerate() {
            let end = stretches.get(position + 1).map_or(out, |(next, _)| *next);
            // A moment of the stretch where the text is drawn.
            let found = self
                .survey
                .iter()
                .copied()
                .chain([*start])
                .filter(|frame| *frame >= *start && *frame < end)
                .find_map(|frame| {
                    text_node(timeline.scenes.at(frame).as_deref(), &node.id)
                        .map(|(node, content)| (frame, node, content))
                });
            if let Some((frame, node, content)) = found {
                states.push(State {
                    start: *start,
                    end,
                    frame,
                    node,
                    content,
                });
            }
        }
        states
    }

    /// The outlines of the letters of `content` in `range`, in the text
    /// node's own space, drawn by the renderer.
    fn outlines(
        &mut self,
        node: &Node,
        scene_node: &SceneNode,
        content: &TextContent,
        range: Range<usize>,
    ) -> Vec<Contour> {
        let mut content = content.clone();
        content.fill = Some(ScenePaint::solid(Color::BLACK));
        content.stroke = None;
        content.units = vec![TextUnit {
            start: range.start,
            end: range.end,
            transform: Affine::IDENTITY,
            opacity: 1.0,
            blur: 0.0,
        }];
        let drawn = SceneNode {
            id: node.id.clone(),
            transform: Affine::translate(SHIFT, SHIFT),
            width: scene_node.width,
            height: scene_node.height,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            blur: 0.0,
            shadow: None,
            clip: None,
            mask: None,
            content: Content::Text(content),
        };
        let scene = Scene {
            artboard: node.id.clone(),
            time_ms: 0.0,
            width: 2.0 * SHIFT,
            height: 2.0 * SHIFT,
            background: None,
            nodes: vec![drawn],
        };
        let bounds = sk::Rect::from_wh((2.0 * SHIFT) as f32, (2.0 * SHIFT) as f32);
        let flags =
            sk::svg::canvas::Flags::CONVERT_TEXT_TO_PATHS | sk::svg::canvas::Flags::NO_PRETTY_XML;
        let canvas = sk::svg::Canvas::new(bounds, Some(flags));
        let warnings = self.renderer.render_into(&scene, &canvas);
        for warning in warnings {
            self.warnings
                .layer(node, &format!("is drawn differently: {warning}."));
        }
        let data = canvas.end();
        let text = String::from_utf8_lossy(data.as_bytes());
        svg::paths(&text)
            .into_iter()
            .map(|contour| moved(contour, [-SHIFT, -SHIFT]))
            .collect()
    }

    /// A text that does not change: one shape layer, with a group for each
    /// piece that text animations move.
    fn add_still_text(
        &mut self,
        node: &Node,
        state: &State,
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) {
        let hints = self.timeline.hints(&node.id);
        let transform = self.text_transform(node, &hints, offset, &state.node);
        let paints = self.text_paints(node, &hints, &state.node);
        let effects = self.text_effects(&node.id);
        let mut shapes = Vec::new();
        if effects.is_empty() {
            let contours = self.outlines(
                node,
                &state.node,
                &state.content,
                0..state.content.text.len(),
            );
            shapes.extend(static_paths(&contours));
            shapes.extend(paints.iter().cloned());
        } else {
            let layout = self.renderer.fonts().measure(&TextRequest {
                text: &state.content.text,
                style: &state.content.style,
                wrap_width: state.content.wrap_width,
            });
            let finest = effects
                .iter()
                .map(|effect| effect.animation.split)
                .min_by_key(|split| fineness(*split))
                .unwrap_or(TextSplit::Letters);
            let atoms = split(&state.content.text, &layout, finest);
            let mut blurred = false;
            // Lottie draws the first group in front; the text reads
            // forward, and pieces do not overlap, so the order is free.
            for atom in &atoms {
                let contours =
                    self.outlines(node, &state.node, &state.content, atom.start..atom.end);
                if contours.is_empty() {
                    continue;
                }
                let mut atom_hints = Hints::default();
                for effect in &effects {
                    let pieces = split(&state.content.text, &layout, effect.animation.split);
                    let Some(position) = pieces
                        .iter()
                        .position(|piece| piece.start <= atom.start && atom.start < piece.end)
                    else {
                        continue;
                    };
                    let position = match effect.animation.order {
                        TextOrder::Forward => position,
                        TextOrder::Backward => pieces.len() - 1 - position,
                    };
                    let (start, end) = slot(
                        effect.start,
                        effect.end,
                        effect.animation.stagger,
                        position,
                        pieces.len(),
                    );
                    let clock = self.timeline.clock;
                    atom_hints.add(
                        clock.frame(start),
                        clock.frame(end),
                        effect.animation.timing.easing,
                    );
                }
                let (piece, blur) = self.piece_transform(&node.id, atom.start, &atom_hints);
                blurred |= blur;
                let mut items = static_paths(&contours);
                items.extend(paints.iter().cloned());
                items.push(piece.to_group_item());
                shapes.push(object([
                    ("ty", Value::from("gr")),
                    ("it", Value::Array(items)),
                ]));
            }
            if blurred {
                self.warnings.layer(
                    node,
                    "has letters that blur as they come or go; Lottie cannot blur part of a \
                     layer, so they come and go sharp.",
                );
            }
        }
        let index = comp.index();
        let mut layer = self.layer(node, 4, index, parent_index, &transform);
        layer.insert("shapes".into(), Value::Array(shapes));
        self.add_masks(node, &mut layer, [0.0, 0.0], super::groups::Masks::REVEAL);
        comp.layers.push(Value::Object(layer));
    }

    /// The `text_in` and `text_out` animations of a layer, in milliseconds.
    fn text_effects(&self, id: &NodeId) -> Vec<Effect<'a>> {
        self.timeline
            .document
            .animations_targeting(id)
            .into_iter()
            .filter_map(|animation| match &animation.data {
                NodeData::TextIn(text) | NodeData::TextOut(text) => Some(Effect {
                    animation: text,
                    start: text.timing.start,
                    end: text.timing.end,
                }),
                _ => None,
            })
            .collect()
    }

    /// The transform of a text layer: Moshionly's turn, scale and opacity,
    /// about the box the text is measured into, which can differ from the
    /// stored box when the box follows the text.
    fn text_transform(
        &self,
        node: &Node,
        hints: &Hints,
        offset: [f64; 2],
        rest: &SceneNode,
    ) -> Transform {
        let id = &node.id;
        let timeline = self.timeline;
        let length = places_for(tolerance::LENGTH);
        let drawn =
            |frame: f64| text_node(timeline.scenes.at(frame).as_deref(), id).map(|(n, _)| n);
        let box_center = |node: &SceneNode| {
            node.transform
                .apply(Point::new(node.width / 2.0, node.height / 2.0))
        };
        let stored_center = |frame: f64| {
            let (width, height) = self.size(id, frame);
            Point::new(
                timeline.number(id, Property::X, frame) + width / 2.0,
                timeline.number(id, Property::Y, frame) + height / 2.0,
            )
        };
        // Where the measured box sits from the stored one when the text is
        // not drawn: as at the moment it was.
        let rest_frame = 0.0;
        let rest_shift = {
            let measured = box_center(rest);
            let stored = stored_center(rest_frame);
            [measured.x - stored.x, measured.y - stored.y]
        };
        let center = |frame: f64| match drawn(frame) {
            Some(node) => {
                let center = box_center(&node);
                [center.x, center.y]
            }
            None => {
                let stored = stored_center(frame);
                [stored.x + rest_shift[0], stored.y + rest_shift[1]]
            }
        };
        let x = timeline.fit(
            &|frame| vec![center(frame)[0] + offset[0]],
            hints,
            tolerance::LENGTH,
            length,
        );
        let y = timeline.fit(
            &|frame| vec![center(frame)[1] + offset[1]],
            hints,
            tolerance::LENGTH,
            length,
        );
        let anchor = |frame: f64| {
            let node = drawn(frame).unwrap_or_else(|| rest.clone());
            vec![node.width / 2.0, node.height / 2.0]
        };
        Transform {
            anchor: timeline.fit(&anchor, hints, tolerance::LENGTH, length),
            position: Position::from_parts(x, y),
            scale: timeline.fit_number(id, Property::Scale, hints, 100.0, tolerance::SCALE),
            rotation: timeline.fit_number(id, Property::Rotation, hints, 1.0, tolerance::ANGLE),
            opacity: self.opacity(id, hints),
        }
    }

    /// The fill and stroke items of a text, to put in each of its groups.
    fn text_paints(&mut self, node: &Node, hints: &Hints, rest: &SceneNode) -> Vec<Value> {
        let id = &node.id;
        let timeline = self.timeline;
        let size = |frame: f64| match text_node(timeline.scenes.at(frame).as_deref(), id) {
            Some((node, _)) => (node.width, node.height),
            None => (rest.width, rest.height),
        };
        let fill_paint = |frame: f64| timeline.fill(id, frame);
        let (fill, fill_notes) = paint::item(
            &Source {
                timeline,
                hints,
                paint: &fill_paint,
                size: &size,
                survey: &self.survey,
            },
            &Role::Fill(moshionly_core::model::FillRule::NonZero),
        );
        let first_stroke = self
            .survey
            .iter()
            .find_map(|frame| timeline.stroke(id, *frame));
        let mut items = Vec::new();
        let mut notes = vec![fill_notes];
        if let Some(first) = first_stroke {
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
            let (stroke, stroke_notes) = paint::item(
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
            items.extend(stroke);
            notes.push(stroke_notes);
            if first.position != StrokePosition::Center {
                self.warnings.layer(
                    node,
                    "has a stroke inside or outside its letters; Lottie draws it centred on \
                     their edges.",
                );
            }
        }
        items.extend(fill);
        self.note_paints(node, &notes);
        items
    }

    /// How one piece of text is moved, scaled and faded by its text
    /// animations, and whether it is ever blurred.
    fn piece_transform(&self, id: &NodeId, at: usize, hints: &Hints) -> (Transform, bool) {
        let timeline = self.timeline;
        // The unit the piece belongs to: its transform and opacity. Pieces
        // that no unit covers are not drawn at that moment.
        let unit = |frame: f64| -> (Affine, f64, f64) {
            match text_node(timeline.scenes.at(frame).as_deref(), id) {
                Some((_, content)) => content
                    .units
                    .iter()
                    .find(|unit| unit.start <= at && at < unit.end)
                    .map_or((Affine::IDENTITY, 0.0, 0.0), |unit| {
                        (unit.transform, unit.opacity, unit.blur)
                    }),
                None => (Affine::IDENTITY, 1.0, 0.0),
            }
        };
        let length = places_for(tolerance::LENGTH);
        let position = timeline.fit(
            &|frame| {
                let [_, _, _, _, e, f] = unit(frame).0.0;
                vec![e, f]
            },
            hints,
            tolerance::LENGTH,
            length,
        );
        let scale = timeline.fit(
            &|frame| vec![unit(frame).0.0[0] * 100.0],
            hints,
            tolerance::SCALE,
            places_for(tolerance::SCALE),
        );
        let opacity = timeline.fit(
            &|frame| vec![unit(frame).1 * 100.0],
            hints,
            tolerance::OPACITY,
            places_for(tolerance::OPACITY),
        );
        let blurred = self.survey.iter().any(|frame| unit(*frame).2 > 0.0);
        let transform = Transform {
            position: Position::Joined(position),
            scale,
            opacity,
            ..Transform::default()
        };
        (transform, blurred)
    }

    /// A text whose content changes, as a counter's does: each value is
    /// shown for its stretch of time, its letters placed from compositions
    /// made once per letter shape.
    fn add_counting_text(
        &mut self,
        node: &Node,
        states: &[State],
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) -> Result<(), ExportError> {
        let id = &node.id;
        let timeline = self.timeline;
        let hints = timeline.hints(id);
        // The layer turns and scales about its stored box; each value's
        // measured box is placed in it.
        let transform = self.transform(node, &hints, offset);
        let paints = self.counting_paints(node, &hints);
        if !self.text_effects(id).is_empty() {
            self.warnings.layer(
                node,
                "counts and has letters that animate; Lottie shows the count, and the letters \
                 without their animation.",
            );
        }

        /// A letter shown for a stretch of time.
        struct Placed {
            asset: String,
            at: [f64; 2],
            size: [f64; 2],
            start: f64,
            end: f64,
        }
        let mut letters: HashMap<String, (String, [f64; 2])> = HashMap::new();
        let mut placed: Vec<Placed> = Vec::new();
        for state in states {
            let frame = state.frame;
            let (width, height) = self.size(id, frame);
            let stored = Affine::layer(
                timeline.number(id, Property::X, frame),
                timeline.number(id, Property::Y, frame),
                width,
                height,
                timeline.number(id, Property::Rotation, frame),
                timeline.number(id, Property::Scale, frame),
            );
            // From the measured box to the stored one: a shift, when the
            // layer is not turning or scaling at that moment.
            let Some(to_stored) = stored
                .invert()
                .map(|inverse| inverse * state.node.transform)
            else {
                continue;
            };
            let layout = self.renderer.fonts().measure(&TextRequest {
                text: &state.content.text,
                style: &state.content.style,
                wrap_width: state.content.wrap_width,
            });
            for letter in split(&state.content.text, &layout, TextSplit::Letters) {
                let contours =
                    self.outlines(node, &state.node, &state.content, letter.start..letter.end);
                let Some([left, top, right, bottom]) = bounds(&contours) else {
                    continue;
                };
                let corner = [left - PAD, top - PAD];
                let shape: Vec<Contour> = contours
                    .into_iter()
                    .map(|contour| moved(contour, [-corner[0], -corner[1]]))
                    .collect();
                let items = static_paths(&shape);
                let key = Value::Array(items.clone()).to_string();
                let size = [right - left + 2.0 * PAD, bottom - top + 2.0 * PAD];
                let asset = match letters.get(&key) {
                    Some((asset, _)) => asset.clone(),
                    None => {
                        let mut shapes = items;
                        shapes.extend(paints.iter().cloned());
                        let asset = self.letter_composition(node, shapes);
                        letters.insert(key, (asset.clone(), size));
                        asset
                    }
                };
                let at = to_stored.apply(Point::new(corner[0], corner[1]));
                placed.push(Placed {
                    asset,
                    at: [at.x, at.y],
                    size,
                    start: state.start,
                    end: state.end,
                });
            }
        }

        let (width, height) = self.size(id, states[0].frame);
        let mut area = super::groups::Area::new(width, height);
        for letter in &placed {
            area.include(Point::new(letter.at[0], letter.at[1]));
            area.include(Point::new(
                letter.at[0] + letter.size[0],
                letter.at[1] + letter.size[1],
            ));
        }
        let area = area.rounded();
        let origin = area.origin();
        let asset = self.composition(node, |_, inner| {
            for letter in placed.iter().rev() {
                let index = inner.index();
                let mut layer = Map::new();
                layer.insert("ty".into(), Value::from(0));
                layer.insert("ind".into(), Value::from(index));
                layer.insert("refId".into(), Value::from(letter.asset.as_str()));
                layer.insert("w".into(), num(letter.size[0].ceil(), 0));
                layer.insert("h".into(), num(letter.size[1].ceil(), 0));
                let at = [letter.at[0] + origin[0], letter.at[1] + origin[1]];
                layer.insert(
                    "ks".into(),
                    object([(
                        "p",
                        object([("k", super::props::array(&at, places::LENGTH))]),
                    )]),
                );
                layer.insert("ip".into(), num(letter.start, places::TIME));
                layer.insert("op".into(), num(letter.end, places::TIME));
                layer.insert("st".into(), Value::from(0));
                inner.layers.push(Value::Object(layer));
            }
            Ok(())
        })?;
        let transform = super::groups::anchored(transform, origin);
        let index = comp.index();
        let mut layer = self.composition_layer(node, &asset, area, &transform, index, parent_index);
        self.add_masks(node, &mut layer, origin, super::groups::Masks::REVEAL);
        comp.layers.push(Value::Object(layer));
        Ok(())
    }

    /// A composition drawing one letter shape with the text's paints.
    fn letter_composition(&mut self, node: &Node, shapes: Vec<Value>) -> String {
        let out = self.timeline.clock.out_point();
        let layer = object([
            ("ty", Value::from(4)),
            ("ind", Value::from(1)),
            ("ks", object([])),
            ("ip", Value::from(0)),
            ("op", num(out, places::TIME)),
            ("st", Value::from(0)),
            ("shapes", Value::Array(shapes)),
        ]);
        let id = format!("letter_{}", self.assets.len());
        self.assets.push(object([
            ("id", Value::from(id.as_str())),
            ("nm", Value::from(format!("{} letter", node.name()))),
            ("layers", Value::Array(vec![layer])),
        ]));
        id
    }

    /// The fill and stroke of a counting text, in solid colours: its
    /// letters are drawn apart, so a gradient across the text cannot be
    /// kept.
    fn counting_paints(&mut self, node: &Node, hints: &Hints) -> Vec<Value> {
        let id = &node.id;
        let timeline = self.timeline;
        let mut gradient = false;
        let solid = |paint: Option<Paint>| -> Option<Paint> {
            match paint? {
                Paint::Gradient(gradient) => gradient
                    .stops()
                    .first()
                    .map(|stop| Paint::Solid(stop.color)),
                solid => Some(solid),
            }
        };
        for frame in &self.survey {
            let fill = timeline.fill(id, *frame);
            let stroke = timeline.stroke(id, *frame).map(|stroke| stroke.paint);
            gradient |= matches!(fill, Some(Paint::Gradient(_)))
                || matches!(stroke, Some(Paint::Gradient(_)));
        }
        if gradient {
            self.warnings.layer(
                node,
                "counts and is painted with a gradient; in Lottie each value is drawn letter by \
                 letter, so it takes the gradient's first colour.",
            );
        }
        let size = |_: f64| (1.0, 1.0);
        let fill_paint = |frame: f64| solid(timeline.fill(id, frame));
        let (fill, _) = paint::item(
            &Source {
                timeline,
                hints,
                paint: &fill_paint,
                size: &size,
                survey: &self.survey,
            },
            &Role::Fill(moshionly_core::model::FillRule::NonZero),
        );
        let mut items = Vec::new();
        if let Some(first) = self
            .survey
            .iter()
            .find_map(|frame| timeline.stroke(id, *frame))
        {
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
            let stroke_paint = |frame: f64| solid(timeline.stroke(id, frame).map(|s| s.paint));
            let (stroke, _) = paint::item(
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
            items.extend(stroke);
        }
        items.extend(fill);
        items
    }
}

/// The box around contours, points and handles: left, top, right, bottom.
fn bounds(contours: &[Contour]) -> Option<[f64; 4]> {
    let mut points = contours.iter().flat_map(|contour| {
        contour
            .vertices
            .iter()
            .zip(contour.ins.iter().zip(&contour.outs))
            .flat_map(|(vertex, (into, out))| {
                [
                    *vertex,
                    [vertex[0] + into[0], vertex[1] + into[1]],
                    [vertex[0] + out[0], vertex[1] + out[1]],
                ]
            })
    });
    let first = points.next()?;
    let mut edges = [first[0], first[1], first[0], first[1]];
    for [x, y] in points {
        edges = [
            edges[0].min(x),
            edges[1].min(y),
            edges[2].max(x),
            edges[3].max(y),
        ];
    }
    Some(edges)
}

/// The scene node of the text layer `id` in `scene`, and its text.
fn text_node(scene: Option<&Scene>, id: &NodeId) -> Option<(SceneNode, TextContent)> {
    let node = scene?.find(id)?;
    match &node.content {
        Content::Text(content) => Some((node.clone(), content.clone())),
        _ => None,
    }
}

/// Path items for contours that do not change.
fn static_paths(contours: &[Contour]) -> Vec<Value> {
    let outline = Outline {
        template: contours.to_vec(),
        numbers: Animated::Static(super::shapes::flatten(contours)),
    };
    outline.items(places::OUTLINE)
}

/// A contour moved by `offset`.
fn moved(mut contour: Contour, offset: [f64; 2]) -> Contour {
    for vertex in &mut contour.vertices {
        vertex[0] += offset[0];
        vertex[1] += offset[1];
    }
    contour
}

/// A frame number rounded as keyframe times are written.
fn json_round(frame: f64) -> f64 {
    super::json::round(frame, places::TIME)
}

/// How fine a split is: letters are the finest.
fn fineness(split: TextSplit) -> u8 {
    match split {
        TextSplit::Letters => 0,
        TextSplit::Words => 1,
        TextSplit::Lines => 2,
    }
}

/// When the piece in position `position` of `count` plays, for a text
/// animation from `start` to `end` milliseconds: pieces start `stagger`
/// apart and last equally long, the last ending with the animation, and
/// staggers that do not fit shrink until the pieces play in turn. This is
/// the rule evaluation follows; the keyframes are checked against
/// evaluation, so a difference would cost keyframes, not correctness.
fn slot(start: f64, end: f64, stagger: f64, position: usize, count: usize) -> (f64, f64) {
    let length = (end - start).max(0.0);
    let stagger = stagger.min(length / count.max(1) as f64);
    let each = length - stagger * count.saturating_sub(1) as f64;
    let begin = start + stagger * position as f64;
    (begin, begin + each)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_fill_the_animation() {
        assert_eq!(slot(1000.0, 2000.0, 100.0, 0, 5), (1000.0, 1600.0));
        assert_eq!(slot(1000.0, 2000.0, 100.0, 4, 5), (1400.0, 2000.0));
        assert_eq!(slot(0.0, 1000.0, 900.0, 1, 4), (250.0, 500.0));
    }
}

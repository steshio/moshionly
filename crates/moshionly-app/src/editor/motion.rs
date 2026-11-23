//! Animating on the canvas, and the paths layers travel.
//!
//! In the Design tab, dragging a layer moves it at rest. In the Animate
//! tab the canvas shows a moment of the animation, and a drag there means
//! "be over here by then": it makes a move action instead.
//!
//! - With a move action of the dragged layer selected, the drag sets where
//!   that move ends.
//! - Otherwise each dragged layer gets a new move action starting at the
//!   playhead, ending where the layer is dropped.
//!
//! Either way the playhead goes to the end of the move while dragging, so
//! the layer stays under the pointer, and the whole drag is one undo step.
//!
//! [`Editor::motion_path`] gives the path a layer's centre travels through
//! its moves and slides, for drawing on the canvas.

use egui::{Modifiers, Pos2, Vec2};
use moshionly_core::eval::{Property, PropertyValue, value_at};
use moshionly_core::model::NodeData;
use moshionly_core::{Edit, EditError, Family, NodeId, NodeType, Point, build};
use serde_json::json;

use super::Editor;
use super::actions::{ActionError, new_start};
use super::transform::{Item, locked_to_axis};

/// A drag on the canvas in the Animate tab, from start to end.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveDrag {
    /// Where the pointer went down.
    start: Pos2,
    /// Each move being set, with its layer as the drag found it and the
    /// offset the move ended on before the drag.
    moves: Vec<(NodeId, Item, (f64, f64))>,
    /// Whether the moves were made by this drag.
    created: bool,
}

impl MoveDrag {
    /// The move actions the drag sets.
    pub fn actions(&self) -> Vec<NodeId> {
        self.moves.iter().map(|(id, ..)| id.clone()).collect()
    }

    /// Whether the drag made the moves it sets.
    pub fn created(&self) -> bool {
        self.created
    }
}

/// How many points a motion path is sampled at, besides the ends of its
/// actions.
const PATH_SAMPLES: usize = 48;

/// The path a layer's centre travels, on the canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionPath {
    /// The centre at each sampled moment, in order.
    pub points: Vec<Pos2>,
    /// Where the centre is when each of its moves starts and ends.
    pub stops: Vec<Pos2>,
}

/// The `to` offset of a move action, zero when it is left out.
fn move_to(data: &NodeData) -> Option<(f64, f64)> {
    match data {
        NodeData::Move(movement) => Some(movement.to.map_or((0.0, 0.0), |to| (to.x, to.y))),
        _ => None,
    }
}

/// Whether an action changes where its layer is.
fn moves_layer(ty: NodeType) -> bool {
    matches!(ty, NodeType::Move | NodeType::SlideIn | NodeType::SlideOut)
}

impl Editor {
    /// Starts dragging the selection on the canvas in the Animate tab, from
    /// `world`. Opens the gesture, makes the moves when needed, selects them
    /// and puts the playhead at their end. `None` when nothing selected can
    /// move.
    pub fn begin_animate_move(&mut self, world: Pos2) -> Result<Option<MoveDrag>, ActionError> {
        let items: Vec<Item> = self
            .drag_items()
            .into_iter()
            .filter(|item| !item.artboard)
            .collect();
        if items.is_empty() {
            return Ok(None);
        }
        let document = std::sync::Arc::clone(&self.document);
        // The selected move of the one dragged layer is the one to set.
        let chosen = match (self.single_action(), items.as_slice()) {
            (Some(action), [item]) => document
                .get(action)
                .filter(|node| node.target() == Some(&item.id))
                .and_then(|node| Some((node.id.clone(), move_to(&node.data)?, node.data.span()?))),
            _ => None,
        };
        if let Some((action, to, (_, end))) = chosen {
            self.begin_gesture("Change move");
            self.set_playhead(end);
            let item = items.into_iter().next().expect("one item");
            return Ok(Some(MoveDrag {
                start: world,
                moves: vec![(action, item, to)],
                created: false,
            }));
        }
        let scene = document
            .artboard_of(&items[0].id)
            .and_then(|artboard| moshionly_core::eval::duration_of(&document, &artboard.id))
            .unwrap_or(0.0);
        let length = build::default_duration(NodeType::Move);
        let start = new_start(self.playhead, length, scene);
        let mut scratch = (*document).clone();
        let mut edits = Vec::new();
        let mut moves = Vec::new();
        for item in items {
            let Some(mut data) = build::default_animation(NodeType::Move, &item.id, start) else {
                continue;
            };
            if let NodeData::Move(movement) = &mut data {
                movement.to = None;
            }
            let (id, step) = build::add_animation(&scratch, data, None)?;
            scratch.apply(&step)?;
            edits.extend(step);
            moves.push((id, item, (0.0, 0.0)));
        }
        self.begin_gesture("Add move");
        if let Err(error) = self.apply("Add move", &edits) {
            let _ = self.cancel_gesture();
            return Err(error.into());
        }
        self.actions = moves.iter().map(|(id, ..)| id.clone()).collect();
        self.set_playhead(start + length);
        Ok(Some(MoveDrag {
            start: world,
            moves,
            created: true,
        }))
    }

    /// One step of a drag in the Animate tab with the pointer at `world`:
    /// each move ends the distance dragged further on. Shift keeps to one
    /// axis.
    pub fn drag_animate_move(
        &mut self,
        drag: &MoveDrag,
        world: Pos2,
        modifiers: Modifiers,
    ) -> Result<bool, EditError> {
        let mut delta: Vec2 = world - drag.start;
        if modifiers.shift {
            delta = locked_to_axis(delta);
        }
        let edits: Vec<Edit> = drag
            .moves
            .iter()
            .map(|(id, item, (x, y))| {
                let world = Point::new(f64::from(delta.x), f64::from(delta.y));
                let local = item
                    .parent
                    .invert()
                    .map_or(world, |inverse| inverse.apply_vector(world));
                let to = json!({ "x": tidy(x + local.x), "y": tidy(y + local.y) });
                Edit::set(id.clone(), "to", to)
            })
            .collect();
        self.apply("Move", &edits)
    }

    /// Ends a drag in the Animate tab as one undo step. A drag that made
    /// moves but never went anywhere takes them away again.
    pub fn end_animate_move(&mut self, drag: &MoveDrag) {
        self.sync();
        let still = drag.moves.iter().all(|(id, _, before)| {
            self.document
                .get(id)
                .and_then(|node| move_to(&node.data))
                .is_none_or(|to| to == *before)
        });
        if still && drag.created && self.cancel_gesture().is_ok() {
            self.actions.retain(|id| self.document.contains(id));
            return;
        }
        self.commit_gesture();
    }

    /// The path `layer`'s centre takes through its moves and slides, on the
    /// canvas. `None` when nothing moves it.
    pub fn motion_path(&self, layer: &NodeId) -> Option<MotionPath> {
        let document = &self.document;
        let spans: Vec<(f64, f64)> = document
            .animations_targeting(layer)
            .into_iter()
            .filter(|node| moves_layer(node.node_type()))
            .filter_map(|node| node.data.span())
            .collect();
        let first = spans.iter().map(|(start, _)| *start).reduce(f64::min)?;
        let last = spans.iter().map(|(_, end)| *end).reduce(f64::max)?;
        let item = Item::of(document, layer)?;
        let artboard = document.artboard_of(layer)?;
        let origin = artboard.layer().map_or((0.0, 0.0), |a| (a.x, a.y));
        let number = |property, time| match value_at(document, layer, property, time) {
            Some(PropertyValue::Number(n)) => Some(n),
            _ => None,
        };
        let centre = |time: f64| -> Option<Pos2> {
            let x = number(Property::X, time)?;
            let y = number(Property::Y, time)?;
            let w = number(Property::Width, time)?;
            let h = number(Property::Height, time)?;
            let at = item.parent.apply(Point::new(x + w / 2.0, y + h / 2.0));
            Some(Pos2::new(
                (origin.0 + at.x) as f32,
                (origin.1 + at.y) as f32,
            ))
        };
        let mut times: Vec<f64> = (0..=PATH_SAMPLES)
            .map(|n| first + (last - first) * n as f64 / PATH_SAMPLES as f64)
            .collect();
        times.extend(spans.iter().flat_map(|(start, end)| [*start, *end]));
        times.sort_by(f64::total_cmp);
        times.dedup();
        let points: Vec<Pos2> = times.iter().filter_map(|t| centre(*t)).collect();
        let mut stops: Vec<Pos2> = spans
            .iter()
            .flat_map(|(start, end)| [*start, *end])
            .filter_map(centre)
            .collect();
        stops.dedup_by(|a, b| (*a - *b).length() < 0.5);
        Some(MotionPath { points, stops })
    }

    /// The motion paths of the selected layers that move.
    pub fn motion_paths(&self) -> Vec<MotionPath> {
        self.selection
            .ids()
            .iter()
            .filter(|id| {
                self.document
                    .get(id)
                    .is_some_and(|node| node.family() == Family::Layer)
            })
            .filter_map(|id| self.motion_path(id))
            .collect()
    }
}

/// Rounds away floating-point noise.
fn tidy(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::super::view::tests::editor;
    use super::*;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    fn to_of(editor: &Editor, action: &NodeId) -> (f64, f64) {
        move_to(&editor.document().get(action).unwrap().data).unwrap()
    }

    #[test]
    fn a_drag_with_no_move_selected_makes_one_from_the_playhead() {
        let mut editor = editor();
        editor.set_animating(true);
        editor.selection.select(id("loose"));
        editor.set_playhead(400.0);
        let drag = editor
            .begin_animate_move(pos2(720.0, 120.0))
            .unwrap()
            .unwrap();
        assert!(drag.created());
        let action = drag.actions()[0].clone();
        let node = editor.document().get(&action).unwrap();
        assert_eq!(node.node_type(), NodeType::Move);
        assert_eq!(node.data.span(), Some((400.0, 1200.0)));
        assert_eq!(editor.actions(), std::slice::from_ref(&action));
        // The playhead shows where the move ends.
        assert_eq!(editor.playhead(), 1200.0);
        for x in [740.0, 800.0, 870.0] {
            editor
                .drag_animate_move(&drag, pos2(x, 140.0), Modifiers::NONE)
                .unwrap();
        }
        editor.end_animate_move(&drag);
        assert_eq!(to_of(&editor, &action), (150.0, 20.0));
        // The layer itself has not moved at rest.
        let rest = editor
            .document()
            .get(&id("loose"))
            .unwrap()
            .layer()
            .unwrap()
            .x;
        assert_eq!(rest, 700.0);
        // One undo takes the whole move away.
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Add move"));
        editor.undo().unwrap();
        assert!(!editor.document().contains(&action));
    }

    #[test]
    fn a_drag_with_a_move_selected_sets_where_it_ends() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let action = editor.add_actions(NodeType::Move, 0.0).unwrap()[0].clone();
        assert_eq!(to_of(&editor, &action), (100.0, 0.0));
        let drag = editor
            .begin_animate_move(pos2(800.0, 120.0))
            .unwrap()
            .unwrap();
        assert!(!drag.created());
        assert_eq!(drag.actions(), vec![action.clone()]);
        editor
            .drag_animate_move(&drag, pos2(830.0, 180.0), Modifiers::SHIFT)
            .unwrap();
        editor.end_animate_move(&drag);
        // Shift kept it level.
        assert_eq!(to_of(&editor, &action), (100.0, 60.0));
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Change move"));
    }

    #[test]
    fn moves_inside_a_turned_group_follow_the_pointer() {
        let mut editor = editor();
        editor
            .apply("Turn", &[Edit::set("card", "rotation", 90.0)])
            .unwrap();
        editor.selection.select(id("dot"));
        let drag = editor.begin_animate_move(pos2(0.0, 0.0)).unwrap().unwrap();
        editor
            .drag_animate_move(&drag, pos2(0.0, 50.0), Modifiers::NONE)
            .unwrap();
        // Down on the canvas is along the group's own x axis, turned 90°.
        let (x, y) = to_of(&editor, &drag.actions()[0]);
        assert!((x - 50.0).abs() < 1e-6 && y.abs() < 1e-6, "{x}, {y}");
        editor.end_animate_move(&drag);
    }

    #[test]
    fn a_drag_that_goes_nowhere_leaves_nothing_behind() {
        let mut editor = editor();
        editor.selection.set([id("loose"), id("turned")]);
        let drag = editor
            .begin_animate_move(pos2(10.0, 10.0))
            .unwrap()
            .unwrap();
        assert_eq!(drag.actions().len(), 2);
        editor.end_animate_move(&drag);
        assert!(
            drag.actions()
                .iter()
                .all(|a| !editor.document().contains(a))
        );
        assert!(editor.actions().is_empty());
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn a_move_draws_a_path_from_where_it_starts_to_where_it_ends() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        assert!(editor.motion_path(&id("loose")).is_none());
        assert!(editor.motion_paths().is_empty());
        let action = editor.add_actions(NodeType::Move, 0.0).unwrap()[0].clone();
        editor
            .apply(
                "Set",
                &[Edit::set(action, "to", json!({"x": 100, "y": 50}))],
            )
            .unwrap();
        let path = editor.motion_path(&id("loose")).unwrap();
        // The loose rect is 50 by 50 at (700, 100) on an artboard at 0, 0.
        assert_eq!(path.points.first(), Some(&pos2(725.0, 125.0)));
        assert_eq!(path.points.last(), Some(&pos2(825.0, 175.0)));
        assert_eq!(path.stops, [pos2(725.0, 125.0), pos2(825.0, 175.0)]);
        assert!(path.points.len() > 40);
        assert_eq!(editor.motion_paths().len(), 1);
    }
}

//! Dragging the selection: moving it, resizing it from a handle, turning it.
//!
//! A drag opens a gesture, so however many steps it takes it is one undo
//! step. Each step is computed from where the drag started, never from the
//! previous step.

use std::sync::Arc;

use egui::{Modifiers, Pos2, Rect, Vec2};
use moshionly_core::{Document, EditError, Family, NodeId, Point, build};

use super::Editor;
use super::selection::is_held;
use super::snap::{SnapTargets, snap_point, snap_rect};
use super::transform::{
    Item, ResizeMode, WorldBox, angle, bounds_of, box_of, locked_to_axis, resize_edits, resized,
    rotate_edits, rotation_delta,
};
use crate::viewport::{Anchor, Handle};

/// How close, in screen points, something must come to a line to snap.
pub const SNAP_DISTANCE: f32 = 6.0;

/// A drag of the selection in progress.
#[derive(Clone, Debug)]
pub enum Drag {
    /// Moving.
    Move {
        /// Where the pointer went down.
        start: Pos2,
        /// The layers being moved, as they were.
        items: Vec<Item>,
        /// The rectangle around them, as it was.
        bounds: Rect,
        /// What they snap to.
        targets: SnapTargets,
    },
    /// Resizing from a handle.
    Resize {
        /// The handle.
        anchor: Anchor,
        /// Where the pointer went down.
        start: Pos2,
        /// The layers being resized, as they were.
        items: Vec<Item>,
        /// The box being resized, as it was.
        from: WorldBox,
        /// What the handle snaps to.
        targets: SnapTargets,
        /// The document as the drag found it: what groups being resized
        /// hold is stretched from there at every step.
        document: Arc<Document>,
    },
    /// Turning about the centre.
    Rotate {
        /// Where the pointer went down.
        start: Pos2,
        /// The layers being turned, as they were.
        items: Vec<Item>,
        /// The point they turn about.
        pivot: Pos2,
    },
}

/// Whether a drag snaps: Ctrl held turns snapping off.
pub fn snaps(modifiers: Modifiers) -> bool {
    !modifiers.ctrl
}

impl Editor {
    /// The selected nodes a drag works on: not locked (a locked artboard
    /// included), and not inside another selected node (which carries
    /// them along).
    pub(super) fn drag_ids(&self) -> Vec<NodeId> {
        let ids = self.selection.ids();
        ids.iter()
            .filter(|id| {
                !is_held(&self.document, id)
                    && !ids.iter().any(|other| self.document.is_ancestor(other, id))
            })
            .cloned()
            .collect()
    }

    /// The nodes a drag works on, as they are now.
    pub(super) fn drag_items(&self) -> Vec<Item> {
        self.drag_ids()
            .iter()
            .filter_map(|id| Item::of(&self.document, id))
            .collect()
    }

    /// Opens a gesture and gives the auto-sized text the drag works on the
    /// box it is drawn in, so that the drag starts from what is on screen;
    /// with `inside`, the text inside it too, which a resize may stretch.
    /// Returns false, with nothing changed, when that was refused.
    fn begin_drag_gesture(&mut self, summary: &str, inside: bool) -> bool {
        self.begin_gesture(summary);
        let ids = self.drag_ids();
        if self.settle(summary, &ids, inside).is_err() {
            let _ = self.cancel_gesture();
            return false;
        }
        self.drag_start = Some(Arc::clone(&self.document));
        true
    }

    /// What a drag of `items` snaps to: their artboards and every other
    /// layer in them. Artboards snap to the other artboards.
    pub fn snap_targets(&mut self, items: &[Item]) -> SnapTargets {
        let document = std::sync::Arc::clone(&self.document);
        let moving: Vec<&NodeId> = items.iter().map(|item| &item.id).collect();
        let carried = |id: &NodeId| {
            moving
                .iter()
                .any(|m| *m == id || document.is_ancestor(m, id))
        };
        let mut rects = Vec::new();
        let artboards: Vec<NodeId> = document.artboards().to_vec();
        for artboard in artboards {
            let Some(rect) = self.artboard_rect(&artboard) else {
                continue;
            };
            let mine = items
                .iter()
                .any(|item| item.id == artboard || document.is_ancestor(&artboard, &item.id));
            let moving_artboards = items.iter().any(|item| item.artboard);
            if moving_artboards {
                if !carried(&artboard) {
                    rects.push(rect);
                }
                continue;
            }
            if !mine {
                continue;
            }
            rects.push(rect);
            let layers: Vec<NodeId> = document
                .walk(&artboard, Family::Layer)
                .map(|(_, node)| node.id.clone())
                .filter(|id| !carried(id))
                .collect();
            // Only what is drawn: no guides at hidden layers.
            for id in layers {
                if let Some(b) = self.drawn_box_of(&id) {
                    rects.push(b.bounds());
                }
            }
        }
        SnapTargets::from_rects(rects)
    }

    /// Starts moving the selection from `world`. With `duplicate`, the
    /// selection is copied first and the copies move (Alt-drag). Opens the
    /// gesture.
    pub fn begin_move(&mut self, world: Pos2, duplicate: bool) -> Option<Drag> {
        if duplicate {
            let ids: Vec<NodeId> = self.drag_items().into_iter().map(|item| item.id).collect();
            if ids.is_empty() {
                return None;
            }
            let (edits, copies) = build::duplicate(&self.document, &ids).ok()?;
            self.begin_gesture("Duplicate");
            if self.apply("Duplicate", &edits).is_err() {
                let _ = self.cancel_gesture();
                return None;
            }
            self.selection
                .set(ids.iter().filter_map(|id| copies.get(id).cloned()));
            let copied = self.drag_ids();
            if self.settle("Duplicate", &copied, false).is_err() {
                let _ = self.cancel_gesture();
                return None;
            }
            // The copy is kept even where it was made.
            self.drag_start = None;
        } else if !self.begin_drag_gesture("Move", false) {
            return None;
        }
        let items = self.drag_items();
        let Some(bounds) = bounds_of(&items) else {
            self.commit_gesture();
            return None;
        };
        let targets = self.snap_targets(&items);
        Some(Drag::Move {
            start: world,
            items,
            bounds,
            targets,
        })
    }

    /// Starts dragging a handle of the selection box. Opens the gesture.
    pub fn begin_handle(&mut self, handle: Handle, world: Pos2) -> Option<Drag> {
        if self.drag_items().is_empty() {
            return None;
        }
        let (summary, inside) = match handle {
            Handle::Resize(_) => ("Resize", true),
            Handle::Rotate(_) => ("Rotate", false),
        };
        if !self.begin_drag_gesture(summary, inside) {
            return None;
        }
        let items = self.drag_items();
        let drag = match handle {
            Handle::Resize(anchor) => box_of(&items).map(|from| {
                let targets = self.snap_targets(&items);
                Drag::Resize {
                    anchor,
                    start: world,
                    items,
                    from,
                    targets,
                    document: Arc::clone(&self.document),
                }
            }),
            Handle::Rotate(_) => {
                let items: Vec<Item> = items.into_iter().filter(|item| !item.artboard).collect();
                box_of(&items).map(|pivot| Drag::Rotate {
                    start: world,
                    items,
                    pivot: Pos2::new(pivot.center.x as f32, pivot.center.y as f32),
                })
            }
        };
        if drag.is_none() {
            self.commit_gesture();
        }
        drag
    }

    /// Applies one step of a drag with the pointer at `world`. Returns the
    /// snapping guides to draw.
    pub fn drag_to(
        &mut self,
        drag: &Drag,
        world: Pos2,
        modifiers: Modifiers,
        zoom: f32,
    ) -> Result<Vec<[Pos2; 2]>, EditError> {
        let threshold = SNAP_DISTANCE / zoom.max(1e-3);
        match drag {
            Drag::Move {
                start,
                items,
                bounds,
                targets,
            } => {
                let mut delta = world - *start;
                if modifiers.shift {
                    delta = locked_to_axis(delta);
                }
                let mut guides = Vec::new();
                if snaps(modifiers) {
                    let snapped = snap_rect(bounds.translate(delta), targets, threshold);
                    let offset = if modifiers.shift {
                        // Stay on the locked axis.
                        Vec2::new(
                            if delta.y == 0.0 {
                                snapped.offset.x
                            } else {
                                0.0
                            },
                            if delta.x == 0.0 {
                                snapped.offset.y
                            } else {
                                0.0
                            },
                        )
                    } else {
                        snapped.offset
                    };
                    delta += offset;
                    guides = snapped.guides;
                }
                let edits: Vec<_> = items.iter().map(|item| item.moved(delta)).collect();
                self.apply("Move", &edits)?;
                Ok(guides)
            }
            Drag::Resize {
                anchor,
                start,
                items,
                from,
                targets,
                document,
            } => {
                let mut travel = world - *start;
                let mut guides = Vec::new();
                // Upright boxes snap the handle they are dragged by.
                if snaps(modifiers) && from.rotation.rem_euclid(360.0).abs() < 1e-6 {
                    let unit = anchor.unit();
                    let handle = from.point(unit);
                    let at = Pos2::new(handle.x as f32, handle.y as f32) + travel;
                    let snapped = snap_point(at, targets, threshold, unit.x != 0.0, unit.y != 0.0);
                    travel += snapped.offset;
                    guides = snapped.guides;
                }
                let mode = ResizeMode {
                    keep_proportions: modifiers.shift,
                    from_center: modifiers.alt,
                };
                let (to, w, h) = resized(from, *anchor, travel, mode);
                self.apply("Resize", &resize_edits(document, items, from, &to, (w, h)))?;
                Ok(guides)
            }
            Drag::Rotate {
                start,
                items,
                pivot,
            } => {
                let raw = angle(*pivot, world) - angle(*pivot, *start);
                let first = items.first().map_or(0.0, Item::world_rotation);
                let delta = rotation_delta(raw, first, modifiers.shift);
                let center = Point::new(f64::from(pivot.x), f64::from(pivot.y));
                self.apply("Rotate", &rotate_edits(items, center, delta))?;
                Ok(Vec::new())
            }
        }
    }

    /// Ends a drag: whatever it changed becomes one undo step. A drag that
    /// changed nothing but the text it settled leaves no step at all.
    pub fn end_drag(&mut self) {
        self.sync();
        let unchanged = self
            .drag_start
            .take()
            .is_some_and(|start| *start == *self.document);
        if unchanged && self.cancel_gesture().is_ok() {
            return;
        }
        self.commit_gesture();
    }

    /// Moves the selection by `delta` on the canvas as one undo step: what
    /// the arrow keys do.
    pub fn nudge(&mut self, delta: Vec2) -> Result<bool, EditError> {
        let edits: Vec<_> = self
            .drag_items()
            .iter()
            .map(|item| item.moved(delta))
            .collect();
        self.apply("Nudge", &edits)
    }
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};

    use super::super::view::tests::editor;
    use super::*;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    fn frame(editor: &Editor, id_text: &str) -> (f64, f64, f64, f64, f64) {
        let l = editor
            .document()
            .get(&id(id_text))
            .unwrap()
            .layer()
            .unwrap();
        (l.x, l.y, l.width, l.height, l.rotation)
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let drag = editor.begin_move(pos2(720.0, 120.0), false).unwrap();
        for x in [730.0, 760.0, 790.0] {
            editor
                .drag_to(&drag, pos2(x, 120.0), Modifiers::CTRL, 1.0)
                .unwrap();
        }
        editor.end_drag();
        assert_eq!(frame(&editor, "loose").0, 770.0);
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Move"));
        editor.undo().unwrap();
        assert_eq!(frame(&editor, "loose").0, 700.0);
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn moving_snaps_to_the_artboard_and_to_other_layers() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let drag = editor.begin_move(pos2(720.0, 120.0), false).unwrap();
        // The loose rect is 50 wide at x 700; bring its right edge to 997,
        // three short of the artboard's edge.
        let guides = editor
            .drag_to(&drag, pos2(967.0, 120.0), Modifiers::NONE, 1.0)
            .unwrap();
        assert_eq!(frame(&editor, "loose").0, 950.0);
        assert!(guides.iter().any(|[a, b]| a.x == 1000.0 && b.x == 1000.0));
        // With Ctrl held it does not snap.
        editor
            .drag_to(&drag, pos2(967.0, 120.0), Modifiers::CTRL, 1.0)
            .unwrap();
        assert_eq!(frame(&editor, "loose").0, 947.0);
        // The card's top edge (y 100) catches the rect's top.
        editor
            .drag_to(&drag, pos2(720.0, 123.0), Modifiers::NONE, 1.0)
            .unwrap();
        assert_eq!(frame(&editor, "loose").1, 100.0);
        editor.end_drag();
    }

    #[test]
    fn hidden_layers_are_not_snapped_to() {
        let mut editor = editor();
        editor
            .apply(
                "Hide",
                &[moshionly_core::Edit::set("loose", "hidden", true)],
            )
            .unwrap();
        let turned = Item::of(editor.document(), &id("turned")).unwrap();
        let targets = editor.snap_targets(&[turned]);
        let mut rects = vec![editor.artboard_rect(&id("ab")).unwrap()];
        for layer in ["card", "bg", "dot"] {
            rects.push(editor.box_of(&id(layer)).unwrap().bounds());
        }
        assert_eq!(targets, SnapTargets::from_rects(rects));
    }

    #[test]
    fn shift_keeps_a_move_on_one_axis() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let drag = editor.begin_move(pos2(720.0, 120.0), false).unwrap();
        editor
            .drag_to(
                &drag,
                pos2(760.0, 135.0),
                Modifiers::SHIFT | Modifiers::CTRL,
                1.0,
            )
            .unwrap();
        editor.end_drag();
        assert_eq!(
            (frame(&editor, "loose").0, frame(&editor, "loose").1),
            (740.0, 100.0)
        );
    }

    #[test]
    fn alt_drag_moves_a_copy() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let drag = editor.begin_move(pos2(720.0, 120.0), true).unwrap();
        editor
            .drag_to(&drag, pos2(720.0, 220.0), Modifiers::CTRL, 1.0)
            .unwrap();
        editor.end_drag();
        let copy = editor.selection.single().unwrap().clone();
        assert_ne!(copy, id("loose"));
        assert_eq!(frame(&editor, "loose").1, 100.0, "the original stays");
        let moved = editor.document().get(&copy).unwrap().layer().unwrap().y;
        assert_eq!(moved, 200.0);
        // Copy and move are one step.
        editor.undo().unwrap();
        assert!(!editor.document().contains(&copy));
    }

    #[test]
    fn handles_resize_and_rotate() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let drag = editor
            .begin_handle(Handle::Resize(Anchor::BottomRight), pos2(750.0, 150.0))
            .unwrap();
        editor
            .drag_to(&drag, pos2(800.0, 170.0), Modifiers::CTRL, 1.0)
            .unwrap();
        editor.end_drag();
        assert_eq!(frame(&editor, "loose"), (700.0, 100.0, 100.0, 70.0, 0.0));

        let drag = editor
            .begin_handle(Handle::Rotate(Anchor::BottomRight), pos2(850.0, 135.0))
            .unwrap();
        // From the right of the centre (750, 135) to straight below it.
        editor
            .drag_to(&drag, pos2(750.0, 235.0), Modifiers::NONE, 1.0)
            .unwrap();
        editor.end_drag();
        assert_eq!(frame(&editor, "loose").4, 90.0);
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Rotate"));
    }

    #[test]
    fn resizing_snaps_the_handle() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let drag = editor
            .begin_handle(Handle::Resize(Anchor::Right), pos2(750.0, 125.0))
            .unwrap();
        // Toward the artboard's right edge at 1000.
        editor
            .drag_to(&drag, pos2(996.0, 125.0), Modifiers::NONE, 1.0)
            .unwrap();
        editor.end_drag();
        assert_eq!(frame(&editor, "loose").2, 300.0);
    }

    #[test]
    fn a_group_stretches_its_contents_once_however_long_the_drag() {
        let mut editor = editor();
        editor.selection.select(id("card"));
        // The card does not clip: what is in it stretches with it.
        let drag = editor
            .begin_handle(Handle::Resize(Anchor::Right), pos2(500.0, 250.0))
            .unwrap();
        for x in [600.0, 700.0, 900.0] {
            editor
                .drag_to(&drag, pos2(x, 250.0), Modifiers::CTRL, 1.0)
                .unwrap();
        }
        editor.end_drag();
        // 400 wide to 800: the background too, not more.
        assert_eq!(frame(&editor, "card").2, 800.0);
        assert_eq!(frame(&editor, "bg").2, 800.0);
        assert_eq!(frame(&editor, "dot"), (40.0, 20.0, 100.0, 50.0, 0.0));
        editor.undo().unwrap();
        assert_eq!(frame(&editor, "bg").2, 400.0);
    }

    #[test]
    fn instances_go_back_with_a_main_dragged_out_and_back() {
        // Pulled 60 down and back to where it was: with Ctrl, or without
        // and let go 2 off, where it snaps to the bottom of the instance.
        for (modifiers, end) in [(Modifiers::CTRL, 400.0), (Modifiers::NONE, 402.0)] {
            let mut editor = editor();
            let (_, edits) = build::make_component(editor.document(), &[id("card")]).unwrap();
            editor.apply("Create component", &edits).unwrap();
            let (copy, edits) =
                crate::editor::components::instance_beside(editor.document(), &id("card")).unwrap();
            editor.apply("Insert instance", &edits).unwrap();
            assert_eq!(frame(&editor, copy.as_str()).3, 300.0);
            editor.selection.select(id("card"));
            let drag = editor
                .begin_handle(Handle::Resize(Anchor::Bottom), pos2(300.0, 400.0))
                .unwrap();
            for y in [430.0, 460.0, end] {
                editor
                    .drag_to(&drag, pos2(300.0, y), modifiers, 1.0)
                    .unwrap();
                // The instance is the main's height all along.
                assert_eq!(
                    frame(&editor, copy.as_str()).3,
                    frame(&editor, "card").3,
                    "{modifiers:?} at {y}"
                );
            }
            editor.end_drag();
            assert_eq!(frame(&editor, "card").3, 300.0, "{modifiers:?}");
            assert_eq!(frame(&editor, copy.as_str()).3, 300.0, "{modifiers:?}");
            // Nothing changed, so the drag left no step.
            assert_eq!(
                editor.undo_label().as_deref(),
                Some("Undo Insert instance"),
                "{modifiers:?}"
            );
        }
    }

    #[test]
    fn arrow_keys_nudge_the_selection() {
        let mut editor = editor();
        editor.selection.set([id("loose"), id("card")]);
        editor.nudge(vec2(10.0, -1.0)).unwrap();
        assert_eq!(
            (frame(&editor, "loose").0, frame(&editor, "loose").1),
            (710.0, 99.0)
        );
        assert_eq!(
            (frame(&editor, "card").0, frame(&editor, "card").1),
            (110.0, 99.0)
        );
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Nudge"));
    }

    #[test]
    fn dragging_a_scene_moves_it_and_undoes() {
        let mut editor = editor();
        editor.selection.select(id("ab2"));
        let drag = editor.begin_move(pos2(1300.0, 200.0), false).unwrap();
        editor
            .drag_to(&drag, pos2(1360.0, 230.0), Modifiers::CTRL, 1.0)
            .unwrap();
        editor.end_drag();
        assert_eq!(
            (frame(&editor, "ab2").0, frame(&editor, "ab2").1),
            (1160.0, 30.0)
        );
        // The Design panel reads the same x and y.
        let layer = editor.document().get(&id("ab2")).unwrap().layer().unwrap();
        assert_eq!((layer.x, layer.y), (1160.0, 30.0));
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Move"));
        editor.undo().unwrap();
        assert_eq!(
            (frame(&editor, "ab2").0, frame(&editor, "ab2").1),
            (1100.0, 0.0)
        );
    }

    #[test]
    fn a_locked_artboard_stays_where_it_is() {
        let mut editor = editor();
        editor
            .apply("Lock", &[moshionly_core::Edit::set("ab", "locked", true)])
            .unwrap();
        editor.selection.select(id("ab"));
        assert!(!editor.nudge(vec2(10.0, 0.0)).unwrap());
        assert_eq!(frame(&editor, "ab").0, 0.0);
        assert!(editor.begin_move(pos2(500.0, 500.0), false).is_none());
        assert!(
            editor
                .begin_handle(Handle::Resize(Anchor::Right), pos2(1000.0, 500.0))
                .is_none()
        );
        assert!(!editor.in_gesture());
        // Its layers still move.
        editor.selection.select(id("loose"));
        editor.nudge(vec2(10.0, 0.0)).unwrap();
        assert_eq!(frame(&editor, "loose").0, 710.0);
    }

    /// A text layer whose box hugs its words, at 100, 500.
    fn hugging_text(editor: &mut Editor) -> NodeId {
        let mut data = build::default_layer(moshionly_core::NodeType::Text).unwrap();
        if let moshionly_core::model::NodeData::Text(text) = &mut data {
            text.content = "Hello world this is text".to_owned();
            text.layer.x = 100.0;
            text.layer.y = 500.0;
        }
        let (text, edits) = build::add_layer(editor.document(), &id("ab"), data).unwrap();
        editor.apply("Add text", &edits).unwrap();
        editor.selection.select(text.clone());
        text
    }

    #[test]
    fn resizing_text_that_hugs_its_words_starts_from_what_is_shown() {
        let mut editor = editor();
        let text = hugging_text(&mut editor);
        let shown = editor.box_of(&text).unwrap().bounds();
        assert!(shown.width() > 150.0, "the words are wider than the hint");
        // The right handle, where it is drawn, 5 to the right.
        let handle = pos2(shown.right(), shown.center().y);
        let drag = editor
            .begin_handle(Handle::Resize(Anchor::Right), handle)
            .unwrap();
        editor
            .drag_to(&drag, handle + vec2(5.0, 0.0), Modifiers::CTRL, 1.0)
            .unwrap();
        editor.end_drag();
        let after = editor.box_of(&text).unwrap().bounds();
        assert!(
            (after.width() - (shown.width() + 5.0)).abs() < 0.5,
            "{after:?}"
        );
        assert!((after.height() - shown.height()).abs() < 0.5, "no wrapping");
        assert!((after.left() - shown.left()).abs() < 0.5);
        // Dragging the side keeps the width and lets the height follow.
        let node = editor.document().get(&text).unwrap();
        let auto = crate::editor::inspect::text_of(node).unwrap().auto_size;
        assert_eq!(auto, moshionly_core::model::AutoSize::Height);
        // One step.
        editor.undo().unwrap();
        assert_eq!(editor.box_of(&text).unwrap().bounds(), shown);
    }

    #[test]
    fn moving_and_turning_text_work_on_what_is_shown() {
        let mut editor = editor();
        let text = hugging_text(&mut editor);
        let shown = editor.box_of(&text).unwrap().bounds();
        // A move snaps the box that is drawn, not the hint in the file.
        let drag = editor.begin_move(shown.center(), false).unwrap();
        let Drag::Move { bounds, .. } = &drag else {
            panic!("a move");
        };
        assert!((bounds.width() - shown.width()).abs() < 0.5, "{bounds:?}");
        editor.end_drag();
        // A turn pivots about the middle of what is drawn.
        let drag = editor
            .begin_handle(Handle::Rotate(Anchor::TopRight), shown.right_top())
            .unwrap();
        let Drag::Rotate { pivot, .. } = &drag else {
            panic!("a turn");
        };
        assert!((*pivot - shown.center()).length() < 0.5, "{pivot:?}");
        editor.end_drag();
        // Nothing moved, and nothing is left to undo but adding the text.
        assert_eq!(editor.box_of(&text).unwrap().bounds(), shown);
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Add text"));
    }

    #[test]
    fn locked_and_carried_layers_are_left_alone() {
        let mut editor = editor();
        editor
            .apply(
                "Lock",
                &[moshionly_core::Edit::set("loose", "locked", true)],
            )
            .unwrap();
        editor.selection.set([id("loose"), id("card"), id("dot")]);
        editor.nudge(vec2(5.0, 0.0)).unwrap();
        assert_eq!(frame(&editor, "loose").0, 700.0, "locked");
        assert_eq!(frame(&editor, "card").0, 105.0);
        assert_eq!(frame(&editor, "dot").0, 20.0, "moved with its group");
    }
}

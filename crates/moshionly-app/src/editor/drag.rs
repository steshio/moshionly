//! Dragging the selection: moving it, resizing it from a handle, turning it.
//!
//! A drag opens a gesture, so however many steps it takes it is one undo
//! step. Each step is computed from where the drag started, never from the
//! previous step.

use egui::{Modifiers, Pos2, Rect, Vec2};
use moshionly_core::{EditError, Family, NodeId, Point, build};

use super::Editor;
use super::selection::is_locked;
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
    /// The selected nodes a drag works on: not locked, and not inside
    /// another selected node (which carries them along).
    fn drag_items(&self) -> Vec<Item> {
        let ids = self.selection.ids();
        ids.iter()
            .filter(|id| {
                !is_locked(&self.document, id)
                    && !ids.iter().any(|other| self.document.is_ancestor(other, id))
            })
            .filter_map(|id| Item::of(&self.document, id))
            .collect()
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
        } else {
            self.begin_gesture("Move");
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
        let items = self.drag_items();
        if items.is_empty() {
            return None;
        }
        match handle {
            Handle::Resize(anchor) => {
                let from = box_of(&items)?;
                let targets = self.snap_targets(&items);
                self.begin_gesture("Resize");
                Some(Drag::Resize {
                    anchor,
                    start: world,
                    items,
                    from,
                    targets,
                })
            }
            Handle::Rotate(_) => {
                let items: Vec<Item> = items.into_iter().filter(|item| !item.artboard).collect();
                let pivot = box_of(&items)?.center;
                self.begin_gesture("Rotate");
                Some(Drag::Rotate {
                    start: world,
                    items,
                    pivot: Pos2::new(pivot.x as f32, pivot.y as f32),
                })
            }
        }
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
                self.apply("Resize", &resize_edits(items, from, &to, (w, h)))?;
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

    /// Ends a drag: whatever it changed becomes one undo step.
    pub fn end_drag(&mut self) {
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

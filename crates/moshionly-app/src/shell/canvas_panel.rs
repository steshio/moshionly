//! The canvas: the artboards' pictures, what is drawn over them, and what
//! the pointer does there.

use egui::{Color32, Modifiers, Pos2, Rect, Ui};
use moshionly_core::NodeId;
use moshionly_core::model::Paint;

use super::{Shell, parts, tool_bar, tool_cursor};
use crate::{
    canvas::{RenderKey, plan},
    commands::CommandId,
    editor::inspect::artboard_of,
    theme::Theme,
    viewport::{Artboard, Overlays, Viewport, ViewportEvent},
};

/// A drag on the canvas, from the moment it starts until it ends.
#[derive(Clone, Debug)]
pub(super) enum CanvasDrag {
    /// A rubber band selecting what it touches.
    Marquee {
        /// Where the drag started, in world coordinates.
        start: Pos2,
        /// Where the pointer is now.
        current: Pos2,
        /// What was selected before, kept when Shift is held.
        base: Vec<NodeId>,
    },
}

impl Shell {
    /// Draws the canvas and the tool bar over it, and acts on what the
    /// pointer did. Returns the commands of the tool bar's buttons.
    pub(super) fn canvas_panel(&mut self, ui: &mut Ui, canvas: Rect) -> Vec<CommandId> {
        let c = &Theme::of(ui).color;
        if !self.fitted && canvas.is_positive() {
            // A newly opened document starts with everything in view.
            if let Some(bounds) = self.editor.content_bounds() {
                self.fit(canvas, bounds);
            }
            self.fitted = true;
        }
        self.canvas = Some(canvas);
        let ppp = ui.ctx().pixels_per_point();
        self.request_pictures(canvas, ppp);

        let names: Vec<(NodeId, String, Rect, Color32)> = self
            .editor
            .document()
            .artboards()
            .iter()
            .filter_map(|id| {
                let node = self.editor.document().get(id)?;
                let rect = self.editor.artboard_rect(id)?;
                let fill = artboard_of(node)
                    .and_then(|artboard| artboard.fill.as_ref())
                    .map_or(Color32::TRANSPARENT, paint_color);
                Some((id.clone(), node.name().to_owned(), rect, fill))
            })
            .collect();
        let artboards: Vec<Artboard<'_>> = names
            .iter()
            .map(|(id, name, rect, fill)| {
                let picture = self.pictures.picture(id);
                Artboard {
                    key: crate::key::Key::new(id.as_str()),
                    name: if name.is_empty() { "Artboard" } else { name },
                    rect: *rect,
                    texture: picture.map(|p| p.texture.id()),
                    texture_rect: picture
                        .map(|p| p.region.translate(rect.min.to_vec2()))
                        .filter(|covered| !covered.contains_rect(*rect)),
                    fill: *fill,
                    selected: self.editor.selection.contains(id),
                }
            })
            .collect();
        let overlays = self.overlays();
        let tool = self.editor.tool;
        let (viewport, picked) = parts::panel(ui, "canvas", canvas, c.canvas, |ui| {
            let viewport = Viewport::new(&mut self.camera)
                .artboards(&artboards)
                .overlays(&overlays)
                .cursor(tool_cursor(tool))
                .show(ui, canvas);
            (viewport, tool_bar::tool_bar(ui, canvas, tool))
        });
        drop(artboards);

        let zoom = self.camera.zoom;
        for event in viewport.events {
            self.canvas_event(event, zoom);
        }
        // The hover outline shows what a click would select.
        let pointer = viewport.pointer.filter(|_| {
            tool == super::Tool::Select && viewport.handle.is_none() && self.drag.is_none()
        });
        self.editor.update_hover(pointer, zoom);
        picked.into_iter().collect()
    }

    /// Asks for pictures of the artboards in view, at the density the zoom
    /// needs. Artboards out of view keep the picture they have.
    fn request_pictures(&mut self, canvas: Rect, pixels_per_point: f32) {
        let visible = self.camera.visible_world(canvas);
        let near = visible.expand2(visible.size() * 0.5);
        let document = std::sync::Arc::clone(self.editor.document());
        let time = self.editor.view_time();
        for id in document.artboards() {
            let Some(rect) = self.editor.artboard_rect(id) else {
                continue;
            };
            let has_picture = self.pictures.picture(id).is_some();
            if !rect.intersects(near) && has_picture {
                continue;
            }
            let previous = self.pictures.requested(id).map(|key| key.plan);
            let plan = plan(
                rect.size(),
                visible.translate(-rect.min.to_vec2()),
                self.camera.zoom,
                pixels_per_point,
                previous.as_ref(),
            );
            let key = RenderKey {
                revision: self.editor.artboard_version(id),
                time,
                plan,
                skip: None,
            };
            self.pictures.request(id, &document, key);
        }
    }

    /// What is drawn over the artboards: the selection with its handles,
    /// the hover outline, a marquee and snapping guides.
    fn overlays(&mut self) -> Overlays {
        let ids = self.editor.selection.ids().to_vec();
        let mut overlays = Overlays {
            show_size: true,
            ..Overlays::default()
        };
        if ids.len() > 1 {
            overlays.outlines = ids.iter().filter_map(|id| self.editor.box_of(id)).collect();
        }
        overlays.selection = self.editor.selection_box();
        if let Some(hover) = self.editor.hover.clone()
            && !self.editor.selection.contains(&hover)
        {
            overlays.hover = self.editor.box_of(&hover);
        }
        if let Some(CanvasDrag::Marquee { start, current, .. }) = &self.drag {
            overlays.marquee = Some(Rect::from_two_pos(*start, *current));
        }
        overlays.guides.clone_from(&self.guides);
        overlays
    }

    /// Acts on one thing the pointer did on the canvas.
    fn canvas_event(&mut self, event: ViewportEvent, zoom: f32) {
        if self.editor.tool != super::Tool::Select {
            // TODO: the drawing tools come with the creation commit.
            return;
        }
        match event {
            ViewportEvent::Clicked { world, modifiers } => {
                self.editor.click(world, modifiers, zoom);
            }
            ViewportEvent::DoubleClicked { world, .. } => {
                // TODO: editing text on the canvas comes with its own commit.
                let _ = self.editor.double_click(world, zoom);
            }
            ViewportEvent::ArtboardNameClicked { key, modifiers } => {
                let id = NodeId::from(key.as_str());
                if modifiers.shift {
                    self.editor.selection.toggle(id.clone());
                } else {
                    self.editor.selection.select(id.clone());
                }
                self.editor.set_artboard(Some(id));
            }
            ViewportEvent::SecondaryClicked { world, .. } => {
                // A right click on something not selected selects it first.
                let target = self
                    .editor
                    .layer_at(world, crate::editor::Pick::Normal, zoom);
                if let Some(target) = target
                    && !self.editor.selection.contains(&target)
                {
                    self.editor.selection.select(target);
                }
            }
            ViewportEvent::DragStarted { world, modifiers } => {
                self.start_marquee(world, modifiers);
            }
            ViewportEvent::Dragged { world, .. } => {
                if let Some(CanvasDrag::Marquee {
                    start,
                    current,
                    base,
                }) = &mut self.drag
                {
                    *current = world;
                    let rect = Rect::from_two_pos(*start, world);
                    let base = base.clone();
                    let hits = self.editor.marquee_hits(rect);
                    self.editor.selection.set(base);
                    self.editor.selection.extend(hits);
                }
            }
            ViewportEvent::DragStopped { .. } => {
                self.drag = None;
                self.guides.clear();
            }
            ViewportEvent::HandleDragStarted { .. }
            | ViewportEvent::HandleDragged { .. }
            | ViewportEvent::HandleDragStopped { .. } => {}
        }
    }

    /// Starts a marquee at `world`. Shift keeps what is selected.
    fn start_marquee(&mut self, world: Pos2, modifiers: Modifiers) {
        let base = if modifiers.shift {
            self.editor.selection.ids().to_vec()
        } else {
            self.editor.selection.clear();
            Vec::new()
        };
        if let Some(artboard) = self.editor.artboard_at(world) {
            self.editor.set_artboard(Some(artboard));
        }
        self.drag = Some(CanvasDrag::Marquee {
            start: world,
            current: world,
            base,
        });
    }
}

/// The colour an artboard shows before its picture arrives: its fill, or
/// the first colour of a gradient.
fn paint_color(paint: &Paint) -> Color32 {
    let color = match paint {
        Paint::Solid(color) => *color,
        Paint::Gradient(gradient) => gradient
            .stops()
            .first()
            .map_or(moshionly_core::Color::WHITE, |stop| stop.color),
    };
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a)
}

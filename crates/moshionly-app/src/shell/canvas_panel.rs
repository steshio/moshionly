//! The canvas: the artboards' pictures, what is drawn over them, and what
//! the pointer does there with each tool.

use egui::{Color32, FontId, Modifiers, Pos2, Rect, TextEdit, Ui, UiBuilder, Vec2};
use moshionly_core::NodeId;
use moshionly_core::model::{Paint, TextAlign};

use super::{Shell, Tool, parts, tool_bar, tool_cursor};
use crate::{
    canvas::{RenderKey, plan},
    commands::CommandId,
    editor::{
        Pick,
        create::{CreateDrag, CreateError, click_origin, tool_type},
        drag::Drag,
        inspect::{artboard_of, text_of},
        pen::{Pen, PenPress},
    },
    theme::Theme,
    viewport::{Artboard, Overlays, Viewport, ViewportEvent},
    widgets::Toast,
};

/// A drag on the canvas, from the moment it starts until it ends.
#[derive(Clone, Debug)]
pub(super) enum CanvasDrag {
    /// Moving, resizing or turning the selection.
    Selection(Drag),
    /// A rubber band selecting what it touches.
    Marquee {
        /// Where the drag started, in world coordinates.
        start: Pos2,
        /// Where the pointer is now.
        current: Pos2,
        /// What was selected before, kept when Shift is held.
        base: Vec<NodeId>,
    },
    /// Drawing a new layer with a drawing tool.
    Create(CreateDrag),
    /// Pulling out the handles of the pen's newest anchor.
    PenHandles,
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
        self.pointer = viewport.pointer;
        for event in viewport.events {
            self.canvas_event(event, zoom);
        }
        self.text_field(ui, canvas);
        // The hover outline shows what a click would select.
        let pointer = viewport.pointer.filter(|_| {
            self.editor.tool == Tool::Select
                && viewport.handle.is_none()
                && self.drag.is_none()
                && self.text_edit.is_none()
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
        let skip = self.text_edit.as_ref().map(|editing| editing.id.clone());
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
                skip: skip
                    .clone()
                    .filter(|layer| document.artboard_of(layer).is_some_and(|a| a.id == *id)),
            };
            self.pictures.request(id, &document, key);
        }
    }

    /// What is drawn over the artboards: the selection with its handles,
    /// the hover outline, a marquee, snapping guides, a path being drawn
    /// and the box of text being edited.
    fn overlays(&mut self) -> Overlays {
        let mut overlays = Overlays {
            show_size: true,
            ..Overlays::default()
        };
        if let Some(editing) = &self.text_edit {
            let id = editing.id.clone();
            overlays.editing = self.editor.box_of(&id);
            return overlays;
        }
        if let Some(pen) = &self.pen {
            overlays.pen = Some(pen.overlay(self.pointer, self.camera.zoom));
            return overlays;
        }
        let ids = self.editor.selection.ids().to_vec();
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
        // Pressing anywhere on the canvas commits text being edited.
        if self.text_edit.is_some()
            && matches!(
                event,
                ViewportEvent::Clicked { .. }
                    | ViewportEvent::DragStarted { .. }
                    | ViewportEvent::HandleDragStarted { .. }
                    | ViewportEvent::ArtboardNameClicked { .. }
            )
        {
            self.finish_text_edit();
            if matches!(event, ViewportEvent::Clicked { .. }) {
                return;
            }
        }
        match self.editor.tool {
            Tool::Select | Tool::Image => self.select_event(event, zoom),
            Tool::Pen => self.pen_event(event, zoom),
            tool => self.draw_event(tool, event, zoom),
        }
    }

    /// The select tool: selecting, moving, resizing and turning.
    fn select_event(&mut self, event: ViewportEvent, zoom: f32) {
        match event {
            ViewportEvent::Clicked { world, modifiers } => {
                self.editor.click(world, modifiers, zoom);
            }
            ViewportEvent::DoubleClicked { world, .. } => {
                if let Some(text) = self.editor.double_click(world, zoom) {
                    self.text_edit = self.editor.begin_text_edit(&text, false);
                }
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
                let target = self.editor.layer_at(world, Pick::Normal, zoom);
                if let Some(target) = target
                    && !self.editor.selection.contains(&target)
                {
                    self.editor.selection.select(target);
                }
            }
            ViewportEvent::DragStarted { world, modifiers } => {
                self.start_drag(world, modifiers, zoom);
            }
            ViewportEvent::HandleDragStarted { handle, world, .. } => {
                self.drag = self
                    .editor
                    .begin_handle(handle, world)
                    .map(CanvasDrag::Selection);
            }
            ViewportEvent::Dragged { world, modifiers }
            | ViewportEvent::HandleDragged {
                world, modifiers, ..
            } => self.continue_drag(world, modifiers, zoom),
            ViewportEvent::DragStopped { .. } | ViewportEvent::HandleDragStopped { .. } => {
                self.stop_drag();
            }
        }
    }

    /// The drawing tools: a click places a layer at its default size, a drag
    /// draws it. Afterwards the select tool comes back.
    fn draw_event(&mut self, tool: Tool, event: ViewportEvent, zoom: f32) {
        let text = tool == Tool::Text;
        match event {
            ViewportEvent::Clicked { world, .. } => {
                let at = click_origin(world);
                if text {
                    // Adding text and typing it is one undo step.
                    self.editor.begin_gesture("Add text");
                    match self.editor.create_at(tool, at) {
                        Ok(id) => self.text_edit = self.editor.begin_text_edit(&id, true),
                        Err(error) => {
                            let _ = self.editor.cancel_gesture();
                            self.toasts.push(Toast::warning(error.to_string()));
                        }
                    }
                } else if let Err(error) = self.editor.create_at(tool, at) {
                    self.toasts.push(Toast::warning(error.to_string()));
                }
                self.editor.tool = Tool::Select;
            }
            ViewportEvent::DragStarted { world, .. } => match self.editor.begin_create(tool, world)
            {
                Ok(drag) => self.drag = Some(CanvasDrag::Create(drag)),
                Err(error) => {
                    self.toasts.push(Toast::warning(error.to_string()));
                    self.editor.tool = Tool::Select;
                }
            },
            ViewportEvent::Dragged { world, modifiers } => {
                if let Some(CanvasDrag::Create(drag)) = &self.drag {
                    let drag = drag.clone();
                    match self.editor.drag_create(&drag, world, modifiers, zoom) {
                        Ok(guides) => self.guides = guides,
                        Err(error) => self.toasts.push(Toast::error(error.to_string())),
                    }
                }
            }
            ViewportEvent::DragStopped { .. } => {
                if let Some(CanvasDrag::Create(drag)) = self.drag.take() {
                    if drag.text {
                        // The words are typed next, in the same undo step.
                        self.text_edit = self.editor.begin_text_edit(&drag.id, true);
                    } else {
                        self.editor.end_create(&drag);
                    }
                }
                self.guides.clear();
                self.editor.tool = Tool::Select;
            }
            _ => {}
        }
        if tool_type(tool).is_none() {
            self.editor.tool = Tool::Select;
        }
    }

    /// The pen: clicks add corners, drags add curves, the first anchor
    /// closes the path.
    fn pen_event(&mut self, event: ViewportEvent, zoom: f32) {
        let press = |shell: &mut Self, world: Pos2| -> bool {
            if shell.pen.is_none() {
                match shell.editor.artboard_at(world) {
                    Some(artboard) => shell.pen = Some(Pen::new(artboard)),
                    None => {
                        shell
                            .toasts
                            .push(Toast::warning(CreateError::NoArtboard.to_string()));
                        return false;
                    }
                }
            }
            let closed = shell
                .pen
                .as_mut()
                .is_some_and(|pen| pen.press(world, zoom) == PenPress::Closed);
            if closed {
                shell.finish_pen(true);
                return false;
            }
            true
        };
        match event {
            ViewportEvent::Clicked { world, .. } => {
                press(self, world);
            }
            ViewportEvent::DragStarted { world, .. } => {
                if press(self, world) {
                    self.drag = Some(CanvasDrag::PenHandles);
                }
            }
            ViewportEvent::Dragged { world, .. } => {
                if let (Some(CanvasDrag::PenHandles), Some(pen)) = (&self.drag, &mut self.pen) {
                    pen.pull_handles(world);
                }
            }
            ViewportEvent::DragStopped { .. } => self.drag = None,
            _ => {}
        }
    }

    /// Ends the path being drawn with the pen and goes back to the select
    /// tool. A path with fewer than two anchors is dropped.
    pub(super) fn finish_pen(&mut self, closed: bool) {
        if let Some(pen) = self.pen.take() {
            match self.editor.finish_pen(&pen, closed) {
                Ok(_) => {}
                Err(error) => self.toasts.push(Toast::error(error.to_string())),
            }
        }
        self.drag = None;
        self.editor.tool = Tool::Select;
    }

    /// Commits text being edited on the canvas.
    pub(super) fn finish_text_edit(&mut self) {
        if let Some(editing) = self.text_edit.take()
            && let Err(error) = self.editor.finish_text_edit(editing)
        {
            self.toasts.push(Toast::error(error.problem.message));
        }
    }

    /// The text field over text being edited: in the layer's place, at its
    /// size on screen, in its colour.
    fn text_field(&mut self, ui: &mut Ui, canvas: Rect) {
        let Some(editing) = &mut self.text_edit else {
            return;
        };
        let document = std::sync::Arc::clone(self.editor.document());
        let Some(text) = document.get(&editing.id).and_then(text_of) else {
            self.text_edit = None;
            return;
        };
        let zoom = self.camera.zoom;
        let id = editing.id.clone();
        let Some(area) = self.editor.box_of(&id) else {
            return;
        };
        let editing = self.text_edit.as_mut().expect("checked above");
        let top_left = self
            .camera
            .world_to_screen(canvas, area.center - area.size / 2.0);
        let width = area.size.x * zoom;
        let wraps = text.auto_size != moshionly_core::model::AutoSize::WidthAndHeight;
        let color = text
            .fill
            .as_ref()
            .map_or(Color32::BLACK, |paint| paint_color(paint).to_opaque());
        let size = (text.font_size as f32 * zoom).max(1.0);
        let room = Rect::from_min_max(top_left, canvas.max.max(top_left + Vec2::splat(4.0)));
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(("text-edit", id.as_str()))
                .max_rect(room),
        );
        child.set_clip_rect(canvas.intersect(ui.clip_rect()));
        let align = match text.align {
            TextAlign::Left => egui::Align2::LEFT_TOP,
            TextAlign::Center => egui::Align2::CENTER_TOP,
            TextAlign::Right => egui::Align2::RIGHT_TOP,
        };
        let field = TextEdit::multiline(&mut editing.buffer)
            .id_salt("canvas-text")
            .font(FontId::proportional(size))
            .text_color(color)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .background_color(Color32::TRANSPARENT)
            .align(align)
            .desired_rows(1)
            .desired_width(if wraps {
                width.max(size)
            } else {
                f32::INFINITY
            });
        let response = child.add(field).accessible_name("Text");
        if !response.has_focus() && !response.lost_focus() {
            response.request_focus();
        }
        if response.lost_focus() {
            self.finish_text_edit();
        }
    }

    /// Starts a drag at `world`: on the selection it moves the selection, on
    /// another layer it selects and moves that layer, and elsewhere it draws
    /// a marquee. Alt moves a copy.
    fn start_drag(&mut self, world: Pos2, modifiers: Modifiers, zoom: f32) {
        let pick = if modifiers.command {
            Pick::Deep
        } else {
            Pick::Normal
        };
        let target = self.editor.layer_at(world, pick, zoom);
        let document = std::sync::Arc::clone(self.editor.document());
        let selected = |id: &NodeId, editor: &crate::editor::Editor| {
            editor.selection.contains(id)
                || editor
                    .selection
                    .ids()
                    .iter()
                    .any(|s| document.is_ancestor(s, id))
        };
        let moves = match target {
            Some(target) if selected(&target, &self.editor) => true,
            Some(target) => {
                if modifiers.shift {
                    self.editor.selection.toggle(target);
                } else {
                    self.editor.selection.select(target);
                }
                true
            }
            None => self
                .editor
                .selection_box()
                .is_some_and(|b| b.contains(world)),
        };
        if moves && !self.editor.selection.is_empty() {
            self.drag = self
                .editor
                .begin_move(world, modifiers.alt)
                .map(CanvasDrag::Selection);
        } else {
            self.start_marquee(world, modifiers);
        }
    }

    /// Carries a drag on to `world`.
    fn continue_drag(&mut self, world: Pos2, modifiers: Modifiers, zoom: f32) {
        match &mut self.drag {
            Some(CanvasDrag::Selection(drag)) => {
                let drag = drag.clone();
                match self.editor.drag_to(&drag, world, modifiers, zoom) {
                    Ok(guides) => self.guides = guides,
                    Err(error) => {
                        self.toasts.push(Toast::error(error.problem.message));
                        let _ = self.editor.cancel_gesture();
                        self.drag = None;
                        self.guides.clear();
                    }
                }
            }
            Some(CanvasDrag::Marquee {
                start,
                current,
                base,
            }) => {
                *current = world;
                let rect = Rect::from_two_pos(*start, world);
                let base = base.clone();
                let hits = self.editor.marquee_hits(rect);
                self.editor.selection.set(base);
                self.editor.selection.extend(hits);
            }
            _ => {}
        }
    }

    /// Ends whatever drag is going on on the canvas, before something else
    /// changes the document: a move, resize or turn becomes its undo step,
    /// a drawn shape is kept, a text box with no words yet is dropped, and
    /// a marquee stops. A path being drawn with the pen is left to go on.
    pub(super) fn end_canvas_drag(&mut self) {
        match self.drag.take() {
            Some(CanvasDrag::Selection(_)) => self.editor.end_drag(),
            Some(CanvasDrag::Create(drag)) => {
                if drag.text {
                    if let Err(error) = self.editor.cancel_gesture() {
                        self.toasts.push(Toast::error(error.problem.message));
                    }
                } else {
                    self.editor.end_create(&drag);
                }
                self.editor.tool = Tool::Select;
            }
            Some(CanvasDrag::Marquee { .. }) | None => {}
            Some(pen @ CanvasDrag::PenHandles) => self.drag = Some(pen),
        }
        self.guides.clear();
    }

    /// Ends a drag of the select tool.
    fn stop_drag(&mut self) {
        if let Some(CanvasDrag::Selection(_)) = self.drag {
            self.editor.end_drag();
        }
        self.drag = None;
        self.guides.clear();
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

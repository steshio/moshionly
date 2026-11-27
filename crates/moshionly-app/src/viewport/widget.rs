//! The viewport widget: draws the canvas and turns gestures into events.

use egui::{
    Align2, Color32, CursorIcon, Key as KeyboardKey, LayerId, Modifiers, Order, PointerButton,
    Pos2, Rect, Response, Sense, Stroke, TextureId, Ui, Vec2, pos2,
};

use super::{
    camera::Camera,
    handles::{Handle, HandleMetrics, handle_cursor, hit_handle},
    overlay::{self, Overlays},
};
use crate::{
    key::Key,
    theme::{Theme, icons},
    widgets::paint,
};

/// One artboard to show on the canvas.
#[derive(Clone, Debug)]
pub struct Artboard<'a> {
    /// The artboard's key, handed back in events.
    pub key: Key,
    /// The name shown above the artboard.
    pub name: &'a str,
    /// Position and size in world coordinates.
    pub rect: Rect,
    /// The rendered artboard. Until it arrives, `fill` is shown.
    pub texture: Option<TextureId>,
    /// The part of the artboard the texture covers, in world coordinates.
    /// `None` when it covers the whole artboard. Close up, only the part on
    /// screen is rendered; the rest shows `fill`.
    pub texture_rect: Option<Rect>,
    /// The artboard's background colour.
    pub fill: Color32,
    /// Whether the artboard is selected; its name is then accent-coloured.
    pub selected: bool,
}

/// Something the user did on the canvas.
///
/// Positions are in world coordinates unless named otherwise. The viewport
/// reports gestures; deciding what they hit and what they mean (select, draw
/// a shape, move a layer) is up to the caller.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewportEvent {
    /// A click with the primary button. A double click sends this for both
    /// clicks and then [`DoubleClicked`](Self::DoubleClicked).
    Clicked {
        /// Where.
        world: Pos2,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
    /// The second click of a double click.
    DoubleClicked {
        /// Where.
        world: Pos2,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
    /// A click with the secondary button: time for a context menu.
    SecondaryClicked {
        /// Where, in world coordinates.
        world: Pos2,
        /// Where, on screen.
        screen: Pos2,
    },
    /// The name above an artboard was clicked.
    ArtboardNameClicked {
        /// The artboard.
        key: Key,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
    /// A drag began on the name above an artboard. `world` is where the
    /// button went down. Later movement of the same drag is reported as
    /// [`Dragged`](Self::Dragged) and [`DragStopped`](Self::DragStopped).
    ArtboardNameDragStarted {
        /// The artboard.
        key: Key,
        /// Where the button went down.
        world: Pos2,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
    /// A drag with the primary button began. `world` is where the button
    /// went down, not where the pointer is by now.
    DragStarted {
        /// Where the button went down.
        world: Pos2,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
    /// The pointer moved during a drag. Sent every frame of the drag.
    Dragged {
        /// Where the pointer is.
        world: Pos2,
        /// The modifier keys held now.
        modifiers: Modifiers,
    },
    /// The drag ended.
    DragStopped {
        /// Where the button was released.
        world: Pos2,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
    /// A drag began on a handle of the selection box. `world` is where the
    /// button went down.
    HandleDragStarted {
        /// Which handle.
        handle: Handle,
        /// Where the button went down.
        world: Pos2,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
    /// The pointer moved while dragging a handle. Sent every frame of the
    /// drag.
    HandleDragged {
        /// Which handle.
        handle: Handle,
        /// Where the pointer is.
        world: Pos2,
        /// The modifier keys held now: Shift usually keeps proportions or
        /// snaps the angle, Alt resizes about the centre.
        modifiers: Modifiers,
    },
    /// The handle was let go.
    HandleDragStopped {
        /// Which handle.
        handle: Handle,
        /// Where the button was released.
        world: Pos2,
        /// The modifier keys held.
        modifiers: Modifiers,
    },
}

/// What [`Viewport::show`] reports back.
pub struct ViewportOutput {
    /// The response of the canvas area.
    pub response: Response,
    /// What the user did this frame, in order.
    pub events: Vec<ViewportEvent>,
    /// The world position under the pointer while it hovers the canvas.
    pub pointer: Option<Pos2>,
    /// The handle of the selection box under the pointer, or being dragged.
    pub handle: Option<Handle>,
}

/// An infinite canvas.
///
/// - **Pan**: drag with the middle button, drag while holding Space, or
///   scroll (two fingers on a trackpad).
/// - **Zoom**: pinch, or scroll while holding ⌘ (Ctrl elsewhere); the point
///   under the pointer stays put.
/// - **Handles**: the selection box in the [`Overlays`] can be grabbed by
///   its corners and edges to resize and just outside its corners to rotate.
///   The viewport finds the handle; what dragging it does is up to the
///   caller.
///
/// The viewport owns no state except the gesture in progress: the camera is
/// the caller's, and everything it draws is handed to it each frame.
#[must_use = "call show()"]
pub struct Viewport<'a> {
    camera: &'a mut Camera,
    artboards: &'a [Artboard<'a>],
    overlays: Option<&'a Overlays>,
    cursor: Option<CursorIcon>,
}

/// The gesture a drag on the canvas belongs to, remembered while it lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gesture {
    /// Moving the camera.
    Pan,
    /// A drag on a handle of the selection box.
    Handle(Handle),
    /// A drag on the content, reported as events.
    Content,
    /// A drag that means nothing (the secondary button).
    Ignored,
}

impl<'a> Viewport<'a> {
    /// A viewport looking through `camera`.
    pub fn new(camera: &'a mut Camera) -> Self {
        Self {
            camera,
            artboards: &[],
            overlays: None,
            cursor: None,
        }
    }

    /// What to draw over the artboards: selection, hover, marquee, guides.
    pub fn overlays(mut self, overlays: &'a Overlays) -> Self {
        self.overlays = Some(overlays);
        self
    }

    /// The artboards to draw, back to front.
    pub fn artboards(mut self, artboards: &'a [Artboard<'a>]) -> Self {
        self.artboards = artboards;
        self
    }

    /// The pointer shape over the canvas, for the active tool.
    pub fn cursor(mut self, cursor: CursorIcon) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Draws the canvas in `rect` and handles its gestures.
    pub fn show(self, ui: &mut Ui, rect: Rect) -> ViewportOutput {
        let theme = Theme::of(ui);
        let id = ui.make_persistent_id("viewport");
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let camera = self.camera;
        let mut events = Vec::new();

        let (space_down, modifiers) = ui.input(|i| (i.key_down(KeyboardKey::Space), i.modifiers));
        // Space types a space while a text field has focus.
        let space_down = space_down && !ui.ctx().text_edit_focused();

        // Scrolling pans, pinching and ⌘-scrolling zoom.
        if response.hovered() {
            let (zoom, scroll, pointer) = ui.input(|i| {
                (
                    i.zoom_delta(),
                    i.smooth_scroll_delta(),
                    i.pointer.hover_pos(),
                )
            });
            if zoom != 1.0 {
                camera.zoom_about(rect, pointer.unwrap_or(rect.center()), zoom);
            } else if scroll != Vec2::ZERO {
                camera.pan(scroll);
            }
        }

        // The handle of the selection box under a screen position.
        let metrics = HandleMetrics {
            size: theme.size.handle,
            rotate_zone: theme.size.rotate_zone,
        };
        let selection = self.overlays.and_then(|overlays| overlays.selection);
        let handle_at = |camera: &Camera, screen: Pos2| {
            let on_screen = selection?.to_screen(camera, rect);
            hit_handle(&on_screen, screen, metrics)
        };

        // Drags: decide once, at the start, what the drag is.
        let gesture_id = id.with("gesture");
        if response.drag_started() {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos())
                .unwrap_or(rect.center());
            let primary = response.dragged_by(PointerButton::Primary);
            let gesture = if response.dragged_by(PointerButton::Middle) || (space_down && primary) {
                Gesture::Pan
            } else if !primary {
                Gesture::Ignored
            } else if let Some(handle) = handle_at(camera, origin) {
                Gesture::Handle(handle)
            } else {
                Gesture::Content
            };
            ui.data_mut(|d| d.insert_temp(gesture_id, gesture));
            let world = camera.screen_to_world(rect, origin);
            match gesture {
                // A drag is only recognised after the pointer has moved a
                // little; make up for that first stretch.
                Gesture::Pan => camera
                    .pan(response.total_drag_delta().unwrap_or_default() - response.drag_delta()),
                Gesture::Handle(handle) => events.push(ViewportEvent::HandleDragStarted {
                    handle,
                    world,
                    modifiers,
                }),
                Gesture::Content => events.push(ViewportEvent::DragStarted { world, modifiers }),
                Gesture::Ignored => {}
            }
        }
        let gesture: Option<Gesture> = ui.data(|d| d.get_temp(gesture_id));
        let pointer_screen = response
            .interact_pointer_pos()
            .or(ui.input(|i| i.pointer.latest_pos()));
        let pointer_world =
            |camera: &Camera| camera.screen_to_world(rect, pointer_screen.unwrap_or(rect.center()));
        if response.dragged() {
            match gesture {
                Some(Gesture::Pan) => camera.pan(response.drag_delta()),
                Some(Gesture::Handle(handle)) => events.push(ViewportEvent::HandleDragged {
                    handle,
                    world: pointer_world(camera),
                    modifiers,
                }),
                Some(Gesture::Content) => events.push(ViewportEvent::Dragged {
                    world: pointer_world(camera),
                    modifiers,
                }),
                Some(Gesture::Ignored) | None => {}
            }
        }
        if response.drag_stopped() {
            match gesture {
                Some(Gesture::Handle(handle)) => events.push(ViewportEvent::HandleDragStopped {
                    handle,
                    world: pointer_world(camera),
                    modifiers,
                }),
                Some(Gesture::Content) => events.push(ViewportEvent::DragStopped {
                    world: pointer_world(camera),
                    modifiers,
                }),
                _ => {}
            }
            ui.data_mut(|d| d.remove::<Gesture>(gesture_id));
        }

        // Clicks. Holding Space means "about to pan", not "click".
        if let Some(screen) = response.interact_pointer_pos() {
            let world = camera.screen_to_world(rect, screen);
            if response.clicked() && !space_down {
                events.push(ViewportEvent::Clicked { world, modifiers });
            }
            if response.double_clicked() && !space_down {
                events.push(ViewportEvent::DoubleClicked { world, modifiers });
            }
            if response.secondary_clicked() {
                events.push(ViewportEvent::SecondaryClicked { world, screen });
            }
        }

        // The handle in play: the one being dragged, or the one under the
        // pointer.
        let panning = response.dragged() && gesture == Some(Gesture::Pan);
        let handle = match gesture {
            Some(Gesture::Handle(handle)) if response.dragged() => Some(handle),
            Some(_) if response.dragged() => None,
            _ if space_down => None,
            _ => response
                .hover_pos()
                .and_then(|screen| handle_at(camera, screen)),
        };

        // Painting.
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.color.canvas);
        for artboard in self.artboards {
            let screen = snap_rect(camera.rect_to_screen(rect, artboard.rect));
            if !screen.intersects(rect.expand(32.0)) {
                continue;
            }
            painter.add(theme.shadow.artboard.as_shape(screen, 0.0));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            match (artboard.texture, artboard.texture_rect) {
                (Some(texture), None) => {
                    painter.image(texture, screen, uv, Color32::WHITE);
                }
                (Some(texture), Some(part)) => {
                    painter.rect_filled(screen, 0.0, artboard.fill);
                    let part = camera.rect_to_screen(rect, part);
                    painter.with_clip_rect(screen.intersect(rect)).image(
                        texture,
                        part,
                        uv,
                        Color32::WHITE,
                    );
                }
                (None, _) => {
                    painter.rect_filled(screen, 0.0, artboard.fill);
                }
            }
            events.extend(artboard_name(ui, rect, screen, artboard, camera, modifiers));
        }
        if let Some(overlays) = self.overlays {
            overlay::paint(ui, rect, camera, overlays);
        }

        // Pointer shape.
        if panning {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if let Some(handle) = handle {
            let rotation = selection.map_or(0.0, |s| s.rotation);
            match handle_cursor(handle, rotation) {
                Some(cursor) => ui.ctx().set_cursor_icon(cursor),
                None => {
                    if let Some(screen) = pointer_screen {
                        rotate_badge(ui, id, screen);
                    }
                }
            }
        } else if response.hovered() {
            if space_down {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            } else if let Some(cursor) = self.cursor {
                ui.ctx().set_cursor_icon(cursor);
            }
        }

        let pointer = response
            .hover_pos()
            .map(|screen| camera.screen_to_world(rect, screen));
        ViewportOutput {
            response,
            events,
            pointer,
            handle,
        }
    }
}

/// There is no system pointer for "rotate", so a small badge beside the
/// pointer says it instead.
fn rotate_badge(ui: &Ui, id: egui::Id, pointer: Pos2) {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, id.with("rotate-badge")));
    let center = pointer + Vec2::new(14.0, 16.0);
    painter.circle(center, 9.0, c.raised, Stroke::new(1.0, c.border_strong));
    icons::paint(&painter, center, icons::ARROW_CLOCKWISE, 12.0, c.text);
}

/// Draws the name above an artboard and reports a click or a drag on it.
fn artboard_name(
    ui: &Ui,
    viewport: Rect,
    screen: Rect,
    artboard: &Artboard<'_>,
    camera: &Camera,
    modifiers: Modifiers,
) -> Vec<ViewportEvent> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    // No room for a name above a tiny artboard.
    if screen.width() < 32.0 {
        return Vec::new();
    }
    let line = Rect::from_min_max(
        pos2(screen.left(), screen.top() - 20.0),
        pos2(screen.right(), screen.top() - 5.0),
    );
    // Laid out without a colour, so the ink chosen below applies.
    let galley = paint::elided(
        ui,
        artboard.name,
        theme.small(),
        Color32::PLACEHOLDER,
        line.width(),
    );
    let text_rect = Align2::LEFT_BOTTOM.align_size_within_rect(galley.size(), line);
    let hit = text_rect.expand(2.0).intersect(viewport);
    let response = if hit.is_positive() {
        Some(ui.interact(
            hit,
            ui.make_persistent_id(("artboard-name", artboard.key.as_str())),
            Sense::click_and_drag(),
        ))
    } else {
        None
    };
    let hovered = response.as_ref().is_some_and(Response::hovered);
    let ink = if artboard.selected {
        c.accent_text
    } else if hovered {
        c.text
    } else {
        c.text_secondary
    };
    ui.painter_at(viewport)
        .galley(paint::snap(text_rect.min), galley, ink);
    let Some(response) = response else {
        return Vec::new();
    };
    let world_at = |screen: Pos2| camera.screen_to_world(viewport, screen);
    let mut events = Vec::new();
    if response.drag_started() {
        let origin = ui
            .input(|i| i.pointer.press_origin())
            .unwrap_or_else(|| response.interact_pointer_pos().unwrap_or(hit.center()));
        events.push(ViewportEvent::ArtboardNameDragStarted {
            key: artboard.key.clone(),
            world: world_at(origin),
            modifiers,
        });
    }
    if response.dragged()
        && let Some(screen) = response.interact_pointer_pos()
    {
        events.push(ViewportEvent::Dragged {
            world: world_at(screen),
            modifiers,
        });
    }
    if response.drag_stopped() {
        let screen = response.interact_pointer_pos().unwrap_or_else(|| {
            ui.input(|i| i.pointer.press_origin())
                .unwrap_or(hit.center())
        });
        events.push(ViewportEvent::DragStopped {
            world: world_at(screen),
            modifiers,
        });
    }
    if response.clicked() {
        events.push(ViewportEvent::ArtboardNameClicked {
            key: artboard.key.clone(),
            modifiers,
        });
    }
    events
}

/// Moves a rectangle so its top-left corner is on a whole point, which keeps
/// artboard edges crisp without changing their size.
fn snap_rect(rect: Rect) -> Rect {
    Rect::from_min_size(paint::snap(rect.min), rect.size())
}

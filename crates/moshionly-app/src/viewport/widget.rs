//! The viewport widget: draws the canvas and turns gestures into events.

use egui::{
    Align2, Color32, CursorIcon, Key as KeyboardKey, Modifiers, PointerButton, Pos2, Rect,
    Response, Sense, TextureId, Ui, Vec2, pos2,
};

use super::camera::Camera;
use crate::{key::Key, theme::Theme, widgets::paint};

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
}

/// What [`Viewport::show`] reports back.
pub struct ViewportOutput {
    /// The response of the canvas area.
    pub response: Response,
    /// What the user did this frame, in order.
    pub events: Vec<ViewportEvent>,
    /// The world position under the pointer while it hovers the canvas.
    pub pointer: Option<Pos2>,
}

/// An infinite canvas.
///
/// - **Pan**: drag with the middle button, drag while holding Space, or
///   scroll (two fingers on a trackpad).
/// - **Zoom**: pinch, or scroll while holding ⌘ (Ctrl elsewhere); the point
///   under the pointer stays put.
///
/// The viewport owns no state except the gesture in progress: the camera is
/// the caller's, and everything it draws is handed to it each frame.
#[must_use = "call show()"]
pub struct Viewport<'a> {
    camera: &'a mut Camera,
    artboards: &'a [Artboard<'a>],
    cursor: Option<CursorIcon>,
}

/// The gesture a drag on the canvas belongs to, remembered while it lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gesture {
    /// Moving the camera.
    Pan,
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
            cursor: None,
        }
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

        // Drags: decide once, at the start, what the drag is.
        let gesture_id = id.with("gesture");
        if response.drag_started() {
            let gesture = if response.dragged_by(PointerButton::Middle)
                || (space_down && response.dragged_by(PointerButton::Primary))
            {
                Gesture::Pan
            } else if response.dragged_by(PointerButton::Primary) {
                Gesture::Content
            } else {
                Gesture::Ignored
            };
            ui.data_mut(|d| d.insert_temp(gesture_id, gesture));
            match gesture {
                // A drag is only recognised after the pointer has moved a
                // little; make up for that first stretch.
                Gesture::Pan => camera
                    .pan(response.total_drag_delta().unwrap_or_default() - response.drag_delta()),
                Gesture::Content => {
                    let origin = ui
                        .input(|i| i.pointer.press_origin())
                        .or(response.interact_pointer_pos())
                        .unwrap_or(rect.center());
                    events.push(ViewportEvent::DragStarted {
                        world: camera.screen_to_world(rect, origin),
                        modifiers,
                    });
                }
                Gesture::Ignored => {}
            }
        }
        let gesture: Option<Gesture> = ui.data(|d| d.get_temp(gesture_id));
        let pointer_screen = response
            .interact_pointer_pos()
            .or(ui.input(|i| i.pointer.latest_pos()));
        if response.dragged() {
            match gesture {
                Some(Gesture::Pan) => camera.pan(response.drag_delta()),
                Some(Gesture::Content) => {
                    if let Some(screen) = pointer_screen {
                        events.push(ViewportEvent::Dragged {
                            world: camera.screen_to_world(rect, screen),
                            modifiers,
                        });
                    }
                }
                Some(Gesture::Ignored) | None => {}
            }
        }
        if response.drag_stopped() {
            if gesture == Some(Gesture::Content) {
                events.push(ViewportEvent::DragStopped {
                    world: camera.screen_to_world(rect, pointer_screen.unwrap_or(rect.center())),
                    modifiers,
                });
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

        // Pointer shape.
        let panning = response.dragged() && gesture == Some(Gesture::Pan);
        if panning {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if response.hovered() {
            if space_down {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            } else if let Some(cursor) = self.cursor {
                ui.ctx().set_cursor_icon(cursor);
            }
        }

        // Painting.
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.color.canvas);
        for artboard in self.artboards {
            let screen = snap_rect(camera.rect_to_screen(rect, artboard.rect));
            if !screen.intersects(rect.expand(32.0)) {
                continue;
            }
            painter.add(theme.shadow.artboard.as_shape(screen, 0.0));
            match artboard.texture {
                Some(texture) => {
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    painter.image(texture, screen, uv, Color32::WHITE);
                }
                None => {
                    painter.rect_filled(screen, 0.0, artboard.fill);
                }
            }
            if let Some(event) = artboard_name(ui, rect, screen, artboard, modifiers) {
                events.push(event);
            }
        }

        let pointer = response
            .hover_pos()
            .map(|screen| camera.screen_to_world(rect, screen));
        ViewportOutput {
            response,
            events,
            pointer,
        }
    }
}

/// Draws the name above an artboard and reports a click on it.
fn artboard_name(
    ui: &Ui,
    viewport: Rect,
    screen: Rect,
    artboard: &Artboard<'_>,
    modifiers: Modifiers,
) -> Option<ViewportEvent> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    // No room for a name above a tiny artboard.
    if screen.width() < 32.0 {
        return None;
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
            Sense::click(),
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
    response
        .is_some_and(|r| r.clicked())
        .then(|| ViewportEvent::ArtboardNameClicked {
            key: artboard.key.clone(),
            modifiers,
        })
}

/// Moves a rectangle so its top-left corner is on a whole point, which keeps
/// artboard edges crisp without changing their size.
fn snap_rect(rect: Rect) -> Rect {
    Rect::from_min_size(paint::snap(rect.min), rect.size())
}

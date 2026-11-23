//! Making new layers with the tools of the tool bar.
//!
//! A drag draws the layer from corner to corner (Shift for a square or a
//! circle, Alt from the centre), a click places it at its default size.
//! Shapes and text go into the artboard under the pointer; an artboard can
//! be drawn anywhere. The new layer is named after its type with the next
//! free number, and drawing it is one undo step.

use std::path::Path;

use egui::{Modifiers, Pos2, Rect, Vec2, pos2, vec2};
use moshionly_agent::{Host, Tools};
use moshionly_core::model::NodeData;
use moshionly_core::{Edit, EditError, Family, NodeId, NodeType, build};
use serde_json::{Value, json};

use super::Editor;
use super::describe::{next_name, type_label};
use super::drag::{SNAP_DISTANCE, snaps};
use super::inspect::tidy;
use super::snap::{SnapTargets, snap_point};
use crate::shell::Tool;

/// The side of a shape placed with a click.
pub const DEFAULT_SHAPE: f32 = 100.0;

/// The side of an artboard placed with a click.
pub const DEFAULT_ARTBOARD: f32 = 1080.0;

/// The largest share of the artboard a placed image covers.
const IMAGE_SHARE: f32 = 0.8;

/// Why a layer could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CreateError {
    /// There is no artboard to put a layer in.
    #[error("Layers go inside an artboard. Draw on one, or draw an artboard first with A.")]
    NoArtboard,
    /// The tool does not make layers.
    #[error("This tool does not draw layers.")]
    NotADrawingTool,
    /// The document refused the new layer.
    #[error("{0}")]
    Refused(String),
}

impl From<EditError> for CreateError {
    fn from(error: EditError) -> Self {
        CreateError::Refused(error.problem.message)
    }
}

impl From<build::BuildError> for CreateError {
    fn from(error: build::BuildError) -> Self {
        CreateError::Refused(error.to_string())
    }
}

/// The node type a drawing tool makes.
pub fn tool_type(tool: Tool) -> Option<NodeType> {
    Some(match tool {
        Tool::Artboard => NodeType::Artboard,
        Tool::Rectangle => NodeType::Rect,
        Tool::Ellipse => NodeType::Ellipse,
        Tool::Star => NodeType::Star,
        Tool::Text => NodeType::Text,
        Tool::Select | Tool::Pen | Tool::Image => return None,
    })
}

/// The rectangle drawn by a drag from `start` to `end`: Shift makes it a
/// square, Alt makes `start` its centre.
pub fn drawn_rect(start: Pos2, end: Pos2, modifiers: Modifiers) -> Rect {
    let mut delta = end - start;
    if modifiers.shift {
        let side = delta.x.abs().max(delta.y.abs());
        delta = vec2(side.copysign(delta.x), side.copysign(delta.y));
    }
    if modifiers.alt {
        Rect::from_center_size(start, delta.abs() * 2.0)
    } else {
        Rect::from_two_pos(start, start + delta)
    }
}

/// A shape being drawn by a drag.
#[derive(Clone, Debug)]
pub struct CreateDrag {
    /// The new node.
    pub id: NodeId,
    /// Where the drag started.
    pub start: Pos2,
    /// The top-left corner of the artboard it is in (for an artboard: the
    /// canvas origin).
    pub origin: Pos2,
    /// What the dragged corner snaps to.
    pub targets: SnapTargets,
    /// Whether it is text, which gets a fixed width when drawn.
    pub text: bool,
}

impl Editor {
    /// The layer data for a new node of `ty`, named with the next free
    /// number, with its box at `rect` relative to its artboard.
    fn new_layer(&self, ty: NodeType, rect: Rect) -> Option<NodeData> {
        let mut data = build::default_layer(ty)?;
        let base = if ty == NodeType::Artboard {
            "Scene"
        } else {
            type_label(ty)
        };
        let names: Vec<&str> = self
            .document
            .nodes_unordered()
            .filter(|node| node.node_type() == ty)
            .map(|node| node.name())
            .collect();
        // Text is named by its words, so a new text layer has no name.
        let name = if ty == NodeType::Text {
            String::new()
        } else {
            next_name(base, names)
        };
        if let NodeData::Text(text) = &mut data {
            text.content.clear();
        }
        let layer = data.layer_mut()?;
        layer.name = name;
        layer.x = tidy(f64::from(rect.min.x));
        layer.y = tidy(f64::from(rect.min.y));
        if rect.width() > 0.0 && ty != NodeType::Text {
            layer.width = tidy(f64::from(rect.width()));
            layer.height = tidy(f64::from(rect.height()));
        }
        Some(data)
    }

    /// Adds a node of `ty` with its box at `world` on the canvas (inside
    /// `artboard`, unless it is an artboard itself). Returns its id.
    fn add_node(
        &mut self,
        ty: NodeType,
        world: Rect,
        artboard: Option<&NodeId>,
    ) -> Result<NodeId, CreateError> {
        let (id, edits) = if ty == NodeType::Artboard {
            let data = self
                .new_layer(ty, world)
                .ok_or(CreateError::NotADrawingTool)?;
            let index = build::last_index(&self.document, None, Family::Layer)?;
            let node = moshionly_core::Node::new(data).with_index(index);
            (node.id.clone(), vec![Edit::add(node)])
        } else {
            let artboard = artboard.ok_or(CreateError::NoArtboard)?;
            let origin = self
                .artboard_rect(artboard)
                .ok_or(CreateError::NoArtboard)?
                .min;
            let data = self
                .new_layer(ty, world.translate(-origin.to_vec2()))
                .ok_or(CreateError::NotADrawingTool)?;
            build::add_layer(&self.document, artboard, data)?
        };
        self.apply(&format!("Add {}", type_label(ty).to_lowercase()), &edits)?;
        Ok(id)
    }

    /// Places a node of the tool's type at its default size with its
    /// top-left corner at `world`, selects it and returns it. One undo step.
    pub fn create_at(&mut self, tool: Tool, world: Pos2) -> Result<NodeId, CreateError> {
        let ty = tool_type(tool).ok_or(CreateError::NotADrawingTool)?;
        let side = if ty == NodeType::Artboard {
            DEFAULT_ARTBOARD
        } else {
            DEFAULT_SHAPE
        };
        // Text starts as a point and grows with what is typed.
        let rect = if ty == NodeType::Text {
            Rect::from_min_size(world, Vec2::ZERO)
        } else {
            Rect::from_min_size(world, Vec2::splat(side))
        };
        let artboard = self.artboard_at(world);
        let id = self.add_node(ty, rect, artboard.as_ref())?;
        self.selection.select(id.clone());
        Ok(id)
    }

    /// Starts drawing a node of the tool's type from `world`. Opens a
    /// gesture that [`end_create`](Self::end_create) closes.
    pub fn begin_create(&mut self, tool: Tool, world: Pos2) -> Result<CreateDrag, CreateError> {
        let ty = tool_type(tool).ok_or(CreateError::NotADrawingTool)?;
        let artboard = self.artboard_at(world);
        if ty != NodeType::Artboard && artboard.is_none() {
            return Err(CreateError::NoArtboard);
        }
        let origin = match (&artboard, ty) {
            (Some(artboard), t) if t != NodeType::Artboard => {
                self.artboard_rect(artboard).map_or(Pos2::ZERO, |r| r.min)
            }
            _ => Pos2::ZERO,
        };
        self.begin_gesture(&format!("Add {}", type_label(ty).to_lowercase()));
        let rect = Rect::from_min_size(world, Vec2::splat(1.0));
        let id = match self.add_node(ty, rect, artboard.as_ref()) {
            Ok(id) => id,
            Err(error) => {
                let _ = self.cancel_gesture();
                return Err(error);
            }
        };
        let rects: Vec<Rect> = if ty == NodeType::Artboard {
            self.document
                .artboards()
                .iter()
                .filter(|other| **other != id)
                .filter_map(|other| self.artboard_rect(other))
                .collect()
        } else {
            let mut rects = Vec::new();
            if let Some(artboard) = &artboard {
                rects.extend(self.artboard_rect(artboard));
                let layers: Vec<NodeId> = self
                    .document
                    .walk(artboard, Family::Layer)
                    .map(|(_, node)| node.id.clone())
                    .filter(|other| *other != id)
                    .collect();
                for layer in layers {
                    if let Some(b) = self.drawn_box_of(&layer) {
                        rects.push(b.bounds());
                    }
                }
            }
            rects
        };
        Ok(CreateDrag {
            id,
            start: world,
            origin,
            targets: SnapTargets::from_rects(rects),
            text: ty == NodeType::Text,
        })
    }

    /// Draws the node out to `world`. Returns the snapping guides.
    pub fn drag_create(
        &mut self,
        drag: &CreateDrag,
        world: Pos2,
        modifiers: Modifiers,
        zoom: f32,
    ) -> Result<Vec<[Pos2; 2]>, CreateError> {
        let mut end = world;
        let mut guides = Vec::new();
        if snaps(modifiers) {
            let snapped = snap_point(
                world,
                &drag.targets,
                SNAP_DISTANCE / zoom.max(1e-3),
                true,
                true,
            );
            end += snapped.offset;
            guides = snapped.guides;
        }
        let rect = drawn_rect(drag.start, end, modifiers).translate(-drag.origin.to_vec2());
        let mut patch = serde_json::Map::new();
        patch.insert("x".into(), tidy(f64::from(rect.min.x)).into());
        patch.insert("y".into(), tidy(f64::from(rect.min.y)).into());
        patch.insert(
            "width".into(),
            tidy(f64::from(rect.width().max(1.0))).into(),
        );
        if drag.text {
            // A text box drawn by a drag keeps its width and wraps.
            patch.insert("auto_size".into(), json!("height"));
        } else {
            patch.insert(
                "height".into(),
                tidy(f64::from(rect.height().max(1.0))).into(),
            );
        }
        self.apply("Draw", &[Edit::update(drag.id.clone(), patch)])?;
        Ok(guides)
    }

    /// Finishes drawing: the new node is selected and the drawing is one
    /// undo step.
    pub fn end_create(&mut self, drag: &CreateDrag) {
        self.commit_gesture();
        if self.document.contains(&drag.id) {
            self.selection.select(drag.id.clone());
            self.follow_selection();
        }
    }

    /// Brings image files into the project and places them as image layers,
    /// centred on `at` (or on the current artboard), as one undo step.
    ///
    /// Each image is copied into the project's `assets` folder when the
    /// project has been saved, exactly as the agent's `import_image` tool
    /// does it, and is shown at its own size, made smaller when it would
    /// not fit the artboard. Returns the new layers and a message for each
    /// file that could not be used.
    pub fn import_images(
        &mut self,
        paths: &[&Path],
        at: Option<Pos2>,
    ) -> (Vec<NodeId>, Vec<String>) {
        let mut problems = Vec::new();
        let artboard = at
            .and_then(|at| self.artboard_at(at))
            .or_else(|| self.artboard.clone());
        let Some(artboard) = artboard else {
            problems.push(CreateError::NoArtboard.to_string());
            return (Vec::new(), problems);
        };
        let Some(board) = self.artboard_rect(&artboard) else {
            return (Vec::new(), problems);
        };
        let center = at.unwrap_or(board.center());
        let tools = Tools::new(self.session.clone(), Host::new());
        let mut edits = Vec::new();
        let mut ids = Vec::new();
        let mut working = (*self.document).clone();
        for (n, path) in paths.iter().enumerate() {
            let file = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            let output = tools.call("import_image", json!({ "source": path.to_string_lossy() }));
            let text = output.joined_text();
            if output.is_error {
                problems.push(format!("Could not use {file}: {}", first_sentence(&text)));
                continue;
            }
            let Ok(result) = serde_json::from_str::<Value>(&text) else {
                problems.push(format!("Could not use {file}."));
                continue;
            };
            let (Some(source), Some(width), Some(height)) = (
                result["source"].as_str(),
                result["width"].as_f64(),
                result["height"].as_f64(),
            ) else {
                problems.push(format!("Could not use {file}."));
                continue;
            };
            let size = fitted(
                vec2(width as f32, height as f32),
                board.size() * IMAGE_SHARE,
            );
            // Several images fan out a little so none hides another.
            let offset = vec2(24.0, 24.0) * n as f32;
            let rect =
                Rect::from_center_size(center + offset, size).translate(-board.min.to_vec2());
            let stem = path
                .file_stem()
                .map_or_else(|| "Image".to_owned(), |s| s.to_string_lossy().into_owned());
            let Some(mut data) = build::default_layer(NodeType::Image) else {
                continue;
            };
            if let NodeData::Image(image) = &mut data {
                image.source = source.to_owned();
                image.layer.name = stem;
                image.layer.x = tidy(f64::from(rect.min.x));
                image.layer.y = tidy(f64::from(rect.min.y));
                image.layer.width = tidy(f64::from(rect.width()));
                image.layer.height = tidy(f64::from(rect.height()));
            }
            match build::add_layer(&working, &artboard, data) {
                Ok((id, more)) => {
                    if working.apply(&more).is_ok() {
                        edits.extend(more);
                        ids.push(id);
                    }
                }
                Err(error) => problems.push(error.to_string()),
            }
        }
        if !edits.is_empty() {
            let summary = if ids.len() == 1 {
                "Add image"
            } else {
                "Add images"
            };
            match self.apply(summary, &edits) {
                Ok(_) => self.selection.set(ids.clone()),
                Err(error) => {
                    problems.push(error.problem.message);
                    ids.clear();
                }
            }
        }
        (ids, problems)
    }
}

impl Editor {
    /// Copies an image file into the project's assets folder the way the
    /// agent's `import_image` tool does, and returns the source to give an
    /// image layer with the image's size in pixels.
    pub fn import_file(&self, path: &Path) -> Result<(String, f64, f64), String> {
        let file = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let tools = Tools::new(self.session.clone(), Host::new());
        let output = tools.call("import_image", json!({ "source": path.to_string_lossy() }));
        let text = output.joined_text();
        if output.is_error {
            return Err(format!("Could not use {file}: {}", first_sentence(&text)));
        }
        let result: Value =
            serde_json::from_str(&text).map_err(|_| format!("Could not use {file}."))?;
        match (
            result["source"].as_str(),
            result["width"].as_f64(),
            result["height"].as_f64(),
        ) {
            (Some(source), Some(width), Some(height)) => Ok((source.to_owned(), width, height)),
            _ => Err(format!("Could not use {file}.")),
        }
    }

    /// Shows the image at `path` in the image layers `ids`, which keep
    /// their boxes. One undo step.
    pub fn replace_image(&mut self, ids: &[NodeId], path: &Path) -> Result<(), String> {
        let (source, ..) = self.import_file(path)?;
        let edits: Vec<Edit> = ids
            .iter()
            .map(|id| Edit::set(id.clone(), "source", source.clone()))
            .collect();
        self.apply("Replace image", &edits)
            .map(|_| ())
            .map_err(|error| error.problem.message)
    }
}

/// `size` made smaller, keeping its proportions, until it fits in `room`.
pub fn fitted(size: Vec2, room: Vec2) -> Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return Vec2::splat(DEFAULT_SHAPE);
    }
    let scale = (room.x / size.x).min(room.y / size.y).min(1.0);
    (size * scale).max(Vec2::splat(1.0))
}

/// The first sentence of a message.
fn first_sentence(text: &str) -> String {
    let text = text.trim();
    match text.find(". ") {
        Some(end) => text[..=end].to_owned(),
        None => text.to_owned(),
    }
}

/// Where a click with a drawing tool puts the layer's top-left corner.
pub fn click_origin(world: Pos2) -> Pos2 {
    pos2(world.x.round(), world.y.round())
}

#[cfg(test)]
mod tests {
    use moshionly_core::model::AutoSize;

    use super::super::view::tests::editor;
    use super::*;
    use crate::editor::inspect::text_of;

    fn layer(editor: &Editor, id: &NodeId) -> moshionly_core::model::LayerProps {
        editor.document().get(id).unwrap().layer().unwrap().clone()
    }

    #[test]
    fn a_drag_draws_a_rectangle_inside_the_artboard() {
        let mut editor = editor();
        let drag = editor
            .begin_create(Tool::Rectangle, pos2(203.0, 503.0))
            .unwrap();
        editor
            .drag_create(&drag, pos2(353.0, 603.0), Modifiers::CTRL, 1.0)
            .unwrap();
        editor.end_create(&drag);
        let id = editor.selection.single().unwrap().clone();
        let node = editor.document().get(&id).unwrap();
        assert_eq!(node.node_type(), NodeType::Rect);
        assert_eq!(node.name(), "Rectangle 1");
        assert_eq!(node.parent.as_ref().unwrap(), &NodeId::from("ab"));
        let l = layer(&editor, &id);
        assert_eq!((l.x, l.y, l.width, l.height), (203.0, 503.0, 150.0, 100.0));
        // One undo takes it away.
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Add rectangle"));
        editor.undo().unwrap();
        assert!(!editor.document().contains(&id));
    }

    #[test]
    fn shift_draws_a_circle_and_alt_draws_from_the_centre() {
        assert_eq!(
            drawn_rect(pos2(0.0, 0.0), pos2(30.0, -10.0), Modifiers::SHIFT),
            Rect::from_min_max(pos2(0.0, -30.0), pos2(30.0, 0.0))
        );
        assert_eq!(
            drawn_rect(pos2(50.0, 50.0), pos2(60.0, 70.0), Modifiers::ALT),
            Rect::from_min_max(pos2(40.0, 30.0), pos2(60.0, 70.0))
        );
        let mut editor = editor();
        let drag = editor
            .begin_create(Tool::Ellipse, pos2(200.0, 500.0))
            .unwrap();
        editor
            .drag_create(
                &drag,
                pos2(240.0, 520.0),
                Modifiers::SHIFT | Modifiers::CTRL,
                1.0,
            )
            .unwrap();
        editor.end_create(&drag);
        let id = editor.selection.single().unwrap().clone();
        let l = layer(&editor, &id);
        assert_eq!((l.width, l.height), (40.0, 40.0));
        assert_eq!(editor.document().get(&id).unwrap().name(), "Ellipse 1");
    }

    #[test]
    fn a_click_places_a_default_shape_and_names_count_up() {
        let mut editor = editor();
        let first = editor.create_at(Tool::Star, pos2(300.0, 600.0)).unwrap();
        let second = editor.create_at(Tool::Star, pos2(500.0, 600.0)).unwrap();
        let l = layer(&editor, &first);
        assert_eq!((l.x, l.y, l.width, l.height), (300.0, 600.0, 100.0, 100.0));
        assert_eq!(editor.document().get(&second).unwrap().name(), "Star 2");
        assert_eq!(editor.selection.ids(), [second]);
    }

    #[test]
    fn drawing_snaps_the_corner() {
        let mut editor = editor();
        let drag = editor
            .begin_create(Tool::Rectangle, pos2(203.0, 503.0))
            .unwrap();
        // The artboard's right edge is at 1000.
        let guides = editor
            .drag_create(&drag, pos2(997.0, 603.0), Modifiers::NONE, 1.0)
            .unwrap();
        editor.end_create(&drag);
        let id = editor.selection.single().unwrap().clone();
        assert_eq!(layer(&editor, &id).width, 797.0);
        assert!(!guides.is_empty());
    }

    #[test]
    fn shapes_need_an_artboard_but_artboards_go_anywhere() {
        let mut editor = editor();
        assert_eq!(
            editor.create_at(Tool::Rectangle, pos2(1050.0, 50.0)),
            Err(CreateError::NoArtboard)
        );
        assert!(
            editor
                .begin_create(Tool::Ellipse, pos2(1050.0, 50.0))
                .is_err()
        );
        assert!(
            !editor.in_gesture(),
            "a refused drawing leaves no gesture open"
        );
        let drag = editor
            .begin_create(Tool::Artboard, pos2(2000.0, 0.0))
            .unwrap();
        editor
            .drag_create(&drag, pos2(2400.0, 300.0), Modifiers::CTRL, 1.0)
            .unwrap();
        editor.end_create(&drag);
        let id = editor.selection.single().unwrap().clone();
        let node = editor.document().get(&id).unwrap();
        assert_eq!(node.node_type(), NodeType::Artboard);
        assert_eq!(node.name(), "Scene 1");
        let l = layer(&editor, &id);
        assert_eq!((l.x, l.y, l.width, l.height), (2000.0, 0.0, 400.0, 300.0));
        let clicked = editor.create_at(Tool::Artboard, pos2(3000.0, 0.0)).unwrap();
        assert_eq!(layer(&editor, &clicked).width, 1080.0);
    }

    #[test]
    fn text_drawn_by_a_drag_wraps_at_its_width() {
        let mut editor = editor();
        let drag = editor.begin_create(Tool::Text, pos2(100.0, 600.0)).unwrap();
        editor
            .drag_create(&drag, pos2(400.0, 700.0), Modifiers::CTRL, 1.0)
            .unwrap();
        editor.end_create(&drag);
        let id = editor.selection.single().unwrap().clone();
        let node = editor.document().get(&id).unwrap();
        let text = text_of(node).unwrap();
        assert_eq!(text.auto_size, AutoSize::Height);
        assert_eq!(text.layer.width, 300.0);
        let clicked = editor.create_at(Tool::Text, pos2(100.0, 800.0)).unwrap();
        let text = text_of(editor.document().get(&clicked).unwrap()).unwrap();
        assert_eq!(text.auto_size, AutoSize::WidthAndHeight);
        assert_eq!(click_origin(pos2(1.4, 2.6)), pos2(1.0, 3.0));
    }

    #[test]
    fn images_fit_the_artboard() {
        assert_eq!(
            fitted(vec2(400.0, 200.0), vec2(800.0, 800.0)),
            vec2(400.0, 200.0)
        );
        assert_eq!(
            fitted(vec2(4000.0, 2000.0), vec2(800.0, 800.0)),
            vec2(800.0, 400.0)
        );
        assert_eq!(
            fitted(vec2(0.0, 10.0), vec2(800.0, 800.0)),
            Vec2::splat(DEFAULT_SHAPE)
        );
    }

    #[test]
    fn an_image_file_becomes_an_image_layer() {
        let folder =
            std::env::temp_dir().join(format!("moshionly-app-image-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let png = folder.join("dot.png");
        // A 2 by 1 PNG, written out by hand so the test needs no encoder.
        let bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0xF4, 0x22, 0x7F, 0x8A, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x60, 0x80, 0x01, 0x00, 0x00, 0x09, 0x00, 0x01, 0x5C, 0xCD, 0xFF, 0x69,
            0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&png, bytes).unwrap();
        let missing = folder.join("missing.png");
        let mut editor = editor();
        let (ids, problems) = editor.import_images(
            &[png.as_path(), missing.as_path()],
            Some(pos2(500.0, 500.0)),
        );
        assert_eq!(ids.len(), 1, "{problems:?}");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("missing.png"), "{problems:?}");
        let node = editor.document().get(&ids[0]).unwrap();
        assert_eq!(node.name(), "dot");
        let l = node.layer().unwrap();
        assert_eq!((l.width, l.height), (2.0, 1.0));
        assert_eq!((l.x, l.y), (499.0, 499.5));
        assert_eq!(editor.selection.ids(), ids.as_slice());
        let _ = std::fs::remove_dir_all(&folder);
    }
}

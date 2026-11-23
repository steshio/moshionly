//! Where things are on the canvas, and what a click there selects.
//!
//! The canvas uses world coordinates: an artboard sits at its own `x` and
//! `y`, and its layers are placed inside it. Layer boxes come from the
//! renderer's bounds in the scene the canvas shows, so a rotated layer has a
//! rotated box and text has the size it is drawn at.

use egui::{Modifiers, Pos2, Rect, Vec2, pos2, vec2};
use moshionly_core::{Family, NodeId, NodeType, Point};

use super::Editor;
use super::selection::{self, Pick};
use crate::viewport::OrientedBox;

/// How far, in screen points, a click may miss a thin line and still hit it.
pub const HIT_REACH: f32 = 4.0;

/// A box from the four corners of a layer's box (top-left, top-right,
/// bottom-right, bottom-left before turning), moved by `origin`.
pub fn box_from_corners(corners: [Point; 4], origin: Pos2) -> OrientedBox {
    let at = |p: Point| origin + vec2(p.x as f32, p.y as f32);
    let [tl, tr, br, bl] = corners.map(at);
    let across = tr - tl;
    let down = bl - tl;
    OrientedBox {
        center: tl + (br - tl) / 2.0,
        size: vec2(across.length(), down.length()),
        rotation: across.y.atan2(across.x).to_degrees(),
    }
}

/// The rectangle that encloses every box.
pub fn union_of(boxes: impl IntoIterator<Item = OrientedBox>) -> Option<Rect> {
    boxes
        .into_iter()
        .map(|b| b.bounds())
        .reduce(|a, b| a.union(b))
}

impl Editor {
    /// An artboard's rectangle on the canvas.
    pub fn artboard_rect(&self, id: &NodeId) -> Option<Rect> {
        let node = self.document.get(id)?;
        if node.node_type() != NodeType::Artboard {
            return None;
        }
        let layer = node.layer()?;
        Some(Rect::from_min_size(
            pos2(layer.x as f32, layer.y as f32),
            vec2(layer.width as f32, layer.height as f32),
        ))
    }

    /// The frontmost artboard under a point.
    pub fn artboard_at(&self, world: Pos2) -> Option<NodeId> {
        self.document
            .artboards()
            .iter()
            .rev()
            .find(|id| self.artboard_rect(id).is_some_and(|r| r.contains(world)))
            .cloned()
    }

    /// The rectangle around every artboard, or `None` for a document
    /// without one.
    pub fn content_bounds(&self) -> Option<Rect> {
        self.document
            .artboards()
            .iter()
            .filter_map(|id| self.artboard_rect(id))
            .reduce(|a, b| a.union(b))
    }

    /// The box of a node on the canvas: an artboard's rectangle, or a
    /// layer's box as drawn now. A layer that is not drawn at this moment
    /// (hidden, or not yet faded in) gets its box at rest.
    pub fn box_of(&mut self, id: &NodeId) -> Option<OrientedBox> {
        if let Some(drawn) = self.drawn_box_of(id) {
            return Some(drawn);
        }
        let artboard = self.document.artboard_of(id)?.id.clone();
        let origin = self.artboard_rect(&artboard)?.min;
        let node = self.document.get(id)?;
        let layer = node.layer()?;
        let transform = self.document.world_transform(id)?;
        let (w, h) = (layer.width, layer.height);
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(w, 0.0),
            Point::new(w, h),
            Point::new(0.0, h),
        ]
        .map(|p| transform.apply(p));
        Some(box_from_corners(corners, origin))
    }

    /// The box of a node as the canvas shows it now: an artboard's
    /// rectangle, or the box a layer is drawn in. `None` for a layer that
    /// is not drawn at this moment, so that what cannot be seen is neither
    /// caught by a marquee nor snapped to.
    pub fn drawn_box_of(&mut self, id: &NodeId) -> Option<OrientedBox> {
        if let Some(rect) = self.artboard_rect(id) {
            return Some(OrientedBox::from_rect(rect));
        }
        let artboard = self.document.artboard_of(id)?.id.clone();
        let origin = self.artboard_rect(&artboard)?.min;
        let scene = self.scene(&artboard)?;
        let bounds = self.renderer.bounds_of(&scene, id)?;
        Some(box_from_corners(bounds.corners, origin))
    }

    /// The box the selection handles go on: the selected node's own box,
    /// or for several the rectangle around them all.
    pub fn selection_box(&mut self) -> Option<OrientedBox> {
        let ids = self.selection.ids().to_vec();
        match ids.as_slice() {
            [] => None,
            [only] => self.box_of(only),
            several => {
                let boxes: Vec<OrientedBox> =
                    several.iter().filter_map(|id| self.box_of(id)).collect();
                union_of(boxes).map(OrientedBox::from_rect)
            }
        }
    }

    /// The layer a click at `world` would select. `zoom` sets how far a
    /// thin line may be missed.
    pub fn layer_at(&mut self, world: Pos2, pick: Pick, zoom: f32) -> Option<NodeId> {
        let artboard = self.artboard_at(world)?;
        let origin = self.artboard_rect(&artboard)?.min;
        let scene = self.scene(&artboard)?;
        let local = world - origin;
        let point = Point::new(f64::from(local.x), f64::from(local.y));
        let reach = f64::from(HIT_REACH / zoom.max(1e-3));
        let hits = self.renderer.hit_test_within(&scene, point, reach);
        let selected = self.selection.ids().to_vec();
        hits.iter()
            .find_map(|leaf| selection::pick(&self.document, leaf, &selected, pick))
    }

    /// Updates the selection for a click at `world`: shift adds or removes,
    /// ⌘ picks the deepest layer. A click on an empty artboard selects it;
    /// elsewhere on nothing it clears the selection. A plain click on a
    /// layer is about that layer, even one already selected: it lets go of
    /// the selected actions.
    pub fn click(&mut self, world: Pos2, modifiers: Modifiers, zoom: f32) {
        let pick = if modifiers.command {
            Pick::Deep
        } else {
            Pick::Normal
        };
        let target = self.layer_at(world, pick, zoom).or_else(|| {
            // An artboard with nothing in it is picked by its inside.
            self.artboard_at(world).filter(|id| {
                self.document.children(id, Family::Layer).is_empty()
                    && !selection::is_held(&self.document, id)
            })
        });
        if let Some(artboard) = self.artboard_at(world) {
            self.artboard = Some(artboard);
        }
        match (target, modifiers.shift) {
            (Some(id), true) => self.selection.toggle(id),
            (Some(id), false) => {
                self.selection.select(id);
                self.actions.clear();
            }
            (None, true) => {}
            (None, false) => self.selection.clear(),
        }
    }

    /// Updates the selection for a double click at `world`: one level into
    /// the selected group. Returns the text layer to edit when the double
    /// click lands on a text layer that was already selected.
    pub fn double_click(&mut self, world: Pos2, zoom: f32) -> Option<NodeId> {
        let before = self.selection.ids().to_vec();
        let target = self.layer_at(world, Pick::EnterGroup, zoom)?;
        let is_text = self
            .document
            .get(&target)
            .is_some_and(|node| node.node_type() == NodeType::Text);
        if is_text && before.contains(&target) {
            return Some(target);
        }
        self.selection.select(target);
        None
    }

    /// The layers a marquee over `rect` catches: the top-level layers of
    /// every artboard it touches whose boxes it touches. Artboards the
    /// marquee encloses entirely are caught instead, when there are any.
    pub fn marquee_hits(&mut self, rect: Rect) -> Vec<NodeId> {
        let artboards: Vec<NodeId> = self.document.artboards().to_vec();
        let enclosed: Vec<NodeId> = artboards
            .iter()
            .filter(|id| {
                self.artboard_rect(id)
                    .is_some_and(|r| rect.contains_rect(r))
            })
            .cloned()
            .collect();
        if !enclosed.is_empty() {
            return enclosed;
        }
        let mut hits = Vec::new();
        for artboard in artboards {
            if !self
                .artboard_rect(&artboard)
                .is_some_and(|r| r.intersects(rect))
            {
                continue;
            }
            for id in selection::marquee_candidates(&self.document, &artboard) {
                if self.drawn_box_of(&id).is_some_and(|b| touches(&b, rect)) {
                    hits.push(id);
                }
            }
        }
        hits
    }

    /// The layer the pointer is over, as a click would pick it, for the
    /// hover outline.
    pub fn update_hover(&mut self, world: Option<Pos2>, zoom: f32) {
        self.hover = world.and_then(|world| self.layer_at(world, Pick::Normal, zoom));
    }
}

/// Whether an oriented box and an upright rectangle overlap.
fn touches(b: &OrientedBox, rect: Rect) -> bool {
    if !b.bounds().intersects(rect) {
        return false;
    }
    if b.rotation.rem_euclid(90.0) == 0.0 {
        return true;
    }
    // Separating axes: the rectangle's (done above) and the box's own.
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let half = b.size / 2.0;
    let locals: Vec<Vec2> = corners.iter().map(|c| b.to_local(*c)).collect();
    let outside = |axis: fn(&Vec2) -> f32, extent: f32| {
        locals.iter().all(|p| axis(p) > extent) || locals.iter().all(|p| axis(p) < -extent)
    };
    !(outside(|p| p.x, half.x) || outside(|p| p.y, half.y))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use moshionly_agent::{Session, SessionHandle};
    use moshionly_core::io;
    use moshionly_render::{AssetLoader, FontLibrary, Renderer};

    use super::*;

    /// Two artboards side by side. The first holds a card group with a
    /// background and a dot, a rect turned by 45° and a small loose rect.
    pub(crate) fn editor() -> Editor {
        let document = io::parse(
            r##"{"format":"moshionly","version":1,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","width":1000,"height":1000,"fill":"#FFFFFF"},
            {"id":"card","parent":"ab","index":"a0","type":"group","x":100,"y":100,"width":400,"height":300},
            {"id":"bg","parent":"card","index":"a0","type":"rect","width":400,"height":300,"fill":"#EEEEEE"},
            {"id":"dot","parent":"card","index":"a1","type":"ellipse","x":20,"y":20,"width":50,"height":50,"fill":"#FF0000"},
            {"id":"turned","parent":"ab","index":"a1","type":"rect","x":600,"y":600,"width":200,"height":100,"rotation":45,"fill":"#00FF00"},
            {"id":"loose","parent":"ab","index":"a2","type":"rect","x":700,"y":100,"width":50,"height":50,"fill":"#0000FF"},
            {"id":"ab2","parent":null,"index":"a1","type":"artboard","x":1100,"width":500,"height":500}
            ]}"##,
        )
        .unwrap();
        let session = SessionHandle::new(Session::new(document));
        Editor::new(
            session,
            Renderer::new(FontLibrary::bundled(), AssetLoader::default()),
        )
    }

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    #[test]
    fn boxes_come_from_where_layers_are_drawn() {
        let mut editor = editor();
        let card = editor.box_of(&id("card")).unwrap();
        assert_eq!(
            card.bounds(),
            Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 300.0))
        );
        let dot = editor.box_of(&id("dot")).unwrap();
        assert!((dot.center - pos2(145.0, 145.0)).length() < 0.01);
        let turned = editor.box_of(&id("turned")).unwrap();
        assert!((turned.rotation - 45.0).abs() < 0.01);
        assert!((turned.size - vec2(200.0, 100.0)).length() < 0.01);
        assert!((turned.center - pos2(700.0, 650.0)).length() < 0.01);
        // Artboards are their rectangles, wherever they are on the canvas.
        let second = editor.box_of(&id("ab2")).unwrap();
        assert_eq!(second.bounds().min, pos2(1100.0, 0.0));
        assert_eq!(editor.artboard_at(pos2(1200.0, 10.0)), Some(id("ab2")));
        assert_eq!(editor.artboard_at(pos2(1050.0, 10.0)), None);
        assert_eq!(
            editor.content_bounds(),
            Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(1600.0, 1000.0)))
        );
    }

    #[test]
    fn a_hidden_layer_still_has_its_box_at_rest() {
        let mut editor = editor();
        editor
            .apply(
                "Hide",
                &[moshionly_core::Edit::set("loose", "hidden", true)],
            )
            .unwrap();
        let loose = editor.box_of(&id("loose")).unwrap();
        assert_eq!(
            loose.bounds(),
            Rect::from_min_size(pos2(700.0, 100.0), vec2(50.0, 50.0))
        );
    }

    #[test]
    fn a_marquee_does_not_catch_what_is_not_drawn() {
        let mut editor = editor();
        editor
            .apply(
                "Hide",
                &[moshionly_core::Edit::set("loose", "hidden", true)],
            )
            .unwrap();
        let over = Rect::from_min_max(pos2(690.0, 90.0), pos2(760.0, 160.0));
        assert!(editor.marquee_hits(over).is_empty());
        assert_eq!(editor.drawn_box_of(&id("loose")), None);
        assert!(editor.drawn_box_of(&id("card")).is_some());
    }

    #[test]
    fn clicks_select_by_what_is_drawn() {
        let mut editor = editor();
        let none = Modifiers::NONE;
        editor.click(pos2(140.0, 140.0), none, 1.0);
        assert_eq!(editor.selection.ids(), [id("card")]);
        // A double click goes into the card.
        assert_eq!(editor.double_click(pos2(140.0, 140.0), 1.0), None);
        assert_eq!(editor.selection.ids(), [id("dot")]);
        // ⌘ picks the deepest layer straight away.
        editor.click(pos2(300.0, 300.0), Modifiers::COMMAND, 1.0);
        assert_eq!(editor.selection.ids(), [id("bg")]);
        // Shift adds and removes.
        editor.click(pos2(720.0, 120.0), Modifiers::SHIFT, 1.0);
        assert_eq!(editor.selection.ids(), [id("bg"), id("loose")]);
        editor.click(pos2(720.0, 120.0), Modifiers::SHIFT, 1.0);
        assert_eq!(editor.selection.ids(), [id("bg")]);
        // The corner of the turned rect's bounding box is not the rect.
        editor.click(pos2(600.0, 550.0), none, 1.0);
        assert!(editor.selection.is_empty());
        editor.click(pos2(700.0, 650.0), none, 1.0);
        assert_eq!(editor.selection.ids(), [id("turned")]);
        // Off every artboard, nothing.
        editor.click(pos2(1050.0, 500.0), none, 1.0);
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn a_plain_click_on_a_layer_lets_go_of_the_selected_actions() {
        let mut editor = editor();
        editor.selection.select(id("loose"));
        let fade = editor.add_actions(NodeType::FadeIn, 0.0).unwrap();
        assert_eq!(editor.actions(), fade.as_slice());
        // Shift adds and removes layers; the actions stay.
        editor.click(pos2(700.0, 650.0), Modifiers::SHIFT, 1.0);
        editor.click(pos2(700.0, 650.0), Modifiers::SHIFT, 1.0);
        assert_eq!(editor.selection.ids(), [id("loose")]);
        assert_eq!(editor.actions(), fade.as_slice());
        // A plain click on the layer, though it is selected already, is
        // about the layer.
        editor.click(pos2(720.0, 120.0), Modifiers::NONE, 1.0);
        assert_eq!(editor.selection.ids(), [id("loose")]);
        assert!(editor.actions().is_empty());
    }

    #[test]
    fn an_empty_artboard_is_selected_by_its_inside() {
        let mut editor = editor();
        editor.click(pos2(1300.0, 200.0), Modifiers::NONE, 1.0);
        assert_eq!(editor.selection.ids(), [id("ab2")]);
        assert_eq!(editor.artboard(), Some(&id("ab2")));
        // An artboard with layers is not: a click on its empty part clears.
        editor.click(pos2(900.0, 300.0), Modifiers::NONE, 1.0);
        assert!(editor.selection.is_empty());
        assert_eq!(editor.artboard(), Some(&id("ab")));
        // Nor is a locked one.
        editor
            .apply("Lock", &[moshionly_core::Edit::set("ab2", "locked", true)])
            .unwrap();
        editor.click(pos2(1300.0, 200.0), Modifiers::NONE, 1.0);
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn a_double_click_on_selected_text_asks_to_edit_it() {
        let mut editor = editor();
        let artboard = id("ab");
        let mut text = moshionly_core::build::default_layer(NodeType::Text).unwrap();
        if let moshionly_core::model::NodeData::Text(t) = &mut text {
            t.layer.x = 300.0;
            t.layer.y = 800.0;
        }
        let (text_id, edits) =
            moshionly_core::build::add_layer(editor.document(), &artboard, text).unwrap();
        editor.apply("Add text", &edits).unwrap();
        let center = editor.box_of(&text_id).unwrap().center;
        editor.click(center, Modifiers::NONE, 1.0);
        assert_eq!(editor.selection.ids(), std::slice::from_ref(&text_id));
        assert_eq!(editor.double_click(center, 1.0), Some(text_id));
    }

    #[test]
    fn a_marquee_catches_the_top_level_layers_it_touches() {
        let mut editor = editor();
        let rect = Rect::from_min_max(pos2(450.0, 50.0), pos2(720.0, 120.0));
        assert_eq!(editor.marquee_hits(rect), [id("loose"), id("card")]);
        // The turned rect is caught by its real outline, not its bounds.
        let corner = Rect::from_min_max(pos2(594.0, 544.0), pos2(610.0, 556.0));
        assert!(editor.marquee_hits(corner).is_empty());
        let through = Rect::from_min_max(pos2(690.0, 640.0), pos2(710.0, 660.0));
        assert_eq!(editor.marquee_hits(through), [id("turned")]);
        // An artboard inside the marquee wins.
        let all = Rect::from_min_max(pos2(1050.0, -50.0), pos2(1700.0, 600.0));
        assert_eq!(editor.marquee_hits(all), [id("ab2")]);
    }

    #[test]
    fn the_selection_box_surrounds_several_layers() {
        let mut editor = editor();
        editor.selection.set([id("card"), id("loose")]);
        let b = editor.selection_box().unwrap();
        assert_eq!(b.rotation, 0.0);
        assert_eq!(
            b.bounds(),
            Rect::from_min_max(pos2(100.0, 100.0), pos2(750.0, 400.0))
        );
        editor.update_hover(Some(pos2(720.0, 120.0)), 1.0);
        assert_eq!(editor.hover, Some(id("loose")));
        editor.update_hover(None, 1.0);
        assert_eq!(editor.hover, None);
        let _ = Arc::clone(editor.document());
    }
}

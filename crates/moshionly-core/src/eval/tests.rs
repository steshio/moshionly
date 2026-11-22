//! Tests of evaluation: documents built from JSON, evaluated at chosen
//! times, checked number by number.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;
use crate::color::Color;
use crate::model::{BlendMode, GradientStop, TextAlign};
use crate::scene::TextUnit;
use crate::text::MonospaceMeasurer;

/// Builds a document from node JSON. Every document gets a 400 by 300
/// artboard "ab" with a 2 second timeline. Nodes without a `parent` go in
/// it, and nodes without an `index` are ordered as they are listed.
pub(super) fn document(nodes: Value) -> Document {
    let mut all = vec![json!({
        "id": "ab", "index": "a0", "type": "artboard",
        "width": 400, "height": 300, "duration": 2000, "fill": "#FFFFFF"
    })];
    all.extend(nodes.as_array().unwrap().iter().cloned());
    let mut counters: HashMap<(String, bool), usize> = HashMap::new();
    let keys = crate::index::keys_between(None, None, all.len()).unwrap();
    let nodes = all.into_iter().map(|mut value| {
        let is_artboard = value["type"] == "artboard";
        if !is_artboard && value.get("parent").is_none() {
            value["parent"] = json!("ab");
        }
        if value.get("index").is_none() {
            let ty = NodeType::parse(value["type"].as_str().unwrap()).unwrap();
            let group = (value["parent"].to_string(), ty.family() == Family::Layer);
            let position = counters.entry(group).or_default();
            value["index"] = json!(keys[*position]);
            *position += 1;
        }
        crate::schema::read_node(value).unwrap_or_else(|problem| panic!("{problem}"))
    });
    Document::from_nodes("Test", nodes).unwrap_or_else(|problem| panic!("{problem}"))
}

pub(super) fn id(text: &str) -> NodeId {
    NodeId::from(text)
}

pub(super) fn at(document: &Document, time: f64) -> Scene {
    evaluate(document, &id("ab"), time, &MonospaceMeasurer::default()).unwrap()
}

pub(super) fn node<'a>(scene: &'a Scene, name: &str) -> &'a SceneNode {
    scene
        .find(&id(name))
        .unwrap_or_else(|| panic!("\"{name}\" is not in the scene at {}", scene.time_ms))
}

pub(super) fn number(document: &Document, layer: &str, property: Property, time: f64) -> f64 {
    match value_at(document, &id(layer), property, time) {
        Some(PropertyValue::Number(value)) => value,
        other => panic!("{property:?} of {layer} at {time} is {other:?}"),
    }
}

pub(super) fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

pub(super) fn same_transform(a: Affine, b: Affine) -> bool {
    a.0.iter().zip(b.0).all(|(a, b)| close(*a, b))
}

fn solid(hex: &str) -> Option<ScenePaint> {
    Some(ScenePaint::solid(Color::parse(hex).unwrap()))
}

const LINEAR: &str = r#"{ "type": "linear" }"#;

fn linear() -> Value {
    serde_json::from_str(LINEAR).unwrap()
}

#[test]
fn a_resting_layer_becomes_one_node() {
    let document = document(json!([{
        "id": "r", "type": "rect", "x": 40, "y": 30, "width": 100, "height": 50,
        "rotation": 90, "opacity": 50, "blend_mode": "multiply", "blur": 3,
        "shadow": { "y": 8 }, "fill": "#FF0000", "corner_radius": 40,
        "stroke": { "paint": "#0000FF", "width": 2, "position": "inside" }
    }]));
    let scene = at(&document, 0.0);
    assert_eq!(
        (scene.width, scene.height, scene.time_ms),
        (400.0, 300.0, 0.0)
    );
    assert_eq!(scene.artboard, id("ab"));
    assert_eq!(scene.background, solid("#FFFFFF"));
    assert_eq!(scene.nodes.len(), 1);

    let rect = &scene.nodes[0];
    assert_eq!(rect.id, id("r"));
    assert_eq!(
        rect.transform,
        Affine::layer(40.0, 30.0, 100.0, 50.0, 90.0, 1.0)
    );
    assert_eq!((rect.width, rect.height), (100.0, 50.0));
    assert_eq!(rect.opacity, 0.5);
    assert_eq!(rect.blend_mode, BlendMode::Multiply);
    assert_eq!(rect.blur, 3.0);
    assert_eq!(rect.shadow.unwrap().y, 8.0);
    assert!(rect.clip.is_none() && rect.mask.is_none());
    let Content::Rect {
        radii,
        fill,
        stroke,
    } = &rect.content
    else {
        panic!("expected a rect, got {:?}", rect.content);
    };
    assert_eq!(*radii, [25.0; 4], "limited to half the shorter side");
    assert_eq!(*fill, solid("#FF0000"));
    let stroke = stroke.as_ref().unwrap();
    assert_eq!(stroke.paint, ScenePaint::solid(Color::rgb(0, 0, 255)));
    assert_eq!(
        (stroke.width, stroke.position),
        (2.0, crate::model::StrokePosition::Inside)
    );
}

#[test]
fn every_shape_carries_its_geometry() {
    let document = document(json!([
        { "id": "e", "type": "ellipse", "width": 80, "height": 40, "start_angle": 90, "sweep": 180, "inner_radius": 0.5 },
        { "id": "s", "type": "star", "points": 7, "inner_radius": 0.6 },
        { "id": "p", "type": "path", "width": 200, "height": 100,
          "path": { "subpaths": [{ "closed": true, "anchors": [
              { "point": [0, 0], "out": [0.5, 0] }, { "point": [1, 1] }, { "point": [0, 1] } ] }] } },
        { "id": "i", "type": "image", "source": "photo.png", "fit": "contain", "width": 30, "height": 30, "corner_radius": 99 }
    ]));
    let scene = at(&document, 0.0);
    assert!(matches!(
        node(&scene, "e").content,
        Content::Ellipse {
            start_angle: 90.0,
            sweep: 180.0,
            inner_radius: 0.5,
            ..
        }
    ));
    assert!(matches!(
        node(&scene, "s").content,
        Content::Star {
            points: 7,
            inner_radius: 0.6,
            ..
        }
    ));
    let Content::Path { path, .. } = &node(&scene, "p").content else {
        panic!()
    };
    let anchors = &path.subpaths[0].anchors;
    assert_eq!(
        anchors[0].handle_out,
        Some([100.0, 0.0]),
        "the path is in pixels"
    );
    assert_eq!(anchors[1].point, [200.0, 100.0]);
    let Content::Image {
        source, fit, radii, ..
    } = &node(&scene, "i").content
    else {
        panic!()
    };
    assert_eq!(
        (source.as_str(), *fit, *radii),
        ("photo.png", crate::model::ImageFit::Contain, [15.0; 4])
    );
    // Layers come back to front.
    let order: Vec<&str> = scene.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(order, ["e", "s", "p", "i"]);
}

#[test]
fn layers_that_cannot_be_seen_are_left_out() {
    let document = document(json!([
        { "id": "hidden", "type": "rect", "hidden": true },
        { "id": "clear", "type": "rect", "opacity": 0 },
        { "id": "tiny", "type": "rect", "scale": 0 },
        { "id": "folder", "type": "group", "hidden": true },
        { "id": "inside", "type": "rect", "parent": "folder" },
        { "id": "seen", "type": "rect" }
    ]));
    let scene = at(&document, 0.0);
    assert_eq!(scene.nodes.len(), 1);
    assert_eq!(scene.nodes[0].id, id("seen"));
    assert_eq!(scene.node_count(), 1);
}

#[test]
fn moves_and_rotations_add_up() {
    let document = document(json!([
        { "id": "r", "type": "rect", "x": 100, "y": 100, "width": 50, "height": 50, "rotation": 10 },
        { "id": "m1", "type": "move", "target": "r", "start": 0, "end": 1000, "to": { "x": 100 }, "easing": linear() },
        { "id": "m2", "type": "move", "target": "r", "start": 500, "end": 1500, "to": { "y": 50 }, "easing": linear() },
        { "id": "t1", "type": "rotate", "target": "r", "start": 0, "end": 1000, "to": 90, "easing": linear() },
        { "id": "t2", "type": "rotate", "target": "r", "start": 0, "end": 1000, "from": 20, "easing": linear() }
    ]));
    let sample = |time: f64| {
        (
            number(&document, "r", Property::X, time),
            number(&document, "r", Property::Y, time),
            number(&document, "r", Property::Rotation, time),
        )
    };
    assert_eq!(sample(0.0), (100.0, 100.0, 30.0));
    assert_eq!(sample(500.0), (150.0, 100.0, 65.0));
    assert_eq!(sample(1000.0), (200.0, 125.0, 100.0));
    assert_eq!(sample(1500.0), (200.0, 150.0, 100.0));
    assert_eq!(sample(9000.0), (200.0, 150.0, 100.0));

    let scene = at(&document, 500.0);
    assert!(same_transform(
        node(&scene, "r").transform,
        Affine::layer(150.0, 100.0, 50.0, 50.0, 65.0, 1.0)
    ));
}

#[test]
fn a_move_with_a_from_waits_there() {
    let document = document(json!([
        { "id": "r", "type": "rect", "x": 100 },
        { "id": "m", "type": "move", "target": "r", "start": 400, "end": 800, "from": { "x": -300 }, "easing": linear() }
    ]));
    let x = |time: f64| number(&document, "r", Property::X, time);
    assert_eq!(
        (x(0.0), x(400.0), x(600.0), x(800.0), x(1200.0)),
        (-200.0, -200.0, -50.0, 100.0, 100.0)
    );
}

#[test]
fn scale_multiplies_the_resting_scale() {
    let document = document(json!([
        { "id": "r", "type": "rect", "x": 100, "y": 100, "width": 40, "height": 20, "scale": 2 },
        { "id": "s1", "type": "scale", "target": "r", "start": 0, "end": 1000, "from": 0.5, "to": 1.5, "easing": linear() },
        { "id": "s2", "type": "scale", "target": "r", "start": 1500, "end": 2000, "easing": linear() }
    ]));
    let scale = |time: f64| number(&document, "r", Property::Scale, time);
    assert_eq!(
        (scale(0.0), scale(500.0), scale(1000.0), scale(1250.0)),
        (1.0, 2.0, 3.0, 3.0)
    );
    // Without a `to` the second one returns to the resting scale.
    assert_eq!((scale(1750.0), scale(2000.0)), (2.5, 2.0));
    // Scaling pivots on the centre of the box, which stays put.
    let scene = at(&document, 500.0);
    let center = node(&scene, "r").transform.apply(Point::new(20.0, 10.0));
    assert!(
        close(center.x, 120.0) && close(center.y, 110.0),
        "{center:?}"
    );
}

#[test]
fn opacity_is_replaced_and_held_before_the_first_cue() {
    let document = document(json!([
        { "id": "r", "type": "rect", "opacity": 80 },
        { "id": "o1", "type": "opacity", "target": "r", "start": 200, "end": 400, "from": 0, "easing": linear() },
        { "id": "o2", "type": "opacity", "target": "r", "start": 300, "end": 500, "to": 20, "easing": linear() }
    ]));
    let opacity = |time: f64| number(&document, "r", Property::Opacity, time);
    assert_eq!(opacity(0.0), 0.0, "the first `from` holds before the start");
    assert_eq!(opacity(250.0), 20.0);
    // The second cuts the first off at 40 and carries on from there.
    assert_eq!(
        (
            opacity(300.0),
            opacity(400.0),
            opacity(500.0),
            opacity(1500.0)
        ),
        (40.0, 30.0, 20.0, 20.0)
    );

    assert!(
        at(&document, 100.0).find(&id("r")).is_none(),
        "invisible, so not in the scene"
    );
    assert!(close(node(&at(&document, 250.0), "r").opacity, 0.2));
}

#[test]
fn equal_starts_are_decided_by_row_order_not_by_folders() {
    let document = document(json!([
        { "id": "r", "type": "rect" },
        { "id": "folder", "type": "animation_group" },
        { "id": "first", "type": "opacity", "target": "r", "start": 0, "end": 100, "to": 10, "parent": "folder" },
        { "id": "second", "type": "opacity", "target": "r", "start": 0, "end": 100, "to": 70 }
    ]));
    assert_eq!(number(&document, "r", Property::Opacity, 100.0), 70.0);
}

#[test]
fn show_and_hide_switch_at_their_moment() {
    let document = document(json!([
        { "id": "a", "type": "rect" },
        { "id": "s1", "type": "show", "target": "a", "start": 300 },
        { "id": "h1", "type": "hide", "target": "a", "start": 600 },
        { "id": "s2", "type": "show", "target": "a", "start": 900 },
        { "id": "b", "type": "rect" },
        { "id": "h2", "type": "hide", "target": "b", "start": 500 },
        { "id": "c", "type": "rect", "hidden": true },
        { "id": "d", "type": "rect", "hidden": true },
        { "id": "h3", "type": "hide", "target": "d", "start": 500 }
    ]));
    let seen = |time: f64| -> Vec<String> {
        at(&document, time)
            .nodes
            .iter()
            .map(|node| node.id.to_string())
            .collect()
    };
    assert_eq!(
        seen(0.0),
        ["b", "d"],
        "a waits for its show; d is shown until its hide"
    );
    assert_eq!(seen(300.0), ["a", "b", "d"]);
    assert_eq!(seen(599.0), ["a"]);
    assert_eq!(seen(600.0), [""; 0]);
    assert_eq!(seen(900.0), ["a"]);
    assert_eq!(
        value_at(&document, &id("a"), Property::Visible, 700.0),
        Some(PropertyValue::Flag(false))
    );
}

#[test]
fn a_resize_keeps_its_anchor_where_it_is() {
    use crate::model::ResizeAnchor;
    for anchor in ResizeAnchor::ALL {
        let name = serde_json::to_value(anchor).unwrap();
        let document = document(json!([
            { "id": "r", "type": "rect", "x": 100, "y": 80, "width": 60, "height": 40, "rotation": 30, "scale": 1.5 },
            { "id": "z", "type": "resize", "target": "r", "start": 200, "end": 1000,
              "from": { "width": 10 }, "to": { "width": 200, "height": 10 }, "anchor": name }
        ]));
        let (fx, fy) = anchor.factors();
        let rest = Affine::layer(100.0, 80.0, 60.0, 40.0, 30.0, 1.5)
            .apply(Point::new(fx * 60.0, fy * 40.0));
        for time in [0.0, 200.0, 450.0, 1000.0, 1800.0] {
            let scene = at(&document, time);
            let rect = node(&scene, "r");
            let now = rect
                .transform
                .apply(Point::new(fx * rect.width, fy * rect.height));
            assert!(
                close(now.x, rest.x) && close(now.y, rest.y),
                "{anchor:?} at {time}: {now:?} is not {rest:?}"
            );
        }
        assert_eq!(number(&document, "r", Property::Width, 0.0), 10.0);
        assert_eq!(number(&document, "r", Property::Height, 0.0), 40.0);
        assert_eq!(number(&document, "r", Property::Width, 1000.0), 200.0);
        assert_eq!(number(&document, "r", Property::Height, 1000.0), 10.0);
    }
}

#[test]
fn a_resize_without_values_returns_to_the_resting_box() {
    let document = document(json!([
        { "id": "r", "type": "rect", "x": 100, "y": 50, "width": 60, "height": 40 },
        { "id": "grow", "type": "resize", "target": "r", "start": 0, "end": 100, "to": { "width": 200 }, "anchor": "left", "easing": linear() },
        { "id": "back", "type": "resize", "target": "r", "start": 200, "end": 300, "anchor": "left", "easing": linear() }
    ]));
    let frame = |time: f64| {
        (
            number(&document, "r", Property::X, time),
            number(&document, "r", Property::Width, time),
        )
    };
    assert_eq!(frame(50.0), (100.0, 130.0));
    assert_eq!(frame(100.0), (100.0, 200.0));
    assert_eq!(frame(250.0), (100.0, 130.0));
    assert_eq!(frame(300.0), (100.0, 60.0));
}

#[test]
fn paints_radii_blur_and_shadow_animate() {
    let document = document(json!([
        { "id": "r", "type": "rect", "width": 200, "height": 100, "fill": "#FF0000", "corner_radius": [0, 0, 40, 40] },
        { "id": "f", "type": "fill", "target": "r", "start": 0, "end": 1000, "to": "#0000FF", "easing": linear() },
        { "id": "s", "type": "stroke", "target": "r", "start": 0, "end": 1000, "to": { "paint": "#00FF00", "width": 8 }, "easing": linear() },
        { "id": "c", "type": "corner_radius", "target": "r", "start": 0, "end": 1000, "to": 20, "easing": linear() },
        { "id": "b", "type": "blur", "target": "r", "start": 0, "end": 1000, "to": 10, "easing": linear() },
        { "id": "d", "type": "shadow", "target": "r", "start": 0, "end": 1000,
          "to": { "color": "#00000080", "y": 20, "blur": 40 }, "easing": linear() }
    ]));
    let scene = at(&document, 500.0);
    let rect = node(&scene, "r");
    let Content::Rect {
        radii,
        fill,
        stroke,
    } = &rect.content
    else {
        panic!()
    };
    assert_eq!(*fill, solid("#800080"));
    assert_eq!(*radii, [10.0, 10.0, 30.0, 30.0]);
    let stroke = stroke.as_ref().unwrap();
    assert_eq!(
        (stroke.width, &stroke.paint),
        (4.0, &ScenePaint::solid(Color::rgb(0, 255, 0)))
    );
    assert_eq!(rect.blur, 5.0);
    assert_eq!(
        rect.shadow,
        Some(Shadow {
            color: Color::rgba(0, 0, 0, 64),
            x: 0.0,
            y: 10.0,
            blur: 20.0
        })
    );

    // At rest there is no stroke and no shadow to draw.
    let rest = at(&document, 0.0);
    let Content::Rect { stroke, .. } = &node(&rest, "r").content else {
        panic!()
    };
    assert!(stroke.is_none() && node(&rest, "r").shadow.is_none());
    assert_eq!(
        value_at(&document, &id("r"), Property::CornerRadius, 1000.0),
        Some(PropertyValue::CornerRadius(CornerRadius::Uniform(20.0)))
    );
    assert_eq!(
        value_at(&document, &id("r"), Property::Fill, 1000.0),
        Some(PropertyValue::Paint(Some(Paint::Solid(Color::rgb(
            0, 0, 255
        )))))
    );
}

#[test]
fn gradients_are_resolved_to_pixels() {
    let stops = json!([{ "offset": 0, "color": "#000000" }, { "offset": 1, "color": "#FFFFFF" }]);
    let document = document(json!([
        { "id": "l", "type": "rect", "width": 200, "height": 100,
          "fill": { "type": "linear_gradient", "stops": stops } },
        { "id": "r", "type": "rect", "width": 200, "height": 100,
          "fill": { "type": "radial_gradient", "center": [0.25, 0.5], "rotation": 30, "stops": stops } },
        { "id": "none", "type": "rect", "fill": "#FF000000", "stroke": { "width": 0 } }
    ]));
    let scene = at(&document, 0.0);
    let fill_of = |name: &str| match &node(&scene, name).content {
        Content::Rect { fill, .. } => fill.clone(),
        other => panic!("{other:?}"),
    };
    let stops = vec![
        GradientStop::new(0.0, Color::BLACK),
        GradientStop::new(1.0, Color::WHITE),
    ];
    assert_eq!(
        fill_of("l"),
        Some(ScenePaint::LinearGradient {
            start: Point::new(100.0, 0.0),
            end: Point::new(100.0, 100.0),
            stops: stops.clone()
        })
    );
    assert_eq!(
        fill_of("r"),
        Some(ScenePaint::RadialGradient {
            center: Point::new(50.0, 50.0),
            radius_x: 100.0,
            radius_y: 50.0,
            rotation: 30.0,
            stops
        })
    );
    assert_eq!(fill_of("none"), None, "a fully transparent fill is no fill");
    let Content::Rect { stroke, .. } = &node(&scene, "none").content else {
        panic!()
    };
    assert!(stroke.is_none(), "a stroke without width is no stroke");
}

#[test]
fn the_artboard_background_animates() {
    let document = document(json!([
        { "id": "bg", "type": "fill", "target": "ab", "start": 0, "end": 1000, "to": "#000000", "easing": linear() }
    ]));
    assert_eq!(at(&document, 0.0).background, solid("#FFFFFF"));
    assert_eq!(at(&document, 500.0).background, solid("#808080"));
    assert_eq!(at(&document, 1000.0).background, solid("#000000"));
}

#[test]
fn groups_nest_their_children() {
    let document = document(json!([
        { "id": "g", "type": "group", "x": 50, "y": 50, "width": 200, "height": 100, "scale": 2, "opacity": 50 },
        { "id": "back", "type": "rect", "parent": "g", "x": 10, "y": 10, "width": 20, "height": 20 },
        { "id": "front", "type": "rect", "parent": "g", "x": 30, "y": 10, "width": 20, "height": 20 },
        { "id": "m", "type": "move", "target": "g", "start": 0, "end": 1000, "to": { "x": 100 }, "easing": linear() },
        { "id": "n", "type": "move", "target": "front", "start": 0, "end": 1000, "to": { "y": 40 }, "easing": linear() }
    ]));
    let scene = at(&document, 1000.0);
    assert_eq!(scene.nodes.len(), 1);
    let group = node(&scene, "g");
    assert_eq!(
        group.opacity, 0.5,
        "opacity belongs to the group as a whole"
    );
    let children: Vec<&str> = group
        .children()
        .iter()
        .map(|child| child.id.as_str())
        .collect();
    assert_eq!(children, ["back", "front"]);
    // A child's transform is relative to the group.
    assert_eq!(
        node(&scene, "front").transform,
        Affine::translate(30.0, 50.0)
    );
    assert_eq!(node(&scene, "front").opacity, 1.0);
    // On the artboard the group's move and scale carry the child along.
    let corner = scene.transform_of(&id("front")).unwrap().apply(Point::ZERO);
    assert!(
        close(corner.x, 50.0 + 100.0 - 100.0 + 60.0) && close(corner.y, 50.0 - 50.0 + 100.0),
        "{corner:?}"
    );
}

#[test]
fn a_group_can_be_a_frame_with_fill_clip_and_outline() {
    let document = document(json!([
        { "id": "g", "type": "group", "x": 50, "y": 50, "width": 200, "height": 100,
          "fill": "#EEEEEE", "stroke": { "width": 2 }, "corner_radius": 16, "clip": true, "shadow": {} },
        { "id": "child", "type": "rect", "parent": "g", "x": 150, "y": 50, "width": 100, "height": 100 }
    ]));
    let scene = at(&document, 0.0);
    let group = &scene.nodes[0];
    assert!(
        group.shadow.is_some() && group.clip.is_none(),
        "the shadow is not clipped away"
    );
    let [background, holder, outline] = group.children() else {
        panic!("expected three parts, got {:?}", group.children());
    };
    assert!(
        matches!(&background.content, Content::Rect { fill: Some(_), stroke: None, radii } if *radii == [16.0; 4])
    );
    assert!(matches!(
        &outline.content,
        Content::Rect {
            fill: None,
            stroke: Some(_),
            ..
        }
    ));
    assert_eq!(
        holder.clip,
        Some(Clip {
            rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            radii: [16.0; 4]
        })
    );
    assert_eq!(holder.children()[0].id, id("child"));
    for part in [background, holder, outline] {
        assert_eq!(
            (part.id.as_str(), part.transform, part.opacity),
            ("g", Affine::IDENTITY, 1.0)
        );
    }
    // The part of the child that sticks out is not hit.
    assert_eq!(
        scene.hit_test(Point::new(240.0, 140.0)),
        [id("g"), id("child")]
    );
    assert!(scene.hit_test(Point::new(280.0, 140.0)).is_empty());
}

#[test]
fn a_mask_group_masks_with_its_first_child() {
    let document = document(json!([
        { "id": "m", "type": "mask_group", "x": 20, "y": 20, "width": 100, "height": 100 },
        { "id": "shape", "type": "ellipse", "parent": "m" },
        { "id": "one", "type": "rect", "parent": "m" },
        { "id": "two", "type": "rect", "parent": "m", "x": 50 },
        { "id": "fade", "type": "opacity", "target": "shape", "start": 0, "end": 1000, "to": 0, "easing": linear() },
        { "id": "empty", "type": "mask_group" }
    ]));
    let scene = at(&document, 500.0);
    assert_eq!(
        scene.nodes.len(),
        1,
        "a mask group without a mask draws nothing"
    );
    let group = &scene.nodes[0];
    let mask = group.mask.as_deref().unwrap();
    assert_eq!((mask.id.as_str(), mask.opacity), ("shape", 0.5));
    assert!(matches!(mask.content, Content::Ellipse { .. }));
    let children: Vec<&str> = group
        .children()
        .iter()
        .map(|child| child.id.as_str())
        .collect();
    assert_eq!(children, ["one", "two"]);
    // Once the mask has faded away, nothing of the group is left.
    assert!(at(&document, 1000.0).nodes.is_empty());
}

fn text_node(extra: Value) -> Document {
    let mut text = json!({
        "id": "t", "type": "text", "content": "Hello\nworld!", "x": 100, "y": 50, "width": 200, "height": 80,
        "font_size": 10, "fill": "#000000"
    });
    text.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    document(json!([text]))
}

fn text_of(scene: &Scene) -> (&SceneNode, &TextContent) {
    let node = node(scene, "t");
    match &node.content {
        Content::Text(text) => (node, text),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn text_is_laid_out_with_the_measurer() {
    // Letters are 6 wide and lines 12 tall: "Hello" is 30, "world!" 36.
    let scene = at(
        &text_node(json!({ "case": "upper", "letter_spacing": 10 })),
        0.0,
    );
    let (node, text) = text_of(&scene);
    assert_eq!(text.text, "HELLO\nWORLD!");
    assert_eq!(text.style.font_size, 10.0);
    assert_eq!(text.style.line_height, 12.0);
    assert_eq!(text.style.letter_spacing, 1.0);
    assert_eq!(text.style.align, TextAlign::Left);
    assert_eq!(text.fill, solid("#000000"));
    assert_eq!(text.wrap_width, None);
    assert_eq!(
        text.units,
        [TextUnit {
            start: 0,
            end: 12,
            transform: Affine::IDENTITY,
            opacity: 1.0,
            blur: 0.0
        }]
    );
    // Six letters of 7 with five gaps of... the spacing follows every letter but the last.
    assert_eq!((node.width, node.height), (41.0, 24.0));
}

#[test]
fn an_auto_sized_text_box_grows_from_its_alignment() {
    let frame = |extra: Value| {
        let scene = at(&text_node(extra), 0.0);
        let (node, text) = text_of(&scene);
        let corner = node.transform.apply(Point::ZERO);
        (corner.x, corner.y, node.width, node.height, text.offset_y)
    };
    // The stored box is 200 x 80 at (100, 50); the text is 36 x 24.
    assert_eq!(frame(json!({})), (100.0, 50.0, 36.0, 24.0, 0.0));
    assert_eq!(
        frame(json!({ "align": "right" })),
        (264.0, 50.0, 36.0, 24.0, 0.0)
    );
    assert_eq!(
        frame(json!({ "align": "center", "vertical_align": "middle" })),
        (182.0, 78.0, 36.0, 24.0, 0.0)
    );
    // Only the height follows the text: it wraps at the stored width.
    assert_eq!(
        frame(json!({ "auto_size": "height", "vertical_align": "bottom" })),
        (100.0, 106.0, 200.0, 24.0, 0.0)
    );
    // A fixed box keeps its size and places the text block inside it.
    assert_eq!(
        frame(json!({ "auto_size": "fixed", "vertical_align": "middle" })),
        (100.0, 50.0, 200.0, 80.0, 28.0)
    );
    assert_eq!(
        frame(json!({ "auto_size": "fixed", "vertical_align": "bottom", "height": 10 })),
        (100.0, 50.0, 200.0, 10.0, -14.0)
    );
}

#[test]
fn text_wraps_at_the_width_of_a_fixed_box() {
    let scene = at(
        &text_node(json!({ "content": "aaa bbb cccc dd", "auto_size": "height", "width": 50 })),
        0.0,
    );
    let (node, text) = text_of(&scene);
    assert_eq!(text.wrap_width, Some(50.0));
    assert_eq!((node.width, node.height), (50.0, 24.0), "two lines of 12");

    let empty = at(&text_node(json!({ "content": "" })), 0.0);
    let (node, text) = text_of(&empty);
    assert!(text.units.is_empty());
    assert_eq!(
        (node.width, node.height),
        (0.0, 12.0),
        "an empty text is one line tall"
    );
}

#[test]
fn values_are_reported_per_property() {
    let document = document(json!([
        { "id": "e", "type": "ellipse", "x": 5, "blur": 2, "shadow": { "y": 3 }, "stroke": { "width": 4 } },
        { "id": "i", "type": "image", "source": "a.png" },
        { "id": "f", "type": "show", "target": "e", "start": 0 }
    ]));
    let value = |layer: &str, property: Property| value_at(&document, &id(layer), property, 0.0);
    assert_eq!(value("e", Property::X), Some(PropertyValue::Number(5.0)));
    assert_eq!(
        value("e", Property::Opacity),
        Some(PropertyValue::Number(100.0))
    );
    assert_eq!(value("e", Property::Blur), Some(PropertyValue::Number(2.0)));
    assert_eq!(
        value("e", Property::Visible),
        Some(PropertyValue::Flag(true))
    );
    assert_eq!(value("e", Property::Fill), Some(PropertyValue::Paint(None)));
    assert!(
        matches!(value("e", Property::Stroke), Some(PropertyValue::Stroke(Some(stroke))) if stroke.width == 4.0)
    );
    assert!(
        matches!(value("e", Property::Shadow), Some(PropertyValue::Shadow(Some(shadow))) if shadow.y == 3.0)
    );
    assert_eq!(
        value("e", Property::CornerRadius),
        None,
        "an ellipse has no corners"
    );
    assert_eq!(value("i", Property::Fill), None, "an image has no fill");
    assert_eq!(
        value("i", Property::CornerRadius),
        Some(PropertyValue::CornerRadius(CornerRadius::Uniform(0.0)))
    );
    assert_eq!(value("f", Property::X), None, "not a layer");
    assert_eq!(value("nope", Property::X), None);
    for property in Property::ALL {
        assert_eq!(Property::parse(property.as_str()), Some(property));
    }
    assert_eq!(Property::parse("colour"), None);
}

#[test]
fn timeline_helpers() {
    let document = document(json!([
        { "id": "r", "type": "rect" },
        { "id": "folder", "type": "animation_group" },
        { "id": "a", "type": "fade_in", "target": "r", "start": 0, "end": 500, "parent": "folder" },
        { "id": "b", "type": "move", "target": "r", "start": 400, "end": 900 },
        { "id": "c", "type": "show", "target": "r", "start": 450 }
    ]));
    assert_eq!(duration_of(&document, &id("ab")), Some(2000.0));
    assert_eq!(duration_of(&document, &id("r")), None);
    assert_eq!(duration_of(&document, &id("nope")), None);
    let active = |time: f64| -> Vec<&str> {
        animations_active_at(&document, &id("ab"), time)
            .iter()
            .map(|node| node.id.as_str())
            .collect()
    };
    assert_eq!(active(0.0), ["a"]);
    assert_eq!(active(450.0), ["a", "b", "c"]);
    assert_eq!(active(500.0), ["a", "b"]);
    assert_eq!(active(950.0), [""; 0]);
}

#[test]
fn only_artboards_can_be_evaluated() {
    let document = document(json!([{ "id": "r", "type": "rect" }]));
    let measurer = MonospaceMeasurer::default();
    assert_eq!(
        evaluate(&document, &id("nope"), 0.0, &measurer),
        Err(EvalError::UnknownArtboard(id("nope")))
    );
    let error = evaluate(&document, &id("r"), 0.0, &measurer).unwrap_err();
    assert_eq!(
        error.to_string(),
        "rect \"r\" is not an artboard; a scene is evaluated for a whole artboard, here \"ab\""
    );
}

#[test]
fn at_rest_no_animation_applies() {
    let document = document(json!([
        { "id": "r", "type": "rect", "x": 10 },
        { "id": "o", "type": "opacity", "target": "r", "start": 500, "end": 900, "from": 0 },
        { "id": "m", "type": "move", "target": "r", "start": 500, "end": 900, "from": { "x": 200 } },
        { "id": "s", "type": "show", "target": "r", "start": 500 }
    ]));
    assert!(at(&document, 0.0).nodes.is_empty());
    let rest = evaluate_at_rest(&document, &id("ab"), &MonospaceMeasurer::default()).unwrap();
    assert_eq!(rest.nodes.len(), 1);
    assert_eq!(rest.nodes[0].transform, Affine::translate(10.0, 0.0));
    assert_eq!(rest.nodes[0].opacity, 1.0);
}

#[test]
fn a_frame_depends_only_on_the_document_and_the_time() {
    let document = document(json!([
        { "id": "r", "type": "rect" },
        { "id": "m", "type": "move", "target": "r", "start": 0, "end": 900, "to": { "x": 200 }, "easing": { "type": "bounce" } },
        { "id": "t", "type": "rotate", "target": "r", "start": 100, "end": 700, "to": 360, "easing": { "type": "elastic" } }
    ]));
    let forward: Vec<Scene> = [0.0, 250.0, 500.0, 750.0]
        .map(|time| at(&document, time))
        .into();
    let backward: Vec<Scene> = [750.0, 500.0, 250.0, 0.0]
        .map(|time| at(&document, time))
        .into();
    assert_eq!(forward, backward.into_iter().rev().collect::<Vec<_>>());
    assert_ne!(forward[1], forward[2]);
    // Odd times are handled: before the start is the start.
    assert_eq!(at(&document, -50.0), at(&document, 0.0));
    assert_eq!(at(&document, f64::NAN), at(&document, 0.0));
    assert_eq!(at(&document, 1e9).time_ms, 1e9);
}

/// A 100 by 100 rect "r" at (100, 100) with the given extra fields, and the
/// given animations, which target it unless they say otherwise and are
/// linear unless they say otherwise.
fn animated(layer: Value, animations: Value) -> Document {
    let mut rect = json!({ "id": "r", "type": "rect", "x": 100, "y": 100, "fill": "#000000" });
    rect.as_object_mut()
        .unwrap()
        .extend(layer.as_object().unwrap().clone());
    let mut nodes = vec![rect];
    for (position, animation) in animations.as_array().unwrap().iter().enumerate() {
        let mut node = json!({ "id": format!("a{position}"), "target": "r", "easing": linear() });
        node.as_object_mut()
            .unwrap()
            .extend(animation.as_object().unwrap().clone());
        nodes.push(node);
    }
    document(Value::Array(nodes))
}

fn samples(document: &Document, property: Property, times: &[f64]) -> Vec<f64> {
    times
        .iter()
        .map(|time| number(document, "r", property, *time))
        .collect()
}

#[test]
fn fades_multiply_the_layers_own_opacity() {
    let document = animated(
        json!({ "opacity": 50 }),
        json!([
            { "type": "fade_in", "start": 200, "end": 600 },
            { "type": "fade_out", "start": 1000, "end": 1400 }
        ]),
    );
    assert_eq!(
        samples(
            &document,
            Property::Opacity,
            &[0.0, 200.0, 400.0, 600.0, 900.0, 1200.0, 1400.0, 1900.0]
        ),
        [0.0, 0.0, 25.0, 50.0, 50.0, 25.0, 0.0, 0.0]
    );
    assert!(
        at(&document, 100.0).nodes.is_empty(),
        "invisible until its cue"
    );
    assert_eq!(node(&at(&document, 400.0), "r").opacity, 0.25);
    assert!(at(&document, 1500.0).nodes.is_empty());

    // With an opacity animation on the same layer both take effect.
    let both = animated(
        json!({}),
        json!([
            { "type": "fade_in", "start": 0, "end": 1000 },
            { "type": "opacity", "start": 0, "end": 1000, "from": 100, "to": 50 }
        ]),
    );
    assert_eq!(
        samples(&both, Property::Opacity, &[500.0, 1000.0]),
        [37.5, 50.0]
    );
}

#[test]
fn a_slide_and_a_move_both_take_effect() {
    let document = animated(
        json!({}),
        json!([
            { "type": "slide_in", "start": 0, "end": 1000, "direction": "left", "distance": 200, "fade": false },
            { "type": "move", "start": 0, "end": 1000, "to": { "y": 50 } }
        ]),
    );
    // Sliding in toward the left starts 200 to the right of rest.
    assert_eq!(
        samples(&document, Property::X, &[0.0, 500.0, 1000.0]),
        [300.0, 200.0, 100.0]
    );
    assert_eq!(
        samples(&document, Property::Y, &[0.0, 500.0, 1000.0]),
        [100.0, 125.0, 150.0]
    );
    assert_eq!(
        node(&at(&document, 0.0), "r").opacity,
        1.0,
        "without the fade it shows throughout"
    );

    let faded = animated(
        json!({}),
        json!([{ "type": "slide_in", "start": 400, "end": 800 }]),
    );
    assert_eq!(
        samples(&faded, Property::Y, &[0.0, 400.0, 600.0, 800.0]),
        [200.0, 200.0, 150.0, 100.0]
    );
    assert_eq!(
        samples(&faded, Property::Opacity, &[0.0, 600.0, 800.0]),
        [0.0, 50.0, 100.0]
    );
    let out = animated(
        json!({}),
        json!([{ "type": "slide_out", "start": 400, "end": 800, "direction": "down", "distance": 40 }]),
    );
    assert_eq!(
        samples(&out, Property::Y, &[0.0, 600.0, 800.0, 1500.0]),
        [100.0, 120.0, 140.0, 140.0]
    );
    assert_eq!(
        samples(&out, Property::Opacity, &[0.0, 600.0, 800.0]),
        [100.0, 50.0, 0.0]
    );
}

#[test]
fn growing_and_spinning_compose_with_scale_and_rotation() {
    let grow = animated(
        json!({ "scale": 2 }),
        json!([
            { "type": "grow_in", "start": 0, "end": 1000, "scale": 0.5, "fade": false },
            { "type": "scale", "start": 0, "end": 1000, "to": 3 }
        ]),
    );
    // Resting scale 2, the scale animation at 2 of 1..3, the preset at 0.75.
    assert_eq!(
        samples(&grow, Property::Scale, &[0.0, 500.0, 1000.0]),
        [1.0, 3.0, 6.0]
    );

    let spin = animated(
        json!({ "rotation": 10 }),
        json!([
            { "type": "spin_in", "start": 0, "end": 1000, "angle": 180 },
            { "type": "rotate", "start": 0, "end": 1000, "to": 90 }
        ]),
    );
    assert_eq!(
        samples(&spin, Property::Rotation, &[0.0, 500.0, 1000.0]),
        [-170.0, -35.0, 100.0]
    );
    assert_eq!(
        samples(&spin, Property::Scale, &[0.0, 500.0, 1000.0]),
        [0.0, 0.5, 1.0]
    );
    assert!(at(&spin, 0.0).nodes.is_empty(), "scaled to nothing");

    let shrink = animated(
        json!({}),
        json!([
            { "type": "shrink_in", "start": 0, "end": 1000 },
            { "type": "grow_out", "start": 1000, "end": 2000, "scale": 3 }
        ]),
    );
    assert_eq!(
        samples(
            &shrink,
            Property::Scale,
            &[0.0, 500.0, 1000.0, 1500.0, 2000.0]
        ),
        [1.5, 1.25, 1.0, 2.0, 3.0]
    );
    assert_eq!(
        samples(&shrink, Property::Opacity, &[0.0, 500.0, 1500.0, 2000.0]),
        [0.0, 50.0, 50.0, 0.0]
    );
    let spin_out = animated(
        json!({}),
        json!([{ "type": "spin_out", "start": 0, "end": 1000, "angle": -90 }]),
    );
    assert_eq!(
        samples(&spin_out, Property::Rotation, &[0.0, 500.0, 1000.0]),
        [0.0, -45.0, -90.0]
    );
    assert_eq!(
        samples(&spin_out, Property::Opacity, &[500.0]),
        [100.0],
        "spins do not fade unless asked"
    );
}

#[test]
fn blur_presets_add_to_the_layers_blur() {
    let document = animated(
        json!({ "blur": 4 }),
        json!([
            { "type": "blur_in", "start": 0, "end": 1000 },
            { "type": "blur_out", "start": 1000, "end": 2000, "blur": 10 }
        ]),
    );
    assert_eq!(
        samples(
            &document,
            Property::Blur,
            &[0.0, 500.0, 1000.0, 1500.0, 2000.0]
        ),
        [24.0, 14.0, 4.0, 9.0, 14.0]
    );
    assert_eq!(
        samples(&document, Property::Opacity, &[0.0, 500.0, 1000.0, 1500.0]),
        [0.0, 50.0, 100.0, 50.0]
    );
    assert_eq!(node(&at(&document, 500.0), "r").blur, 14.0);
}

#[test]
fn an_exit_takes_over_smoothly_from_an_unfinished_entrance() {
    let document = animated(
        json!({}),
        json!([
            { "type": "fade_in", "start": 0, "end": 1000 },
            { "type": "fade_out", "start": 500, "end": 1500 }
        ]),
    );
    assert_eq!(
        samples(
            &document,
            Property::Opacity,
            &[250.0, 500.0, 1000.0, 1500.0]
        ),
        [25.0, 50.0, 25.0, 0.0]
    );
}

#[test]
fn an_entrance_brings_the_layer_back_in_place() {
    let document = animated(
        json!({}),
        json!([
            { "type": "slide_out", "start": 0, "end": 500, "direction": "right", "distance": 300 },
            { "type": "blur_out", "start": 0, "end": 500, "blur": 30 },
            { "type": "shrink_out", "start": 0, "end": 500, "scale": 0.2, "fade": false },
            { "type": "fade_in", "start": 1000, "end": 1500 }
        ]),
    );
    let pose = |time: f64| {
        (
            number(&document, "r", Property::X, time),
            number(&document, "r", Property::Blur, time),
            number(&document, "r", Property::Scale, time),
            number(&document, "r", Property::Opacity, time),
        )
    };
    assert_eq!(
        pose(750.0),
        (400.0, 30.0, 0.2, 0.0),
        "gone, and left where the exits put it"
    );
    assert_eq!(
        pose(1000.0),
        (100.0, 0.0, 1.0, 0.0),
        "the entrance starts from a clean pose"
    );
    assert_eq!(pose(1250.0), (100.0, 0.0, 1.0, 50.0));
    assert_eq!(pose(1500.0), (100.0, 0.0, 1.0, 100.0));
    // A second exit does not reset anything: it adds to what is there.
    let twice = animated(
        json!({}),
        json!([
            { "type": "slide_out", "start": 0, "end": 500, "direction": "right", "distance": 300, "fade": false },
            { "type": "fade_out", "start": 600, "end": 900 }
        ]),
    );
    assert_eq!(
        samples(&twice, Property::X, &[750.0, 900.0]),
        [400.0, 400.0]
    );
}

#[test]
fn reveals_clip_along_the_wipe() {
    let reach = 100_000.0;
    let clip_at = |document: &Document, time: f64| {
        let scene = at(document, time);
        scene
            .find(&id("r"))
            .map(|node| node.clip.map(|clip| (clip.rect, clip.radii)))
    };
    let layer = json!({ "width": 200, "height": 100 });

    let right = animated(
        layer.clone(),
        json!([{ "type": "reveal_in", "start": 0, "end": 1000 }]),
    );
    assert_eq!(clip_at(&right, 0.0), None, "nothing is uncovered yet");
    assert_eq!(
        clip_at(&right, 250.0),
        Some(Some((
            Rect::new(-reach, -reach, 50.0 + reach, 100.0 + 2.0 * reach),
            [0.0; 4]
        )))
    );
    assert_eq!(
        clip_at(&right, 1000.0),
        Some(None),
        "fully uncovered: no clip at all"
    );

    let left = animated(
        layer.clone(),
        json!([{ "type": "reveal_in", "start": 0, "end": 1000, "direction": "left" }]),
    );
    let (rect, _) = clip_at(&left, 250.0).unwrap().unwrap();
    assert_eq!((rect.x, rect.right()), (150.0, 200.0 + reach));

    let down = animated(
        layer.clone(),
        json!([{ "type": "reveal_out", "start": 1000, "end": 2000, "direction": "down" }]),
    );
    assert_eq!(
        clip_at(&down, 500.0),
        Some(None),
        "whole until the cover starts"
    );
    let (rect, _) = clip_at(&down, 1500.0).unwrap().unwrap();
    assert_eq!(
        (rect.y, rect.bottom(), rect.x),
        (50.0, 100.0 + reach, -reach)
    );
    assert_eq!(clip_at(&down, 2000.0), None, "fully covered");

    let up = animated(
        layer,
        json!([{ "type": "reveal_out", "start": 0, "end": 1000, "direction": "up" }]),
    );
    let (rect, _) = clip_at(&up, 250.0).unwrap().unwrap();
    assert_eq!((rect.y, rect.bottom()), (-reach, 75.0));
}

#[test]
fn presets_on_a_group_carry_its_children() {
    let document = document(json!([
        { "id": "g", "type": "group", "x": 50, "y": 50, "width": 100, "height": 100, "clip": true },
        { "id": "child", "type": "rect", "parent": "g", "x": 10, "y": 10 },
        { "id": "in", "type": "slide_in", "target": "g", "start": 0, "end": 1000, "direction": "right",
          "distance": 80, "easing": linear() },
        { "id": "wipe", "type": "reveal_in", "target": "g", "start": 0, "end": 1000, "easing": linear() }
    ]));
    let scene = at(&document, 500.0);
    let group = node(&scene, "g");
    assert_eq!(group.opacity, 0.5);
    let corner = scene.transform_of(&id("child")).unwrap().apply(Point::ZERO);
    assert_eq!(
        (corner.x, corner.y),
        (20.0, 60.0),
        "40 short of its place at (60, 60)"
    );
    // The reveal clips the group as a whole; its own clip sits inside.
    assert_eq!(group.clip.unwrap().rect.right(), 50.0);
    assert!(group.children()[0].clip.is_some());
}

#[test]
fn presets_do_not_apply_at_rest() {
    let document = animated(
        json!({}),
        json!([
            { "type": "fade_in", "start": 500, "end": 900 },
            { "type": "spin_in", "start": 500, "end": 900 },
            { "type": "reveal_in", "start": 500, "end": 900 }
        ]),
    );
    assert!(at(&document, 0.0).nodes.is_empty());
    let rest = evaluate_at_rest(&document, &id("ab"), &MonospaceMeasurer::default()).unwrap();
    assert_eq!(rest.nodes[0].opacity, 1.0);
    assert_eq!(rest.nodes[0].transform, Affine::translate(100.0, 100.0));
    assert!(rest.nodes[0].clip.is_none());
}

/// A text layer "t" saying "Hi you" in letters 6 wide on lines 12 tall, with
/// the given extra fields and the given linear animations on it.
fn animated_text(layer: Value, animations: Value) -> Document {
    let mut text = json!({
        "id": "t", "type": "text", "content": "Hi you", "font_size": 10, "fill": "#000000"
    });
    text.as_object_mut()
        .unwrap()
        .extend(layer.as_object().unwrap().clone());
    let mut nodes = vec![text];
    for (position, animation) in animations.as_array().unwrap().iter().enumerate() {
        let mut node = json!({ "id": format!("a{position}"), "target": "t", "easing": linear() });
        node.as_object_mut()
            .unwrap()
            .extend(animation.as_object().unwrap().clone());
        nodes.push(node);
    }
    document(Value::Array(nodes))
}

/// The units of the text layer at a time, as the text they cover with
/// their opacity.
fn pieces(document: &Document, time: f64) -> Vec<(String, f64)> {
    let scene = at(document, time);
    let Some(node) = scene.find(&id("t")) else {
        return Vec::new();
    };
    let Content::Text(text) = &node.content else {
        panic!()
    };
    text.units
        .iter()
        .map(|unit| {
            (
                text.text[unit.start..unit.end].to_owned(),
                (unit.opacity * 1e6).round() / 1e6,
            )
        })
        .collect()
}

fn units(document: &Document, time: f64) -> Vec<TextUnit> {
    let scene = at(document, time);
    match &node(&scene, "t").content {
        Content::Text(text) => text.units.clone(),
        other => panic!("{other:?}"),
    }
}

fn piece(text: &str, opacity: f64) -> (String, f64) {
    (text.to_owned(), opacity)
}

#[test]
fn text_comes_in_letter_by_letter() {
    // Five letters, 100 apart, each taking 600 of the 1000.
    let document = animated_text(
        json!({}),
        json!([{ "type": "text_in", "start": 0, "end": 1000, "stagger": 100 }]),
    );
    assert!(pieces(&document, 0.0).is_empty(), "nothing has come in yet");
    assert_eq!(
        pieces(&document, 300.0),
        [piece("H", 0.5), piece("i", 0.333333), piece("y", 0.166667)]
    );
    assert_eq!(
        pieces(&document, 700.0),
        [
            piece("Hi", 1.0),
            piece("y", 0.833333),
            piece("o", 0.666667),
            piece("u", 0.5)
        ]
    );
    // Once everything is in, the text is one plain unit again.
    assert_eq!(pieces(&document, 1000.0), [piece("Hi you", 1.0)]);
    assert_eq!(units(&document, 1000.0)[0].transform, Affine::IDENTITY);
}

#[test]
fn text_waits_unseen_until_its_cue() {
    let document = animated_text(
        json!({}),
        json!([{ "type": "text_in", "start": 500, "end": 900, "split": "words" }]),
    );
    assert!(pieces(&document, 0.0).is_empty());
    assert!(pieces(&document, 500.0).is_empty());
    assert!(!pieces(&document, 600.0).is_empty());
    // The layer itself is there the whole time; only its units are not.
    assert!(at(&document, 0.0).find(&id("t")).is_some());
}

#[test]
fn words_rise_into_place() {
    // Two words 200 apart, 800 each; half a line (6 pixels) of travel.
    let document = animated_text(
        json!({}),
        json!([{ "type": "text_in", "start": 0, "end": 1000, "split": "words", "effect": "slide_up",
                  "stagger": 200, "distance": 50 }]),
    );
    let units = units(&document, 400.0);
    assert_eq!(units.len(), 2);
    assert_eq!(
        (units[0].start, units[0].end, units[0].opacity),
        (0, 2, 0.5)
    );
    assert_eq!(
        units[0].transform,
        Affine::translate(0.0, 3.0),
        "half way up from 6 below"
    );
    assert_eq!(
        (units[1].start, units[1].end, units[1].opacity),
        (3, 6, 0.25)
    );
    assert_eq!(units[1].transform, Affine::translate(0.0, 4.5));

    let down = animated_text(
        json!({}),
        json!([{ "type": "text_in", "start": 0, "end": 1000, "split": "lines", "effect": "slide_down" }]),
    );
    assert_eq!(
        super::tests::units(&down, 500.0)[0].transform,
        Affine::translate(0.0, -6.0)
    );
}

#[test]
fn pieces_scale_about_their_own_centre() {
    let document = animated_text(
        json!({ "auto_size": "fixed", "width": 200, "height": 80, "vertical_align": "middle" }),
        json!([{ "type": "text_in", "start": 0, "end": 1000, "split": "words", "effect": "scale",
                  "stagger": 0, "scale": 0.5 }]),
    );
    let units = units(&document, 500.0);
    // "Hi" is 12 wide on a line 12 tall, and the text block sits 34 down in
    // the box: its centre is (6, 40).
    let center = units[0].transform.apply(Point::new(6.0, 40.0));
    assert!(close(center.x, 6.0) && close(center.y, 40.0), "{center:?}");
    let corner = units[0].transform.apply(Point::new(0.0, 34.0));
    assert!(close(corner.x, 1.5) && close(corner.y, 35.5), "{corner:?}");
    // "you" scales about its own centre, 27 along.
    let other = units[1].transform.apply(Point::new(27.0, 40.0));
    assert!(close(other.x, 27.0) && close(other.y, 40.0), "{other:?}");
}

#[test]
fn pieces_come_into_focus() {
    let document = animated_text(
        json!({}),
        json!([{ "type": "text_in", "start": 0, "end": 1000, "split": "lines", "effect": "blur", "blur": 16 }]),
    );
    let units = units(&document, 250.0);
    assert_eq!((units[0].blur, units[0].opacity), (12.0, 0.25));
    assert_eq!(super::tests::units(&document, 1000.0)[0].blur, 0.0);
}

#[test]
fn text_goes_out_backward_from_the_last_letter() {
    // Five letters, 100 apart, 100 each: the last letter goes first.
    let document = animated_text(
        json!({}),
        json!([{ "type": "text_out", "start": 0, "end": 500, "stagger": 100, "order": "backward" }]),
    );
    assert_eq!(pieces(&document, 0.0), [piece("Hi you", 1.0)]);
    assert_eq!(
        pieces(&document, 150.0),
        [piece("Hi y", 1.0), piece("o", 0.5)]
    );
    assert_eq!(pieces(&document, 450.0), [piece("H", 0.5)]);
    assert!(pieces(&document, 500.0).is_empty());
}

#[test]
fn text_can_come_in_go_out_and_come_back() {
    let document = animated_text(
        json!({}),
        json!([
            { "type": "text_in", "start": 0, "end": 400, "stagger": 0 },
            { "type": "text_out", "start": 1000, "end": 1400, "split": "words", "effect": "slide_down", "stagger": 0 },
            { "type": "text_in", "start": 2000, "end": 2400, "split": "lines" }
        ]),
    );
    assert_eq!(pieces(&document, 200.0), [piece("Hi you", 0.5)]);
    assert_eq!(pieces(&document, 700.0), [piece("Hi you", 1.0)]);
    // Going out, both words drop together: still one unit, on the move.
    let leaving = units(&document, 1200.0);
    assert_eq!(leaving.len(), 1);
    assert_eq!(
        (leaving[0].opacity, leaving[0].transform),
        (0.5, Affine::translate(0.0, 6.0))
    );
    assert!(pieces(&document, 1700.0).is_empty());
    // Coming back, the drop is forgotten.
    let back = units(&document, 2200.0);
    assert_eq!(
        (back[0].opacity, back[0].transform),
        (0.5, Affine::IDENTITY)
    );
}

#[test]
fn a_text_out_takes_over_from_an_unfinished_text_in() {
    let document = animated_text(
        json!({}),
        json!([
            { "type": "text_in", "start": 0, "end": 1000, "split": "lines" },
            { "type": "text_out", "start": 500, "end": 1500, "split": "lines" }
        ]),
    );
    assert_eq!(pieces(&document, 500.0), [piece("Hi you", 0.5)]);
    assert_eq!(pieces(&document, 1000.0), [piece("Hi you", 0.25)]);
}

#[test]
fn text_is_one_unit_at_rest_and_without_animations() {
    let document = animated_text(
        json!({}),
        json!([{ "type": "text_in", "start": 500, "end": 900 }]),
    );
    let rest = evaluate_at_rest(&document, &id("ab"), &MonospaceMeasurer::default()).unwrap();
    let Content::Text(text) = &rest.nodes[0].content else {
        panic!()
    };
    assert_eq!(
        text.units,
        [TextUnit {
            start: 0,
            end: 6,
            transform: Affine::IDENTITY,
            opacity: 1.0,
            blur: 0.0
        }]
    );
}

fn content(document: &Document, time: f64) -> String {
    match value_at(document, &id("t"), Property::Content, time) {
        Some(PropertyValue::Text(text)) => text,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_counter_counts_the_number_in_the_text() {
    let document = animated_text(
        json!({ "content": "$1,250.50 raised", "case": "upper" }),
        json!([{ "type": "counter", "start": 0, "end": 1000, "from": 0 }]),
    );
    assert_eq!(content(&document, 0.0), "$0.00 raised");
    assert_eq!(content(&document, 500.0), "$625.25 raised");
    assert_eq!(content(&document, 1000.0), "$1,250.50 raised");
    assert_eq!(content(&document, 5000.0), "$1,250.50 raised");

    // The scene carries the counted text, in the layer's letter case, and
    // the box follows it: 14 letters of 6, then 16.
    let scene = at(&document, 500.0);
    let (node, text) = text_of(&scene);
    assert_eq!(text.text, "$625.25 RAISED");
    assert_eq!(text.units[0].end, 14);
    assert_eq!(node.width, 84.0);
    assert_eq!(text_of(&at(&document, 1000.0)).0.width, 96.0);
}

#[test]
fn counters_follow_the_replacing_rule() {
    let document = animated_text(
        json!({ "content": "Top 10" }),
        json!([
            { "type": "counter", "start": 200, "end": 400, "from": 0, "to": 100 },
            { "type": "counter", "start": 300, "end": 500, "to": 20 },
            { "type": "counter", "start": 800, "end": 900 }
        ]),
    );
    let number = |time: f64| content(&document, time);
    assert_eq!(
        number(0.0),
        "Top 0",
        "the first `from` holds before the start"
    );
    assert_eq!(number(300.0), "Top 50");
    assert_eq!(
        number(400.0),
        "Top 35",
        "picked up at 50 and heading for 20"
    );
    assert_eq!(number(600.0), "Top 20");
    assert_eq!(
        number(850.0),
        "Top 15",
        "back toward the number that is written"
    );
    assert_eq!(number(900.0), "Top 10");
}

#[test]
fn a_counter_leaves_text_without_a_number_alone() {
    let document = animated_text(
        json!({ "content": "No digits" }),
        json!([{ "type": "counter", "start": 0, "end": 1000, "from": 0, "to": 9 }]),
    );
    assert_eq!(content(&document, 500.0), "No digits");
    // Layers that are not text have no content.
    let other = animated(json!({}), json!([]));
    assert_eq!(value_at(&other, &id("r"), Property::Content, 0.0), None);
}

#[test]
fn counters_and_text_animations_work_on_the_same_layer() {
    let document = animated_text(
        json!({ "content": "7 days" }),
        json!([
            { "type": "counter", "start": 0, "end": 1000, "from": 100 },
            { "type": "text_in", "start": 0, "end": 1000, "split": "words", "stagger": 500 }
        ]),
    );
    // At 750 the counter shows 30 (from 30.25), the first word is in and the
    // second is half way.
    assert_eq!(
        pieces(&document, 750.0),
        [piece("30", 1.0), piece("days", 0.5)]
    );
}

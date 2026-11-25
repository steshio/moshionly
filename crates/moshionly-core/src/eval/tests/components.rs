//! Components and instances: a main component's look, layers and
//! animations shown elsewhere.

use serde_json::{Value, json};

use super::*;
use crate::edit::Edit;

/// A red 100 by 50 component "m" with a blue dot "dot" in it, and two
/// instances of it, "i1" and "i2", plus whatever `extra` adds.
fn with_component(extra: Value) -> Document {
    let mut nodes = vec![
        json!({ "id": "m", "type": "component", "x": 10, "y": 10, "width": 100, "height": 50,
                "fill": "#FF0000", "corner_radius": 4 }),
        json!({ "id": "dot", "parent": "m", "index": "a0", "type": "rect", "width": 10, "height": 10,
                "fill": "#0000FF" }),
        json!({ "id": "i1", "type": "instance", "component": "m", "x": 200, "width": 100, "height": 50 }),
        json!({ "id": "i2", "type": "instance", "component": "m", "y": 200, "width": 100, "height": 50 }),
    ];
    nodes.extend(extra.as_array().unwrap().iter().cloned());
    document(Value::Array(nodes))
}

/// A node's content with every id replaced by `id`.
fn relabeled(content: &Content, id: &str) -> Content {
    let mut content = content.clone();
    relabel(&mut content, &NodeId::from(id));
    content
}

/// Where the dot is inside the instance (or the component) `layer`.
fn dot_x(scene: &Scene, layer: &str) -> f64 {
    let inside = node(scene, layer).children();
    // The component's background comes first, then the dot.
    inside[1].transform.0[4]
}

fn ids(node: &SceneNode, found: &mut Vec<String>) {
    found.push(node.id.to_string());
    if let Some(mask) = &node.mask {
        ids(mask, found);
    }
    for child in node.children() {
        ids(child, found);
    }
}

#[test]
fn an_instance_draws_its_component_under_its_own_id() {
    let document = with_component(json!([]));
    let scene = at(&document, 0.0);
    let main = node(&scene, "m");
    let copy = node(&scene, "i1");
    assert!(same_transform(
        copy.transform,
        Affine::layer(200.0, 0.0, 100.0, 50.0, 0.0, 1.0)
    ));
    assert_eq!(copy.content, relabeled(&main.content, "i1"));
    let mut found = Vec::new();
    ids(copy, &mut found);
    assert!(
        found.len() >= 3 && found.iter().all(|id| id == "i1"),
        "{found:?}"
    );
    // The dot inside the main is still found by its own id.
    assert_eq!(scene.find(&id("dot")).map(|dot| dot.width), Some(10.0));
}

#[test]
fn editing_the_component_changes_every_instance() {
    let mut document = with_component(json!([]));
    document
        .apply(&[
            Edit::set("dot", "fill", "#00FF00"),
            Edit::set("m", "fill", "#FFFF00"),
        ])
        .unwrap();
    let scene = at(&document, 0.0);
    for layer in ["m", "i1", "i2"] {
        let inside = node(&scene, layer).children();
        let fill = |node: &SceneNode| match &node.content {
            Content::Rect { fill, .. } => fill.as_ref().and_then(ScenePaint::as_solid),
            other => panic!("expected a rect, got {other:?}"),
        };
        assert_eq!(fill(&inside[0]), Color::parse("#FFFF00").ok(), "{layer}");
        assert_eq!(fill(&inside[1]), Color::parse("#00FF00").ok(), "{layer}");
    }
}

#[test]
fn animations_inside_the_component_play_in_every_instance_later_by_the_offset() {
    let document = with_component(json!([
        { "id": "go", "type": "move", "target": "dot", "start": 0, "end": 1000,
          "to": { "x": 100 }, "easing": linear() },
    ]));
    let mut document = document;
    document
        .apply(&[Edit::set("i2", "time_offset", 500)])
        .unwrap();
    let x = |document: &Document, time: f64| {
        let scene = at(document, time);
        (dot_x(&scene, "m"), dot_x(&scene, "i1"), dot_x(&scene, "i2"))
    };
    assert_eq!(x(&document, 0.0), (0.0, 0.0, 0.0));
    assert_eq!(x(&document, 500.0), (50.0, 50.0, 0.0));
    assert_eq!(x(&document, 1000.0), (100.0, 100.0, 50.0));
    assert_eq!(x(&document, 1500.0), (100.0, 100.0, 100.0));

    // A negative offset plays it earlier.
    document
        .apply(&[Edit::set("i2", "time_offset", -250)])
        .unwrap();
    assert_eq!(x(&document, 0.0).2, 25.0);
}

#[test]
fn the_instance_has_its_own_animations_and_not_the_components() {
    let document = with_component(json!([
        { "id": "fade", "type": "opacity", "target": "i1", "start": 0, "end": 1000, "from": 0,
          "to": 100, "easing": linear() },
        { "id": "away", "type": "move", "target": "m", "start": 0, "end": 1000,
          "to": { "y": 100 }, "easing": linear() },
        { "id": "tint", "type": "fill", "target": "m", "start": 0, "end": 1000,
          "to": "#000000", "easing": linear() },
    ]));
    let scene = at(&document, 500.0);
    let copy = node(&scene, "i1");
    assert!(close(copy.opacity, 0.5));
    assert_eq!(node(&scene, "i2").opacity, 1.0);
    // The component moves and darkens where it is placed; its instances
    // stay red where they are.
    assert!(close(node(&scene, "m").transform.0[5], 60.0));
    assert!(close(copy.transform.0[5], 0.0));
    let Content::Rect { fill, .. } = &copy.children()[0].content else {
        panic!("expected the background");
    };
    assert_eq!(fill, &solid("#FF0000"));
}

#[test]
fn a_resized_instance_scales_the_content_to_its_box() {
    let mut document = with_component(json!([]));
    document
        .apply(&[
            Edit::update("i1", [("width", json!(200)), ("height", json!(100))]),
            Edit::update("i2", [("width", json!(300))]),
        ])
        .unwrap();
    let scene = at(&document, 0.0);
    let main = node(&scene, "m");
    for (layer, sx, sy) in [("i1", 2.0, 2.0), ("i2", 3.0, 1.0)] {
        let copy = node(&scene, layer);
        let [holder] = copy.children() else {
            panic!("expected one scaled holder in {layer}");
        };
        assert_eq!(holder.id, id(layer));
        assert_eq!(holder.transform, Affine::scale(sx, sy));
        assert_eq!((holder.width, holder.height), (100.0, 50.0));
        assert_eq!(holder.content, relabeled(&main.content, layer));
    }
}

#[test]
fn a_resize_animation_on_an_instance_grows_its_box_not_its_content() {
    // "i1" rests at twice the component's size and grows to four times.
    let mut document = with_component(json!([
        { "id": "grow", "type": "resize", "target": "i1", "start": 0, "end": 1000,
          "to": { "width": 400, "height": 200 }, "anchor": "top_left", "easing": linear() },
    ]));
    document
        .apply(&[Edit::update(
            "i1",
            [("width", json!(200)), ("height", json!(100))],
        )])
        .unwrap();
    let scene = at(&document, 1000.0);
    let copy = node(&scene, "i1");
    assert_eq!((copy.width, copy.height), (400.0, 200.0));
    let [holder] = copy.children() else {
        panic!("expected one scaled holder");
    };
    // The content keeps the resting fit; the component's box, in its own
    // units, grows so that its background fills the instance's box.
    assert_eq!(holder.transform, Affine::scale(2.0, 2.0));
    assert_eq!((holder.width, holder.height), (200.0, 100.0));
    let inside = holder.children();
    assert_eq!((inside[0].width, inside[0].height), (200.0, 100.0));
    assert_eq!((inside[1].width, inside[1].height), (10.0, 10.0));
}

#[test]
fn instances_nest_and_their_offsets_add_up() {
    let document = with_component(json!([
        { "id": "go", "type": "move", "target": "dot", "start": 0, "end": 1000,
          "to": { "x": 100 }, "easing": linear() },
        { "id": "outer", "type": "component", "y": 100, "width": 100, "height": 50 },
        { "id": "nested", "parent": "outer", "index": "a0", "type": "instance", "component": "m",
          "width": 100, "height": 50, "time_offset": 200 },
        { "id": "o1", "type": "instance", "component": "outer", "x": 200, "y": 100,
          "width": 100, "height": 50, "time_offset": 300 },
    ]));
    let scene = at(&document, 1000.0);
    let copy = node(&scene, "o1");
    let mut found = Vec::new();
    ids(copy, &mut found);
    assert!(found.iter().all(|id| id == "o1"), "{found:?}");
    // o1 shows "outer" at 700, which shows "m" at 500.
    let nested = &copy.children()[0];
    assert_eq!(nested.children()[1].transform.0[4], 50.0);
    // "outer" itself shows "m" at 800.
    assert_eq!(dot_x(&scene, "nested"), 80.0);
}

#[test]
fn a_component_in_another_artboard_brings_its_animations() {
    let document = document(json!([
        { "id": "ab2", "index": "a1", "type": "artboard", "x": 500, "width": 200, "height": 200 },
        { "id": "m", "parent": "ab2", "index": "a0", "type": "component", "width": 100, "height": 50 },
        { "id": "dot", "parent": "m", "index": "a0", "type": "rect", "width": 10, "height": 10,
          "fill": "#0000FF" },
        { "id": "go", "parent": "ab2", "index": "a0", "type": "move", "target": "dot",
          "start": 0, "end": 1000, "to": { "x": 100 }, "easing": linear() },
        { "id": "i1", "type": "instance", "component": "m", "width": 100, "height": 50 },
    ]));
    let scene = at(&document, 250.0);
    assert_eq!(node(&scene, "i1").children()[0].transform.0[4], 25.0);
    assert!(
        scene.find(&id("m")).is_none(),
        "the component itself is elsewhere"
    );
}

#[test]
fn an_instance_is_one_layer_to_hit_testing() {
    let document = with_component(json!([]));
    let scene = at(&document, 0.0);
    // On the dot inside "i1", and on its background.
    assert_eq!(scene.hit_test(Point::new(205.0, 5.0)), vec![id("i1")]);
    assert_eq!(scene.hit_test(Point::new(250.0, 30.0)), vec![id("i1")]);
    // Inside the component itself the dot is its own layer.
    assert_eq!(
        scene.hit_test(Point::new(15.0, 15.0)),
        vec![id("m"), id("dot")]
    );
    let found = scene.find(&id("i1")).unwrap();
    assert_eq!((found.width, found.height), (100.0, 50.0));
}

#[test]
fn at_rest_instances_show_the_component_as_designed() {
    let document = with_component(json!([
        { "id": "go", "type": "move", "target": "dot", "start": 0, "end": 1000,
          "from": { "x": 300 }, "easing": linear() },
    ]));
    let rest = evaluate_at_rest(&document, &id("ab"), &MonospaceMeasurer::default()).unwrap();
    assert_eq!(dot_x(&rest, "i1"), 0.0);
    assert_eq!(dot_x(&at(&document, 0.0), "i1"), 300.0);
}

#[test]
fn a_broken_document_still_evaluates() {
    // Validation forbids a component that shows itself; evaluation stops
    // anyway.
    let nodes = [
        json!({ "id": "ab", "index": "a0", "type": "artboard" }),
        json!({ "id": "m", "parent": "ab", "index": "a0", "type": "component" }),
        json!({ "id": "self", "parent": "m", "index": "a0", "type": "instance", "component": "m" }),
        json!({ "id": "lost", "parent": "ab", "index": "a1", "type": "instance", "component": "gone" }),
    ];
    let document = Document::from_nodes_unchecked(
        "Broken",
        nodes.map(|value| Node::from_value(value).unwrap()),
    );
    let scene = at(&document, 0.0);
    assert_eq!(node(&scene, "lost").children(), &[]);
    assert!(scene.node_count() > MAX_INSTANCE_DEPTH);
}

use serde_json::{Value, json};

use super::*;
use crate::build::{clone_subtree, duplicate, remove};
use crate::eval::evaluate;
use crate::geometry::Affine;
use crate::import::{self, Destination};
use crate::scene::{Content, Scene, SceneNode};
use crate::text::MonospaceMeasurer;
use crate::validate::ProblemKind;

fn id(text: &str) -> NodeId {
    NodeId::from(text)
}

fn add(value: Value) -> Edit {
    Edit::add(Node::from_value(value).unwrap())
}

/// One 400 by 300 artboard "ab" with a clipped, rounded, outlined red
/// component "card" holding a blue dot that moves and fades in, and an
/// opacity animation on the card itself. An instance "copy" of it plays
/// 300 ms later and turns; "big" is twice its size, "wide" three times as
/// wide. A second artboard "ab2" holds a component "badge" with an
/// instance of "card" in it.
fn sample() -> Document {
    let mut document = Document::new("Components");
    document
        .apply(&[
            add(json!({ "id": "ab", "index": "a0", "type": "artboard", "width": 400, "height": 300,
                        "duration": 2000 })),
            add(json!({ "id": "ab2", "index": "a1", "type": "artboard", "x": 500 })),
            add(json!({ "id": "card", "parent": "ab", "index": "a0", "type": "component",
                        "name": "Card", "x": 10, "y": 10, "width": 100, "height": 50,
                        "fill": "#FF0000", "corner_radius": 8, "clip": true, "opacity": 90,
                        "stroke": { "paint": "#000000", "width": 2 } })),
            add(json!({ "id": "dot", "parent": "card", "index": "a0", "type": "rect",
                        "x": 5, "y": 5, "width": 10, "height": 10, "fill": "#0000FF",
                        "rotation": 30 })),
            add(json!({ "id": "copy", "parent": "ab", "index": "a1", "type": "instance",
                        "component": "card", "x": 200, "y": 100, "width": 100, "height": 50,
                        "time_offset": 300 })),
            add(json!({ "id": "big", "parent": "ab", "index": "a2", "type": "instance",
                        "component": "card", "y": 150, "width": 200, "height": 100 })),
            add(json!({ "id": "wide", "parent": "ab", "index": "a3", "type": "instance",
                        "component": "card", "y": 260, "width": 300, "height": 50 })),
            add(json!({ "id": "go", "parent": "ab", "index": "a0", "type": "move", "target": "dot",
                        "start": 0, "end": 1000, "to": { "x": 80 }, "easing": { "type": "linear" } })),
            add(json!({ "id": "in", "parent": "ab", "index": "a1", "type": "fade_in", "target": "dot",
                        "start": 100, "end": 600 })),
            add(json!({ "id": "dim", "parent": "ab", "index": "a2", "type": "opacity", "target": "card",
                        "start": 0, "end": 500, "to": 20 })),
            add(json!({ "id": "turn", "parent": "ab", "index": "a3", "type": "rotate", "target": "copy",
                        "start": 0, "end": 1000, "to": 90 })),
            add(json!({ "id": "badge", "parent": "ab2", "index": "a0", "type": "component",
                        "width": 120, "height": 60 })),
            add(json!({ "id": "inner", "parent": "badge", "index": "a0", "type": "instance",
                        "component": "card", "width": 100, "height": 50 })),
        ])
        .unwrap();
    document
}

/// Applies edits to a copy, checks the rules, and checks that the inverse
/// restores the original.
fn applied(document: &Document, edits: &[Edit]) -> Document {
    let mut result = document.clone();
    let inverse = result
        .apply(edits)
        .unwrap_or_else(|error| panic!("{error}"));
    result.validate().unwrap();
    let mut undone = result.clone();
    undone.apply(&inverse).unwrap();
    assert_eq!(&undone, document, "one undo step restores it");
    result
}

fn scene(document: &Document, artboard: &str, time: f64) -> Scene {
    evaluate(document, &id(artboard), time, &MonospaceMeasurer::default()).unwrap()
}

/// What one drawn leaf of a scene looks like on the artboard, whatever
/// the nodes are called and however the transforms are split between
/// them.
#[derive(Debug)]
struct Drawn {
    corners: Vec<[f64; 2]>,
    clips: Vec<Vec<[f64; 2]>>,
    opacity: f64,
    look: Value,
}

fn drawn(scene: &Scene) -> Vec<Drawn> {
    fn corners(world: Affine, width: f64, height: f64) -> Vec<[f64; 2]> {
        [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
            .map(|(x, y)| {
                let point = world.apply(Point::new(x, y));
                [point.x, point.y]
            })
            .to_vec()
    }
    fn walk(
        node: &SceneNode,
        outer: Affine,
        opacity: f64,
        clips: &[Vec<[f64; 2]>],
        found: &mut Vec<Drawn>,
    ) {
        let world = outer * node.transform;
        let [a, b, ..] = world.0;
        let scale = a.hypot(b);
        let opacity = opacity * node.opacity;
        let mut clips = clips.to_vec();
        if let Some(clip) = node.clip {
            let mut box_corners = corners(
                world * Affine::translate(clip.rect.x, clip.rect.y),
                clip.rect.width,
                clip.rect.height,
            );
            box_corners.push([clip.radii[0] * scale, 0.0]);
            clips.push(box_corners);
        }
        let mut look = match &node.content {
            Content::Group { children } => {
                for child in children {
                    walk(child, world, opacity, &clips, found);
                }
                return;
            }
            Content::Rect {
                radii,
                fill,
                stroke,
            } => json!({
                "radii": radii.map(|radius| radius * scale),
                "fill": fill,
                "stroke": stroke.as_ref().map(|stroke| (stroke.width * scale, &stroke.paint)),
            }),
            other => serde_json::to_value(other).unwrap(),
        };
        look["blur"] = json!(node.blur * scale);
        found.push(Drawn {
            corners: corners(world, node.width, node.height),
            clips,
            opacity,
            look,
        });
    }
    let mut found = Vec::new();
    for node in &scene.nodes {
        walk(node, Affine::IDENTITY, 1.0, &[], &mut found);
    }
    found
}

/// Numbers equal within a hair, everything else exactly.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-6
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same(a, b)))
        }
        _ => a == b,
    }
}

fn looks_the_same(before: &Scene, after: &Scene) -> bool {
    let value = |scene: &Scene| {
        Value::Array(
            drawn(scene)
                .into_iter()
                .map(|drawn| {
                    json!({ "corners": drawn.corners, "clips": drawn.clips,
                            "opacity": drawn.opacity, "look": drawn.look })
                })
                .collect(),
        )
    };
    same(&value(before), &value(after))
}

const TIMES: [f64; 7] = [0.0, 150.0, 350.0, 600.0, 900.0, 1300.0, 2000.0];

#[test]
fn one_group_becomes_a_component_in_place() {
    let mut document = sample();
    document
        .apply(&[
            add(json!({ "id": "g", "parent": "ab", "index": "a5", "type": "group", "name": "Box" })),
            add(json!({ "id": "kid", "parent": "g", "index": "a0", "type": "ellipse" })),
            add(json!({ "id": "wave", "parent": "ab", "index": "a5", "type": "rotate", "target": "g",
                        "start": 0, "end": 100, "to": 10 })),
        ])
        .unwrap();
    let (made, edits) = make_component(&document, &[id("g")]).unwrap();
    assert_eq!(made, id("g"));
    let result = applied(&document, &edits);
    let node = result.get(&id("g")).unwrap();
    assert_eq!(node.node_type(), NodeType::Component);
    assert_eq!(node.name(), "Box");
    assert_eq!(result.children(&id("g"), Family::Layer), [id("kid")]);
    assert_eq!(result.animations_targeting(&id("g")).len(), 1);
}

#[test]
fn other_layers_are_wrapped_in_a_new_component() {
    let mut document = sample();
    document
        .apply(&[
            add(json!({ "id": "r1", "parent": "ab", "index": "a5", "type": "rect", "x": 300, "y": 20 })),
            add(json!({ "id": "r2", "parent": "ab", "index": "a6", "type": "rect", "x": 320, "y": 60,
                        "width": 50, "height": 50 })),
        ])
        .unwrap();
    let (made, edits) = make_component(&document, &[id("r1"), id("r2")]).unwrap();
    let result = applied(&document, &edits);
    let node = result.get(&made).unwrap();
    assert_eq!(node.node_type(), NodeType::Component);
    assert_eq!(node.name(), "Component");
    assert_eq!(result.children(&made, Family::Layer), [id("r1"), id("r2")]);
    assert_eq!(
        result.world_bounds(&id("r2")),
        document.world_bounds(&id("r2"))
    );
}

#[test]
fn some_layers_cannot_be_made_into_a_component() {
    let document = sample();
    let refused = |ids: &[&str]| {
        let ids: Vec<NodeId> = ids.iter().map(|text| id(text)).collect();
        make_component(&document, &ids).unwrap_err()
    };
    assert!(matches!(refused(&["ab"]), BuildError::Artboard(..)));
    assert!(matches!(
        refused(&["card"]),
        BuildError::AlreadyAComponent(_)
    ));
    let inside = refused(&["dot"]);
    assert!(matches!(inside, BuildError::NestedComponent(_)), "{inside}");
    assert!(inside.to_string().contains("inside component"), "{inside}");
    assert!(matches!(
        refused(&["card", "copy"]),
        BuildError::NestedComponent(_)
    ));
    assert!(matches!(refused(&[]), BuildError::Empty));
}

#[test]
fn a_new_instance_looks_like_its_component() {
    let document = sample();
    let (new, edits) = add_instance(
        &document,
        &id("card"),
        &id("ab"),
        &Place::Last,
        Point::new(40.0, 200.0),
    )
    .unwrap();
    let result = applied(&document, &edits);
    let node = result.get(&new).unwrap();
    let NodeData::Instance(instance) = &node.data else {
        panic!("expected an instance");
    };
    assert_eq!(instance.component, id("card"));
    assert_eq!(instance.layer.name, "Card");
    assert_eq!(
        (instance.layer.x, instance.layer.y, instance.layer.width),
        (40.0, 200.0, 100.0)
    );
    assert_eq!(instance.layer.opacity, 90.0);
    assert_eq!(result.children(&id("ab"), Family::Layer).last(), Some(&new));

    let not_one = add_instance(&document, &id("dot"), &id("ab"), &Place::Last, Point::ZERO);
    assert!(matches!(not_one, Err(BuildError::NotAComponent(_))));
    let into_itself = add_instance(
        &document,
        &id("card"),
        &id("card"),
        &Place::Last,
        Point::ZERO,
    );
    assert!(matches!(into_itself, Err(BuildError::ComponentLoop { .. })));
    // "badge" shows "card", so "card" cannot hold an instance of "badge".
    let around = add_instance(
        &document,
        &id("badge"),
        &id("card"),
        &Place::Last,
        Point::ZERO,
    );
    assert!(matches!(around, Err(BuildError::ComponentLoop { .. })));
    let into_a_rect = add_instance(
        &document,
        &id("card"),
        &id("dot"),
        &Place::Last,
        Point::ZERO,
    );
    assert!(matches!(into_a_rect, Err(BuildError::CannotHold { .. })));
}

#[test]
fn a_detached_instance_looks_the_same_at_every_time() {
    let document = sample();
    for instance in ["copy", "big"] {
        let (group, edits) = detach(&document, &id(instance)).unwrap();
        let result = applied(&document, &edits);
        assert!(!result.contains(&id(instance)));
        let node = result.get(&group).unwrap();
        assert_eq!(node.node_type(), NodeType::Group);
        assert_eq!(result.children(&group, Family::Layer).len(), 1);
        for time in TIMES {
            assert!(
                looks_the_same(&scene(&document, "ab", time), &scene(&result, "ab", time)),
                "{instance} at {time}: {:#?}\nbecame\n{:#?}",
                drawn(&scene(&document, "ab", time)),
                drawn(&scene(&result, "ab", time)),
            );
        }
    }
}

#[test]
fn a_resized_instance_grows_its_box_like_a_group() {
    // "big" shows "card" at twice its size, and then grows to four times.
    // "wide" is stretched like a group when detached, so it is left out.
    let mut document = applied(&sample(), &[Edit::remove("wide")]);
    document
        .apply(&[add(json!({
            "id": "grow", "parent": "ab", "index": "a5", "type": "resize", "target": "big",
            "start": 0, "end": 1000, "from": { "width": 200, "height": 100 },
            "to": { "width": 400, "height": 200 }, "easing": { "type": "linear" }
        }))])
        .unwrap();
    let (_, edits) = detach(&document, &id("big")).unwrap();
    let detached = applied(&document, &edits);
    let (flat, _) = without_instances(&document);
    for (how, result) in [("detached", &detached), ("without instances", &flat)] {
        for time in TIMES {
            assert!(
                looks_the_same(&scene(&document, "ab", time), &scene(result, "ab", time)),
                "{how} at {time}: {:#?}\nbecame\n{:#?}",
                drawn(&scene(&document, "ab", time)),
                drawn(&scene(result, "ab", time)),
            );
        }
    }
}

#[test]
fn detaching_keeps_the_instances_own_animations_and_leaves_the_components() {
    let document = sample();
    let (group, edits) = detach(&document, &id("copy")).unwrap();
    let result = applied(&document, &edits);
    assert_eq!(
        result.get(&id("turn")).and_then(Node::target),
        Some(&group),
        "the turn now targets the group"
    );
    // Copies of the move and the fade, later by 300 ms; not of the
    // component's own dimming.
    let copied: Vec<(NodeType, (f64, f64))> = result
        .walk(&id("ab"), Family::Animation)
        .map(|(_, node)| node)
        .filter(|node| !document.contains(&node.id))
        .map(|node| (node.node_type(), node.data.span().unwrap()))
        .collect();
    assert_eq!(
        copied,
        [
            (NodeType::Move, (300.0, 1300.0)),
            (NodeType::FadeIn, (400.0, 900.0))
        ]
    );
    // The component and its other instances are as they were.
    assert_eq!(result.get(&id("card")), document.get(&id("card")));
    assert_eq!(result.instances_of(&id("card")).len(), 3);
}

#[test]
fn a_negative_offset_moves_animations_earlier_but_not_before_the_start() {
    let mut document = sample();
    document
        .apply(&[Edit::set("copy", "time_offset", -700)])
        .unwrap();
    let (_, edits) = detach(&document, &id("copy")).unwrap();
    let result = applied(&document, &edits);
    let spans: Vec<(f64, f64)> = result
        .walk(&id("ab"), Family::Animation)
        .map(|(_, node)| node)
        .filter(|node| !document.contains(&node.id))
        .map(|node| node.data.span().unwrap())
        .collect();
    // The move is squeezed into what is left of it; the fade is over.
    assert_eq!(spans, [(0.0, 300.0), (0.0, 0.0)]);
    // From where the squeezed move ends on, the copy matches again.
    for time in [300.0, 900.0, 2000.0] {
        assert!(looks_the_same(
            &scene(&document, "ab", time),
            &scene(&result, "ab", time)
        ));
    }
}

#[test]
fn an_instance_resized_out_of_proportion_is_stretched_like_a_group() {
    let document = sample();
    let (group, edits) = detach(&document, &id("wide")).unwrap();
    let result = applied(&document, &edits);
    let NodeData::Group(look) = &result.get(&group).unwrap().data else {
        panic!("expected a group");
    };
    assert_eq!((look.layer.width, look.layer.height), (300.0, 50.0));
    assert_eq!(look.stroke.as_ref().map(|stroke| stroke.width), Some(2.0));
    let dot = result.child_nodes(&group, Family::Layer).next().unwrap();
    let layer = dot.layer().unwrap();
    // Its centre moved three times as far across; it is wider.
    assert!((layer.center().x - 30.0).abs() < 1e-6, "{layer:?}");
    assert!(layer.width > 10.0 && layer.height > 10.0, "{layer:?}");
    let moved = result
        .animations_targeting(&dot.id)
        .into_iter()
        .find_map(|node| match &node.data {
            NodeData::Move(animation) => animation.to,
            _ => None,
        });
    assert_eq!(moved, Some(Offset::new(240.0, 0.0)));
}

#[test]
fn detaching_says_what_it_cannot_keep() {
    let mut document = sample();
    let losses = |document: &Document, instance: &str| detach_losses(document, &id(instance));
    assert_eq!(losses(&document, "copy"), []);
    assert_eq!(losses(&document, "big"), []);
    assert_eq!(losses(&document, "wide"), [DetachLoss::Stretched]);
    assert_eq!(losses(&document, "dot"), [], "not an instance");
    // 700 ms in, the move inside "card" is under way and gets squeezed.
    document
        .apply(&[Edit::set("copy", "time_offset", -700)])
        .unwrap();
    assert_eq!(losses(&document, "copy"), [DetachLoss::Squeezed]);
    // 1200 ms in, everything inside "card" is over.
    document
        .apply(&[Edit::set("copy", "time_offset", -1200)])
        .unwrap();
    assert_eq!(losses(&document, "copy"), []);
}

#[test]
fn a_copy_without_instances_says_what_it_cannot_keep() {
    // An instance of "badge" starts 500 ms in. Detached on its own, the
    // "card" inside it stays an instance and starts 500 ms in too; with
    // every instance detached, the card's move is squeezed.
    let document = sample();
    let (outer, edits) = add_instance(
        &document,
        &id("badge"),
        &id("ab"),
        &Place::Last,
        Point::ZERO,
    )
    .unwrap();
    let mut document = applied(&document, &edits);
    document
        .apply(&[Edit::set(&outer, "time_offset", -500)])
        .unwrap();
    assert_eq!(detach_losses(&document, &outer), []);
    let (_, losses) = without_instances(&document);
    assert_eq!(
        losses,
        [
            (id("wide"), DetachLoss::Stretched),
            (outer, DetachLoss::Squeezed)
        ]
    );
}

#[test]
fn instances_inside_the_component_stay_instances() {
    let document = sample();
    let (_, edits) = add_instance(
        &document,
        &id("badge"),
        &id("ab"),
        &Place::Last,
        Point::ZERO,
    )
    .unwrap();
    let document = applied(&document, &edits);
    let outer = document.instances_of(&id("badge"))[0].id.clone();
    let (group, edits) = detach(&document, &outer).unwrap();
    let result = applied(&document, &edits);
    let inside: Vec<NodeType> = result
        .child_nodes(&group, Family::Layer)
        .map(Node::node_type)
        .collect();
    assert_eq!(inside, [NodeType::Instance]);
    assert_eq!(result.instances_of(&id("card")).len(), 5);
}

#[test]
fn instances_inside_the_component_keep_playing_later_by_the_offset() {
    // An instance of "badge", which holds an animated "card", plays 400 ms
    // late; so does the card inside it, before and after detaching.
    let document = sample();
    let (outer, edits) = add_instance(
        &document,
        &id("badge"),
        &id("ab"),
        &Place::Last,
        Point::ZERO,
    )
    .unwrap();
    let mut document = applied(&document, &edits);
    document
        .apply(&[Edit::set(&outer, "time_offset", 400)])
        .unwrap();
    let (_, detached) = detach(&document, &outer).unwrap();
    let detached = applied(&document, &detached);
    let removed = applied(&document, &remove(&document, &[id("badge")]));
    for (how, result) in [("detached", &detached), ("removed", &removed)] {
        for time in TIMES {
            assert!(
                looks_the_same(&scene(&document, "ab", time), &scene(result, "ab", time)),
                "{how} at {time}"
            );
        }
    }
}

#[test]
fn removing_a_component_detaches_its_instances_first() {
    // "wide" is stretched like a group when detached, so it is left out.
    let document = applied(&sample(), &[Edit::remove("wide")]);
    let edits = remove(&document, &[id("card")]);
    let result = applied(&document, &edits);
    assert!(!result.contains(&id("card")));
    assert!(
        result
            .nodes()
            .all(|node| node.node_type() != NodeType::Instance)
    );
    for time in TIMES {
        let (before, after) = (scene(&document, "ab", time), scene(&result, "ab", time));
        // The component itself is gone; the rest looks the same.
        let rest = |scene: &Scene| {
            let mut scene = scene.clone();
            scene.nodes.retain(|node| node.id != id("card"));
            scene
        };
        assert!(looks_the_same(&rest(&before), &after), "at {time}");
        assert!(looks_the_same(
            &scene(&document, "ab2", time),
            &scene(&result, "ab2", time)
        ));
    }

    // A raw removal is refused, so nothing is lost by accident.
    let mut raw = document.clone();
    let error = raw.apply(&[Edit::remove("card")]).unwrap_err();
    assert_eq!(error.problem.kind, ProblemKind::Component);
}

#[test]
fn removing_a_component_inside_another_removed_one_detaches_both() {
    // "badge" holds an instance of "card"; an instance of "badge" lives in
    // "ab". Removing both components detaches it all the way down.
    let document = sample();
    let (_, edits) = add_instance(
        &document,
        &id("badge"),
        &id("ab"),
        &Place::Last,
        Point::ZERO,
    )
    .unwrap();
    let document = applied(&document, &edits);
    let result = applied(&document, &remove(&document, &[id("card"), id("badge")]));
    assert!(
        result
            .nodes()
            .all(|node| node.node_type() != NodeType::Instance)
    );
}

#[test]
fn copies_of_components_are_new_components() {
    let mut document = sample();
    document
        .apply(&[
            add(json!({ "id": "kit", "parent": "ab", "index": "a9", "type": "group" })),
            Edit::update("card", [("parent", json!("kit")), ("index", json!("a0"))]),
            Edit::update("copy", [("parent", json!("kit")), ("index", json!("a1"))]),
        ])
        .unwrap();
    // The group with the component and an instance of it: the copied
    // instance shows the copied component.
    let (edits, ids) = clone_subtree(&document, &id("kit"), &id("ab"), &Place::Last).unwrap();
    let result = applied(&document, &edits);
    let new_card = &ids[&id("card")];
    assert_eq!(
        result.get(&ids[&id("copy")]).and_then(Node::component),
        Some(new_card)
    );
    assert_eq!(
        result.get(new_card).map(Node::node_type),
        Some(NodeType::Component)
    );
    // An instance alone keeps showing the same component.
    let (edits, ids) = duplicate(&document, &[id("big")]).unwrap();
    let result = applied(&document, &edits);
    assert_eq!(
        result.get(&ids[&id("big")]).and_then(Node::component),
        Some(&id("card"))
    );
}

#[test]
fn a_component_duplicated_with_its_instance_is_shown_by_the_copy() {
    let document = sample();
    for order in [["card", "copy"], ["copy", "card"]] {
        let ids: Vec<NodeId> = order.iter().map(|text| id(text)).collect();
        let (edits, copies) = duplicate(&document, &ids).unwrap();
        let result = applied(&document, &edits);
        let new_card = &copies[&id("card")];
        assert_eq!(
            result.get(&copies[&id("copy")]).and_then(Node::component),
            Some(new_card),
            "{order:?}"
        );
        assert_eq!(result.instances_of(new_card).len(), 1, "{order:?}");
        assert_eq!(result.instances_of(&id("card")).len(), 4, "{order:?}");
    }
}

#[test]
fn designs_bring_their_components_along() {
    let document = sample();
    let design = import::parse_nodes(&json!([
        { "id": "1", "type": "group", "width": 300, "height": 100 },
        { "id": "2", "parent": "1", "type": "component", "width": 50, "height": 50 },
        { "id": "3", "parent": "1", "type": "instance", "component": "2", "x": 100 },
        { "id": "4", "parent": "1", "type": "instance", "component": "card", "x": 200 },
    ]))
    .unwrap();
    let imported = import::insert(&document, &design, &Destination::Into(id("ab"))).unwrap();
    let result = applied(&document, &imported.edits);
    let new = |old: &str| imported.ids[&id(old)].clone();
    assert_eq!(
        result.get(&new("3")).and_then(Node::component),
        Some(&new("2"))
    );
    assert_eq!(
        result.get(&new("4")).and_then(Node::component),
        Some(&id("card"))
    );
}

#[test]
fn a_copy_without_instances_looks_the_same_and_is_the_same_every_time() {
    // "wide" is stretched like a group when detached, so it is left out.
    let document = applied(&sample(), &[Edit::remove("wide")]);
    let (_, edits) = add_instance(
        &document,
        &id("badge"),
        &id("ab"),
        &Place::Last,
        Point::ZERO,
    )
    .unwrap();
    let document = applied(&document, &edits);
    let (flat, losses) = without_instances(&document);
    assert_eq!(losses, [], "nothing to lose");
    flat.validate().unwrap();
    assert!(
        flat.nodes()
            .all(|node| node.node_type() != NodeType::Instance)
    );
    assert_eq!(
        flat,
        without_instances(&document).0,
        "the same ids every time"
    );
    for time in TIMES {
        for artboard in ["ab", "ab2"] {
            assert!(
                looks_the_same(
                    &scene(&document, artboard, time),
                    &scene(&flat, artboard, time)
                ),
                "{artboard} at {time}"
            );
        }
    }
}

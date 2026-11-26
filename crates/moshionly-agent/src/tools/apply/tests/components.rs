//! Components through apply_edits, get_node and query_nodes.

use moshionly_core::eval::evaluate;
use moshionly_core::model::NodeData;
use moshionly_core::text::MonospaceMeasurer;
use moshionly_core::{NodeId, NodeType};
use serde_json::{Value, json};

use super::{apply, created_id, document, refused};
use crate::tools::testing::{call, demo_tools};

fn node_type(tools: &crate::Tools, id: &str) -> Option<NodeType> {
    document(tools)
        .get(&NodeId::from(id))
        .map(|node| node.node_type())
}

#[test]
fn a_group_becomes_a_component_and_instances_of_it_are_added_by_temp_id() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "make_component", "id": "card", "temp_id": "main" },
            { "op": "add", "temp_id": "copy", "parent": "second",
              "node": { "type": "instance", "component": "main", "x": 100, "y": 100 } },
            { "op": "clone", "id": "copy", "temp_id": "later", "offset": { "x": 0, "y": 420 },
              "time_offset": 150 },
        ]),
    );
    assert_eq!(
        result["summary"],
        "Add 1 layer, copy 1 layer and make 1 component"
    );
    assert_eq!(created_id(&result, "main"), "card", "made in place");
    assert_eq!(node_type(&tools, "card"), Some(NodeType::Component));

    let document = document(&tools);
    let copy = document
        .get(&NodeId::from(created_id(&result, "copy").as_str()))
        .unwrap();
    let NodeData::Instance(instance) = &copy.data else {
        panic!("expected an instance, got {copy:?}");
    };
    assert_eq!(instance.component, NodeId::from("card"));
    // Sized and named like the component, where it was put.
    assert_eq!(
        (instance.layer.width, instance.layer.height),
        (600.0, 400.0)
    );
    assert_eq!(instance.layer.name, "Card");
    assert_eq!((instance.layer.x, instance.layer.y), (100.0, 100.0));
    let later = document
        .get(&NodeId::from(created_id(&result, "later").as_str()))
        .unwrap();
    let NodeData::Instance(later) = &later.data else {
        panic!("expected an instance");
    };
    assert_eq!(later.time_offset, 150.0, "the copy plays later");
}

#[test]
fn an_instance_takes_its_components_effects() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "update", "id": "card", "set": {
                "glow": { "radius": 30 }, "noise": { "amount": 0.4, "animated": false },
                "directional_blur": { "amount": 6, "angle": 45 } } },
            { "op": "make_component", "id": "card" },
            { "op": "add", "temp_id": "copy", "parent": "second",
              "node": { "type": "instance", "component": "card", "noise": null } },
        ]),
    );
    let document = document(&tools);
    let copy = document
        .get(&NodeId::from(created_id(&result, "copy").as_str()))
        .unwrap();
    let main = document
        .get(&NodeId::from("card"))
        .unwrap()
        .layer()
        .unwrap();
    let layer = copy.layer().unwrap();
    assert_eq!(layer.glow, main.glow);
    assert_eq!(layer.directional_blur, main.directional_blur);
    assert!(
        main.noise.is_some() && layer.noise.is_none(),
        "left out on purpose"
    );
}

#[test]
fn a_component_cloned_with_its_instance_is_delayed_once() {
    // A group holding the animated card as a component and an instance
    // of it, copied 500 ms later.
    let tools = demo_tools();
    let made = apply(
        &tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "temp_id": "row", "parent": "scene", "node": { "type": "group" } },
            { "op": "move", "id": "card", "parent": "row" },
            { "op": "add", "temp_id": "copy", "parent": "row",
              "node": { "type": "instance", "component": "card", "y": 600 } },
        ]),
    );
    let (row, copy) = (created_id(&made, "row"), created_id(&made, "copy"));
    let cloned = apply(
        &tools,
        json!([{ "op": "clone", "id": row, "time_offset": 500 }]),
    );
    let later = cloned["created"][0]["copies"][copy.as_str()]
        .as_str()
        .unwrap()
        .to_owned();
    let document = document(&tools);
    let NodeData::Instance(instance) = &document.get(&NodeId::from(later.as_str())).unwrap().data
    else {
        panic!("expected an instance");
    };
    assert_eq!(
        instance.time_offset, 0.0,
        "its copied component plays later already"
    );
    // The copy shows at t + 500 what the original shows at t.
    let drawn = |id: &str, time: f64| {
        let scene = evaluate(
            &document,
            &NodeId::from("scene"),
            time,
            &MonospaceMeasurer::default(),
        )
        .unwrap();
        let node = scene.find(&NodeId::from(id)).unwrap();
        serde_json::to_string(node)
            .unwrap()
            .replace(id, "the instance")
    };
    for time in [0.0, 300.0, 700.0, 900.0, 1500.0] {
        assert_eq!(drawn(&copy, time), drawn(&later, time + 500.0), "at {time}");
    }
}

/// The width and height of the layer `id`.
fn size(tools: &crate::Tools, id: &str) -> (f64, f64) {
    let document = document(tools);
    let layer = document.get(&NodeId::from(id)).unwrap().layer().unwrap();
    (layer.width, layer.height)
}

#[test]
fn instances_the_size_of_a_resized_component_follow_it() {
    // "same" is sized like the card; "wide" was made wider.
    let tools = demo_tools();
    let made = apply(
        &tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "temp_id": "same", "parent": "second",
              "node": { "type": "instance", "component": "card" } },
            { "op": "add", "temp_id": "wide", "parent": "second",
              "node": { "type": "instance", "component": "card", "y": 500, "width": 900,
                        "height": 400 } },
        ]),
    );
    let (same, wide) = (created_id(&made, "same"), created_id(&made, "wide"));
    let before = document(&tools);
    let result = apply(
        &tools,
        json!([{ "op": "update", "id": "card", "set": { "width": 800, "height": 500 } }]),
    );
    assert_eq!(
        result["summary"],
        "Change 1 node and resize 2 instances to match"
    );
    assert_eq!(size(&tools, "card"), (800.0, 500.0));
    assert_eq!(size(&tools, &same), (800.0, 500.0));
    // Each side on its own: "wide" keeps its width and follows the height.
    assert_eq!(size(&tools, &wide), (900.0, 500.0));
    // One undo step puts them all back.
    assert_eq!(
        tools.session().read(|session| session.undo_steps().len()),
        2
    );
    tools.session().undo().unwrap();
    assert_eq!(*document(&tools), *before);

    // A side the same operation sets on an instance keeps what it sets.
    let result = apply(
        &tools,
        json!([{ "op": "update", "ids": ["card", same], "set": { "width": 700 } }]),
    );
    assert_eq!(result["summary"], "Change 2 nodes");
    assert_eq!(size(&tools, &same), (700.0, 400.0));
    assert_eq!(size(&tools, &wide), (900.0, 400.0));

    // Setting the size the card has resizes nothing.
    let result = apply(
        &tools,
        json!([{ "op": "update", "id": "card", "set": { "width": 700, "fill": "#00FF00" } }]),
    );
    assert_eq!(result["summary"], "Change 1 node");
    assert_eq!(size(&tools, &same), (700.0, 400.0));
}

#[test]
fn layers_are_wrapped_in_a_new_component() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "make_component", "ids": ["badge"], "temp_id": "kit" },
            { "op": "add", "parent": "scene",
              "node": { "type": "instance", "component": "kit", "x": 900 } },
        ]),
    );
    let kit = created_id(&result, "kit");
    assert_ne!(kit, "badge");
    let document = document(&tools);
    assert_eq!(
        document.get(&NodeId::from("badge")).unwrap().parent,
        Some(NodeId::from(kit.as_str()))
    );
    assert_eq!(document.instances_of(&NodeId::from(kit.as_str())).len(), 1);
}

#[test]
fn an_instance_is_detached_into_a_group() {
    let tools = demo_tools();
    apply(
        &tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "temp_id": "copy", "parent": "second",
              "node": { "type": "instance", "component": "card" } },
        ]),
    );
    let copy = document(&tools).instances_of(&NodeId::from("card"))[0]
        .id
        .clone();
    let result = apply(
        &tools,
        json!([{ "op": "detach", "id": copy.as_str(), "temp_id": "own" }]),
    );
    assert_eq!(result["summary"], "Detach 1 instance");
    let group = created_id(&result, "own");
    assert_eq!(node_type(&tools, &group), Some(NodeType::Group));
    assert_eq!(result["created"][0]["detached_from"], json!(copy));
    let document = document(&tools);
    assert!(!document.contains(&copy));
    // Copies of the card's layers and of the animations on them.
    assert_eq!(
        document
            .descendants(&NodeId::from(group.as_str()), moshionly_core::Family::Layer)
            .len(),
        3
    );
    assert_eq!(
        document
            .walk(&NodeId::from("second"), moshionly_core::Family::Animation)
            .count(),
        2
    );
}

#[test]
fn removing_a_component_detaches_its_instances() {
    let tools = demo_tools();
    apply(
        &tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "parent": "second", "node": { "type": "instance", "component": "card" } },
            { "op": "add", "parent": "scene", "node": { "type": "instance", "component": "card", "y": 700 } },
        ]),
    );
    // The card, its three layers and the three animations on them go.
    let result = apply(&tools, json!([{ "op": "remove", "id": "card" }]));
    assert_eq!(result["summary"], "Remove 7 nodes and detach 2 instances");
    let warning = result["warnings"][0].as_str().unwrap();
    assert!(warning.contains("detached"), "{warning}");
    let document = document(&tools);
    assert!(!document.contains(&NodeId::from("card")));
    assert!(
        document
            .nodes()
            .all(|node| node.node_type() != NodeType::Instance)
    );
    assert_eq!(
        document
            .nodes()
            .filter(|node| node.node_type() == NodeType::Group)
            .count(),
        2
    );
}

/// The card as a component with three instances in "second": "Wide",
/// stretched out of proportion, "Early", which starts 700 ms into the
/// card's animations, and "Plain". Returns their ids in that order.
fn card_and_three_instances(tools: &crate::Tools) -> [String; 3] {
    let made = apply(
        tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "temp_id": "wide", "parent": "second",
              "node": { "type": "instance", "component": "card", "name": "Wide",
                        "width": 900, "height": 400 } },
            { "op": "add", "temp_id": "early", "parent": "second",
              "node": { "type": "instance", "component": "card", "name": "Early",
                        "time_offset": -700 } },
            { "op": "add", "temp_id": "plain", "parent": "second",
              "node": { "type": "instance", "component": "card", "name": "Plain" } },
        ]),
    );
    ["wide", "early", "plain"].map(|temp| created_id(&made, temp))
}

fn warnings(result: &Value) -> Vec<&str> {
    result["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

#[test]
fn detaching_says_when_the_group_does_not_look_the_same() {
    let tools = demo_tools();
    let ids = card_and_three_instances(&tools);
    let result = apply(&tools, json!([{ "op": "detach", "ids": ids }]));
    let said = warnings(&result);
    assert_eq!(said.len(), 2, "{said:#?}");
    assert!(
        said[0].contains("(Wide)") && said[0].contains("out of proportion"),
        "{said:#?}"
    );
    assert!(
        said[1].contains("(Early)") && said[1].contains("squeezed"),
        "{said:#?}"
    );
}

#[test]
fn removing_a_component_says_which_groups_do_not_look_the_same() {
    let tools = demo_tools();
    card_and_three_instances(&tools);
    let result = apply(&tools, json!([{ "op": "remove", "id": "card" }]));
    let said = warnings(&result).join("\n");
    assert!(said.contains("(Plain)"), "{said}");
    assert!(said.contains("except"), "{said}");
    assert!(
        said.contains("(Wide): it is resized out of proportion"),
        "{said}"
    );
    assert!(
        said.contains("(Early): its time offset is negative"),
        "{said}"
    );
}

#[test]
fn removing_two_components_counts_the_copies_detached_from_an_instance() {
    // "Kit" holds an instance of the animated card; "Outer", an instance
    // of the kit, starts 700 ms in. Removing both components detaches
    // Outer, then the copy of the card's instance in it, which starts 700
    // ms in too, so the card's title squeezes what it does by then.
    let tools = demo_tools();
    let made = apply(
        &tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "temp_id": "kit", "parent": "second",
              "node": { "type": "group", "name": "Kit" } },
            { "op": "add", "parent": "kit", "node": { "type": "instance", "component": "card" } },
        ]),
    );
    let kit = created_id(&made, "kit");
    apply(
        &tools,
        json!([
            { "op": "make_component", "id": kit },
            { "op": "add", "parent": "scene",
              "node": { "type": "instance", "component": kit, "name": "Outer",
                        "time_offset": -700 } },
        ]),
    );
    let result = apply(&tools, json!([{ "op": "remove", "ids": [kit, "card"] }]));
    let said = warnings(&result).join("\n");
    assert!(said.contains("except"), "{said}");
    assert!(
        said.contains("(Outer): its time offset is negative"),
        "{said}"
    );
    // Only Outer was an instance before; the copy is part of its group.
    assert!(
        result["summary"]
            .as_str()
            .unwrap()
            .ends_with("detach 1 instance"),
        "{result}"
    );
}

#[test]
fn mistakes_with_components_are_explained() {
    let tools = demo_tools();
    let error = refused(&tools, json!([{ "op": "make_component", "id": "scene" }]));
    assert!(
        error["message"].as_str().unwrap().contains("artboard"),
        "{error}"
    );
    let error = refused(&tools, json!([{ "op": "detach", "id": "card" }]));
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("not an instance"),
        "{error}"
    );
    let error = refused(
        &tools,
        json!([{ "op": "add", "parent": "scene", "node": { "type": "instance" } }]),
    );
    assert_eq!(error["field"], "node.component", "{error}");
    let error = refused(
        &tools,
        json!([{ "op": "add", "parent": "scene",
                 "node": { "type": "instance", "component": "badge" } }]),
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("not a component"),
        "{error}"
    );
    // An instance cannot go inside the component it shows.
    let error = refused(
        &tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "parent": "card", "node": { "type": "instance", "component": "card" } },
        ]),
    );
    assert!(
        error["message"].as_str().unwrap().contains("inside itself"),
        "{error}"
    );
}

#[test]
fn get_node_and_query_nodes_show_the_links() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "make_component", "id": "card" },
            { "op": "add", "temp_id": "copy", "parent": "second",
              "node": { "type": "instance", "component": "card" } },
        ]),
    );
    let copy = created_id(&result, "copy");

    let main = call(&tools, "get_node", json!({ "id": "card" }));
    assert_eq!(main["instances"][0]["id"], json!(copy));
    let instance = call(&tools, "get_node", json!({ "id": copy }));
    assert_eq!(instance["shows_component"]["id"], "card");
    let inside = call(&tools, "get_node", json!({ "id": "title" }));
    assert_eq!(inside["in_component"]["id"], "card");
    assert!(inside["component_note"].is_string());

    let found = call(&tools, "query_nodes", json!({ "component": "card" }));
    let ids: Vec<&Value> = found["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| &node["id"])
        .collect();
    assert_eq!(ids, [&json!(copy)]);
    let counted = call(
        &tools,
        "query_nodes",
        json!({ "type": ["component"], "fields": ["instance_count"] }),
    );
    assert_eq!(counted["nodes"][0]["instance_count"], 1);
}

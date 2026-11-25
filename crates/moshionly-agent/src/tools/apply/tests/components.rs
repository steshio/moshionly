//! Components through apply_edits, get_node and query_nodes.

use moshionly_core::model::NodeData;
use moshionly_core::{NodeId, NodeType};
use serde_json::json;

use super::{apply, created_id, document, refused};
use crate::tools::testing::demo_tools;

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

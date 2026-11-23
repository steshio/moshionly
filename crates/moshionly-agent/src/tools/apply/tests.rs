use moshionly_core::{Document, NodeId, NodeType};
use serde_json::{Value, json};

use crate::tools::testing::{call, call_error, demo_session, demo_tools};
use crate::{Host, Origin, SessionHandle, Tools};

fn apply(tools: &Tools, operations: Value) -> Value {
    call(tools, "apply_edits", json!({ "operations": operations }))
}

/// Calls apply_edits expecting a refusal, and returns the error object.
fn refused(tools: &Tools, operations: Value) -> Value {
    let text = call_error(tools, "apply_edits", json!({ "operations": operations }));
    let value: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "text": text }));
    assert_eq!(value["applied"], 0, "{value}");
    value["error"].clone()
}

fn document(tools: &Tools) -> std::sync::Arc<Document> {
    tools.session().document()
}

fn field(tools: &Tools, id: &str, name: &str) -> Value {
    document(tools)
        .get(&NodeId::from(id))
        .unwrap_or_else(|| panic!("no node {id}"))
        .to_full_value()[name]
        .clone()
}

fn created_id(result: &Value, temp: &str) -> String {
    result["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["temp_id"] == temp)
        .unwrap_or_else(|| panic!("{temp} not created: {result}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_layer_and_its_animation_are_added_through_a_temp_id() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "add", "temp_id": "cta", "parent": "scene",
              "node": { "type": "rect", "name": "Button", "x": 340, "y": 820, "width": 400,
                        "height": 96, "fill": "#0D99FF", "corner_radius": 48 } },
            { "op": "add", "node": { "type": "fade_in", "target": "cta", "start": 800 } },
        ]),
    );
    assert_eq!(result["applied"], 2);
    assert_eq!(result["summary"], "Add 1 layer and 1 animation");
    let button = created_id(&result, "cta");
    assert_eq!(button.len(), 12, "a real, generated id");
    let created = result["created"].as_array().unwrap();
    assert_eq!(created[0]["name"], "Button");
    assert_eq!(created[1]["type"], "fade_in");
    assert!(result.get("warnings").is_none(), "{result}");

    let document = document(&tools);
    let animations = document.animations_targeting(&NodeId::from(button.as_str()));
    assert_eq!(animations.len(), 1);
    // The start was given; the end is a typical length after it.
    assert_eq!(animations[0].data.span(), Some((800.0, 1300.0)));
    assert_eq!(
        animations[0].parent.as_ref().map(NodeId::as_str),
        Some("scene")
    );
    // The button is the frontmost layer of the artboard.
    let layers = document.children(&NodeId::from("scene"), moshionly_core::Family::Layer);
    assert_eq!(layers.last().map(NodeId::as_str), Some(button.as_str()));
}

#[test]
fn a_batch_is_one_undo_step_made_by_the_agent() {
    let tools = demo_tools();
    let before = document(&tools);
    let result = call(
        &tools,
        "apply_edits",
        json!({
            "label": "Restyle the badge",
            "operations": [
                { "op": "update", "id": "badge", "set": { "fill": "#FF5A1F" } },
                { "op": "add", "parent": "card", "node": { "type": "star", "fill": "#FFD000" } },
                { "op": "remove", "id": "dotmove" },
            ]
        }),
    );
    assert_eq!(result["summary"], "Restyle the badge");
    let steps = tools
        .session()
        .read(|session| session.undo_steps().to_vec());
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].origin, Origin::Agent);
    assert_eq!(steps[0].summary, "Restyle the badge");
    assert_eq!(result["revision"], tools.session().revision());

    tools.session().undo().unwrap();
    assert_eq!(*document(&tools), *before);
}

#[test]
fn a_batch_that_fails_halfway_changes_nothing() {
    let tools = demo_tools();
    let before = document(&tools);
    let revision = tools.session().revision();
    let error = refused(
        &tools,
        json!([
            { "op": "update", "id": "badge", "set": { "x": 80 } },
            { "op": "add", "temp_id": "new", "parent": "scene", "node": { "type": "rect" } },
            { "op": "update", "id": "card", "set": { "opacity": 140 } },
        ]),
    );
    assert_eq!(error["operation"], 2);
    assert_eq!(error["op"], "update");
    assert_eq!(error["field"], "set.opacity");
    assert!(
        error["message"].as_str().unwrap().contains("opacity"),
        "{error}"
    );
    assert_eq!(*document(&tools), *before);
    assert_eq!(tools.session().revision(), revision);
    assert!(!tools.session().read(|session| session.can_undo()));
}

#[test]
fn an_unknown_field_is_named_with_a_suggestion_and_where_to_look() {
    let error = refused(
        &demo_tools(),
        json!([{ "op": "add", "parent": "scene", "node": { "type": "rect", "colour": "#000000" } }]),
    );
    assert_eq!(error["operation"], 0);
    assert_eq!(error["field"], "node.colour");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("Did you mean \"fill\"?"), "{message}");
    assert!(
        error["fix"]
            .as_str()
            .unwrap()
            .contains("get_schema with {\"type\":\"rect\"}"),
        "{error}"
    );
}

#[test]
fn an_unknown_type_or_operation_gets_directions() {
    let tools = demo_tools();
    let error = refused(
        &tools,
        json!([{ "op": "add", "parent": "scene", "node": { "type": "rectangle" } }]),
    );
    assert_eq!(error["field"], "node.type");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Did you mean \"rect\"?")
    );

    let error = refused(&tools, json!([{ "op": "updte", "id": "badge" }]));
    assert_eq!(error["field"], "op");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Did you mean \"update\"?"),
        "{error}"
    );

    let error = refused(
        &tools,
        json!([{ "op": "update", "id": "badge", "fill": "#000" }]),
    );
    assert_eq!(error["field"], "fill");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("It takes: op, id, ids, set."),
        "{error}"
    );
}

#[test]
fn common_names_for_operations_are_understood() {
    let tools = demo_tools();
    apply(&tools, json!([{ "op": "delete", "id": "badge" }]));
    assert!(!document(&tools).contains(&NodeId::from("badge")));
}

#[test]
fn one_operation_may_come_without_a_list() {
    let tools = demo_tools();
    call(
        &tools,
        "apply_edits",
        json!({ "operations": { "op": "update", "id": "badge", "set": { "x": 64 } } }),
    );
    assert_eq!(field(&tools, "badge", "x"), 64);
}

#[test]
fn update_merges_objects_and_null_resets_a_field() {
    let tools = demo_tools();
    apply(
        &tools,
        json!([{ "op": "update", "id": "badge",
                 "set": { "stroke": { "paint": "#000000", "width": 2 } } }]),
    );
    apply(
        &tools,
        json!([{ "op": "update", "id": "badge",
                 "set": { "stroke": { "width": 6 }, "corner_radius": null } }]),
    );
    assert_eq!(field(&tools, "badge", "stroke")["width"], 6);
    assert_eq!(field(&tools, "badge", "stroke")["paint"], "#000000");
    assert_eq!(field(&tools, "badge", "corner_radius"), 0);
}

#[test]
fn update_works_on_several_nodes_and_keeps_the_tree_to_move() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([{ "op": "update", "ids": ["accent", "dot"], "set": { "fill": "#00C853" } }]),
    );
    assert_eq!(result["summary"], "Change 2 nodes");
    assert_eq!(field(&tools, "accent", "fill"), "#00C853");
    assert_eq!(field(&tools, "dot", "fill"), "#00C853");

    let error = refused(
        &tools,
        json!([{ "op": "update", "id": "dot", "set": { "parent": "scene" } }]),
    );
    assert_eq!(error["field"], "set.parent");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Use the move operation")
    );
    let error = refused(
        &tools,
        json!([{ "op": "update", "id": "dot", "ids": ["badge"], "set": { "x": 1 } }]),
    );
    assert_eq!(error["field"], "ids");
}

#[test]
fn remove_takes_what_is_inside_and_the_animations_along() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([{ "op": "remove", "ids": ["card", "title"] }]),
    );
    // The card, its three layers and the three animations on them.
    assert_eq!(result["removed"], 7);
    let document = document(&tools);
    for gone in [
        "card",
        "accent",
        "title",
        "dot",
        "cardfade",
        "dotmove",
        "titlefade",
    ] {
        assert!(!document.contains(&NodeId::from(gone)), "{gone}");
    }
    assert!(document.contains(&NodeId::from("extras")));
}

#[test]
fn set_duration_warns_about_animations_that_are_cut_off() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([{ "op": "set_duration", "artboard": "scene", "duration": 1000 }]),
    );
    assert_eq!(field(&tools, "scene", "duration"), 1000);
    let warning = result["warnings"][0].as_str().unwrap();
    assert!(warning.contains("dotmove"), "{warning}");
    assert!(warning.contains("titlefade"), "{warning}");
    assert!(!warning.contains("cardfade"), "{warning}");

    let error = refused(
        &tools,
        json!([{ "op": "set_duration", "artboard": "card", "duration": 1000 }]),
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("It is in the artboard \"scene\""),
        "{error}"
    );
}

#[test]
fn the_parent_of_a_layer_may_be_left_out_only_with_one_artboard() {
    let tools = demo_tools();
    let error = refused(&tools, json!([{ "op": "add", "node": { "type": "rect" } }]));
    assert_eq!(error["field"], "parent");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("artboard \"scene\" (Launch)"), "{message}");
    assert!(
        message.contains("artboard \"second\" (Second)"),
        "{message}"
    );

    apply(&tools, json!([{ "op": "remove", "id": "second" }]));
    let result = apply(
        &tools,
        json!([{ "op": "add", "node": { "type": "ellipse", "fill": "#000000" } }]),
    );
    let id = result["created"][0]["id"].as_str().unwrap();
    assert_eq!(
        document(&tools).get(&NodeId::from(id)).unwrap().parent,
        Some(NodeId::from("scene"))
    );
}

#[test]
fn an_empty_file_gets_its_first_artboard_and_layers_in_one_batch() {
    let tools = Tools::new(
        SessionHandle::from_document(Document::new("New")),
        Host::new(),
    );
    let error = refused(&tools, json!([{ "op": "add", "node": { "type": "rect" } }]));
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("The file has no artboard yet"),
        "{error}"
    );
    let result = apply(
        &tools,
        json!([
            { "op": "add", "temp_id": "scene",
              "node": { "type": "artboard", "width": 1920, "height": 1080 } },
            { "op": "add", "temp_id": "second",
              "node": { "type": "artboard", "width": 800, "height": 600 } },
            { "op": "add", "parent": "scene", "node": { "type": "text", "content": "Hi",
              "fill": "#111111" } },
        ]),
    );
    let document = document(&tools);
    assert_eq!(document.artboards().len(), 2);
    // A second artboard is put to the right of the first.
    let second = created_id(&result, "second");
    assert_eq!(field(&tools, &second, "x"), 2020);
    assert_eq!(result["summary"], "Add 3 layers");
}

#[test]
fn before_and_after_place_a_node_among_its_siblings() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "add", "temp_id": "back", "before": "accent",
              "node": { "type": "rect", "fill": "#EEEEEE" } },
            { "op": "add", "temp_id": "mid", "after": "back",
              "node": { "type": "rect", "fill": "#DDDDDD" } },
            { "op": "add", "temp_id": "first", "parent": "card", "position": "first",
              "node": { "type": "rect", "fill": "#CCCCCC" } },
        ]),
    );
    let ids: Vec<String> = ["first", "back", "mid"]
        .iter()
        .map(|temp| created_id(&result, temp))
        .collect();
    let document = document(&tools);
    let children: Vec<&str> = document
        .children(&NodeId::from("card"), moshionly_core::Family::Layer)
        .iter()
        .map(NodeId::as_str)
        .collect();
    assert_eq!(
        children,
        [
            ids[0].as_str(),
            ids[1].as_str(),
            ids[2].as_str(),
            "accent",
            "title",
            "dot"
        ]
    );

    let error = refused(
        &tools,
        json!([{ "op": "add", "parent": "scene", "before": "dot",
                 "node": { "type": "rect" } }]),
    );
    assert_eq!(error["field"], "before");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("is not a layer directly in artboard \"scene\""),
        "{error}"
    );
    let error = refused(
        &tools,
        json!([{ "op": "add", "parent": "scene", "position": "first", "after": "badge",
                 "node": { "type": "rect" } }]),
    );
    assert_eq!(error["field"], "position");
}

#[test]
fn temp_ids_are_checked() {
    let tools = demo_tools();
    let error = refused(
        &tools,
        json!([{ "op": "add", "temp_id": "badge", "parent": "scene",
                 "node": { "type": "rect" } }]),
    );
    assert_eq!(error["field"], "temp_id");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("already the id of rect \"badge\""),
        "{error}"
    );
    let error = refused(
        &tools,
        json!([
            { "op": "add", "temp_id": "x", "parent": "scene", "node": { "type": "rect" } },
            { "op": "add", "temp_id": "x", "parent": "scene", "node": { "type": "rect" } },
        ]),
    );
    assert_eq!(error["operation"], 1);
    let error = refused(
        &tools,
        json!([
            { "op": "add", "temp_id": "x", "parent": "scene", "node": { "type": "rect" } },
            { "op": "remove", "id": "x" },
            { "op": "update", "id": "x", "set": { "y": 4 } },
        ]),
    );
    assert_eq!(error["operation"], 2);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("has been removed by an operation since"),
        "{error}"
    );
    // A temp id cannot be used before the node it names is made.
    let error = refused(
        &tools,
        json!([
            { "op": "add", "node": { "type": "fade_in", "target": "later" } },
            { "op": "add", "temp_id": "later", "parent": "scene", "node": { "type": "rect" } },
        ]),
    );
    assert_eq!(
        (error["operation"].clone(), error["field"].clone()),
        (json!(0), json!("node.target"))
    );
}

#[test]
fn an_id_in_the_node_counts_as_its_temp_id() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "add", "parent": "scene",
              "node": { "id": "logo", "parent": "scene", "type": "ellipse", "fill": "#000000" } },
            { "op": "update", "id": "logo", "set": { "width": 64 } },
        ]),
    );
    let logo = created_id(&result, "logo");
    assert_eq!(field(&tools, &logo, "width"), 64);

    let error = refused(
        &tools,
        json!([{ "op": "add", "parent": "scene",
                 "node": { "type": "rect", "index": "a5" } }]),
    );
    assert_eq!(error["field"], "node.index");
    let error = refused(
        &tools,
        json!([{ "op": "add", "parent": "scene",
                 "node": { "type": "rect", "parent": "card" } }]),
    );
    assert_eq!(error["field"], "node.parent");
}

#[test]
fn errors_name_new_nodes_by_their_temp_ids() {
    let tools = demo_tools();
    let error = refused(
        &tools,
        json!([
            { "op": "add", "temp_id": "box", "parent": "scene",
              "node": { "type": "rect", "fill": "#000000" } },
            { "op": "add", "node": { "type": "text_in", "target": "box" } },
        ]),
    );
    assert_eq!(error["operation"], 1);
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("\"box\""), "{message}");
    assert!(message.contains("text"), "{message}");
}

#[test]
fn an_animation_may_go_in_a_timeline_folder() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "add", "temp_id": "intro", "parent": "scene",
              "node": { "type": "animation_group", "name": "Intro" } },
            { "op": "add", "parent": "intro",
              "node": { "type": "scale", "target": "badge", "start": 200, "to": 1.2 } },
        ]),
    );
    let intro = created_id(&result, "intro");
    let document = document(&tools);
    let rows = document.children(
        &NodeId::from(intro.as_str()),
        moshionly_core::Family::Animation,
    );
    assert_eq!(rows.len(), 1);
    let scale = document.get(&rows[0]).unwrap();
    assert_eq!(scale.node_type(), NodeType::Scale);
    assert_eq!(scale.data.span(), Some((200.0, 1000.0)));
}

#[test]
fn warnings_point_at_things_nobody_will_see() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "add", "parent": "scene", "node": { "type": "rect", "name": "Ghost" } },
            { "op": "add", "parent": "scene",
              "node": { "type": "rect", "fill": "#000000", "x": 2000, "name": "Away" } },
            { "op": "add", "node": { "type": "fade_in", "target": "badge", "start": 3800 } },
        ]),
    );
    let warnings: Vec<&str> = result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning.as_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings[0].contains("(Ghost) has no fill and no stroke"));
    assert!(warnings[1].contains("(Away) is entirely outside its 1080 by 1080 artboard"));
    assert!(warnings[2].contains("ends at 4300 ms, after the end"));
}

#[test]
fn a_dry_run_checks_without_applying() {
    let tools = demo_tools();
    let before = document(&tools);
    let result = call(
        &tools,
        "apply_edits",
        json!({ "dry_run": true, "operations": [
            { "op": "add", "temp_id": "a", "parent": "scene",
              "node": { "type": "rect", "fill": "#000000" } },
        ]}),
    );
    assert_eq!(result["valid"], true);
    assert_eq!(
        result["created"],
        json!([{ "type": "rect", "temp_id": "a", "operation": 0 }])
    );
    assert_eq!(*document(&tools), *before);
    assert!(!tools.session().read(|session| session.can_undo()));
}

#[test]
fn a_batch_that_changes_nothing_says_so() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([{ "op": "update", "id": "badge", "set": { "x": 40 } }]),
    );
    assert!(
        result["note"]
            .as_str()
            .unwrap()
            .starts_with("Nothing changed")
    );
    assert!(!tools.session().read(|session| session.can_undo()));
}

#[test]
fn the_arguments_are_checked() {
    let tools = demo_tools();
    let text = call_error(&tools, "apply_edits", json!({}));
    assert!(text.starts_with("\"operations\" is required"), "{text}");
    let text = call_error(&tools, "apply_edits", json!({ "operations": [] }));
    assert!(text.contains("is empty"), "{text}");
    let error = refused(&tools, json!([{ "id": "badge" }]));
    assert_eq!(error["field"], "op");
}

#[test]
fn edits_are_seen_by_the_other_tools_and_by_subscribers() {
    let session = demo_session();
    let changes = session.changes();
    let tools = Tools::new(session, Host::new());
    apply(
        &tools,
        json!([{ "op": "update", "id": "dot", "set": { "x": 10 } }]),
    );
    let change = changes.try_recv().unwrap();
    assert_eq!(change.origin, Origin::Agent);
    let node = call(&tools, "get_node", json!({ "id": "dot" }));
    assert_eq!(node["node"]["x"], 10);
}

fn layer_ids(tools: &Tools, parent: &str) -> Vec<String> {
    document(tools)
        .children(&NodeId::from(parent), moshionly_core::Family::Layer)
        .iter()
        .map(|id| id.to_string())
        .collect()
}

#[test]
fn move_reorders_and_reparents_keeping_the_layer_in_place() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([{ "op": "move", "id": "badge", "position": "first" }]),
    );
    assert_eq!(result["summary"], "Move 1 node");
    assert_eq!(layer_ids(&tools, "scene"), ["badge", "card"]);

    // Into the card: the badge keeps its spot on the artboard.
    apply(
        &tools,
        json!([{ "op": "move", "id": "badge", "parent": "card", "after": "title" }]),
    );
    assert_eq!(
        layer_ids(&tools, "card"),
        ["accent", "title", "badge", "dot"]
    );
    assert_eq!(field(&tools, "badge", "x"), 40 - 240);
    assert_eq!(field(&tools, "badge", "y"), 40 - 340);
    let node = call(&tools, "get_node", json!({ "id": "badge" }));
    assert_eq!(
        node["box"],
        json!({ "x": 40, "y": 40, "width": 120, "height": 48 })
    );

    // Without keep_position the numbers stay and the layer jumps.
    apply(
        &tools,
        json!([{ "op": "move", "id": "dot", "parent": "scene", "keep_position": false }]),
    );
    assert_eq!(field(&tools, "dot", "x"), 500);
    assert_eq!(
        layer_ids(&tools, "scene").last().map(String::as_str),
        Some("dot")
    );
}

#[test]
fn moving_a_layer_to_another_artboard_takes_its_animations_along() {
    let tools = demo_tools();
    apply(
        &tools,
        json!([{ "op": "move", "id": "dot", "parent": "second" }]),
    );
    let document = document(&tools);
    let animation = document.get(&NodeId::from("dotmove")).unwrap();
    assert_eq!(animation.parent, Some(NodeId::from("second")));
}

#[test]
fn move_explains_what_it_is_not_for() {
    let tools = demo_tools();
    let error = refused(&tools, json!([{ "op": "move", "id": "badge", "x": 100 }]));
    assert_eq!(error["field"], "x");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("use update with \"set\""),
        "{error}"
    );
    let error = refused(
        &tools,
        json!([{ "op": "move", "id": "card", "parent": "dot" }]),
    );
    assert_eq!(error["field"], "parent");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("cannot be placed in ellipse \"dot\""),
        "{error}"
    );
    let error = refused(
        &tools,
        json!([{ "op": "move", "id": "card", "parent": "title" }]),
    );
    assert_eq!(error["field"], "parent");
    let error = refused(
        &tools,
        json!([{ "op": "move", "id": "second", "parent": "scene" }]),
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("cannot go inside artboard \"scene\""),
        "{error}"
    );
}

#[test]
fn artboards_can_change_their_order() {
    let tools = demo_tools();
    apply(
        &tools,
        json!([{ "op": "move", "id": "second", "before": "scene" }]),
    );
    let order: Vec<String> = document(&tools)
        .artboards()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(order, ["second", "scene"]);
}

#[test]
fn clone_copies_a_layer_with_its_animations_in_front_of_it() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([{ "op": "clone", "id": "card", "temp_id": "card2",
                 "offset": { "y": 420 }, "time_offset": 200 }]),
    );
    assert_eq!(result["summary"], "Copy 1 layer");
    let entry = &result["created"][0];
    let copy = entry["id"].as_str().unwrap();
    assert_eq!(entry["temp_id"], "card2");
    let copies = entry["copies"].as_object().unwrap();
    // The card, its three layers and the three animations on them.
    assert_eq!(copies.len(), 7);
    assert_eq!(copies["card"], copy);

    assert_eq!(layer_ids(&tools, "scene"), ["card", copy, "badge"]);
    assert_eq!(field(&tools, copy, "y"), 340 + 420);
    assert_eq!(field(&tools, copy, "x"), 240);

    let document = document(&tools);
    let title_copy = NodeId::from(copies["title"].as_str().unwrap());
    let fade = document.animations_targeting(&title_copy);
    assert_eq!(fade.len(), 1);
    assert_eq!(fade[0].data.span(), Some((700.0, 1700.0)));
    let dot_copy = NodeId::from(copies["dot"].as_str().unwrap());
    assert_eq!(
        document.animations_targeting(&dot_copy)[0].data.span(),
        Some((800.0, 1400.0))
    );
    // The originals are untouched.
    assert_eq!(
        document
            .get(&NodeId::from("titlefade"))
            .unwrap()
            .data
            .span(),
        Some((500.0, 1500.0))
    );
}

#[test]
fn clones_make_a_stagger_and_can_be_edited_in_the_same_batch() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "clone", "id": "dot", "temp_id": "dot2",
              "offset": { "x": -80 }, "time_offset": 100 },
            { "op": "clone", "id": "dot2", "temp_id": "dot3",
              "offset": { "x": -80 }, "time_offset": 100 },
            { "op": "update", "ids": ["dot2", "dot3"], "set": { "fill": "#FF5A1F" } },
        ]),
    );
    let third = created_id(&result, "dot3");
    assert_eq!(field(&tools, &third, "x"), 340);
    assert_eq!(field(&tools, &third, "fill"), "#FF5A1F");
    let document = document(&tools);
    let span = document.animations_targeting(&NodeId::from(third.as_str()))[0]
        .data
        .span();
    assert_eq!(span, Some((800.0, 1400.0)));
    assert_eq!(result["summary"], "Copy 2 layers and change 2 nodes");
}

#[test]
fn an_animation_is_cloned_onto_another_layer() {
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([{ "op": "clone", "id": "cardfade", "target": "badge", "time_offset": 150 }]),
    );
    assert_eq!(result["summary"], "Copy 1 animation");
    let copy = result["created"][0]["id"].as_str().unwrap();
    let document = document(&tools);
    let node = document.get(&NodeId::from(copy)).unwrap();
    assert_eq!(node.target(), Some(&NodeId::from("badge")));
    assert_eq!(node.data.span(), Some((150.0, 650.0)));

    // Onto a layer of another artboard, it moves to that timeline.
    let tools = demo_tools();
    let result = apply(
        &tools,
        json!([
            { "op": "add", "temp_id": "box", "parent": "second",
              "node": { "type": "rect", "fill": "#000000" } },
            { "op": "clone", "id": "cardfade", "target": "box" },
        ]),
    );
    let copy = result["created"][1]["id"].as_str().unwrap();
    let document = self::document(&tools);
    assert_eq!(
        document.get(&NodeId::from(copy)).unwrap().parent,
        Some(NodeId::from("second"))
    );

    let error = refused(
        &tools,
        json!([{ "op": "clone", "id": "cardfade", "time_offset": -100 }]),
    );
    assert_eq!(error["field"], "time_offset");
    let error = refused(
        &tools,
        json!([{ "op": "clone", "id": "badge", "target": "dot" }]),
    );
    assert_eq!(error["field"], "target");
}

#[test]
fn an_artboard_is_cloned_to_the_right_with_its_timeline() {
    let tools = demo_tools();
    let result = apply(&tools, json!([{ "op": "clone", "id": "scene" }]));
    let copy = result["created"][0]["id"].as_str().unwrap();
    assert_eq!(field(&tools, copy, "x"), 2080);
    let overview = call(&tools, "get_overview", json!({}));
    assert_eq!(overview["artboards"][2]["animations"], 3);
    assert_eq!(overview["artboards"][2]["layers"], 5);
    let error = refused(
        &tools,
        json!([{ "op": "clone", "id": "scene", "parent": "second" }]),
    );
    assert_eq!(error["field"], "parent");
}

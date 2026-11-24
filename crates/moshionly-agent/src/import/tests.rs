use std::sync::atomic::{AtomicUsize, Ordering};

use moshionly_core::build::PLACEHOLDER_IMAGE;
use moshionly_core::{Edit, Node};
use serde_json::{Value, json};

use super::*;
use crate::tools::testing::demo_session;

/// A frame with a box in it, as the plugin would send it.
fn design(fill: &str) -> Value {
    json!([
        { "id": "1:1", "type": "artboard", "name": "Frame", "width": 400, "height": 300,
          "fill": "#FFFFFF", "source_id": "figma:abc:1:1" },
        { "id": "1:2", "parent": "1:1", "type": "rect", "name": "Box", "x": 20, "y": 20,
          "width": 100, "height": 50, "fill": fill, "source_id": "figma:abc:1:2" },
    ])
}

fn envelope(nodes: Value) -> Value {
    json!({
        "format": "moshionly-import", "version": 1,
        "source": { "app": "figma", "file_name": "Launch", "file_key": "abc" },
        "nodes": nodes,
        "notes": [{ "source_id": "figma:abc:1:2", "layer_name": "Box",
                    "message": "The inner shadow was left out." }],
        "mode": "replace_matching",
        "plugin_version": "0.3"
    })
}

fn read(value: &Value) -> Result<PendingImport, EnvelopeError> {
    read_envelope(value.to_string().as_bytes())
}

#[test]
fn a_good_envelope_is_read_with_its_notes() {
    let pending = read(&envelope(design("#0D99FF"))).unwrap();
    assert_eq!(pending.source.app, "figma");
    assert_eq!(pending.source.file_key.as_deref(), Some("abc"));
    assert_eq!(pending.nodes.len(), 2);
    assert_eq!(pending.notes[0].layer_name.as_deref(), Some("Box"));
    assert_eq!(pending.mode, Some(RequestedMode::Replace));
    assert_eq!(
        pending.to_string(),
        "2 layers from \"Launch\" (figma), 1 note"
    );
}

#[test]
fn what_is_not_a_good_envelope_is_refused() {
    let refuse = |value: Value| read(&value).unwrap_err().to_string();
    assert!(
        read_envelope(b"{oops")
            .unwrap_err()
            .to_string()
            .starts_with("the body is not JSON")
    );
    let mut wrong = envelope(design("#000000"));
    wrong["format"] = json!("moshionly");
    assert!(refuse(wrong).contains("\"format\" should be \"moshionly-import\""));
    let mut wrong = envelope(design("#000000"));
    wrong["version"] = json!(2);
    assert!(refuse(wrong).contains("version 2"));
    let mut wrong = envelope(design("#000000"));
    wrong["source"] = json!({ "file_name": "x" });
    assert!(refuse(wrong).contains("\"source\" should be"));
    let mut wrong = envelope(design("#000000"));
    wrong["notes"] = json!([{ "layer_name": "x" }]);
    assert!(refuse(wrong).contains("\"notes\""));
    assert!(refuse(envelope(json!({}))).contains("\"nodes\" should be a list"));
    assert!(refuse(envelope(json!([]))).contains("the design has no layers"));

    // Animations are not part of a design.
    let mut nodes = design("#000000");
    nodes.as_array_mut().unwrap().push(json!({
        "id": "f", "parent": "1:1", "type": "fade_in", "target": "1:2", "start": 0, "end": 300
    }));
    assert!(refuse(envelope(nodes)).contains("is an animation"));
    // Unknown fields and values out of range are caught by core.
    let mut nodes = design("#000000");
    nodes[1]["colour"] = json!("#FFFFFF");
    assert!(refuse(envelope(nodes)).contains("\"colour\" is not a field of rect"));
    let mut nodes = design("#000000");
    nodes[1]["opacity"] = json!(300);
    assert!(refuse(envelope(nodes)).contains("opacity"));
}

#[test]
fn images_must_carry_their_pixels() {
    let image = |source: &str| {
        let mut nodes = design("#000000");
        nodes.as_array_mut().unwrap().push(json!({
            "id": "1:3", "parent": "1:1", "type": "image", "source": source
        }));
        envelope(nodes)
    };
    assert!(read(&image(PLACEHOLDER_IMAGE)).is_ok());
    for source in [
        "/Users/someone/.ssh/id_rsa",
        "file:///etc/hosts",
        "assets/a.png",
    ] {
        let error = read(&image(source)).unwrap_err();
        assert!(
            matches!(error, EnvelopeError::ImageNotEmbedded(_)),
            "{source}: {error}"
        );
    }
}

#[test]
fn an_accepted_design_is_one_import_step() {
    let session = demo_session();
    let pending = read(&envelope(design("#0D99FF"))).unwrap();
    let outcome = pending
        .apply(&session, &ImportTarget::NewArtboards)
        .unwrap();
    assert_eq!(outcome.roots.len(), 1);
    let document = session.document();
    assert_eq!(document.artboards().len(), 3);
    assert_eq!(document.get(&outcome.roots[0]).unwrap().name(), "Frame");
    let steps = session.read(|session| session.undo_steps().to_vec());
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].origin, Origin::Import);
    assert!(
        steps[0]
            .summary
            .starts_with("Import 2 layers from \"Launch\"")
    );

    let into = pending
        .apply(&session, &ImportTarget::Into(NodeId::from("card")))
        .unwrap();
    let frame = session.document().get(&into.roots[0]).unwrap().clone();
    assert_eq!(frame.parent, Some(NodeId::from("card")));
}

/// The id of the layer imported from `source`.
fn imported(session: &SessionHandle, source: &str) -> NodeId {
    let document = session.document();
    layers_from(&document, source, true)
        .into_iter()
        .chain(layers_from(&document, source, false))
        .next()
        .unwrap()
}

#[test]
fn sending_again_replaces_the_layers_and_keeps_their_animations() {
    let session = demo_session();
    read(&envelope(design("#0D99FF")))
        .unwrap()
        .apply(&session, &ImportTarget::ReplaceMatching)
        .unwrap();
    let old_box = imported(&session, "figma:abc:1:2");
    let fade = Node::from_value(json!({
        "id": "boxfade", "parent": imported(&session, "figma:abc:1:1"), "index": "a0",
        "type": "fade_in", "target": old_box, "start": 0, "end": 400
    }))
    .unwrap();
    session
        .apply(Origin::User, "Fade the box", &[Edit::add(fade)])
        .unwrap();

    // The second frame was never sent before, so it is added.
    let mut nodes = design("#FF5A1F");
    nodes.as_array_mut().unwrap().push(json!({
        "id": "9:1", "type": "artboard", "name": "Other", "source_id": "figma:abc:9:1"
    }));
    let outcome = read(&envelope(nodes))
        .unwrap()
        .apply(&session, &ImportTarget::ReplaceMatching)
        .unwrap();
    assert_eq!(outcome.replaced, 1);
    assert_eq!(outcome.roots.len(), 2);
    assert_eq!(outcome.dropped_animations, 0);

    let document = session.document();
    let new_box = imported(&session, "figma:abc:1:2");
    assert_eq!(
        document.get(&new_box).unwrap().to_value()["fill"],
        "#FF5A1F"
    );
    let animations = document.animations_targeting(&new_box);
    assert_eq!(animations.len(), 1, "the fade moved over to the new box");
    assert_eq!(document.artboards().len(), 4);
    assert!(!layers_from(&document, "figma:abc:9:1", true).is_empty());
}

#[test]
fn a_frame_moved_into_another_frame_since_comes_over_as_new() {
    let session = demo_session();
    read(&envelope(design("#0D99FF")))
        .unwrap()
        .apply(&session, &ImportTarget::ReplaceMatching)
        .unwrap();
    let artboards = session.document().artboards().len();

    // In Figma the frame now sits inside another frame, so it comes over as
    // a group, which cannot take the artboard's place.
    let mut nodes = design("#FF5A1F");
    nodes[0]["type"] = json!("group");
    let moved = read(&envelope(nodes)).unwrap();
    let outcome = moved
        .apply(&session, &ImportTarget::ReplaceMatching)
        .expect("the design comes over instead of being refused");
    assert_eq!(outcome.replaced, 0);
    assert_eq!(session.document().artboards().len(), artboards + 1);

    // Sending it again updates that copy instead of adding another.
    let outcome = moved
        .apply(&session, &ImportTarget::ReplaceMatching)
        .unwrap();
    assert_eq!(outcome.replaced, 1);
    let document = session.document();
    assert_eq!(document.artboards().len(), artboards + 1);
    let copies = document
        .nodes()
        .filter(|node| {
            node.layer()
                .and_then(|layer| layer.source_id.as_deref())
                .is_some_and(|id| id == "figma:abc:1:1")
        })
        .count();
    assert_eq!(copies, 2, "the first artboard and one group");
}

#[test]
fn a_layer_moved_out_of_a_frame_is_kept_whatever_the_order() {
    let session = demo_session();
    read(&envelope(design("#0D99FF")))
        .unwrap()
        .apply(&session, &ImportTarget::ReplaceMatching)
        .unwrap();

    // In Figma the box now sits on the canvas, beside its old frame, and is
    // selected before it.
    let nodes = json!([
        { "id": "1:2", "type": "rect", "name": "Box", "x": 500, "y": 20,
          "width": 100, "height": 50, "fill": "#FF5A1F", "source_id": "figma:abc:1:2" },
        { "id": "1:1", "type": "artboard", "name": "Frame", "width": 400, "height": 300,
          "fill": "#FFFFFF", "source_id": "figma:abc:1:1" },
    ]);
    let outcome = read(&envelope(nodes))
        .unwrap()
        .apply(&session, &ImportTarget::ReplaceMatching)
        .unwrap();
    assert_eq!(outcome.replaced, 1, "only the frame is put in place");
    let document = session.document();
    assert!(outcome.roots.iter().all(|id| document.contains(id)));
    let boxes: Vec<&Node> = document
        .nodes()
        .filter(|node| {
            node.layer()
                .and_then(|layer| layer.source_id.as_deref())
                .is_some_and(|id| id == "figma:abc:1:2")
        })
        .collect();
    assert_eq!(boxes.len(), 1, "the box is there once");
    let frame = imported(&session, "figma:abc:1:1");
    assert!(
        !document.is_ancestor(&frame, &boxes[0].id),
        "and outside its old frame"
    );
}

#[test]
fn a_layer_moved_out_and_back_in_keeps_one_copy_outside() {
    let session = demo_session();
    let apply = |nodes: Value| {
        read(&envelope(nodes))
            .unwrap()
            .apply(&session, &ImportTarget::ReplaceMatching)
            .unwrap()
    };
    let frame = json!({ "id": "1:1", "type": "artboard", "name": "Frame", "width": 400,
                        "height": 300, "fill": "#FFFFFF", "source_id": "figma:abc:1:1" });
    let outside = json!([
        { "id": "1:2", "type": "rect", "name": "Box", "x": 500, "y": 20,
          "width": 100, "height": 50, "fill": "#FF5A1F", "source_id": "figma:abc:1:2" },
        frame.clone(),
    ]);
    apply(design("#0D99FF"));
    let before = session.document().artboards().len();
    // Out of the frame, back in, and out again.
    apply(outside.clone());
    apply(design("#0D99FF"));
    let outcome = apply(outside);
    assert_eq!(
        outcome.replaced, 2,
        "the frame and the copy outside are updated"
    );
    assert_eq!(
        session.document().artboards().len(),
        before + 1,
        "one copy outside, not one more each time"
    );
}

#[test]
fn new_frames_sent_together_keep_their_arrangement() {
    let session = demo_session();
    let nodes = json!([
        { "id": "1:1", "type": "artboard", "name": "Top", "x": 0, "y": 0,
          "width": 400, "height": 300, "source_id": "figma:abc:1:1" },
        { "id": "2:1", "type": "artboard", "name": "Below", "x": 0, "y": 500,
          "width": 400, "height": 300, "source_id": "figma:abc:2:1" },
    ]);
    let outcome = read(&envelope(nodes))
        .unwrap()
        .apply(&session, &ImportTarget::ReplaceMatching)
        .unwrap();
    assert_eq!(outcome.roots.len(), 2);
    let document = session.document();
    let place = |source: &str| {
        let value = document
            .get(&imported(&session, source))
            .unwrap()
            .to_value();
        (
            value["x"].as_f64().unwrap_or(0.0),
            value["y"].as_f64().unwrap_or(0.0),
        )
    };
    let (top, below) = (place("figma:abc:1:1"), place("figma:abc:2:1"));
    assert_eq!(top.0, below.0, "still one above the other");
    assert_eq!(below.1 - top.1, 500.0);
}

#[test]
fn the_sink_hands_designs_on_and_wakes_the_app() {
    let woken = std::sync::Arc::new(AtomicUsize::new(0));
    let (sink, inbox) = ImportSink::new();
    let sink = sink.with_wake({
        let woken = woken.clone();
        move || {
            woken.fetch_add(1, Ordering::SeqCst);
        }
    });
    let pending = read(&envelope(design("#000000"))).unwrap();
    assert_eq!(sink.deliver(pending.clone()), Delivery::Delivered);
    assert_eq!(inbox.try_recv().unwrap(), pending);
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    drop(inbox);
    assert_eq!(sink.deliver(pending), Delivery::Closed);
}

#[test]
fn only_a_few_designs_wait_for_the_app_at_once() {
    let (sink, inbox) = ImportSink::new();
    let pending = read(&envelope(design("#000000"))).unwrap();
    for _ in 0..MAX_WAITING {
        assert_eq!(sink.deliver(pending.clone()), Delivery::Delivered);
    }
    assert_eq!(sink.deliver(pending.clone()), Delivery::Full);
    // Taking one makes room for one more.
    inbox.try_recv().unwrap();
    assert_eq!(sink.deliver(pending.clone()), Delivery::Delivered);
    assert_eq!(sink.deliver(pending), Delivery::Full);
}

//! Lottie export: the file parses, has what players need, converts every
//! kind of layer, and its keyframes land on Moshionly's own values.

mod common;

use std::fs;
use std::sync::Arc;

use moshionly_core::eval::{Property, PropertyValue, value_at};
use moshionly_core::{Document, Easing, NodeId, ease, io};
use moshionly_export::{ExportJob, Exported, Format, FrameRate, Settings};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};
use serde_json::Value;

/// Exports `document` as Lottie with `settings` and returns the parsed
/// file and what the export reported.
fn export_document(document: Arc<Document>, settings: Settings) -> (Value, Exported) {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("animation.json");
    let artboard = document.artboards()[0].clone();
    let job = ExportJob::new(document, artboard, Format::Lottie, &destination)
        .with_project_file(common::demo_path())
        .with_settings(settings)
        .with_renderer(Renderer::new(
            FontLibrary::bundled(),
            AssetLoader::new(Some(common::demo_path())),
        ));
    let (result, reports) = common::run(&job);
    let exported = result.expect("the document exports as Lottie");
    assert!(!reports.is_empty(), "progress is reported");
    let text = fs::read_to_string(&destination).unwrap();
    assert_eq!(exported.bytes, text.len() as u64, "the size is reported");
    let lottie = serde_json::from_str(&text).expect("the file is JSON");
    (lottie, exported)
}

/// Exports the demo as Lottie with `settings`.
fn export(settings: Settings) -> (Value, Exported) {
    export_document(common::demo(), settings)
}

/// A document from its nodes, on a 1000 by 600 artboard lasting
/// `duration` milliseconds.
fn document(duration: f64, nodes: &str) -> Arc<Document> {
    let text = format!(
        r##"{{"format":"moshionly","version":1,"name":"Test","nodes":[
            {{"id":"board","parent":null,"index":"a0","type":"artboard","name":"Board",
              "width":1000,"height":600,"duration":{duration},"fill":"#FFFFFF"}}
            {nodes}
        ]}}"##
    );
    Arc::new(io::parse(&text).expect("the test document is valid"))
}

/// The layer named `name` in a list of Lottie layers.
fn layer<'a>(layers: &'a Value, name: &str) -> &'a Value {
    layers
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["nm"] == name)
        .unwrap_or_else(|| panic!("a layer named {name}"))
}

/// A Lottie property's value at `frame`, worked out the way players do.
fn evaluate(property: &Value, frame: f64) -> Vec<f64> {
    let numbers = |value: &Value| -> Vec<f64> {
        match value {
            Value::Array(items) => items.iter().map(|v| v.as_f64().unwrap()).collect(),
            number => vec![number.as_f64().unwrap()],
        }
    };
    if property["a"] != 1 {
        return numbers(&property["k"]);
    }
    let keys = property["k"].as_array().unwrap();
    let time = |key: &Value| key["t"].as_f64().unwrap();
    let next = keys.partition_point(|key| time(key) <= frame);
    if next == 0 {
        return numbers(&keys[0]["s"]);
    }
    if next == keys.len() {
        return numbers(&keys[keys.len() - 1]["s"]);
    }
    let (key, after) = (&keys[next - 1], &keys[next]);
    let from = numbers(&key["s"]);
    if key["h"] == 1 {
        return from;
    }
    let to = numbers(&after["s"]);
    let handle = |side: &str, axis: &str| key[side][axis].as_f64().unwrap();
    let easing = Easing::Bezier {
        x1: handle("o", "x"),
        y1: handle("o", "y"),
        x2: handle("i", "x"),
        y2: handle("i", "y"),
    };
    let progress = ease(&easing, (frame - time(key)) / (time(after) - time(key)));
    from.iter()
        .zip(&to)
        .map(|(from, to)| from + (to - from) * progress)
        .collect()
}

/// A layer's position at `frame`, joined or split.
fn position(layer: &Value, frame: f64) -> [f64; 2] {
    let p = &layer["ks"]["p"];
    if p["s"] == true {
        [evaluate(&p["x"], frame)[0], evaluate(&p["y"], frame)[0]]
    } else if p.is_null() {
        [0.0, 0.0]
    } else {
        let value = evaluate(p, frame);
        [value[0], value[1]]
    }
}

/// A transform value of a layer at `frame`, or `default` when it is left
/// out.
fn transform(layer: &Value, key: &str, frame: f64, default: f64) -> f64 {
    let property = &layer["ks"][key];
    if property.is_null() {
        default
    } else {
        evaluate(property, frame)[0]
    }
}

fn number(document: &Document, layer: &str, property: Property, ms: f64) -> f64 {
    match value_at(document, &NodeId::from(layer), property, ms) {
        Some(PropertyValue::Number(value)) => value,
        other => panic!("{layer} {property:?}: {other:?}"),
    }
}

#[test]
fn the_file_has_what_players_need() {
    let (lottie, exported) = export(Settings::default());
    assert_eq!(lottie["v"].as_str().map(|v| v.starts_with('5')), Some(true));
    assert_eq!(lottie["fr"], 30);
    assert_eq!(lottie["ip"], 0);
    assert_eq!(lottie["op"], 120, "four seconds at 30 frames per second");
    assert_eq!(
        (lottie["w"].clone(), lottie["h"].clone()),
        (1080.into(), 1080.into())
    );
    assert!(lottie["layers"].is_array());
    assert!(lottie["assets"].is_array());
    for layer in lottie["layers"].as_array().unwrap() {
        for key in ["ty", "ind", "ks", "ip", "op", "st"] {
            assert!(!layer[key].is_null(), "every layer has {key}: {layer}");
        }
    }
    assert_eq!(exported.frames, 120);
    assert_eq!(exported.duration_ms, Some(4000.0));
}

#[test]
fn the_range_and_rate_are_kept() {
    let settings = Settings::default()
        .with_frame_rate(FrameRate::FPS_60)
        .with_range(Some(1000.0), Some(2000.0));
    let (lottie, _) = export(settings);
    assert_eq!(lottie["fr"], 60);
    assert_eq!(lottie["op"], 60);
}

/// Every easing, by the name of the layer that uses it.
fn easings() -> Vec<(&'static str, Easing)> {
    vec![
        ("linear", Easing::Linear),
        ("smooth", Easing::smooth()),
        ("natural", Easing::natural()),
        ("slow_down", Easing::slow_down()),
        ("accelerate", Easing::accelerate()),
        ("overshoot", Easing::Overshoot { intensity: 100.0 }),
        ("elastic", Easing::Elastic { intensity: 100.0 }),
        ("bounce", Easing::Bounce { intensity: 100.0 }),
        (
            "bezier",
            Easing::Bezier {
                x1: 0.7,
                y1: -0.6,
                x2: 0.2,
                y2: 1.5,
            },
        ),
    ]
}

#[test]
fn keyframes_land_on_the_values_of_every_easing_at_every_frame() {
    let mut nodes = String::new();
    for (index, (name, easing)) in easings().iter().enumerate() {
        let easing = serde_json::to_string(easing).unwrap();
        nodes.push_str(&format!(
            r##",{{"id":"{name}","parent":"board","index":"a{index}","type":"rect","name":"{name}",
                 "x":20,"y":{y},"width":40,"height":40,"fill":"#0D99FF"}}
               ,{{"id":"{name}m","parent":"board","index":"a{index}","type":"move","target":"{name}",
                 "start":300,"end":1700,"easing":{easing},"to":{{"x":800,"y":-10}}}}
               ,{{"id":"{name}o","parent":"board","index":"b{index}","type":"opacity","target":"{name}",
                 "start":500,"end":1500,"easing":{easing},"from":10}}"##,
            y = 40 + index * 60,
        ));
    }
    let document = document(2000.0, &nodes);
    let (lottie, exported) = export_document(document.clone(), Settings::default());
    assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);
    for (name, easing) in easings() {
        let layer = layer(&lottie["layers"], name);
        let keyframes = layer["ks"]["p"]["k"].as_array().map_or(0, Vec::len);
        if easing.control_points().is_some() {
            assert!(keyframes <= 4, "{name}: {keyframes} keyframes");
        }
        for frame in 0..=60 {
            let ms = f64::from(frame) * 1000.0 / 30.0;
            let x = number(&document, name, Property::X, ms) + 20.0;
            let y = number(&document, name, Property::Y, ms) + 20.0;
            let [px, py] = position(layer, f64::from(frame));
            assert!(
                (px - x).abs() <= 0.11 && (py - y).abs() <= 0.11,
                "{name} at frame {frame}: ({px}, {py}) instead of ({x}, {y})"
            );
            let opacity = number(&document, name, Property::Opacity, ms);
            let got = transform(layer, "o", f64::from(frame), 100.0);
            assert!(
                (got - opacity).abs() <= 0.1,
                "{name} at frame {frame}: opacity {got} instead of {opacity}"
            );
        }
    }
}

#[test]
fn combined_animations_land_on_the_values_at_every_frame() {
    // Two moves that add up, a fade cut off by another, a scale that
    // replaces another and a hide and show.
    let nodes = r##",{"id":"box","parent":"board","index":"a0","type":"rect","name":"Box",
            "x":100,"y":100,"width":80,"height":60,"fill":"#0D99FF","rotation":10}
        ,{"id":"m1","parent":"board","index":"a0","type":"move","target":"box",
            "start":0,"end":1200,"easing":{"type":"smooth"},"to":{"x":400,"y":0}}
        ,{"id":"m2","parent":"board","index":"a1","type":"move","target":"box",
            "start":600,"end":1800,"easing":{"type":"elastic"},"to":{"x":0,"y":300}}
        ,{"id":"o1","parent":"board","index":"a2","type":"opacity","target":"box",
            "start":0,"end":1000,"easing":{"type":"linear"},"from":0,"to":100}
        ,{"id":"o2","parent":"board","index":"a3","type":"opacity","target":"box",
            "start":500,"end":1500,"easing":{"type":"bounce"},"to":20}
        ,{"id":"s1","parent":"board","index":"a4","type":"scale","target":"box",
            "start":200,"end":900,"easing":{"type":"overshoot"},"to":2}
        ,{"id":"s2","parent":"board","index":"a5","type":"scale","target":"box",
            "start":700,"end":1600,"easing":{"type":"slow_down"},"to":0.5}
        ,{"id":"r1","parent":"board","index":"a6","type":"spin_in","target":"box",
            "start":0,"end":800,"easing":{"type":"natural"},"angle":180}
        ,{"id":"h1","parent":"board","index":"a7","type":"hide","target":"box","start":1100}
        ,{"id":"h2","parent":"board","index":"a8","type":"show","target":"box","start":1300}"##;
    let document = document(2000.0, nodes);
    for fps in [FrameRate::FPS_30, FrameRate::FPS_24, FrameRate::FPS_60] {
        let settings = Settings::default().with_frame_rate(fps);
        let (lottie, _) = export_document(document.clone(), settings);
        let layer = layer(&lottie["layers"], "Box");
        let frames = (2.0 * fps.fps()) as u32;
        for frame in 0..frames {
            let ms = fps.time_of(u64::from(frame));
            let x = number(&document, "box", Property::X, ms) + 40.0;
            let y = number(&document, "box", Property::Y, ms) + 30.0;
            let [px, py] = position(layer, f64::from(frame));
            assert!(
                (px - x).abs() <= 0.11 && (py - y).abs() <= 0.11,
                "at frame {frame} of {fps}: ({px}, {py}) instead of ({x}, {y})"
            );
            let visible = matches!(
                value_at(&document, &NodeId::from("box"), Property::Visible, ms),
                Some(PropertyValue::Flag(true))
            );
            let opacity = if visible {
                number(&document, "box", Property::Opacity, ms)
            } else {
                0.0
            };
            let got = transform(layer, "o", f64::from(frame), 100.0);
            assert!(
                (got - opacity).abs() <= 0.1,
                "at frame {frame} of {fps}: opacity {got} instead of {opacity}"
            );
            let scale = 100.0 * number(&document, "box", Property::Scale, ms);
            let got = transform(layer, "s", f64::from(frame), 100.0);
            assert!(
                (got - scale).abs() <= 0.04,
                "at frame {frame} of {fps}: scale {got} instead of {scale}"
            );
            let rotation = number(&document, "box", Property::Rotation, ms);
            let got = transform(layer, "r", f64::from(frame), 0.0);
            assert!(
                (got - rotation).abs() <= 0.02,
                "at frame {frame} of {fps}: rotation {got} instead of {rotation}"
            );
        }
    }
}

#[test]
fn every_shape_and_paint_converts() {
    let nodes = r##",{"id":"r1","parent":"board","index":"a0","type":"rect","name":"Rounded",
            "width":200,"height":120,"corner_radius":24,"fill":"#0D99FF",
            "stroke":{"paint":"#111111","width":8,"position":"inside"}}
        ,{"id":"r2","parent":"board","index":"a1","type":"rect","name":"Corners",
            "width":200,"height":120,"corner_radius":[0,40,10,60],
            "fill":{"type":"linear_gradient","start":[0,0],"end":[1,1],"stops":[
                {"offset":0,"color":"#FF5A5F"},{"offset":1,"color":"#06D6A080"}]}}
        ,{"id":"e1","parent":"board","index":"a2","type":"ellipse","name":"Ring",
            "inner_radius":0.5,"sweep":270,
            "fill":{"type":"radial_gradient","center":[0.5,0.5],"radius":[0.5,0.5],"stops":[
                {"offset":0,"color":"#7B61FF"},{"offset":1,"color":"#0D99FF"}]}}
        ,{"id":"e2","parent":"board","index":"a3","type":"ellipse","name":"Dot","fill":"#118AB2",
            "blend_mode":"multiply"}
        ,{"id":"s1","parent":"board","index":"a4","type":"star","name":"Star","fill":"#FFB703"}
        ,{"id":"p1","parent":"board","index":"a5","type":"path","name":"Wave",
            "path":{"subpaths":[{"anchors":[{"point":[0,0.5],"out":[0.25,0]},
                {"point":[1,0.5],"in":[0.75,1]}],"closed":false}]},
            "stroke":{"paint":"#EF476F","width":10,"cap":"round","join":"round"}}
        ,{"id":"gone","parent":"board","index":"a6","type":"rect","name":"Hidden","hidden":true}"##;
    let (lottie, exported) = export_document(document(1000.0, nodes), Settings::default());
    assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);
    let layers = &lottie["layers"];
    let kinds = |name: &str| -> String { layer(layers, name)["shapes"].to_string() };
    assert!(kinds("Rounded").contains(r#""ty":"rc""#));
    assert!(
        kinds("Rounded").contains(r#""s":{"k":[192,112]}"#),
        "an inside stroke sits inside"
    );
    assert!(
        kinds("Corners").contains(r#""ty":"sh""#),
        "corners of their own need a path"
    );
    assert!(kinds("Corners").contains(r#""ty":"gf""#) && kinds("Corners").contains(r#""t":1"#));
    assert!(kinds("Ring").contains(r#""ty":"sh""#) && kinds("Ring").contains(r#""t":2"#));
    assert!(kinds("Dot").contains(r#""ty":"el""#));
    assert_eq!(layer(layers, "Dot")["bm"], 1, "multiply");
    assert!(kinds("Star").contains(r#""ty":"sh""#));
    assert!(kinds("Wave").contains(r#""lc":2"#) && kinds("Wave").contains(r#""lj":2"#));
    assert!(
        layers
            .as_array()
            .unwrap()
            .iter()
            .all(|layer| layer["nm"] != "Hidden"),
        "hidden layers are left out"
    );
    assert_eq!(layer(layers, "Background")["shapes"][1]["ty"], "fl");
}

/// The precomposition with id `id` among the file's assets.
fn asset<'a>(lottie: &'a Value, id: &Value) -> &'a Value {
    lottie["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| &asset["id"] == id)
        .unwrap_or_else(|| panic!("an asset {id}"))
}

#[test]
fn groups_clips_and_masks_keep_their_structure() {
    let nodes = r##",{"id":"plain","parent":"board","index":"a0","type":"group","name":"Plain",
            "x":10,"y":10,"width":200,"height":100}
        ,{"id":"p1","parent":"plain","index":"a0","type":"rect","name":"In plain","fill":"#111111"}
        ,{"id":"faded","parent":"board","index":"a1","type":"group","name":"Faded",
            "x":300,"y":10,"width":200,"height":100,"opacity":50}
        ,{"id":"f1","parent":"faded","index":"a0","type":"rect","name":"In faded","fill":"#111111"}
        ,{"id":"clip","parent":"board","index":"a2","type":"group","name":"Clip",
            "x":10,"y":200,"width":200,"height":100,"clip":true,"corner_radius":20}
        ,{"id":"c1","parent":"clip","index":"a0","type":"ellipse","name":"In clip",
            "x":-50,"y":-50,"width":300,"height":300,"fill":"#0D99FF"}
        ,{"id":"mask","parent":"board","index":"a3","type":"mask_group","name":"Mask",
            "x":300,"y":200,"width":200,"height":200}
        ,{"id":"m1","parent":"mask","index":"a0","type":"ellipse","name":"Shape","fill":"#000000"}
        ,{"id":"m2","parent":"mask","index":"a1","type":"rect","name":"Masked","fill":"#FF0000"}
        ,{"id":"shown","parent":"board","index":"a4","type":"rect","name":"Shown","x":600,"y":10}
        ,{"id":"wipe","parent":"board","index":"a0","type":"reveal_in","target":"shown",
            "start":0,"end":500,"direction":"down"}"##;
    let (lottie, exported) = export_document(document(1000.0, nodes), Settings::default());
    assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);
    let layers = &lottie["layers"];

    // A group that only moves its children is a null layer they follow.
    let plain = layer(layers, "Plain");
    assert_eq!(plain["ty"], 3);
    assert_eq!(layer(layers, "In plain")["parent"], plain["ind"]);

    // A faded group draws its children together, in a composition.
    let faded = layer(layers, "Faded");
    assert_eq!(faded["ty"], 0);
    assert_eq!(faded["ks"]["o"]["k"], 50);
    let inside = &asset(&lottie, &faded["refId"])["layers"];
    assert_eq!(layer(inside, "In faded")["ty"], 4);

    // A clipping group is masked by its rounded box.
    let clip = layer(layers, "Clip");
    assert_eq!(clip["hasMask"], true);
    assert_eq!(clip["masksProperties"][0]["mode"], "a");
    assert_eq!(
        clip["masksProperties"][0]["pt"]["k"]["v"]
            .as_array()
            .unwrap()
            .len(),
        8
    );

    // A mask group is a track matte: the mask just above what it masks.
    let mask = layer(layers, "Mask");
    let pair = asset(&lottie, &mask["refId"])["layers"].as_array().unwrap();
    assert_eq!(pair.len(), 2);
    assert_eq!(
        (pair[0]["nm"].as_str(), pair[0]["td"].as_i64()),
        (Some("Shape"), Some(1))
    );
    assert_eq!(pair[1]["tt"], 1);
    let masked = &asset(&lottie, &pair[1]["refId"])["layers"];
    assert_eq!(layer(masked, "Masked")["ty"], 4);

    // A reveal is a mask that grows.
    let shown = layer(layers, "Shown");
    assert_eq!(shown["masksProperties"][0]["pt"]["a"], 1);

    // Every precomposition is large enough for what is seen of it.
    for name in ["Faded", "Clip", "Mask"] {
        let layer = layer(layers, name);
        assert!(layer["w"].as_f64().unwrap() >= 200.0 && layer["h"].as_f64().unwrap() >= 100.0);
    }
}

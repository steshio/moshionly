//! Layer effects in exports: raster formats draw them through the renderer,
//! and grain is the same every time the same frame is exported.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use moshionly_core::{Document, io};
use moshionly_export::{ExportJob, Format, FrameRate, Settings};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};

/// A 120 by 80 artboard, half a second long, holding one rect with
/// `effect` (the rest of a JSON object, such as `,"blur":4`).
fn document(effect: &str) -> Arc<Document> {
    let text = format!(
        r##"{{"format":"moshionly","version":4,"name":"Effects","nodes":[
            {{"id":"board","parent":null,"index":"a0","type":"artboard","width":120,"height":80,
              "duration":500,"fill":"#202020"}},
            {{"id":"card","parent":"board","index":"a0","type":"rect","x":30,"y":20,"width":60,
              "height":40,"fill":"#0D99FF"{effect}}}
        ]}}"##
    );
    Arc::new(io::parse(&text).expect("the test document is valid"))
}

/// Exports `document` as `format` to `path` and returns the file's bytes.
fn export(document: Arc<Document>, format: Format, path: &Path, settings: Settings) {
    let artboard = document.artboards()[0].clone();
    let job = ExportJob::new(document, artboard, format, path)
        .with_settings(settings)
        .with_renderer(Renderer::new(
            FontLibrary::bundled(),
            AssetLoader::default(),
        ));
    let (result, _) = common::run(&job);
    let exported = result.expect("the export works");
    assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);
}

#[test]
fn a_png_of_a_layer_with_each_effect_differs_from_one_without() {
    let dir = tempfile::tempdir().unwrap();
    let png = |name: &str, effect: &str| {
        let path = dir.path().join(format!("{name}.png"));
        export(document(effect), Format::Png, &path, Settings::default());
        common::decode_png(&fs::read(&path).unwrap()).2
    };
    let plain = png("plain", "");
    for (name, effect) in [
        (
            "glow",
            r##","glow":{"color":"#FFFFFF","radius":8,"spread":2}"##,
        ),
        ("noise", r#","noise":{"amount":0.5}"#),
        (
            "directional",
            r#","directional_blur":{"amount":12,"angle":30}"#,
        ),
    ] {
        let pixels = png(name, effect);
        assert_eq!(pixels.len(), plain.len());
        let changed = pixels
            .chunks(4)
            .zip(plain.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(changed > 200, "{name} changed only {changed} pixels");
    }
}

#[test]
fn two_exports_of_moving_grain_give_the_same_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let grainy = || document(r#","noise":{"amount":0.6,"size":2}"#);
    let settings = Settings::default().with_frame_rate(FrameRate::FPS_25);
    let (first, second) = (dir.path().join("first.gif"), dir.path().join("second.gif"));
    export(grainy(), Format::Gif, &first, settings);
    export(grainy(), Format::Gif, &second, settings);
    assert!(fs::read(&first).unwrap() == fs::read(&second).unwrap());

    // The grain moves: two frames in a row differ, and the same frame of
    // two exports is the same.
    let frames = |name: &str| {
        let folder = dir.path().join(name);
        export(grainy(), Format::PngSequence, &folder, settings);
        let read = |index: usize| fs::read(folder.join(format!("frame_{index:04}.png"))).unwrap();
        (read(0), read(1))
    };
    let (a0, a1) = frames("a");
    let (b0, b1) = frames("b");
    assert!(a0 != a1, "the grain changes from frame to frame");
    assert!(a0 == b0 && a1 == b1, "and is the same in every export");
}

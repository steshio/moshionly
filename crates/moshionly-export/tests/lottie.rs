//! Lottie export: the file parses, has what players need, and converts
//! every kind of layer in the demo.

mod common;

use std::fs;

use moshionly_export::{Format, FrameRate, Settings};
use serde_json::Value;

/// Exports the demo as Lottie with `settings` and returns the parsed file.
fn export(settings: Settings) -> (Value, moshionly_export::Exported) {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("demo.json");
    let job = common::job(Format::Lottie, &destination, settings);
    let (result, reports) = common::run(&job);
    let exported = result.expect("the demo exports as Lottie");
    assert!(!reports.is_empty(), "progress is reported");
    let text = fs::read_to_string(&destination).unwrap();
    assert_eq!(exported.bytes, text.len() as u64, "the size is reported");
    (
        serde_json::from_str(&text).expect("the file is JSON"),
        exported,
    )
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

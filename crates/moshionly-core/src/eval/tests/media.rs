//! Video layers: which moment of their file they show, and when.

use serde_json::json;

use super::*;

/// A 160 by 90 video "v" that plays from 500 to 1500 ms, starting 2000 ms
/// into its file, and an instance of a component with a video in it.
fn with_video() -> Document {
    document(json!([
        { "id": "v", "type": "video", "source": "clip.mp4", "width": 160, "height": 90,
          "fit": "contain", "corner_radius": 200, "stroke": { "width": 2 },
          "start": 500, "end": 1500, "trim_start": 2000 },
        { "id": "m", "type": "component", "width": 100, "height": 50 },
        { "id": "inner", "parent": "m", "index": "a0", "type": "video", "source": "inner.mp4",
          "width": 100, "height": 50, "start": 0, "end": 1000 },
        { "id": "late", "type": "instance", "component": "m", "y": 200, "width": 100, "height": 50,
          "time_offset": 300 },
    ]))
}

/// The moment of its file a video layer shows, or `None` when it is not
/// in the scene.
fn media_time(scene: &Scene, layer: &str) -> Option<f64> {
    fn find(node: &SceneNode) -> Option<f64> {
        match &node.content {
            Content::Video { media_time_ms, .. } => Some(*media_time_ms),
            Content::Group { children } => children.iter().find_map(find),
            _ => None,
        }
    }
    scene.find(&id(layer)).and_then(find)
}

#[test]
fn a_video_shows_the_moment_of_its_file_while_its_clip_plays() {
    let document = with_video();
    let shown = |time| media_time(&at(&document, time), "v");
    assert_eq!(shown(0.0), None);
    assert_eq!(shown(499.9), None);
    assert_eq!(shown(500.0), Some(2000.0));
    assert_eq!(shown(750.0), Some(2250.0));
    assert_eq!(shown(1499.0), Some(2999.0));
    assert_eq!(shown(1500.0), None, "the end is not part of the clip");
    assert_eq!(shown(1800.0), None);
}

#[test]
fn at_rest_a_video_shows_where_its_clip_begins() {
    let document = with_video();
    let rest = evaluate_at_rest(&document, &id("ab"), &MonospaceMeasurer::default()).unwrap();
    assert_eq!(media_time(&rest, "v"), Some(2000.0));
    let Content::Video {
        source,
        fit,
        radii,
        stroke,
        ..
    } = &node(&rest, "v").content
    else {
        panic!("a video")
    };
    assert_eq!(source, "clip.mp4");
    assert_eq!(*fit, crate::model::ImageFit::Contain);
    // Corners are limited to what fits, as for an image.
    assert_eq!(*radii, [45.0; 4]);
    assert_eq!(stroke.as_ref().map(|stroke| stroke.width), Some(2.0));
    // An instance at rest shows its component's video at rest too.
    assert_eq!(media_time(&rest, "late"), Some(0.0));
}

#[test]
fn an_instance_plays_the_video_of_its_component_later() {
    let document = with_video();
    let shown = |time| media_time(&at(&document, time), "late");
    assert_eq!(shown(200.0), None, "the offset has not passed");
    assert_eq!(shown(300.0), Some(0.0));
    assert_eq!(shown(1000.0), Some(700.0));
    assert_eq!(shown(1300.0), None);
    // The component itself plays it on time.
    assert_eq!(media_time(&at(&document, 1000.0), "m"), None);
    assert_eq!(media_time(&at(&document, 999.0), "m"), Some(999.0));
}

#[test]
fn a_video_takes_layer_animations_like_an_image() {
    let mut document = with_video();
    document
        .apply(&[crate::edit::Edit::add(
            Node::from_value(
                json!({ "id": "fade", "parent": "ab", "index": "a5", "type": "opacity",
                                     "target": "v", "start": 500, "end": 1000, "from": 0, "to": 100,
                                     "easing": { "type": "linear" } }),
            )
            .unwrap(),
        )])
        .unwrap();
    let scene = at(&document, 750.0);
    assert!((node(&scene, "v").opacity - 0.5).abs() < 1e-9);
    assert_eq!(media_time(&scene, "v"), Some(2250.0));
}

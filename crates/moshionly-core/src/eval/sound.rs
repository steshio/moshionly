//! The sound of an artboard: which files play when, from where, how loud.
//!
//! A [`Scene`](crate::Scene) is one frame, and sound is not made of frames,
//! so sound is listed separately: [`sounds`] gives every stretch of sound
//! that plays during a stretch of the timeline. The editor plays them in
//! step with the picture and the exporters mix them into the sound track.
//!
//! Sound comes from audio items and from video layers whose `volume` is
//! above 0, including videos inside the components that instances show,
//! later by each instance's `time_offset`. Sound does not follow
//! visibility: a hidden or faded video still plays its sound; set its
//! `volume` to 0 to silence it.

use crate::document::Document;
use crate::id::NodeId;
use crate::model::{ClipTiming, Family, NodeData};

/// How many instances deep sound is looked for inside components, like
/// evaluation's limit. The rules forbid a component that shows itself, so
/// this only guards against a broken document.
const MAX_INSTANCE_DEPTH: usize = 32;

/// One stretch of sound on an artboard's timeline: the part of a file that
/// plays from `start_ms` for `duration_ms`.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    /// The audio item or video layer the sound comes from. A video inside a
    /// component that an instance shows is listed by the video's own id,
    /// once for the component and once for each instance.
    pub node: NodeId,
    /// The file, as written in the document. Resolve it with
    /// [`resolve_asset`](crate::io::resolve_asset).
    pub source: String,
    /// When the sound starts on the artboard's timeline, in milliseconds.
    /// It is the whole clip, so it can start before the range asked for,
    /// and before 0 for a video in an instance that plays early.
    pub start_ms: f64,
    /// Where in the file it starts, in milliseconds from the file's start.
    pub media_start_ms: f64,
    /// How long it plays, in milliseconds.
    pub duration_ms: f64,
    /// How loud, above 0 and at most 1.
    pub volume: f64,
}

impl Sound {
    /// When the sound stops on the artboard's timeline, in milliseconds.
    pub fn end_ms(&self) -> f64 {
        self.start_ms + self.duration_ms
    }

    fn of(node: &NodeId, source: &str, clip: &ClipTiming, offset: f64) -> Self {
        Self {
            node: node.clone(),
            source: source.to_owned(),
            start_ms: clip.start + offset,
            media_start_ms: clip.trim_start,
            duration_ms: clip.duration(),
            volume: clip.volume,
        }
    }
}

/// The sound that plays on `artboard`'s timeline between `from_ms` and
/// `to_ms`: every audio item and every audible video that plays for some
/// of that time, whole, in a fixed order (audio items in timeline order,
/// then videos back to front).
///
/// Each [`Sound`] is the whole clip, so a player or a mixer takes the part
/// of it that falls in the range. Nothing is listed for an id that is not
/// an artboard. Whether the files exist and how long they are is not
/// checked: core does not read files.
pub fn sounds(document: &Document, artboard: &NodeId, from_ms: f64, to_ms: f64) -> Vec<Sound> {
    let mut found = Vec::new();
    if !matches!(
        document.get(artboard).map(|node| &node.data),
        Some(NodeData::Artboard(_))
    ) {
        return found;
    }
    for (_, node) in document.walk(artboard, Family::Animation) {
        if let NodeData::Audio(audio) = &node.data {
            found.push(Sound::of(&node.id, &audio.source, &audio.clip, 0.0));
        }
    }
    videos(document, artboard, 0.0, 0, &mut found);
    found.retain(|sound| sound.volume > 0.0 && sound.start_ms < to_ms && sound.end_ms() > from_ms);
    found
}

/// Adds the sound of the videos below `root`, `offset` milliseconds later,
/// looking into the components that instances show.
fn videos(document: &Document, root: &NodeId, offset: f64, depth: usize, found: &mut Vec<Sound>) {
    for (_, node) in document.walk(root, Family::Layer) {
        match &node.data {
            NodeData::Video(video) => {
                found.push(Sound::of(&node.id, &video.source, &video.clip, offset));
            }
            NodeData::Instance(instance) if depth < MAX_INSTANCE_DEPTH => {
                let shows_a_component = matches!(
                    document.get(&instance.component).map(|main| &main.data),
                    Some(NodeData::Component(_))
                );
                if shows_a_component {
                    let later = offset + instance.time_offset;
                    videos(document, &instance.component, later, depth + 1, found);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;
    use crate::model::Node;
    use serde_json::{Value, json};

    fn add(value: Value) -> Edit {
        Edit::add(Node::from_value(value).unwrap())
    }

    /// An artboard with music, a quiet video, a silent video, and an
    /// instance of a component with a video in it, 1000 ms later.
    fn sample() -> Document {
        let mut document = Document::new("Sound");
        document
            .apply(&[
                add(json!({ "id": "ab", "index": "a0", "type": "artboard", "duration": 6000 })),
                add(json!({ "id": "music", "parent": "ab", "index": "a0", "type": "audio",
                            "source": "music.mp3", "start": 0, "end": 6000, "trim_start": 2000 })),
                add(json!({ "id": "folder", "parent": "ab", "index": "a1", "type": "animation_group" })),
                add(json!({ "id": "click", "parent": "folder", "index": "a0", "type": "audio",
                            "source": "click.wav", "start": 4000, "end": 4100, "volume": 0.5 })),
                add(json!({ "id": "clip", "parent": "ab", "index": "a0", "type": "video",
                            "source": "clip.mp4", "start": 1000, "end": 3000, "volume": 0.25 })),
                add(json!({ "id": "mute", "parent": "ab", "index": "a1", "type": "video",
                            "source": "mute.mp4", "start": 0, "end": 3000, "volume": 0 })),
                add(json!({ "id": "card", "parent": "ab", "index": "a2", "type": "component" })),
                add(json!({ "id": "inner", "parent": "card", "index": "a0", "type": "video",
                            "source": "inner.mov", "start": 500, "end": 1500, "trim_start": 100 })),
                add(json!({ "id": "copy", "parent": "ab", "index": "a3", "type": "instance",
                            "component": "card", "time_offset": 1000 })),
            ])
            .unwrap();
        document
    }

    fn listed(document: &Document, from: f64, to: f64) -> Vec<(String, f64, f64, f64, f64)> {
        sounds(document, &NodeId::from("ab"), from, to)
            .into_iter()
            .map(|sound| {
                (
                    sound.source,
                    sound.start_ms,
                    sound.media_start_ms,
                    sound.duration_ms,
                    sound.volume,
                )
            })
            .collect()
    }

    #[test]
    fn the_whole_timeline_lists_every_audible_clip_in_a_fixed_order() {
        let document = sample();
        assert_eq!(
            listed(&document, 0.0, 6000.0),
            [
                ("music.mp3".into(), 0.0, 2000.0, 6000.0, 1.0),
                ("click.wav".into(), 4000.0, 0.0, 100.0, 0.5),
                ("clip.mp4".into(), 1000.0, 0.0, 2000.0, 0.25),
                // The component's own video, then the instance's, later.
                ("inner.mov".into(), 500.0, 100.0, 1000.0, 1.0),
                ("inner.mov".into(), 1500.0, 100.0, 1000.0, 1.0),
            ]
        );
    }

    #[test]
    fn only_clips_that_play_in_the_range_are_listed_whole() {
        let document = sample();
        let sources = |from, to| -> Vec<String> {
            listed(&document, from, to)
                .into_iter()
                .map(|sound| sound.0)
                .collect()
        };
        // At 3000 the video has stopped: its end is not part of it.
        assert_eq!(sources(3000.0, 3500.0), ["music.mp3"]);
        assert_eq!(
            sources(2400.0, 2600.0),
            ["music.mp3", "clip.mp4", "inner.mov"]
        );
        assert_eq!(sources(4099.0, 4100.0), ["music.mp3", "click.wav"]);
        assert_eq!(sources(4100.0, 5000.0), ["music.mp3"]);
        // A clip that starts before the range is still given whole.
        assert_eq!(
            listed(&document, 2900.0, 2950.0)[1],
            ("clip.mp4".into(), 1000.0, 0.0, 2000.0, 0.25)
        );
        assert!(sounds(&document, &NodeId::from("clip"), 0.0, 6000.0).is_empty());
    }

    #[test]
    fn an_instance_that_plays_early_starts_its_sound_before_zero() {
        let mut document = sample();
        document
            .apply(&[Edit::set("copy", "time_offset", -1000)])
            .unwrap();
        let inner: Vec<_> = listed(&document, 0.0, 6000.0)
            .into_iter()
            .filter(|sound| sound.0 == "inner.mov")
            .collect();
        assert_eq!(inner[1].1, -500.0);
        // Only the part after 0 is in the range [0, 400).
        assert_eq!(listed(&document, 0.0, 400.0).len(), 2);
    }
}

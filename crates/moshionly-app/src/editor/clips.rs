//! Video and sound clips on the timeline: moving them and trimming their
//! ends.
//!
//! A clip is a video layer or an audio item: between `start` and `end` on
//! the timeline it plays its file from `trim_start` on. Its bar on the
//! timeline is that stretch, and dragging it follows three rules:
//!
//! - Dragging the bar moves the clip: `start` and `end` together.
//! - Dragging its left edge trims the clip's beginning: `start` and
//!   `trim_start` move together, so what plays stays where it was on the
//!   timeline and only more or less of the file's beginning is heard. It
//!   cannot reach before the file's first moment.
//! - Dragging its right edge changes `end`.
//!
//! A video layer's bar is keyed by the layer's id and `/clip`, so that it
//! is not taken for an action; an audio item's bar is keyed by its own id,
//! as it is a row of the timeline like an action.

use moshionly_core::model::ClipTiming;
use moshionly_core::{Document, Edit, EditError, NodeId, NodeType};
use serde_json::{Map, Value};

use super::Editor;
use crate::{key::Key, timeline::Edge};

/// What ends the key of a video layer's clip bar.
const CLIP_SUFFIX: &str = "/clip";

/// The shortest a clip can be trimmed to, in milliseconds.
pub const SHORTEST_CLIP: f64 = 10.0;

/// The key of the bar of the clip of `node`, a video layer or an audio
/// item.
pub fn clip_key(node: &NodeId, ty: NodeType) -> Key {
    match ty {
        NodeType::Video => Key::new(format!("{node}{CLIP_SUFFIX}")),
        _ => Key::new(node.as_str()),
    }
}

/// The video layer or audio item whose clip the bar `key` is, if it is a
/// clip's bar.
pub fn clip_of(document: &Document, key: &Key) -> Option<NodeId> {
    let (id, ty) = match key.as_str().strip_suffix(CLIP_SUFFIX) {
        Some(layer) => (NodeId::from(layer), NodeType::Video),
        None => (NodeId::from(key.as_str()), NodeType::Audio),
    };
    (document.get(&id)?.node_type() == ty).then_some(id)
}

/// How the bar of a clip was dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipDrag {
    /// The whole bar: the clip moves.
    Move,
    /// One end: the clip is trimmed there.
    Trim(Edge),
}

/// The timing of `clip` once its bar was dragged so that it runs from
/// `start` to `end`, by the rules above. Nothing starts before 0.
pub fn dragged(clip: &ClipTiming, drag: ClipDrag, start: f64, end: f64) -> ClipTiming {
    match drag {
        ClipDrag::Move => {
            let start = start.max(0.0);
            ClipTiming {
                start,
                end: start + clip.duration(),
                ..*clip
            }
        }
        ClipDrag::Trim(Edge::Start) => {
            // Where the file's first moment sits on the timeline.
            let first = (clip.start - clip.trim_start).max(0.0);
            let start = start.max(first).min(clip.end - SHORTEST_CLIP);
            ClipTiming {
                start,
                trim_start: (clip.trim_start + (start - clip.start)).max(0.0),
                ..*clip
            }
        }
        ClipDrag::Trim(Edge::End) => ClipTiming {
            end: end.max(clip.start + SHORTEST_CLIP),
            ..*clip
        },
    }
}

/// The edit that gives the clip of `id` the timing `next`, or `None` when
/// it has it already or is not a clip.
pub fn clip_edit(document: &Document, id: &NodeId, next: &ClipTiming) -> Option<Edit> {
    let clip = document.get(id)?.data.clip()?;
    let mut patch = Map::new();
    for (field, now, wanted) in [
        ("start", clip.start, next.start),
        ("end", clip.end, next.end),
        ("trim_start", clip.trim_start, next.trim_start),
        ("volume", clip.volume, next.volume),
    ] {
        if now != wanted {
            patch.insert(field.to_owned(), Value::from(wanted));
        }
    }
    (!patch.is_empty()).then(|| Edit::update(id.clone(), patch))
}

impl Editor {
    /// Retimes the clip of `id` as a drag of its bar to `start`..`end`
    /// asks. Inside a gesture this is one more step of it; otherwise it is
    /// an undo step of its own. Returns whether anything changed.
    pub fn drag_clip(
        &mut self,
        summary: &str,
        id: &NodeId,
        drag: ClipDrag,
        start: f64,
        end: f64,
    ) -> Result<bool, EditError> {
        let Some(clip) = self.document.get(id).and_then(|node| node.data.clip()) else {
            return Ok(false);
        };
        let next = dragged(clip, drag, start, end);
        match clip_edit(&self.document, id, &next) {
            Some(edit) => self.apply(summary, &[edit]),
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn clip(start: f64, end: f64, trim_start: f64) -> ClipTiming {
        ClipTiming {
            trim_start,
            ..ClipTiming::new(start, end)
        }
    }

    #[test]
    fn dragging_the_bar_moves_the_whole_clip() {
        let moved = dragged(&clip(1000.0, 3000.0, 500.0), ClipDrag::Move, 1500.0, 3500.0);
        assert_eq!(moved, clip(1500.0, 3500.0, 500.0));
        // Not before the start of the scene, and never shorter.
        let early = dragged(&clip(1000.0, 3000.0, 0.0), ClipDrag::Move, -400.0, 1600.0);
        assert_eq!(early, clip(0.0, 2000.0, 0.0));
    }

    #[test]
    fn the_left_edge_trims_the_beginning_of_the_file() {
        let start = ClipDrag::Trim(Edge::Start);
        // Later: less of the file's beginning plays; the rest stays put.
        let later = dragged(&clip(1000.0, 3000.0, 500.0), start, 1300.0, 3000.0);
        assert_eq!(later, clip(1300.0, 3000.0, 800.0));
        assert_eq!(later.media_time(2000.0), 1500.0);
        // Earlier: more of it, down to the file's first moment.
        let earlier = dragged(&clip(1000.0, 3000.0, 500.0), start, 200.0, 3000.0);
        assert_eq!(earlier, clip(500.0, 3000.0, 0.0));
        // Never past the end.
        let short = dragged(&clip(1000.0, 3000.0, 0.0), start, 5000.0, 3000.0);
        assert_eq!(
            short,
            clip(3000.0 - SHORTEST_CLIP, 3000.0, 2000.0 - SHORTEST_CLIP)
        );
        // A clip moved to before the scene starts its file late; its
        // beginning stays out of reach.
        let at_zero = dragged(&clip(0.0, 1000.0, 300.0), start, -100.0, 1000.0);
        assert_eq!(at_zero, clip(0.0, 1000.0, 300.0));
    }

    #[test]
    fn the_right_edge_changes_the_end() {
        let end = ClipDrag::Trim(Edge::End);
        let longer = dragged(&clip(1000.0, 3000.0, 500.0), end, 1000.0, 4200.0);
        assert_eq!(longer, clip(1000.0, 4200.0, 500.0));
        let none = dragged(&clip(1000.0, 3000.0, 500.0), end, 1000.0, 400.0);
        assert_eq!(none, clip(1000.0, 1000.0 + SHORTEST_CLIP, 500.0));
    }

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":3,"name":"Clips","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","duration":5000},
            {"id":"film","parent":"ab","index":"a0","type":"video","source":"a.mp4",
             "start":1000,"end":3000,"trim_start":500},
            {"id":"song","parent":"ab","index":"a0","type":"audio","source":"a.mp3",
             "start":0,"end":2000},
            {"id":"fade","parent":"ab","index":"a1","type":"fade_in","target":"film",
             "start":0,"end":500}
            ]}"##,
        )
        .unwrap()
    }

    #[test]
    fn clip_bars_are_told_from_actions_by_their_keys() {
        let doc = document();
        let film = NodeId::from("film");
        let key = clip_key(&film, NodeType::Video);
        assert_eq!(key.as_str(), "film/clip");
        assert_eq!(clip_of(&doc, &key), Some(film.clone()));
        let song = NodeId::from("song");
        assert_eq!(clip_of(&doc, &clip_key(&song, NodeType::Audio)), Some(song));
        // An action's bar, a layer's own id and things that are gone are
        // not clips.
        assert_eq!(clip_of(&doc, &Key::new("fade")), None);
        assert_eq!(clip_of(&doc, &Key::new("film")), None);
        assert_eq!(clip_of(&doc, &Key::new("gone/clip")), None);
    }

    #[test]
    fn a_drag_becomes_one_edit_of_what_changed() {
        let doc = document();
        let film = NodeId::from("film");
        let next = clip(1300.0, 3000.0, 800.0);
        let edit = clip_edit(&doc, &film, &next).unwrap();
        let mut after = doc.clone();
        after.apply(&[edit]).unwrap();
        assert_eq!(after.get(&film).unwrap().data.clip(), Some(&next));
        let same = *doc.get(&film).unwrap().data.clip().unwrap();
        assert!(clip_edit(&doc, &film, &same).is_none());
        assert!(clip_edit(&doc, &NodeId::from("fade"), &next).is_none());
    }
}

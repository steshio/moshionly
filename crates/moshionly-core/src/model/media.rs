//! Media: video layers and audio items, which play a file on the timeline.
//!
//! Both are clips: between `start` and `end` on the artboard's timeline
//! they play their file from `trim_start` on, so at time `t` a clip is at
//! `trim_start + (t - start)` in its file. A video is a layer and draws
//! that moment's frame like an image; an audio item draws nothing. Both
//! play their sound at `volume`.
//!
//! Files are read with `ffmpeg`, outside this crate: core cannot tell how
//! long a file is, so a clip that runs past the end of its file is not an
//! error. Past the end a video keeps its last frame and the sound is
//! silent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::layer::{ImageFit, LayerProps};
use super::paint::{CornerRadius, Stroke};
use crate::defaults;

/// When a clip plays on the artboard's timeline, from which point of its
/// file, and how loud.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClipTiming {
    /// When the clip starts playing, in milliseconds from the start of the
    /// artboard, 0 or more.
    #[schemars(range(min = 0.0))]
    pub start: f64,
    /// When the clip stops playing, in milliseconds from the start of the
    /// artboard; after `start`.
    #[schemars(range(min = 0.0))]
    pub end: f64,
    /// Where in the file the clip begins, in milliseconds from the start
    /// of the file, 0 or more: at `start` the clip is this far into its
    /// file. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    #[schemars(range(min = 0.0))]
    pub trim_start: f64,
    /// How loud the clip's sound plays, from 0 (silent) to 1 (as recorded).
    /// Default 1.
    #[serde(default = "defaults::one", skip_serializing_if = "defaults::is_one")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub volume: f64,
}

impl ClipTiming {
    /// A clip from `start` to `end` that plays its file from the beginning
    /// at full volume.
    pub fn new(start: f64, end: f64) -> Self {
        Self {
            start,
            end,
            trim_start: 0.0,
            volume: 1.0,
        }
    }

    /// Length of the clip on the timeline, in milliseconds.
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }

    /// Whether the clip is playing at `time_ms`: from `start` up to, not
    /// including, `end`.
    pub fn plays_at(&self, time_ms: f64) -> bool {
        self.start <= time_ms && time_ms < self.end
    }

    /// Where in the file the clip is at `time_ms` on the timeline, in
    /// milliseconds from the start of the file.
    pub fn media_time(&self, time_ms: f64) -> f64 {
        self.trim_start + (time_ms - self.start)
    }

    /// Moves the clip `offset` milliseconds later on the timeline (earlier
    /// when negative), playing the same part of its file.
    ///
    /// The timeline starts at 0, so a clip moved to start before 0 starts
    /// at 0 and skips the part of its file it would have played before:
    /// from 0 on it plays exactly as it would have. When nothing of the
    /// clip is left after 0 it is not changed, and this returns `false`.
    pub fn shift(&mut self, offset: f64) -> bool {
        let (start, end) = (self.start + offset, self.end + offset);
        if !(end > 0.0) {
            return false;
        }
        if start < 0.0 {
            self.trim_start -= start;
            self.start = 0.0;
        } else {
            self.start = start;
        }
        self.end = end;
        true
    }
}

/// A video: the frames of a video file, drawn like an image while the clip
/// plays, with the file's sound.
///
/// Between `start` and `end` it shows the frame of its file at
/// `trim_start + (t - start)`; before `start` and from `end` on it is not
/// drawn. In the design, with no time, it shows the frame at
/// `trim_start`. The box, corners and outline work as for an image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoLayer {
    /// The shared layer properties.
    #[serde(flatten)]
    pub layer: LayerProps,
    /// Where the video comes from: a path relative to the project file or
    /// an absolute path. Video cannot be embedded as a `data:` URI.
    pub source: String,
    /// How the frames fill the box when their proportions differ.
    /// Default `cover`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub fit: ImageFit,
    /// Outline of the box. Default: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Rounding of the corners in pixels. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub corner_radius: CornerRadius,
    /// When the video plays, from where in its file, and how loud its
    /// sound is.
    #[serde(flatten)]
    pub clip: ClipTiming,
}

/// Sound from a file, played on the artboard's timeline. It draws nothing.
///
/// It sits on the timeline like an animation but changes no layer, so it
/// has no `target`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Audio {
    /// Label shown on the timeline row. Default: empty, the editor shows
    /// the file's name.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub name: String,
    /// Where the sound comes from: a path relative to the project file or
    /// an absolute path, of a sound file or a video file with sound. Sound
    /// cannot be embedded as a `data:` URI.
    pub source: String,
    /// When the sound plays, from where in its file, and how loud.
    #[serde(flatten)]
    pub clip: ClipTiming,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_maps_timeline_time_to_file_time() {
        let clip = ClipTiming {
            trim_start: 500.0,
            ..ClipTiming::new(1000.0, 3000.0)
        };
        assert_eq!(clip.duration(), 2000.0);
        assert!(!clip.plays_at(999.0));
        assert!(clip.plays_at(1000.0));
        assert!(clip.plays_at(2999.0));
        assert!(!clip.plays_at(3000.0), "the end is not part of the clip");
        assert_eq!(clip.media_time(1000.0), 500.0);
        assert_eq!(clip.media_time(1750.0), 1250.0);
    }

    #[test]
    fn a_shifted_clip_plays_the_same_part_of_its_file() {
        let clip = ClipTiming {
            trim_start: 500.0,
            ..ClipTiming::new(1000.0, 3000.0)
        };
        let mut later = clip;
        assert!(later.shift(250.0));
        assert_eq!(
            (later.start, later.end, later.trim_start),
            (1250.0, 3250.0, 500.0)
        );
        // Moved to before 0: it starts at 0, further into its file.
        let mut early = clip;
        assert!(early.shift(-1500.0));
        assert_eq!(
            (early.start, early.end, early.trim_start),
            (0.0, 1500.0, 1000.0)
        );
        assert_eq!(early.media_time(700.0), clip.media_time(2200.0));
        // Moved so far that it is over before 0: nothing changes.
        let mut gone = clip;
        assert!(!gone.shift(-3000.0));
        assert_eq!(gone, clip);
    }
}

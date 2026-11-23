//! What a person chooses about an export, and the checked plan it becomes.
//!
//! [`Settings`] are the choices: size, frame rate, background, quality and
//! so on, each with a sensible default. [`Settings::plan`] checks them
//! against a format and an artboard before any work starts, and either says
//! what is wrong or returns a [`Plan`]: the exact pixel size, render
//! options and moments every writer works from.

use moshionly_core::model::{NodeData, Paint};
use moshionly_core::{Color, Document, NodeId, Rect};
use moshionly_render::{Background, RenderOptions};
use thiserror::Error;

use crate::format::{Format, SizeLimits};
use crate::timing::{FrameRate, FrameTimes, TimingError};

/// The most frames one export may have: a million, which is over nine hours
/// at 30 frames per second. More than that is almost certainly a mistake,
/// such as a duration with extra zeros, and is refused before anything is
/// drawn or written.
pub const MAX_FRAMES: u64 = 1_000_000;

/// How big the exported frames are. Proportions are always kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    /// A multiple of the artboard's size: 1 is its own size, 2 twice as
    /// sharp, 0.5 half.
    Scale(f64),
    /// This many pixels wide, and as tall as the proportions say.
    Width(u32),
    /// This many pixels tall, and as wide as the proportions say.
    Height(u32),
}

impl Default for Size {
    /// The artboard's own size.
    fn default() -> Self {
        Size::Scale(1.0)
    }
}

/// How hard the encoder works and how large the file may get, for video
/// and GIF. PNG is lossless and ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Quality {
    /// Quick and small, for checking timing: visible compression.
    Draft,
    /// Good for sharing: compression is hard to see.
    #[default]
    Standard,
    /// For a final master: larger files, and slower.
    High,
}

impl Quality {
    /// Every quality, lowest first.
    pub const ALL: [Quality; 3] = [Quality::Draft, Quality::Standard, Quality::High];

    /// The short name the command line uses: `draft`, `standard`, `high`.
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Draft => "draft",
            Quality::Standard => "standard",
            Quality::High => "high",
        }
    }

    /// Looks a quality up by its short name, ignoring case.
    pub fn parse(name: &str) -> Option<Quality> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|quality| quality.as_str() == name)
    }
}

/// How many times a GIF plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Loop {
    /// Over and over.
    #[default]
    Forever,
    /// Once, then it stays on its last frame.
    Once,
    /// This many times in all, then it stays on its last frame. `Times(1)`
    /// is the same as [`Loop::Once`].
    Times(u16),
}

/// What a person chooses about an export. Every field has a default; the
/// `with_` methods change one at a time.
///
/// ```
/// use moshionly_export::{Background, FrameRate, Quality, Settings, Size};
///
/// let settings = Settings::default()
///     .with_size(Size::Width(1920))
///     .with_frame_rate(FrameRate::FPS_60)
///     .with_background(Background::Transparent)
///     .with_quality(Quality::High);
/// assert_eq!(settings.frame_rate.fps(), 60.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// How big the frames are. Default: the artboard's own size.
    pub size: Size,
    /// Frames per second, for everything that moves. Default: 30.
    pub frame_rate: FrameRate,
    /// What is behind the layers. Default: the artboard's own fill.
    ///
    /// - [`Background::Artboard`]: the artboard as designed. Where its fill
    ///   is missing or see-through, formats that can be transparent stay
    ///   transparent and MP4 shows white.
    /// - [`Background::Transparent`]: no background at all, even when the
    ///   artboard has a fill. Not possible for MP4.
    /// - [`Background::Over`]: the artboard over a colour that fills the
    ///   frame; an opaque colour makes every frame opaque.
    pub background: Background,
    /// How hard video and GIF encoders work. Default: standard.
    pub quality: Quality,
    /// For a single PNG: the moment to capture, in milliseconds. `None`
    /// captures the design at rest, with no animation applied. Default:
    /// `None`.
    pub still_ms: Option<f64>,
    /// For animated formats: where to start, in milliseconds. Default: the
    /// start of the artboard.
    pub start_ms: Option<f64>,
    /// For animated formats: where to stop, in milliseconds. The frame at
    /// the end itself is not included; see [`crate::timing`]. Default: the
    /// end of the artboard.
    pub end_ms: Option<f64>,
    /// How many times a GIF plays. Default: forever.
    pub gif_loop: Loop,
    /// Whether a GIF mixes palette colours in a fine pattern to show the
    /// colours between them, which smooths gradients and shadows. Default:
    /// on.
    pub dither: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            size: Size::default(),
            frame_rate: FrameRate::default(),
            background: Background::Artboard,
            quality: Quality::default(),
            still_ms: None,
            start_ms: None,
            end_ms: None,
            gif_loop: Loop::default(),
            dither: true,
        }
    }
}

impl Settings {
    /// The same settings at another size.
    pub fn with_size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// The same settings at another scale.
    pub fn with_scale(self, scale: f64) -> Self {
        self.with_size(Size::Scale(scale))
    }

    /// The same settings at another frame rate.
    pub fn with_frame_rate(mut self, frame_rate: FrameRate) -> Self {
        self.frame_rate = frame_rate;
        self
    }

    /// The same settings with another background.
    pub fn with_background(mut self, background: Background) -> Self {
        self.background = background;
        self
    }

    /// The same settings at another quality.
    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }

    /// The same settings capturing a single PNG at `time_ms`, or at rest
    /// for `None`.
    pub fn with_still(mut self, time_ms: Option<f64>) -> Self {
        self.still_ms = time_ms;
        self
    }

    /// The same settings exporting only from `start_ms` to `end_ms`.
    pub fn with_range(mut self, start_ms: Option<f64>, end_ms: Option<f64>) -> Self {
        self.start_ms = start_ms;
        self.end_ms = end_ms;
        self
    }

    /// The same settings with another GIF loop.
    pub fn with_gif_loop(mut self, gif_loop: Loop) -> Self {
        self.gif_loop = gif_loop;
        self
    }

    /// The same settings with GIF dithering on or off.
    pub fn with_dither(mut self, dither: bool) -> Self {
        self.dither = dither;
        self
    }

    /// Checks the settings for exporting `artboard` of `document` as
    /// `format`, and works out exactly what will be written.
    pub fn plan(
        &self,
        format: Format,
        document: &Document,
        artboard: &NodeId,
    ) -> Result<Plan, SettingsError> {
        let node = document
            .get(artboard)
            .ok_or_else(|| SettingsError::UnknownArtboard(artboard.clone()))?;
        let NodeData::Artboard(board) = &node.data else {
            return Err(SettingsError::NotAnArtboard(node.label()));
        };
        let (board_width, board_height) = (board.layer.width, board.layer.height);
        let (width, height, scale) = self.pixel_size(format, board_width, board_height)?;
        let fill_is_opaque = board.fill.as_ref().is_some_and(is_opaque);
        let (background, transparent) = self.resolve_background(format, fill_is_opaque)?;
        let render = RenderOptions::new()
            .with_scale(scale)
            .with_background(background)
            .with_region(Rect::new(
                0.0,
                0.0,
                f64::from(width) / scale,
                f64::from(height) / scale,
            ));
        let moments = self.moments(format, board.duration)?;
        if let Some(times) = moments.frames() {
            let (fps, max_fps) = (times.rate().fps(), format.max_fps_at(width, height));
            if fps > max_fps {
                return Err(SettingsError::FrameRateTooHighForSize {
                    format,
                    fps,
                    max_fps,
                    width,
                    height,
                });
            }
        }
        if self.gif_loop == Loop::Times(0) {
            return Err(SettingsError::NoPlays);
        }
        Ok(Plan {
            format,
            artboard: artboard.clone(),
            artboard_name: node.name().to_owned(),
            width,
            height,
            scale,
            render,
            transparent,
            moments,
            quality: self.quality,
            gif_loop: self.gif_loop,
            dither: self.dither,
        })
    }

    /// The frame size in pixels, and the scale that fills it.
    fn pixel_size(
        &self,
        format: Format,
        board_width: f64,
        board_height: f64,
    ) -> Result<(u32, u32, f64), SettingsError> {
        let scale = match self.size {
            Size::Scale(scale) => scale,
            Size::Width(width) => f64::from(width) / board_width,
            Size::Height(height) => f64::from(height) / board_height,
        };
        if !(scale.is_finite() && scale > 0.0) {
            return Err(match self.size {
                Size::Scale(scale) => SettingsError::BadScale(scale),
                _ => SettingsError::ZeroSize,
            });
        }
        let pixels = |length: f64| (length * scale).round().max(1.0);
        let (width, height) = (pixels(board_width), pixels(board_height));
        let limits = format.size_limits();
        if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
            return Err(SettingsError::TooLarge {
                format,
                width: width.min(1e15) as u64,
                height: height.min(1e15) as u64,
                limits,
            });
        }
        let (mut width, mut height) = (width as u32, height as u32);
        if format.needs_even_size() {
            width = (width & !1).max(2);
            height = (height & !1).max(2);
        }
        if !limits.allows(width, height) {
            let (long, short) = (width.max(height), width.min(height));
            let too_large = long > limits.max_long_side
                || short > limits.max_short_side
                || u64::from(width) * u64::from(height) > limits.max_pixels;
            return Err(if too_large {
                SettingsError::TooLarge {
                    format,
                    width: u64::from(width),
                    height: u64::from(height),
                    limits,
                }
            } else {
                SettingsError::TooSmall {
                    format,
                    width,
                    height,
                    min_side: limits.min_side,
                }
            });
        }
        Ok((width, height, scale))
    }

    /// What the renderer puts behind the layers, and whether the file keeps
    /// transparency.
    fn resolve_background(
        &self,
        format: Format,
        fill_is_opaque: bool,
    ) -> Result<(Background, bool), SettingsError> {
        let can_be_clear = format.supports_transparency();
        Ok(match self.background {
            Background::Transparent if !can_be_clear => {
                return Err(SettingsError::CannotBeTransparent(format));
            }
            Background::Transparent => (Background::Transparent, true),
            Background::Artboard if can_be_clear && !fill_is_opaque => (Background::Artboard, true),
            // A frame that must be opaque gets white behind it, so edges and
            // see-through parts never leave holes.
            Background::Artboard => (Background::Over(Color::WHITE), false),
            Background::Over(color) if color.a == 255 => (Background::Over(color), false),
            Background::Over(color) if can_be_clear => (Background::Over(color), true),
            Background::Over(color) => (Background::Over(over_white(color)), false),
        })
    }

    /// The moments to capture.
    fn moments(&self, format: Format, duration_ms: f64) -> Result<Moments, SettingsError> {
        let within = |time: f64| time.is_finite() && (0.0..=duration_ms).contains(&time);
        if !format.is_animated() {
            return match self.still_ms {
                None => Ok(Moments::Still(Moment::AtRest)),
                Some(time) if within(time) => Ok(Moments::Still(Moment::At(time))),
                Some(time) => Err(SettingsError::OutsideArtboard {
                    time_ms: time,
                    duration_ms,
                }),
            };
        }
        let fps = self.frame_rate.fps();
        if fps > format.max_fps() {
            return Err(SettingsError::FrameRateTooHigh {
                format,
                fps,
                max_fps: format.max_fps(),
            });
        }
        let start = self.start_ms.unwrap_or(0.0);
        let end = self.end_ms.unwrap_or(duration_ms);
        for time in [start, end] {
            if !within(time) {
                return Err(SettingsError::OutsideArtboard {
                    time_ms: time,
                    duration_ms,
                });
            }
        }
        let times = FrameTimes::new(start, end, self.frame_rate)?;
        if times.len() > MAX_FRAMES {
            return Err(SettingsError::TooManyFrames {
                frames: times.len(),
                max: MAX_FRAMES,
            });
        }
        Ok(Moments::Frames(times))
    }
}

/// Whether a paint covers everything behind it.
fn is_opaque(paint: &Paint) -> bool {
    match paint {
        Paint::Solid(color) => color.a == 255,
        Paint::Gradient(gradient) => gradient.stops().iter().all(|stop| stop.color.a == 255),
    }
}

/// A colour laid over white, for formats with no transparency.
fn over_white(color: Color) -> Color {
    let alpha = u32::from(color.a);
    let mix = |channel: u8| ((u32::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
    Color::rgb(mix(color.r), mix(color.g), mix(color.b))
}

/// One moment of an artboard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Moment {
    /// The design as it is drawn, with no animation applied.
    AtRest,
    /// The artboard at this many milliseconds from its start.
    At(f64),
}

/// The moments an export captures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Moments {
    /// One picture.
    Still(Moment),
    /// A frame for each of these times.
    Frames(FrameTimes),
}

impl Moments {
    /// The number of frames.
    pub fn len(&self) -> u64 {
        match self {
            Moments::Still(_) => 1,
            Moments::Frames(times) => times.len(),
        }
    }

    /// Always false: there is at least one frame.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The moment frame `index` shows, or `None` past the last frame.
    pub fn get(&self, index: u64) -> Option<Moment> {
        match self {
            Moments::Still(moment) => (index == 0).then_some(*moment),
            Moments::Frames(times) => times.time(index).map(Moment::At),
        }
    }

    /// Every moment, in order. One entry for every frame, so at most
    /// [`MAX_FRAMES`] for a checked plan; writers ask for moments one at a
    /// time with [`get`](Self::get) instead.
    pub fn to_vec(&self) -> Vec<Moment> {
        (0..self.len())
            .filter_map(|index| self.get(index))
            .collect()
    }

    /// The frame times of an animated export.
    pub fn frames(&self) -> Option<&FrameTimes> {
        match self {
            Moments::Still(_) => None,
            Moments::Frames(times) => Some(times),
        }
    }
}

/// Checked settings for one export: what exactly will be written.
///
/// Every writer works from a plan, including formats that do not render
/// pixels: they use the size, the frame rate and the moments, and ignore
/// the render options.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The kind of file.
    pub format: Format,
    /// The artboard exported.
    pub artboard: NodeId,
    /// Its name, which may be empty.
    pub artboard_name: String,
    /// Width of every frame in pixels.
    pub width: u32,
    /// Height of every frame in pixels.
    pub height: u32,
    /// Output pixels per artboard pixel.
    pub scale: f64,
    /// How each frame is rendered: the scale, the background and the region
    /// that gives exactly `width` by `height` pixels. Formats that need even
    /// sizes lose at most a pixel's width at the right or bottom edge.
    pub render: RenderOptions,
    /// Whether the file keeps transparency. False whenever every frame is
    /// opaque, so encoders can leave the alpha channel out.
    pub transparent: bool,
    /// The moments to capture.
    pub moments: Moments,
    /// How hard the encoder works.
    pub quality: Quality,
    /// How many times a GIF plays.
    pub gif_loop: Loop,
    /// Whether a GIF is dithered.
    pub dither: bool,
}

impl Plan {
    /// The number of frames.
    pub fn frame_count(&self) -> u64 {
        self.moments.len()
    }

    /// How long the export plays for, in milliseconds. `None` for a still.
    pub fn duration_ms(&self) -> Option<f64> {
        self.moments.frames().map(FrameTimes::duration_ms)
    }
}

/// Why settings cannot be used for an export. Each message says what to
/// change.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SettingsError {
    /// No node has the artboard's id.
    #[error("there is no artboard with the id \"{0}\"; choose one of the document's artboards")]
    UnknownArtboard(NodeId),
    /// The id belongs to something that is not an artboard.
    #[error("{0} is not an artboard; exports are made of whole artboards")]
    NotAnArtboard(String),
    /// The scale is zero, negative or not a number.
    #[error("the scale {0} cannot be used; choose a number above 0, for example 1, 2 or 0.5")]
    BadScale(f64),
    /// A width or height of zero pixels was asked for.
    #[error("a width or height of 0 pixels cannot be exported; choose at least 1")]
    ZeroSize,
    /// The frames would be larger than the format allows: a side is too
    /// long, or there are too many pixels in all.
    #[error("{}", too_large(*format, *width, *height, limits))]
    TooLarge {
        /// The format.
        format: Format,
        /// Width asked for, in pixels.
        width: u64,
        /// Height asked for, in pixels.
        height: u64,
        /// What the format allows.
        limits: SizeLimits,
    },
    /// The frames would be smaller than the format allows.
    #[error(
        "{format} frames must be at least {min_side} pixels on each side, and this export would \
         be {width} by {height}; choose a larger scale or width"
    )]
    TooSmall {
        /// The format.
        format: Format,
        /// Width asked for, in pixels.
        width: u32,
        /// Height asked for, in pixels.
        height: u32,
        /// The shortest side allowed.
        min_side: u32,
    },
    /// A transparent background was asked of a format that cannot be
    /// transparent.
    #[error(
        "{0} video cannot be transparent; choose WebM or ProRes for a transparent video, or give \
         the export a background colour"
    )]
    CannotBeTransparent(Format),
    /// The frame rate is higher than the format plays.
    #[error(
        "{format} can play at most {max_fps} frames per second, and {fps} was asked for; choose \
         {max_fps} or fewer"
    )]
    FrameRateTooHigh {
        /// The format.
        format: Format,
        /// The rate asked for.
        fps: f64,
        /// The highest rate the format plays.
        max_fps: f64,
    },
    /// The frame rate is higher than the format plays at this frame size:
    /// see [`Format::max_fps_at`].
    #[error(
        "{format} video of {width} by {height} pixels can play at most {max_fps} frames per \
         second, and {fps} was asked for; choose {max_fps} or fewer, or a smaller size"
    )]
    FrameRateTooHighForSize {
        /// The format.
        format: Format,
        /// The rate asked for.
        fps: f64,
        /// The highest rate the format plays at this size.
        max_fps: f64,
        /// Width of the frames, in pixels.
        width: u32,
        /// Height of the frames, in pixels.
        height: u32,
    },
    /// A moment or range end is before the start or after the end of the
    /// artboard.
    #[error(
        "{time_ms} ms is outside the artboard, which lasts {duration_ms} ms; choose a time from \
         0 to {duration_ms}"
    )]
    OutsideArtboard {
        /// The time asked for, in milliseconds.
        time_ms: f64,
        /// The artboard's length, in milliseconds.
        duration_ms: f64,
    },
    /// The frame rate or the range cannot be used.
    #[error(transparent)]
    Timing(#[from] TimingError),
    /// The export would have more than [`MAX_FRAMES`] frames.
    #[error(
        "this export would have {frames} frames, and at most {max} can be written; shorten the \
         time range or lower the frame rate"
    )]
    TooManyFrames {
        /// How many frames the settings ask for. Saturates at `u64::MAX`.
        frames: u64,
        /// The most there may be: [`MAX_FRAMES`].
        max: u64,
    },
    /// A GIF was asked to play zero times.
    #[error("a GIF must play at least once; choose once, a number of times, or forever")]
    NoPlays,
}

/// What [`SettingsError::TooLarge`] says: the limit on sides when a side is
/// too long, otherwise the limit on pixels in all, so the message never
/// quotes a size larger than the one it refuses.
fn too_large(format: Format, width: u64, height: u64, limits: &SizeLimits) -> String {
    let (long, short) = (width.max(height), width.min(height));
    let sides_fit =
        long <= u64::from(limits.max_long_side) && short <= u64::from(limits.max_short_side);
    if !sides_fit {
        return format!(
            "{format} frames can be at most {} by {} pixels (either way round), and this export \
             would be {width} by {height}; choose a smaller scale or width",
            limits.max_long_side, limits.max_short_side
        );
    }
    // The widest frame that fits, as an example.
    let example_short = (limits.max_pixels / u64::from(limits.max_long_side.max(1)))
        .min(u64::from(limits.max_short_side));
    format!(
        "{format} frames can have at most {} pixels in all, for example {} by {example_short}, \
         and this export would be {width} by {height}, which is {}; choose a smaller scale or \
         width",
        millions(limits.max_pixels),
        limits.max_long_side,
        millions(width.saturating_mul(height)),
    )
}

/// A number of pixels as people read it, such as "35.4 million".
fn millions(pixels: u64) -> String {
    format!("{:.1} million", pixels as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::io;

    fn document(fill: &str) -> Document {
        io::parse(&format!(
            r##"{{"format":"moshionly","version":1,"name":"Test","nodes":[
                {{"id":"board","parent":null,"index":"a0","type":"artboard","name":"Board",
                 "width":1081,"height":600,"duration":2000{fill}}},
                {{"id":"box","parent":"board","index":"a0","type":"rect","name":"Box",
                 "width":10,"height":10,"fill":"#FF0000"}}
            ]}}"##
        ))
        .unwrap()
    }

    /// A document whose artboard lasts `duration_ms`.
    fn lasting(duration_ms: f64) -> Document {
        io::parse(&format!(
            r#"{{"format":"moshionly","version":1,"name":"Long","nodes":[
                {{"id":"board","parent":null,"index":"a0","type":"artboard","name":"Board",
                 "width":100,"height":100,"duration":{duration_ms}}}
            ]}}"#
        ))
        .unwrap()
    }

    fn plan_for(settings: Settings, format: Format) -> Result<Plan, SettingsError> {
        settings.plan(
            format,
            &document(r##","fill":"#FFFFFF""##),
            &NodeId::from("board"),
        )
    }

    #[test]
    fn sizes_keep_proportions() {
        let plan = plan_for(Settings::default(), Format::Png).unwrap();
        assert_eq!((plan.width, plan.height, plan.scale), (1081, 600, 1.0));
        assert_eq!(plan.render.region, Some(Rect::new(0.0, 0.0, 1081.0, 600.0)));

        let half = plan_for(Settings::default().with_scale(0.5), Format::Png).unwrap();
        assert_eq!((half.width, half.height), (541, 300));

        let wide = Settings::default().with_size(Size::Width(2162));
        let wide = plan_for(wide, Format::Gif).unwrap();
        assert_eq!((wide.width, wide.height, wide.scale), (2162, 1200, 2.0));

        let tall = Settings::default().with_size(Size::Height(300));
        let tall = plan_for(tall, Format::Png).unwrap();
        assert_eq!((tall.width, tall.height), (541, 300));
    }

    #[test]
    fn video_sizes_are_even() {
        let plan = plan_for(Settings::default(), Format::Mp4).unwrap();
        assert_eq!((plan.width, plan.height), (1080, 600));
        // The region is a hair narrower than the artboard, so exactly 1080
        // pixels come out.
        let region = plan.render.region.unwrap();
        assert_eq!(region.width * plan.scale, 1080.0);
        let options = plan.render;
        let scene = moshionly_core::Scene {
            artboard: NodeId::from("board"),
            time_ms: 0.0,
            width: 1081.0,
            height: 600.0,
            background: None,
            nodes: Vec::new(),
        };
        assert_eq!(options.frame_size(&scene), Ok((1080, 600)));
        let odd = plan_for(Settings::default().with_scale(0.3), Format::WebM).unwrap();
        assert_eq!((odd.width, odd.height), (324, 180));
    }

    #[test]
    fn sizes_outside_the_limits_are_refused() {
        let error = plan_for(Settings::default().with_scale(4.0), Format::Mp4).unwrap_err();
        assert!(matches!(error, SettingsError::TooLarge { .. }), "{error}");
        assert!(error.to_string().contains("smaller scale"), "{error}");
        let error = plan_for(Settings::default().with_scale(0.01), Format::Mp4).unwrap_err();
        assert!(matches!(error, SettingsError::TooSmall { .. }), "{error}");
        for scale in [0.0, -1.0, f64::NAN] {
            let error = plan_for(Settings::default().with_scale(scale), Format::Png).unwrap_err();
            assert!(matches!(error, SettingsError::BadScale(_)), "{error}");
        }
        let error =
            plan_for(Settings::default().with_size(Size::Width(0)), Format::Png).unwrap_err();
        assert_eq!(error, SettingsError::ZeroSize);
        // A tiny scale still gives at least one pixel.
        let tiny = plan_for(Settings::default().with_scale(0.0001), Format::Png).unwrap();
        assert_eq!((tiny.width, tiny.height), (1, 1));
        let huge = plan_for(Settings::default().with_scale(1e300), Format::Png).unwrap_err();
        assert!(matches!(huge, SettingsError::TooLarge { .. }), "{huge}");
    }

    #[test]
    fn the_limit_on_pixels_in_all_is_named_when_it_is_what_fails() {
        // 8192 by 4546: both sides fit WebM's 8192, but not in all.
        let wide = Settings::default().with_size(Size::Width(8192));
        let error = plan_for(wide, Format::WebM).unwrap_err();
        assert!(matches!(error, SettingsError::TooLarge { .. }), "{error}");
        let text = error.to_string();
        assert!(text.contains("35.4 million pixels in all"), "{text}");
        assert!(text.contains("for example 8192 by 4320"), "{text}");
        assert!(
            text.contains("8192 by 4546, which is 37.2 million"),
            "{text}"
        );
        assert!(!text.contains("8192 by 8192"), "{text}");

        let png = Settings::default().with_size(Size::Width(16_000));
        let text = plan_for(png, Format::Png).unwrap_err().to_string();
        assert!(text.contains("134.2 million pixels in all"), "{text}");
        assert!(text.contains("for example 16384 by 8192"), "{text}");

        // A side that is too long names the limit on sides.
        let wider = Settings::default().with_size(Size::Width(9000));
        let text = plan_for(wider, Format::WebM).unwrap_err().to_string();
        assert!(text.contains("at most 8192 by 8192 pixels"), "{text}");
    }

    #[test]
    fn backgrounds_follow_what_the_format_can_hold() {
        let transparent = Settings::default().with_background(Background::Transparent);
        let webm = plan_for(transparent, Format::WebM).unwrap();
        assert!(webm.transparent);
        assert_eq!(webm.render.background, Background::Transparent);
        let error = plan_for(transparent, Format::Mp4).unwrap_err();
        assert_eq!(error, SettingsError::CannotBeTransparent(Format::Mp4));
        assert!(error.to_string().contains("WebM or ProRes"), "{error}");

        // An opaque artboard is opaque in every format.
        for format in Format::ALL {
            let plan = plan_for(Settings::default(), format).unwrap();
            assert!(!plan.transparent, "{format}");
            assert_eq!(plan.render.background, Background::Over(Color::WHITE));
        }

        // An artboard without a fill stays clear where it can.
        let clear = document("");
        let board = NodeId::from("board");
        let gif = Settings::default()
            .plan(Format::Gif, &clear, &board)
            .unwrap();
        assert!(gif.transparent);
        assert_eq!(gif.render.background, Background::Artboard);
        let mp4 = Settings::default()
            .plan(Format::Mp4, &clear, &board)
            .unwrap();
        assert!(!mp4.transparent);
        assert_eq!(mp4.render.background, Background::Over(Color::WHITE));

        // A see-through colour is laid over white where it must be.
        let glass =
            Settings::default().with_background(Background::Over(Color::rgba(0, 0, 0, 128)));
        let mp4 = glass.plan(Format::Mp4, &clear, &board).unwrap();
        assert_eq!(
            mp4.render.background,
            Background::Over(Color::rgb(127, 127, 127))
        );
        let png = glass.plan(Format::Png, &clear, &board).unwrap();
        assert!(png.transparent);
        let black = Settings::default().with_background(Background::Over(Color::BLACK));
        let png = black.plan(Format::Png, &clear, &board).unwrap();
        assert!(!png.transparent);
    }

    #[test]
    fn moments_stay_inside_the_artboard() {
        let still = plan_for(Settings::default(), Format::Png).unwrap();
        assert_eq!(still.moments, Moments::Still(Moment::AtRest));
        assert_eq!(still.frame_count(), 1);
        assert_eq!(still.duration_ms(), None);
        let at = plan_for(Settings::default().with_still(Some(2000.0)), Format::Png).unwrap();
        assert_eq!(at.moments.get(0), Some(Moment::At(2000.0)));
        assert_eq!(at.moments.get(1), None);
        let error =
            plan_for(Settings::default().with_still(Some(2001.0)), Format::Png).unwrap_err();
        assert!(error.to_string().contains("0 to 2000"), "{error}");

        let whole = plan_for(Settings::default(), Format::Gif).unwrap();
        assert_eq!(whole.frame_count(), 60);
        assert_eq!(whole.duration_ms(), Some(2000.0));
        let part = Settings::default().with_range(Some(500.0), Some(1000.0));
        let part = plan_for(part, Format::Mp4).unwrap();
        assert_eq!(part.frame_count(), 15);
        assert_eq!(part.moments.get(0), Some(Moment::At(500.0)));
        for (start, end) in [(Some(-1.0), None), (None, Some(3000.0))] {
            let range = Settings::default().with_range(start, end);
            assert!(matches!(
                plan_for(range, Format::Mp4),
                Err(SettingsError::OutsideArtboard { .. })
            ));
        }
        let backwards = Settings::default().with_range(Some(1000.0), Some(500.0));
        assert!(matches!(
            plan_for(backwards, Format::Mp4),
            Err(SettingsError::Timing(TimingError::EmptyRange { .. }))
        ));
    }

    #[test]
    fn exports_with_too_many_frames_are_refused() {
        let board = NodeId::from("board");
        // A million frames is the most: 40 000 seconds at 25 fps.
        let most = Settings::default().with_frame_rate(FrameRate::FPS_25);
        let plan = most.plan(Format::Mp4, &lasting(40_000_000.0), &board);
        assert_eq!(plan.unwrap().frame_count(), MAX_FRAMES);
        let error = most
            .plan(Format::Mp4, &lasting(40_000_040.0), &board)
            .unwrap_err();
        assert_eq!(
            error,
            SettingsError::TooManyFrames {
                frames: MAX_FRAMES + 1,
                max: MAX_FRAMES
            }
        );
        assert!(
            error
                .to_string()
                .contains("shorten the time range or lower the frame rate"),
            "{error}"
        );

        // A duration with extra zeros, and one too long to count.
        let typo = Settings::default().plan(Format::Gif, &lasting(1e13), &board);
        assert!(
            matches!(
                typo,
                Err(SettingsError::TooManyFrames {
                    frames: 300_000_000_000,
                    ..
                })
            ),
            "{typo:?}"
        );
        let endless = Settings::default().plan(Format::PngSequence, &lasting(1e300), &board);
        assert!(
            matches!(
                endless,
                Err(SettingsError::TooManyFrames {
                    frames: u64::MAX,
                    ..
                })
            ),
            "{endless:?}"
        );
        // A still is one frame however long the artboard.
        let still = Settings::default().with_still(Some(1e299));
        assert!(still.plan(Format::Png, &lasting(1e300), &board).is_ok());
        // A shorter range of a long artboard is fine.
        let part = Settings::default().with_range(Some(0.0), Some(10_000.0));
        assert!(part.plan(Format::Mp4, &lasting(1e13), &board).is_ok());
    }

    #[test]
    fn gifs_have_their_own_limits() {
        let fast = Settings::default().with_frame_rate(FrameRate::FPS_60);
        let error = plan_for(fast, Format::Gif).unwrap_err();
        assert!(
            matches!(error, SettingsError::FrameRateTooHigh { .. }),
            "{error}"
        );
        assert!(plan_for(fast, Format::Mp4).is_ok());
        // A still ignores the frame rate.
        assert!(plan_for(fast, Format::Png).is_ok());
        let never = Settings::default().with_gif_loop(Loop::Times(0));
        assert_eq!(
            plan_for(never, Format::Gif).unwrap_err(),
            SettingsError::NoPlays
        );
    }

    #[test]
    fn mp4_frame_rates_stop_where_h264_does() {
        let rate = |fps| Settings::default().with_frame_rate(FrameRate::new(fps, 1).unwrap());
        assert!(plan_for(rate(120), Format::Mp4).is_ok());
        let error = plan_for(rate(240), Format::Mp4).unwrap_err();
        assert!(
            matches!(
                error,
                SettingsError::FrameRateTooHigh { max_fps: 172.0, .. }
            ),
            "{error}"
        );
        assert!(plan_for(rate(240), Format::WebM).is_ok());
        // Large frames allow fewer: 3840 by 2130 stops at 64.47.
        let large = rate(120).with_size(Size::Width(3840));
        let error = plan_for(large, Format::Mp4).unwrap_err();
        assert_eq!(
            error,
            SettingsError::FrameRateTooHighForSize {
                format: Format::Mp4,
                fps: 120.0,
                max_fps: 64.47,
                width: 3840,
                height: 2130,
            }
        );
        assert!(error.to_string().contains("or a smaller size"), "{error}");
        assert!(plan_for(rate(60).with_size(Size::Width(3840)), Format::Mp4).is_ok());
    }

    #[test]
    fn only_artboards_are_exported() {
        let document = document("");
        let error = Settings::default()
            .plan(Format::Png, &document, &NodeId::from("box"))
            .unwrap_err();
        assert!(matches!(error, SettingsError::NotAnArtboard(_)), "{error}");
        let error = Settings::default()
            .plan(Format::Png, &document, &NodeId::from("nope"))
            .unwrap_err();
        assert_eq!(error, SettingsError::UnknownArtboard(NodeId::from("nope")));
    }

    #[test]
    fn qualities_have_names() {
        for quality in Quality::ALL {
            assert_eq!(Quality::parse(quality.as_str()), Some(quality));
        }
        assert_eq!(Quality::parse(" HIGH "), Some(Quality::High));
        assert_eq!(Quality::parse("best"), None);
    }
}

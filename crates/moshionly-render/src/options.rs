//! What a caller chooses about a render: size, background and region.

use moshionly_core::{Color, Rect, Scene};

use crate::error::RenderError;
use crate::frame;

/// How a scene is turned into a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    /// Output pixels per artboard pixel. 1 renders a 1080 pixel artboard as
    /// 1080 pixels, 0.5 as 540 (a quick preview), 2 as 2160 (a sharp export).
    pub scale: f64,
    /// What is behind the layers.
    pub background: Background,
    /// The part of the artboard to render, in artboard coordinates. The
    /// frame then holds only that part. `None` renders the whole artboard.
    /// Whatever lies outside the artboard stays transparent.
    pub region: Option<Rect>,
}

/// What is behind the layers of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Background {
    /// The artboard's own fill. Where the artboard has none, or its fill is
    /// partly transparent, the frame is transparent.
    #[default]
    Artboard,
    /// Nothing, even when the artboard has a fill: for stickers, overlays
    /// and video with an alpha channel.
    Transparent,
    /// The artboard's fill over this colour. With an opaque colour the whole
    /// frame is opaque, which is what video and JPEG need.
    Over(Color),
}

impl Default for RenderOptions {
    /// The whole artboard at its own size, on its own fill.
    fn default() -> Self {
        Self {
            scale: 1.0,
            background: Background::Artboard,
            region: None,
        }
    }
}

impl RenderOptions {
    /// The whole artboard at its own size, on its own fill.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same options at another scale.
    pub fn with_scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    /// The same options with another background.
    pub fn with_background(mut self, background: Background) -> Self {
        self.background = background;
        self
    }

    /// The same options for one part of the artboard.
    pub fn with_region(mut self, region: Rect) -> Self {
        self.region = Some(region);
        self
    }

    /// The part of the artboard a frame of `scene` shows.
    pub fn region_of(&self, scene: &Scene) -> Rect {
        self.region
            .unwrap_or_else(|| Rect::new(0.0, 0.0, scene.width, scene.height))
    }

    /// The width and height in pixels of a frame of `scene`.
    ///
    /// Sizes that do not land on whole pixels are rounded up, so nothing of
    /// the region is cut off.
    pub fn frame_size(&self, scene: &Scene) -> Result<(u32, u32), RenderError> {
        if !(self.scale.is_finite() && self.scale > 0.0) {
            return Err(RenderError::BadScale(self.scale));
        }
        let region = self.region_of(scene);
        let (width, height) = (region.width * self.scale, region.height * self.scale);
        let usable = |length: f64| length.is_finite() && length > 0.0;
        if !(usable(width) && usable(height) && region.x.is_finite() && region.y.is_finite()) {
            return Err(RenderError::EmptyFrame { width, height });
        }
        // 540.0000001 is 540 pixels: arithmetic noise must not add a row.
        let pixels = |length: f64| (length - 1e-6).ceil().max(1.0);
        let (width, height) = (pixels(width), pixels(height));
        let limit = f64::from(frame::MAX_FRAME_SIDE);
        if width > limit || height > limit {
            return Err(RenderError::FrameTooLarge {
                width: width.min(1e18) as u64,
                height: height.min(1e18) as u64,
                max_side: frame::MAX_FRAME_SIDE,
                max_pixels: frame::MAX_FRAME_PIXELS,
            });
        }
        let (width, height) = (width as u32, height as u32);
        frame::byte_len(width, height)?;
        Ok((width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::NodeId;

    fn scene(width: f64, height: f64) -> Scene {
        Scene {
            artboard: NodeId::from("ab"),
            time_ms: 0.0,
            width,
            height,
            background: None,
            nodes: Vec::new(),
        }
    }

    #[test]
    fn frames_follow_scale_and_region() {
        let scene = scene(1080.0, 720.0);
        let options = RenderOptions::new();
        assert_eq!(options.frame_size(&scene), Ok((1080, 720)));
        assert_eq!(options.with_scale(0.5).frame_size(&scene), Ok((540, 360)));
        assert_eq!(options.with_scale(2.0).frame_size(&scene), Ok((2160, 1440)));
        // A third of 1080 is 360 exactly, whatever the arithmetic says.
        assert_eq!(
            options.with_scale(1.0 / 3.0).frame_size(&scene),
            Ok((360, 240))
        );
        // Parts of a pixel are kept, not cut off.
        assert_eq!(
            options.with_scale(0.3334).frame_size(&scene),
            Ok((361, 241))
        );
        let part = options
            .with_region(Rect::new(100.0, 50.0, 200.5, 10.0))
            .with_scale(2.0);
        assert_eq!(part.frame_size(&scene), Ok((401, 20)));
        assert_eq!(part.region_of(&scene), Rect::new(100.0, 50.0, 200.5, 10.0));
    }

    #[test]
    fn sizes_that_cannot_be_rendered_are_refused() {
        let scene = scene(1080.0, 1080.0);
        let options = RenderOptions::new();
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                options.with_scale(scale).frame_size(&scene),
                Err(RenderError::BadScale(_))
            ));
        }
        assert!(matches!(
            options
                .with_region(Rect::new(0.0, 0.0, 0.0, 10.0))
                .frame_size(&scene),
            Err(RenderError::EmptyFrame { .. })
        ));
        assert!(matches!(
            RenderOptions::new().frame_size(&self::scene(0.0, 100.0)),
            Err(RenderError::EmptyFrame { .. })
        ));
        assert!(matches!(
            options.with_scale(100.0).frame_size(&scene),
            Err(RenderError::FrameTooLarge { .. })
        ));
        // Within the side limit but too many pixels in all.
        assert!(matches!(
            options.with_scale(14.0).frame_size(&scene),
            Err(RenderError::FrameTooLarge { .. })
        ));
    }
}

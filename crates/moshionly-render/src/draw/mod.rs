//! Drawing a scene onto a Skia canvas.
//!
//! [`Painter`] walks the scene tree. Each node is drawn in its own
//! coordinate space: the painter saves the canvas, applies the node's
//! transform, draws, and restores.

pub(crate) mod effects;
pub(crate) mod geometry;
mod glow;
mod image;
mod paint;
pub(crate) mod stroke;
mod text;

use moshionly_core::Rect;
use moshionly_core::scene::{Content, Scene, SceneNode, ScenePaint, TextContent};
use skia_safe as sk;

use crate::assets::{self, AssetLoader};
use crate::convert;
use crate::error::RenderWarning;
use crate::fonts::FontLibrary;
use crate::hit::{Geometry, Precision};
use crate::options::Background;

/// Draws scenes onto one canvas and collects what went wrong on the way.
pub(crate) struct Painter<'a> {
    canvas: &'a sk::Canvas,
    fonts: &'a FontLibrary,
    assets: &'a AssetLoader,
    warnings: Vec<RenderWarning>,
}

impl<'a> Painter<'a> {
    /// A painter for `canvas`, drawing text with `fonts` and images from
    /// `assets`. The canvas's current transform is where the artboard's
    /// top-left corner goes.
    pub(crate) fn new(
        canvas: &'a sk::Canvas,
        fonts: &'a FontLibrary,
        assets: &'a AssetLoader,
    ) -> Self {
        Self {
            canvas,
            fonts,
            assets,
            warnings: Vec::new(),
        }
    }

    /// The warnings gathered so far.
    pub(crate) fn into_warnings(self) -> Vec<RenderWarning> {
        self.warnings
    }

    /// Draws a whole scene: background first, then the nodes back to front,
    /// everything clipped to the artboard. A colour the scene is drawn
    /// `Over` fills everything the canvas can show, so the edge pixels the
    /// artboard only partly covers, and a region reaching past the
    /// artboard, are not left see-through.
    pub(crate) fn scene(&mut self, scene: &Scene, background: Background) {
        let canvas = self.canvas;
        let artboard = convert::rect(&Rect::new(0.0, 0.0, scene.width, scene.height));
        if let Background::Over(color) = background {
            let mut paint = sk::Paint::default();
            paint.set_color4f(convert::color(color), None);
            canvas.draw_paint(&paint);
        }
        canvas.save();
        canvas.clip_rect(artboard, sk::ClipOp::Intersect, true);
        if background != Background::Transparent
            && let Some(fill) = scene
                .background
                .as_ref()
                .and_then(|paint| paint::fill(paint, 1.0))
        {
            canvas.draw_rect(artboard, &fill);
        }
        for node in &scene.nodes {
            self.node(node);
        }
        canvas.restore();
    }

    /// Draws one node and everything inside it.
    fn node(&mut self, node: &SceneNode) {
        // A transform that flattens space leaves nothing to see, and Skia
        // must not be given numbers that are not finite.
        if !convert::is_finite(&node.transform) || node.transform.invert().is_none() {
            return;
        }
        let opacity = node.opacity as f32;
        if opacity.is_nan() || opacity <= 0.0 {
            return;
        }
        let opacity = opacity.min(1.0);
        let canvas = self.canvas;
        canvas.save();
        canvas.concat(&convert::matrix(&node.transform));
        self.effects(node, opacity);
        canvas.restore();
    }

    /// Draws a node's content with its effects, in the order of
    /// `docs/architecture.md`, innermost first: content, shadow, blur, mask
    /// and clip, blend mode, opacity. Each effect that needs one gets an
    /// offscreen layer, and the layers nest the other way round.
    fn effects(&mut self, node: &SceneNode, opacity: f32) {
        let canvas = self.canvas;
        let filter = effects::filter(node);
        let blend = effects::blend_mode(node.blend_mode);
        let plain = filter.is_none() && node.mask.is_none() && blend == sk::BlendMode::SrcOver;
        if plain && (opacity >= 1.0 || is_flat(node)) {
            // Nothing needs a layer: fading the one thing drawn is fading
            // the node.
            if let Some(clip) = &node.clip {
                canvas.clip_rrect(convert::rounded_rect(&clip.rect, &clip.radii), None, true);
            }
            self.content(node, opacity);
            return;
        }

        // Offscreen layers are only as large as what the node can draw;
        // a node that draws nothing is skipped.
        let geometry = Geometry { fonts: self.fonts };
        let Some(visual) = geometry.visual(node, Precision::Quick) else {
            return;
        };
        let visual = layer_bounds(&visual);
        // Opacity and blend mode apply to everything below as a whole, so
        // that what the node draws does not show through itself.
        let composite = opacity < 1.0 || blend != sk::BlendMode::SrcOver;
        if composite {
            let mut paint = sk::Paint::default();
            paint.set_alpha_f(opacity);
            paint.set_blend_mode(blend);
            canvas.save_layer(
                &sk::canvas::SaveLayerRec::default()
                    .bounds(&visual)
                    .paint(&paint),
            );
        }
        if let Some(clip) = &node.clip {
            canvas.clip_rrect(convert::rounded_rect(&clip.rect, &clip.radii), None, true);
        }
        if node.mask.is_some() {
            // The mask cuts into this layer only, not into what is behind.
            canvas.save_layer(&sk::canvas::SaveLayerRec::default().bounds(&visual));
        }
        if let Some(filter) = filter {
            // The filter's layer holds the content; the shadow and blur
            // it adds may reach past it.
            let content = geometry
                .content(node, Precision::Quick)
                .map(|content| layer_bounds(&content));
            let mut paint = sk::Paint::default();
            paint.set_image_filter(filter);
            let mut layer = sk::canvas::SaveLayerRec::default().paint(&paint);
            if let Some(content) = &content {
                layer = layer.bounds(content);
            }
            canvas.save_layer(&layer);
            self.content(node, 1.0);
            canvas.restore();
        } else {
            self.content(node, 1.0);
        }
        if let Some(mask) = &node.mask {
            // What the mask draws keeps what is under it in proportion to
            // its alpha; the rest goes.
            let mut paint = sk::Paint::default();
            paint.set_blend_mode(sk::BlendMode::DstIn);
            canvas.save_layer(
                &sk::canvas::SaveLayerRec::default()
                    .bounds(&visual)
                    .paint(&paint),
            );
            self.node(mask);
            canvas.restore();
            canvas.restore();
        }
        if composite {
            canvas.restore();
        }
    }

    /// Draws what a node holds, in the node's own coordinates, faded by
    /// `opacity`.
    fn content(&mut self, node: &SceneNode, opacity: f32) {
        match &node.content {
            Content::Group { children } => {
                for child in children {
                    self.node(child);
                }
            }
            Content::Rect { fill, stroke, .. }
            | Content::Ellipse { fill, stroke, .. }
            | Content::Star { fill, stroke, .. }
            | Content::Path { fill, stroke, .. } => {
                if let Some(outline) = geometry::outline(node) {
                    self.fill(&outline.path, fill.as_ref(), opacity);
                    if let Some(stroke) = stroke {
                        stroke::draw(self.canvas, &outline, stroke, opacity);
                    }
                }
            }
            Content::Text(content) => self.text(node, content, opacity),
            Content::Image {
                source,
                fit,
                radii,
                stroke,
            } => {
                let picture = match self.assets.load(source) {
                    Ok(picture) => Some(picture),
                    Err(reason) => {
                        self.warn(RenderWarning::MissingImage {
                            node: node.id.clone(),
                            source: assets::shorten(source),
                            reason,
                        });
                        None
                    }
                };
                let frame = image::ImageBox {
                    size: (node.width, node.height),
                    radii,
                    fit: *fit,
                    stroke: stroke.as_ref(),
                    mipmaps: true,
                };
                image::draw(self.canvas, &frame, picture.as_ref(), opacity);
            }
            Content::Video {
                source,
                media_time_ms,
                fit,
                radii,
                stroke,
            } => {
                // The box's size on the output, so a provider can read
                // smaller frames for a small box.
                let matrix = self.canvas.local_to_device_as_3x3();
                let across = f64::from(matrix.scale_x().hypot(matrix.skew_y()));
                let down = f64::from(matrix.skew_x().hypot(matrix.scale_y()));
                let wanted = (node.width * across, node.height * down);
                let picture =
                    match self
                        .assets
                        .video_frame(source, *media_time_ms, wanted.0, wanted.1)
                    {
                        Ok(picture) => Some(picture),
                        Err(reason) => {
                            self.warn(RenderWarning::MissingVideo {
                                node: node.id.clone(),
                                source: assets::shorten(source),
                                reason,
                            });
                            None
                        }
                    };
                let frame = image::ImageBox {
                    size: (node.width, node.height),
                    radii,
                    fit: *fit,
                    stroke: stroke.as_ref(),
                    mipmaps: false,
                };
                image::draw(self.canvas, &frame, picture.as_ref(), opacity);
            }
        }
    }

    fn text(&mut self, node: &SceneNode, content: &TextContent, opacity: f32) {
        let family = &content.style.font.family;
        if !family.trim().is_empty() && !self.fonts.has_family(family) {
            self.warn(RenderWarning::MissingFont {
                node: node.id.clone(),
                family: family.clone(),
            });
        }
        let block = self
            .fonts
            .layout(&content.text, &content.style, content.wrap_width);
        text::draw(self.canvas, content, &block, opacity);
    }

    /// Notes a warning once per frame.
    fn warn(&mut self, warning: RenderWarning) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn fill(&self, path: &sk::Path, fill: Option<&ScenePaint>, opacity: f32) {
        if let Some(paint) = fill.and_then(|fill| paint::fill(fill, opacity)) {
            self.canvas.draw_path(path, &paint);
        }
    }
}

/// Bounds for an offscreen layer around `rect`, with a pixel to spare for
/// anti-aliased edges.
fn layer_bounds(rect: &Rect) -> sk::Rect {
    convert::rect(rect).with_outset((1.0, 1.0))
}

/// Whether a node draws at most one thing, so that fading that one thing
/// is the same as fading the node.
fn is_flat(node: &SceneNode) -> bool {
    match &node.content {
        Content::Group { children } => children.is_empty(),
        Content::Rect { fill, stroke, .. }
        | Content::Ellipse { fill, stroke, .. }
        | Content::Star { fill, stroke, .. }
        | Content::Path { fill, stroke, .. } => fill.is_none() || stroke.is_none(),
        Content::Text(_) | Content::Image { .. } | Content::Video { .. } => false,
    }
}

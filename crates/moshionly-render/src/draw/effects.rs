//! A node's effects as Skia objects: its drop shadow and blur as one image
//! filter, and its blend mode.
//!
//! Blur amounts in the scene are twice the standard deviation of the
//! Gaussian, so Skia's sigma is half of them. Everything here is in the
//! node's own coordinates; Skia applies filters under the canvas's
//! transform, so a shadow turns and scales with its node.

use moshionly_core::model::{BlendMode, Shadow};
use moshionly_core::scene::SceneNode;
use skia_safe as sk;

use crate::convert;

/// Blurs below this many pixels cannot be seen and are skipped.
const MIN_BLUR: f64 = 0.01;

/// The Skia blend mode for a scene blend mode.
pub(crate) fn blend_mode(mode: BlendMode) -> sk::BlendMode {
    match mode {
        BlendMode::Normal => sk::BlendMode::SrcOver,
        BlendMode::Multiply => sk::BlendMode::Multiply,
        BlendMode::Screen => sk::BlendMode::Screen,
        BlendMode::Overlay => sk::BlendMode::Overlay,
        BlendMode::Darken => sk::BlendMode::Darken,
        BlendMode::Lighten => sk::BlendMode::Lighten,
        BlendMode::ColorDodge => sk::BlendMode::ColorDodge,
        BlendMode::ColorBurn => sk::BlendMode::ColorBurn,
        BlendMode::HardLight => sk::BlendMode::HardLight,
        BlendMode::SoftLight => sk::BlendMode::SoftLight,
        BlendMode::Difference => sk::BlendMode::Difference,
        BlendMode::Exclusion => sk::BlendMode::Exclusion,
        BlendMode::Hue => sk::BlendMode::Hue,
        BlendMode::Saturation => sk::BlendMode::Saturation,
        BlendMode::Color => sk::BlendMode::Color,
        BlendMode::Luminosity => sk::BlendMode::Luminosity,
    }
}

/// Skia's sigma for a scene blur amount, or `None` for no visible blur.
pub(crate) fn sigma(blur: f64) -> Option<f32> {
    (blur.is_finite() && blur > MIN_BLUR).then(|| (blur / 2.0) as f32)
}

/// The filter that adds `node`'s shadow behind what it draws and then
/// blurs the result, or `None` when the node has neither.
pub(crate) fn filter(node: &SceneNode) -> Option<sk::ImageFilter> {
    let shadowed = node.shadow.as_ref().and_then(shadow);
    match sigma(node.blur) {
        Some(sigma) => sk::image_filters::blur((sigma, sigma), None, shadowed, None),
        None => shadowed,
    }
}

/// A filter that draws its input over its shadow, or `None` for a shadow
/// that cannot be seen.
pub(crate) fn shadow(shadow: &Shadow) -> Option<sk::ImageFilter> {
    if shadow.color.a == 0 || !(shadow.x.is_finite() && shadow.y.is_finite()) {
        return None;
    }
    let sigma = sigma(shadow.blur).unwrap_or(0.0);
    sk::image_filters::drop_shadow(
        (shadow.x as f32, shadow.y as f32),
        (sigma, sigma),
        convert::color(shadow.color),
        None,
        None,
        None,
    )
}

/// How far `node`'s shadow and blur reach past what it draws, as a margin
/// on each side: left, top, right, bottom. Three standard deviations hold
/// all of a Gaussian that can be seen in 8 bits.
pub(crate) fn reach(node: &SceneNode) -> [f64; 4] {
    let spread = |blur: f64| sigma(blur).map_or(0.0, |sigma| 3.0 * f64::from(sigma));
    let mut margin = [0.0; 4];
    if let Some(shadow) = node.shadow.as_ref().filter(|shadow| shadow.color.a > 0) {
        let offset = |value: f64| if value.is_finite() { value } else { 0.0 };
        let (x, y, blur) = (offset(shadow.x), offset(shadow.y), spread(shadow.blur));
        margin = [
            (blur - x).max(0.0),
            (blur - y).max(0.0),
            (blur + x).max(0.0),
            (blur + y).max(0.0),
        ];
    }
    let blur = spread(node.blur);
    margin.map(|side| side + blur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::scene::{Content, ScenePaint};
    use moshionly_core::{Affine, Color, NodeId};

    fn node(blur: f64, shadow: Option<Shadow>) -> SceneNode {
        SceneNode {
            id: NodeId::from("n"),
            transform: Affine::IDENTITY,
            width: 10.0,
            height: 10.0,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            blur,
            shadow,
            clip: None,
            mask: None,
            content: Content::Rect {
                radii: [0.0; 4],
                fill: Some(ScenePaint::solid(Color::BLACK)),
                stroke: None,
            },
        }
    }

    #[test]
    fn every_blend_mode_has_its_own_skia_mode() {
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ];
        let mut skia: Vec<sk::BlendMode> = modes.iter().map(|mode| blend_mode(*mode)).collect();
        skia.sort_by_key(|mode| *mode as i32);
        skia.dedup();
        assert_eq!(skia.len(), modes.len());
    }

    #[test]
    fn blur_amounts_are_twice_sigma() {
        assert_eq!(sigma(8.0), Some(4.0));
        assert_eq!(sigma(0.0), None);
        assert_eq!(sigma(-3.0), None);
        assert_eq!(sigma(f64::NAN), None);
    }

    #[test]
    fn filters_exist_only_for_visible_effects() {
        assert!(filter(&node(0.0, None)).is_none());
        assert!(filter(&node(4.0, None)).is_some());
        assert!(filter(&node(0.0, Some(Shadow::default()))).is_some());
        let invisible = Shadow {
            color: Color::TRANSPARENT,
            ..Shadow::default()
        };
        assert!(filter(&node(0.0, Some(invisible))).is_none());
    }

    #[test]
    fn the_reach_covers_the_offset_and_the_blur() {
        let shadow = Shadow {
            color: Color::BLACK,
            x: 5.0,
            y: -2.0,
            blur: 4.0,
        };
        // Sigma 2, so the shadow spreads 6 around its offset copy.
        assert_eq!(reach(&node(0.0, Some(shadow))), [1.0, 8.0, 11.0, 4.0]);
        // A blur of 2 (sigma 1) adds 3 on every side.
        assert_eq!(reach(&node(2.0, Some(shadow))), [4.0, 11.0, 14.0, 7.0]);
        assert_eq!(reach(&node(0.0, None)), [0.0; 4]);
    }
}

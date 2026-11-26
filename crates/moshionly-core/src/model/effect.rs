//! Effects a layer can have besides its drop shadow and blur: an outer glow,
//! film grain and a directional blur.
//!
//! Each is an optional value on every layer except an artboard, with its own
//! animation type that changes its fields. Lengths are in the layer's own
//! pixels, so effects turn and scale with the layer. The order in which they
//! are drawn is in [`crate::scene`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::defaults;

/// An outer glow: a soft halo of colour around what a layer draws, behind it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Glow {
    /// Glow colour; its alpha is the glow's strength. Default `"#0D99FFCC"`.
    #[serde(
        default = "defaults::glow_color",
        skip_serializing_if = "defaults::is_glow_color"
    )]
    pub color: Color,
    /// Softness in pixels, 0 or more: like `blur`, twice the standard
    /// deviation of the Gaussian. Default 20.
    #[serde(
        default = "defaults::glow_radius",
        skip_serializing_if = "defaults::is_glow_radius"
    )]
    #[schemars(range(min = 0.0))]
    pub radius: f64,
    /// How far the shape grows on every side before it is softened, in
    /// pixels, 0 or more. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    #[schemars(range(min = 0.0))]
    pub spread: f64,
}

impl Default for Glow {
    fn default() -> Self {
        Self {
            color: defaults::glow_color(),
            radius: defaults::glow_radius(),
            spread: 0.0,
        }
    }
}

/// Film grain over what a layer draws. It stays inside the layer's shape:
/// the grain lightens and darkens the layer's pixels and keeps their alpha.
///
/// The grain is the same every time the same frame is drawn: it depends only
/// on the layer's id and, when it moves, the time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Noise {
    /// How strong the grain is, from 0 (none) to 1. Default 0.2.
    #[serde(
        default = "defaults::noise_amount",
        skip_serializing_if = "defaults::is_noise_amount"
    )]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub amount: f64,
    /// Size of one grain in pixels, more than 0. Default 1.
    #[serde(default = "defaults::one", skip_serializing_if = "defaults::is_one")]
    #[schemars(range(min = 0.0))]
    pub size: f64,
    /// Whether the grain changes on every frame; when false it stays fixed.
    /// Default true.
    #[serde(default = "defaults::yes", skip_serializing_if = "defaults::is_yes")]
    pub animated: bool,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            amount: defaults::noise_amount(),
            size: defaults::one(),
            animated: true,
        }
    }
}

/// A blur along one direction only, like a fast movement smears a photo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirectionalBlur {
    /// Blur along the direction in pixels, 0 or more: like `blur`, twice the
    /// standard deviation of the Gaussian. Default 20.
    #[serde(
        default = "defaults::directional_blur_amount",
        skip_serializing_if = "defaults::is_directional_blur_amount"
    )]
    #[schemars(range(min = 0.0))]
    pub amount: f64,
    /// Direction of the smear in degrees, clockwise: 0 smears left and
    /// right, 90 up and down. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub angle: f64,
}

impl Default for DirectionalBlur {
    fn default() -> Self {
        Self {
            amount: defaults::directional_blur_amount(),
            angle: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_written_as_empty_objects() {
        assert_eq!(serde_json::to_value(Glow::default()).unwrap(), json!({}));
        assert_eq!(serde_json::to_value(Noise::default()).unwrap(), json!({}));
        assert_eq!(
            serde_json::to_value(DirectionalBlur::default()).unwrap(),
            json!({})
        );
        let glow: Glow = serde_json::from_value(json!({})).unwrap();
        assert_eq!(glow, Glow::default());
        assert_eq!(glow.color, Color::rgba(0x0D, 0x99, 0xFF, 0xCC));
        assert_eq!((glow.radius, glow.spread), (20.0, 0.0));
        let noise: Noise = serde_json::from_value(json!({})).unwrap();
        assert_eq!((noise.amount, noise.size, noise.animated), (0.2, 1.0, true));
        let blur: DirectionalBlur = serde_json::from_value(json!({})).unwrap();
        assert_eq!((blur.amount, blur.angle), (20.0, 0.0));
    }

    #[test]
    fn fields_round_trip_and_unknown_ones_are_refused() {
        let noise = Noise {
            amount: 0.5,
            size: 3.0,
            animated: false,
        };
        let value = serde_json::to_value(noise).unwrap();
        assert_eq!(
            value,
            json!({ "amount": 0.5, "size": 3.0, "animated": false })
        );
        assert_eq!(serde_json::from_value::<Noise>(value).unwrap(), noise);
        assert!(serde_json::from_value::<Glow>(json!({ "blur": 4 })).is_err());
        assert!(serde_json::from_value::<DirectionalBlur>(json!({ "distance": 4 })).is_err());
    }
}

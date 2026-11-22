//! Paints, strokes, shadows, corner radii and blend modes.

use std::fmt;

use schemars::JsonSchema;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::color::Color;
use crate::defaults;

/// What an area or an outline is painted with: one colour, or a gradient.
///
/// In files a paint is either a colour string (`"#0D99FF"`) or a gradient
/// object (`{ "type": "linear_gradient", "stops": [...] }`).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Paint {
    /// One colour.
    Solid(Color),
    /// A linear or radial gradient.
    Gradient(Gradient),
}

impl Paint {
    /// A solid paint.
    pub const fn solid(color: Color) -> Self {
        Paint::Solid(color)
    }

    /// The colour, when the paint is solid.
    pub fn as_solid(&self) -> Option<Color> {
        match self {
            Paint::Solid(color) => Some(*color),
            Paint::Gradient(_) => None,
        }
    }
}

impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Paint::Solid(color)
    }
}

impl From<Gradient> for Paint {
    fn from(gradient: Gradient) -> Self {
        Paint::Gradient(gradient)
    }
}

impl<'de> Deserialize<'de> for Paint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PaintVisitor;

        impl<'de> Visitor<'de> for PaintVisitor {
            type Value = Paint;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a paint: a colour string like \"#0D99FF\", or a gradient object with \
                     \"type\": \"linear_gradient\" or \"radial_gradient\"",
                )
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Paint, E> {
                Color::parse(text).map(Paint::Solid).map_err(E::custom)
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Paint, A::Error> {
                Gradient::deserialize(de::value::MapAccessDeserializer::new(map))
                    .map(Paint::Gradient)
            }
        }

        deserializer.deserialize_any(PaintVisitor)
    }
}

/// A gradient. Its geometry is given in the unit square of the layer's box:
/// `[0, 0]` is the top-left corner of the box and `[1, 1]` the bottom-right.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Gradient {
    /// Colours change along a line.
    LinearGradient(LinearGradient),
    /// Colours change outward from a centre.
    RadialGradient(RadialGradient),
}

impl Gradient {
    /// The colour stops, in order.
    pub fn stops(&self) -> &[GradientStop] {
        match self {
            Gradient::LinearGradient(g) => &g.stops,
            Gradient::RadialGradient(g) => &g.stops,
        }
    }
}

/// A gradient along the line from `start` to `end`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearGradient {
    /// Where offset 0 sits, as `[x, y]` in the unit square of the box.
    /// Default `[0.5, 0]`, the middle of the top edge.
    #[serde(
        default = "defaults::gradient_start",
        skip_serializing_if = "defaults::is_gradient_start"
    )]
    pub start: [f64; 2],
    /// Where offset 1 sits, as `[x, y]` in the unit square of the box.
    /// Default `[0.5, 1]`, the middle of the bottom edge.
    #[serde(
        default = "defaults::gradient_end",
        skip_serializing_if = "defaults::is_gradient_end"
    )]
    pub end: [f64; 2],
    /// Colour stops, at least two, with offsets that do not decrease.
    pub stops: Vec<GradientStop>,
}

/// A gradient that spreads from `center` to an ellipse of `radius`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RadialGradient {
    /// Where offset 0 sits, as `[x, y]` in the unit square of the box.
    /// Default `[0.5, 0.5]`, the centre.
    #[serde(
        default = "defaults::half_half",
        skip_serializing_if = "defaults::is_half_half"
    )]
    pub center: [f64; 2],
    /// Horizontal and vertical radius where offset 1 sits, as fractions of the
    /// box width and height. Default `[0.5, 0.5]`, an ellipse touching the box.
    #[serde(
        default = "defaults::half_half",
        skip_serializing_if = "defaults::is_half_half"
    )]
    pub radius: [f64; 2],
    /// Rotation of the ellipse about its centre in degrees, clockwise. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub rotation: f64,
    /// Colour stops, at least two, with offsets that do not decrease.
    pub stops: Vec<GradientStop>,
}

/// One colour at one position of a gradient.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    /// Position along the gradient, 0 to 1.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub offset: f64,
    /// Colour at that position.
    pub color: Color,
}

impl GradientStop {
    /// Creates a stop.
    pub const fn new(offset: f64, color: Color) -> Self {
        Self { offset, color }
    }
}

/// The outline of a layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    /// Colour or gradient of the outline. Default `"#000000"`.
    #[serde(
        default = "defaults::stroke_paint",
        skip_serializing_if = "defaults::is_stroke_paint"
    )]
    pub paint: Paint,
    /// Thickness in pixels, 0 or more. Default 1.
    #[serde(
        default = "defaults::stroke_width",
        skip_serializing_if = "defaults::is_stroke_width"
    )]
    #[schemars(range(min = 0.0))]
    pub width: f64,
    /// Whether the outline sits inside, on, or outside the edge. Default `center`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub position: StrokePosition,
    /// Shape of the ends of open paths. Default `butt`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub cap: StrokeCap,
    /// Shape of corners. Default `miter`.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub join: StrokeJoin,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            paint: defaults::stroke_paint(),
            width: defaults::stroke_width(),
            position: StrokePosition::default(),
            cap: StrokeCap::default(),
            join: StrokeJoin::default(),
        }
    }
}

impl Stroke {
    /// A centred stroke of one colour.
    pub fn solid(color: Color, width: f64) -> Self {
        Self {
            paint: Paint::Solid(color),
            width,
            ..Self::default()
        }
    }
}

/// Where a stroke sits relative to the edge of the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrokePosition {
    /// Entirely inside the shape.
    Inside,
    /// Half inside, half outside.
    #[default]
    Center,
    /// Entirely outside the shape.
    Outside,
}

/// The shape of the ends of an open stroked path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrokeCap {
    /// Cut off flat at the end point.
    #[default]
    Butt,
    /// A half circle beyond the end point.
    Round,
    /// A half square beyond the end point.
    Square,
}

/// The shape of a stroke's corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrokeJoin {
    /// A sharp corner.
    #[default]
    Miter,
    /// A rounded corner.
    Round,
    /// A corner cut off flat.
    Bevel,
}

/// A drop shadow behind a layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shadow {
    /// Shadow colour; its alpha is the shadow's strength. Default `"#00000040"`.
    #[serde(
        default = "defaults::shadow_color",
        skip_serializing_if = "defaults::is_shadow_color"
    )]
    pub color: Color,
    /// Horizontal offset in pixels, positive to the right. Default 0.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub x: f64,
    /// Vertical offset in pixels, positive downward. Default 4.
    #[serde(
        default = "defaults::shadow_y",
        skip_serializing_if = "defaults::is_shadow_y"
    )]
    pub y: f64,
    /// Blur in pixels, 0 or more: twice the standard deviation of the
    /// Gaussian. Default 12.
    #[serde(
        default = "defaults::shadow_blur",
        skip_serializing_if = "defaults::is_shadow_blur"
    )]
    #[schemars(range(min = 0.0))]
    pub blur: f64,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            color: defaults::shadow_color(),
            x: 0.0,
            y: defaults::shadow_y(),
            blur: defaults::shadow_blur(),
        }
    }
}

/// Corner rounding: one radius for every corner, or one per corner.
///
/// In files this is a number, or an array of four numbers in the order
/// top-left, top-right, bottom-right, bottom-left. Radii are in pixels and are
/// limited to half the shorter side when drawn.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum CornerRadius {
    /// The same radius on all four corners.
    Uniform(f64),
    /// Top-left, top-right, bottom-right, bottom-left.
    PerCorner([f64; 4]),
}

impl Default for CornerRadius {
    fn default() -> Self {
        CornerRadius::Uniform(0.0)
    }
}

impl CornerRadius {
    /// The four radii: top-left, top-right, bottom-right, bottom-left.
    pub fn radii(&self) -> [f64; 4] {
        match *self {
            CornerRadius::Uniform(r) => [r; 4],
            CornerRadius::PerCorner(radii) => radii,
        }
    }

    /// Builds the shortest form: `Uniform` when all four radii are equal.
    pub fn from_radii(radii: [f64; 4]) -> Self {
        if radii.iter().all(|r| *r == radii[0]) {
            CornerRadius::Uniform(radii[0])
        } else {
            CornerRadius::PerCorner(radii)
        }
    }

    /// Whether no corner is rounded.
    pub fn is_zero(&self) -> bool {
        self.radii().iter().all(|r| *r == 0.0)
    }
}

impl From<f64> for CornerRadius {
    fn from(radius: f64) -> Self {
        CornerRadius::Uniform(radius)
    }
}

impl<'de> Deserialize<'de> for CornerRadius {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RadiusVisitor;

        impl<'de> Visitor<'de> for RadiusVisitor {
            type Value = CornerRadius;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a corner radius: one number, or four numbers \
                     [top_left, top_right, bottom_right, bottom_left]",
                )
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<CornerRadius, E> {
                Ok(CornerRadius::Uniform(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<CornerRadius, E> {
                Ok(CornerRadius::Uniform(value as f64))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<CornerRadius, E> {
                Ok(CornerRadius::Uniform(value as f64))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<CornerRadius, A::Error> {
                let mut radii = [0.0; 4];
                for (i, slot) in radii.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(i, &self))?;
                }
                if seq.next_element::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(5, &self));
                }
                Ok(CornerRadius::PerCorner(radii))
            }
        }

        deserializer.deserialize_any(RadiusVisitor)
    }
}

/// How a layer's pixels combine with what is behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Draw over what is behind. The default.
    #[default]
    Normal,
    /// Multiply colours; always darker.
    Multiply,
    /// Inverse multiply; always lighter.
    Screen,
    /// Multiply dark areas, screen light ones.
    Overlay,
    /// Keep the darker of the two colours.
    Darken,
    /// Keep the lighter of the two colours.
    Lighten,
    /// Brighten what is behind to reflect the layer.
    ColorDodge,
    /// Darken what is behind to reflect the layer.
    ColorBurn,
    /// Like overlay, driven by the layer instead of the backdrop.
    HardLight,
    /// A gentler hard light.
    SoftLight,
    /// Absolute difference of the two colours.
    Difference,
    /// Like difference with lower contrast.
    Exclusion,
    /// Hue of the layer, saturation and luminosity of the backdrop.
    Hue,
    /// Saturation of the layer, hue and luminosity of the backdrop.
    Saturation,
    /// Hue and saturation of the layer, luminosity of the backdrop.
    Color,
    /// Luminosity of the layer, hue and saturation of the backdrop.
    Luminosity,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn solid_paint_is_a_colour_string() {
        let paint: Paint = serde_json::from_value(json!("#0d99ff")).unwrap();
        assert_eq!(paint, Paint::Solid(Color::rgb(0x0D, 0x99, 0xFF)));
        assert_eq!(serde_json::to_value(&paint).unwrap(), json!("#0D99FF"));
    }

    #[test]
    fn gradient_paint_round_trips_and_omits_defaults() {
        let source = json!({
            "type": "linear_gradient",
            "stops": [
                { "offset": 0, "color": "#FF0000" },
                { "offset": 1, "color": "#0000FF80" }
            ]
        });
        let paint: Paint = serde_json::from_value(source).unwrap();
        let Paint::Gradient(Gradient::LinearGradient(linear)) = &paint else {
            panic!("expected a linear gradient, got {paint:?}");
        };
        assert_eq!(linear.start, [0.5, 0.0]);
        assert_eq!(linear.end, [0.5, 1.0]);
        let written = serde_json::to_value(&paint).unwrap();
        assert_eq!(written["type"], "linear_gradient");
        assert!(written.get("start").is_none(), "defaults are not written");
        assert_eq!(written["stops"][1]["color"], "#0000FF80");
    }

    #[test]
    fn radial_gradient_keeps_non_default_geometry() {
        let paint: Paint = serde_json::from_value(json!({
            "type": "radial_gradient",
            "center": [0.25, 0.75],
            "rotation": 30,
            "stops": [
                { "offset": 0, "color": "#FFFFFF" },
                { "offset": 1, "color": "#000000" }
            ]
        }))
        .unwrap();
        let written = serde_json::to_value(&paint).unwrap();
        assert_eq!(written["center"], json!([0.25, 0.75]));
        assert_eq!(written["rotation"], json!(30.0));
        assert!(written.get("radius").is_none());
    }

    #[test]
    fn paint_errors_say_what_a_paint_is() {
        let not_a_colour = serde_json::from_value::<Paint>(json!("blue")).unwrap_err();
        assert!(
            not_a_colour.to_string().contains("#RRGGBB"),
            "{not_a_colour}"
        );

        let wrong_shape = serde_json::from_value::<Paint>(json!(12)).unwrap_err();
        assert!(wrong_shape.to_string().contains("a paint"), "{wrong_shape}");

        let unknown_gradient =
            serde_json::from_value::<Paint>(json!({ "type": "conic", "stops": [] })).unwrap_err();
        assert!(
            unknown_gradient.to_string().contains("linear_gradient"),
            "{unknown_gradient}"
        );

        let unknown_field = serde_json::from_value::<Paint>(json!({
            "type": "linear_gradient", "angle": 90, "stops": []
        }))
        .unwrap_err();
        assert!(
            unknown_field.to_string().contains("angle"),
            "{unknown_field}"
        );
    }

    #[test]
    fn stroke_defaults_are_omitted() {
        let stroke: Stroke = serde_json::from_value(json!({})).unwrap();
        assert_eq!(stroke, Stroke::default());
        assert_eq!(serde_json::to_value(&stroke).unwrap(), json!({}));

        let custom: Stroke = serde_json::from_value(json!({
            "paint": "#FF0000", "width": 4, "position": "inside", "cap": "round", "join": "bevel"
        }))
        .unwrap();
        assert_eq!(custom.position, StrokePosition::Inside);
        assert_eq!(custom.cap, StrokeCap::Round);
        assert_eq!(custom.join, StrokeJoin::Bevel);
        assert_eq!(serde_json::to_value(&custom).unwrap()["width"], json!(4.0));
    }

    #[test]
    fn shadow_defaults() {
        let shadow: Shadow = serde_json::from_value(json!({})).unwrap();
        assert_eq!(shadow.color, Color::rgba(0, 0, 0, 0x40));
        assert_eq!((shadow.x, shadow.y, shadow.blur), (0.0, 4.0, 12.0));
        assert!(serde_json::from_value::<Shadow>(json!({ "spread": 2 })).is_err());
    }

    #[test]
    fn corner_radius_takes_one_number_or_four() {
        let uniform: CornerRadius = serde_json::from_value(json!(32)).unwrap();
        assert_eq!(uniform, CornerRadius::Uniform(32.0));
        assert_eq!(uniform.radii(), [32.0; 4]);

        let each: CornerRadius = serde_json::from_value(json!([1, 2.5, 3, 4])).unwrap();
        assert_eq!(each, CornerRadius::PerCorner([1.0, 2.5, 3.0, 4.0]));
        assert_eq!(
            serde_json::to_value(each).unwrap(),
            json!([1.0, 2.5, 3.0, 4.0])
        );

        for bad in [
            json!([1, 2, 3]),
            json!([1, 2, 3, 4, 5]),
            json!("round"),
            json!(null),
        ] {
            let error = serde_json::from_value::<CornerRadius>(bad).unwrap_err();
            assert!(error.to_string().contains("corner radius"), "{error}");
        }
    }

    #[test]
    fn corner_radius_collapses_equal_corners() {
        assert_eq!(
            CornerRadius::from_radii([8.0; 4]),
            CornerRadius::Uniform(8.0)
        );
        assert_eq!(
            CornerRadius::from_radii([8.0, 0.0, 8.0, 0.0]),
            CornerRadius::PerCorner([8.0, 0.0, 8.0, 0.0])
        );
        assert!(CornerRadius::default().is_zero());
    }

    #[test]
    fn blend_modes_use_snake_case() {
        assert_eq!(
            serde_json::to_value(BlendMode::ColorDodge).unwrap(),
            json!("color_dodge")
        );
        let mode: BlendMode = serde_json::from_value(json!("soft_light")).unwrap();
        assert_eq!(mode, BlendMode::SoftLight);
    }
}

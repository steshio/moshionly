//! Colours.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// An sRGB colour with straight (not premultiplied) alpha, 8 bits per channel.
///
/// In files a colour is the string `#RRGGBB` or `#RRGGBBAA`. Parsing accepts
/// either case; printing uses upper case and leaves the alpha out when the
/// colour is opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red, 0 to 255.
    pub r: u8,
    /// Green, 0 to 255.
    pub g: u8,
    /// Blue, 0 to 255.
    pub b: u8,
    /// Alpha, 0 (transparent) to 255 (opaque).
    pub a: u8,
}

/// Why a string is not a colour.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "\"{0}\" is not a colour; write it as \"#RRGGBB\" or \"#RRGGBBAA\" with hexadecimal digits, for example \"#0D99FF\""
)]
pub struct ColorParseError(pub String);

impl Color {
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A colour with alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// The same colour with another alpha.
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Alpha as a fraction from 0 to 1.
    pub fn alpha(self) -> f64 {
        f64::from(self.a) / 255.0
    }

    /// Parses `#RRGGBB` or `#RRGGBBAA`.
    pub fn parse(text: &str) -> Result<Self, ColorParseError> {
        let error = || ColorParseError(text.to_owned());
        let digits = text.strip_prefix('#').ok_or_else(error)?;
        if !(digits.len() == 6 || digits.len() == 8)
            || !digits.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(error());
        }
        let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| error());
        Ok(Self {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
            a: if digits.len() == 8 { channel(6)? } else { 255 },
        })
    }

    /// Prints `#RRGGBB`, or `#RRGGBBAA` when the colour is not opaque.
    pub fn to_hex(self) -> String {
        self.to_string()
    }

    /// Blends from `self` to `other`.
    ///
    /// `t` is 0 at `self` and 1 at `other`. It may leave that range (an
    /// overshooting easing does that); channels are clamped afterwards.
    ///
    /// The blend happens on premultiplied sRGB values. That keeps a fade from a
    /// transparent colour clean: `#FF000000` to `#0000FF` never passes through
    /// a muddy half-transparent mix of both hues.
    pub fn lerp(self, other: Color, t: f64) -> Color {
        let premultiplied = |c: Color| {
            let a = c.alpha();
            [
                f64::from(c.r) * a,
                f64::from(c.g) * a,
                f64::from(c.b) * a,
                a,
            ]
        };
        let from = premultiplied(self);
        let to = premultiplied(other);
        let mix = |i: usize| from[i] + (to[i] - from[i]) * t;
        let alpha = mix(3).clamp(0.0, 1.0);
        if alpha <= f64::EPSILON {
            // Keep the hue of whichever end we are closer to, so a later blend
            // starting from this value does not begin at black.
            let hue = if t < 0.5 { self } else { other };
            return hue.with_alpha(0);
        }
        let channel = |i: usize| (mix(i) / alpha).round().clamp(0.0, 255.0) as u8;
        Color {
            r: channel(0),
            g: channel(1),
            b: channel(2),
            a: (alpha * 255.0).round() as u8,
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02X}", self.a)?;
        }
        Ok(())
    }
}

impl FromStr for Color {
    type Err = ColorParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Color::parse(&text).map_err(de::Error::custom)
    }
}

impl JsonSchema for Color {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A colour as \"#RRGGBB\" or \"#RRGGBBAA\".",
            "type": "string",
            "pattern": "^#([0-9a-fA-F]{6}|[0-9a-fA-F]{8})$"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_both_forms() {
        let blue = Color::parse("#0D99FF").unwrap();
        assert_eq!(blue, Color::rgb(0x0D, 0x99, 0xFF));
        assert_eq!(blue.to_hex(), "#0D99FF");

        let faded = Color::parse("#0d99ff80").unwrap();
        assert_eq!(faded, Color::rgba(0x0D, 0x99, 0xFF, 0x80));
        assert_eq!(faded.to_hex(), "#0D99FF80");

        // An explicit opaque alpha is dropped when printing.
        assert_eq!(Color::parse("#112233FF").unwrap().to_hex(), "#112233");
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "#",
            "0D99FF",
            "#0D99F",
            "#0D99FF8",
            "#GG0000",
            "red",
            "#0D99FF800",
        ] {
            let error = Color::parse(bad).unwrap_err();
            assert!(error.to_string().contains("#RRGGBB"), "{error}");
        }
        // Multi-byte input must not panic on a slice boundary.
        assert!(Color::parse("#é9900").is_err());
    }

    #[test]
    fn serde_round_trip() {
        let color: Color = serde_json::from_str("\"#ff000080\"").unwrap();
        assert_eq!(serde_json::to_string(&color).unwrap(), "\"#FF000080\"");
        let error = serde_json::from_str::<Color>("\"blue\"").unwrap_err();
        assert!(error.to_string().contains("not a colour"));
    }

    #[test]
    fn lerp_hits_both_ends_and_the_middle() {
        let a = Color::rgb(0, 0, 0);
        let b = Color::rgb(255, 100, 50);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        assert_eq!(a.lerp(b, 0.5), Color::rgb(128, 50, 25));
    }

    #[test]
    fn lerp_clamps_when_the_easing_overshoots() {
        let a = Color::rgb(10, 10, 10);
        let b = Color::rgb(250, 250, 250);
        assert_eq!(a.lerp(b, 1.5), Color::rgb(255, 255, 255));
        assert_eq!(a.lerp(b, -0.5), Color::rgb(0, 0, 0));
    }

    #[test]
    fn fading_in_from_transparent_keeps_the_hue() {
        let clear_red = Color::rgba(255, 0, 0, 0);
        let blue = Color::rgb(0, 0, 255);
        let quarter = clear_red.lerp(blue, 0.25);
        assert_eq!((quarter.r, quarter.g, quarter.b), (0, 0, 255));
        assert_eq!(quarter.a, 64);
        // At zero alpha the nearer end's hue is kept.
        assert_eq!(
            clear_red.lerp(Color::rgba(0, 0, 255, 0), 0.9),
            Color::rgba(0, 0, 255, 0)
        );
    }
}

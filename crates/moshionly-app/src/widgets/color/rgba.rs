//! A plain 8-bit colour with straight (not premultiplied) alpha, and its hex
//! notation.

use egui::Color32;
use thiserror::Error;

/// Why a hex colour could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ColorParseError {
    /// The text has the wrong number of digits.
    #[error("a colour needs 3, 4, 6 or 8 hex digits, found {0}")]
    WrongLength(usize),
    /// The text contains something that is not a hex digit.
    #[error("\"{0}\" is not a hex digit")]
    NotHex(char),
}

/// A colour: red, green, blue and alpha, each 0 to 255, alpha not
/// premultiplied. This is the form colours have in a document
/// (`#RRGGBB` or `#RRGGBBAA`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha: 0 is transparent, 255 opaque.
    pub a: u8,
}

impl Rgba {
    /// Opaque black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A colour with alpha.
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Reads `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`. The `#` is optional
    /// and case does not matter.
    pub fn from_hex(text: &str) -> Result<Self, ColorParseError> {
        let text = text.trim();
        let digits: Vec<char> = text.strip_prefix('#').unwrap_or(text).chars().collect();
        let mut values = Vec::with_capacity(digits.len());
        for digit in &digits {
            values.push(digit.to_digit(16).ok_or(ColorParseError::NotHex(*digit))? as u8);
        }
        let pair = |i: usize| values[i] * 16 + values[i + 1];
        let double = |i: usize| values[i] * 17;
        match values.len() {
            3 => Ok(Self::rgb(double(0), double(1), double(2))),
            4 => Ok(Self::new(double(0), double(1), double(2), double(3))),
            6 => Ok(Self::rgb(pair(0), pair(2), pair(4))),
            8 => Ok(Self::new(pair(0), pair(2), pair(4), pair(6))),
            n => Err(ColorParseError::WrongLength(n)),
        }
    }

    /// `#RRGGBB`, or `#RRGGBBAA` when the colour is not opaque.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{}", self.hex_rgb())
        } else {
            format!("#{}{:02X}", self.hex_rgb(), self.a)
        }
    }

    /// The six colour digits without `#` and without alpha: `5B5FEF`.
    pub fn hex_rgb(self) -> String {
        format!("{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Opacity from 0 to 100.
    pub fn opacity(self) -> f64 {
        (f64::from(self.a) / 255.0 * 100.0).round()
    }

    /// The same colour with an opacity from 0 to 100.
    pub fn with_opacity(self, opacity: f64) -> Self {
        let a = (opacity.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8;
        Self { a, ..self }
    }

    /// The same colour, fully opaque.
    pub fn opaque(self) -> Self {
        Self { a: 255, ..self }
    }

    /// The colour as egui paints it.
    pub fn to_color32(self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.r, self.g, self.b, self.a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_hex_form() {
        assert_eq!(Rgba::from_hex("#5B5FEF"), Ok(Rgba::rgb(0x5B, 0x5F, 0xEF)));
        assert_eq!(Rgba::from_hex("5b5fef"), Ok(Rgba::rgb(0x5B, 0x5F, 0xEF)));
        assert_eq!(Rgba::from_hex(" #fff "), Ok(Rgba::WHITE));
        assert_eq!(Rgba::from_hex("#0008"), Ok(Rgba::new(0, 0, 0, 0x88)));
        assert_eq!(
            Rgba::from_hex("#11223344"),
            Ok(Rgba::new(0x11, 0x22, 0x33, 0x44))
        );
    }

    #[test]
    fn rejects_bad_input_without_panicking() {
        assert_eq!(Rgba::from_hex(""), Err(ColorParseError::WrongLength(0)));
        assert_eq!(Rgba::from_hex("#12"), Err(ColorParseError::WrongLength(2)));
        assert_eq!(
            Rgba::from_hex("#1234567"),
            Err(ColorParseError::WrongLength(7))
        );
        assert_eq!(Rgba::from_hex("#GGHHII"), Err(ColorParseError::NotHex('G')));
        assert_eq!(Rgba::from_hex("#ééé"), Err(ColorParseError::NotHex('é')));
    }

    #[test]
    fn writes_the_document_form() {
        assert_eq!(Rgba::rgb(0x0D, 0x99, 0xFF).to_hex(), "#0D99FF");
        assert_eq!(Rgba::new(0x0D, 0x99, 0xFF, 0x80).to_hex(), "#0D99FF80");
        assert_eq!(Rgba::new(1, 2, 3, 0).hex_rgb(), "010203");
        for hex in ["#000000", "#FFFFFF", "#12AB34", "#12AB3400", "#12AB34FE"] {
            assert_eq!(Rgba::from_hex(hex).unwrap().to_hex(), hex);
        }
    }

    #[test]
    fn opacity_round_trips() {
        assert_eq!(Rgba::WHITE.opacity(), 100.0);
        assert_eq!(Rgba::WHITE.with_opacity(50.0).a, 128);
        assert_eq!(Rgba::WHITE.with_opacity(50.0).opacity(), 50.0);
        assert_eq!(Rgba::WHITE.with_opacity(250.0).a, 255);
        assert_eq!(Rgba::WHITE.with_opacity(-3.0).a, 0);
        for percent in 0..=100 {
            let percent = f64::from(percent);
            assert_eq!(Rgba::BLACK.with_opacity(percent).opacity(), percent);
        }
    }
}

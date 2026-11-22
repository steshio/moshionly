//! Text styling values.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::defaults;

/// Which font draws a text layer.
///
/// The renderer resolves the family name against installed and bundled fonts
/// and falls back to a default face when it is missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Font {
    /// Family name, for example `"Inter"`. Default `"Inter"`.
    #[serde(
        default = "defaults::font_family",
        skip_serializing_if = "defaults::is_font_family"
    )]
    pub family: String,
    /// Weight from 1 to 1000; 400 is regular and 700 bold. Default 400.
    #[serde(
        default = "defaults::font_weight",
        skip_serializing_if = "defaults::is_font_weight"
    )]
    #[schemars(range(min = 1, max = 1000))]
    pub weight: u16,
    /// Whether to use the italic face. Default false.
    #[serde(default, skip_serializing_if = "defaults::is_default")]
    pub italic: bool,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: defaults::font_family(),
            weight: defaults::font_weight(),
            italic: false,
        }
    }
}

impl Font {
    /// A regular, upright font of the given family.
    pub fn family(name: impl Into<String>) -> Self {
        Self {
            family: name.into(),
            ..Self::default()
        }
    }
}

/// Horizontal alignment of text lines inside the text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    /// Lines start at the left edge.
    #[default]
    Left,
    /// Lines are centred.
    Center,
    /// Lines end at the right edge.
    Right,
}

impl TextAlign {
    /// The fraction of spare width that goes to the left of a line: 0, 0.5 or 1.
    pub fn factor(self) -> f64 {
        match self {
            TextAlign::Left => 0.0,
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.0,
        }
    }
}

/// Vertical alignment of the text block inside the text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlign {
    /// The first line sits at the top.
    #[default]
    Top,
    /// The block is centred.
    Middle,
    /// The last line sits at the bottom.
    Bottom,
}

impl VerticalAlign {
    /// The fraction of spare height that goes above the text: 0, 0.5 or 1.
    pub fn factor(self) -> f64 {
        match self {
            VerticalAlign::Top => 0.0,
            VerticalAlign::Middle => 0.5,
            VerticalAlign::Bottom => 1.0,
        }
    }
}

/// How a text layer's box follows its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoSize {
    /// The box hugs the text on both axes; lines break only at line breaks in
    /// the content. The stored `width` and `height` are then only a hint, and
    /// the box grows from the side named by the alignment.
    #[default]
    WidthAndHeight,
    /// The width is fixed and text wraps to it; the height follows the text.
    Height,
    /// Both sides are fixed. Text wraps to the width and may overflow the height.
    Fixed,
}

/// Letter case applied to the content before it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextCase {
    /// As typed.
    #[default]
    Original,
    /// ALL CAPITALS.
    Upper,
    /// all lower case.
    Lower,
    /// First Letter Of Each Word Capitalised.
    Title,
}

impl TextCase {
    /// Applies the case to `text`.
    pub fn apply(self, text: &str) -> String {
        match self {
            TextCase::Original => text.to_owned(),
            TextCase::Upper => text.to_uppercase(),
            TextCase::Lower => text.to_lowercase(),
            TextCase::Title => {
                let mut result = String::with_capacity(text.len());
                let mut at_word_start = true;
                for c in text.chars() {
                    if at_word_start && c.is_alphabetic() {
                        result.extend(c.to_uppercase());
                    } else {
                        result.push(c);
                    }
                    at_word_start = c.is_whitespace();
                }
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn font_defaults_and_round_trip() {
        let font: Font = serde_json::from_value(json!({})).unwrap();
        assert_eq!(font, Font::default());
        assert_eq!(
            (font.family.as_str(), font.weight, font.italic),
            ("Inter", 400, false)
        );
        assert_eq!(serde_json::to_value(&font).unwrap(), json!({}));

        let bold: Font =
            serde_json::from_value(json!({ "family": "Lora", "weight": 700, "italic": true }))
                .unwrap();
        assert_eq!(
            serde_json::to_value(&bold).unwrap(),
            json!({ "family": "Lora", "weight": 700, "italic": true })
        );
        assert!(serde_json::from_value::<Font>(json!({ "style": "italic" })).is_err());
    }

    #[test]
    fn enums_use_snake_case() {
        assert_eq!(
            serde_json::to_value(AutoSize::WidthAndHeight).unwrap(),
            json!("width_and_height")
        );
        assert_eq!(
            serde_json::to_value(VerticalAlign::Middle).unwrap(),
            json!("middle")
        );
        assert_eq!(
            serde_json::to_value(TextCase::Upper).unwrap(),
            json!("upper")
        );
        assert_eq!(
            serde_json::from_value::<TextAlign>(json!("right")).unwrap(),
            TextAlign::Right
        );
    }

    #[test]
    fn case_is_applied_to_content() {
        assert_eq!(TextCase::Original.apply("Hello wORLD"), "Hello wORLD");
        assert_eq!(TextCase::Upper.apply("Hello wORLD"), "HELLO WORLD");
        assert_eq!(TextCase::Lower.apply("Hello wORLD"), "hello world");
        assert_eq!(
            TextCase::Title.apply("hello big\nworld 3d"),
            "Hello Big\nWorld 3d"
        );
    }
}

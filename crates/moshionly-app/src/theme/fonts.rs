//! The bundled typefaces and how they are registered with egui.
//!
//! - **Inter** (regular, medium, semibold) for all interface text.
//! - **JetBrains Mono** for numbers that must keep their width: times and
//!   readouts.
//! - **Phosphor** for icons.
//!
//! All three are open source; their licenses sit next to the files in `assets/`.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId};

/// Font family name for medium-weight interface text.
pub const MEDIUM_FAMILY: &str = "moshionly-medium";
/// Font family name for semibold interface text.
pub const SEMIBOLD_FAMILY: &str = "moshionly-semibold";
/// Font family name for the icon font.
pub const ICON_FAMILY: &str = "moshionly-icons";

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const MONO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
const ICONS: &[u8] = include_bytes!("../../assets/icons/Phosphor.ttf");

/// Builds the font set for the whole app.
///
/// Every text family falls back to the icon font, so an icon constant can be
/// mixed into an ordinary label.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    for (name, bytes) in [
        ("inter", INTER_REGULAR),
        ("inter-medium", INTER_MEDIUM),
        ("inter-semibold", INTER_SEMIBOLD),
        ("mono", MONO_REGULAR),
        ("icons", ICONS),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }

    let stack = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    fonts
        .families
        .insert(FontFamily::Proportional, stack(&["inter", "icons"]));
    fonts
        .families
        .insert(FontFamily::Monospace, stack(&["mono", "inter", "icons"]));
    fonts.families.insert(
        FontFamily::Name(MEDIUM_FAMILY.into()),
        stack(&["inter-medium", "icons"]),
    );
    fonts.families.insert(
        FontFamily::Name(SEMIBOLD_FAMILY.into()),
        stack(&["inter-semibold", "icons"]),
    );
    fonts
        .families
        .insert(FontFamily::Name(ICON_FAMILY.into()), stack(&["icons"]));
    fonts
}

/// The font for an icon drawn in a box of `size` points.
pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICON_FAMILY.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_points_at_registered_data() {
        let fonts = definitions();
        for (family, names) in &fonts.families {
            assert!(!names.is_empty(), "{family} has no fonts");
            for name in names {
                assert!(
                    fonts.font_data.contains_key(name),
                    "{family} refers to missing font data {name}"
                );
            }
        }
    }
}

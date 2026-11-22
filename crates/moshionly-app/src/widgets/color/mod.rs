//! Colour: the plain colour type, swatches, the colour field and its picker.

mod field;
mod picker;
mod rgba;

pub use field::{ColorField, ColorResponse, apply_hex, color_swatch, paint_swatch};
pub use picker::{PICKER_WIDTH, PickerResponse, color_picker};
pub use rgba::{ColorParseError, Rgba};

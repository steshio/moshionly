//! The widget kit: the controls the editor is built from.
//!
//! Every widget here is a small function or builder that knows nothing about
//! documents. It takes plain values, paints itself with the tokens from
//! [`crate::theme`], and reports what the user did through its return value.

mod button;
pub mod color;
mod empty_state;
mod list_row;
mod menu;
pub mod numeric;
pub mod paint;
mod property_row;
mod section;
mod segmented;
mod slider;
mod text_field;
mod tooltip;

pub use button::{ButtonKind, IconButton, TextButton, ToolButton, icon_toggle};
pub use color::{ColorField, ColorResponse, Rgba, color_picker, color_swatch};
pub use empty_state::empty_state;
pub use list_row::{ListRow, ListRowResponse, row_toggle};
pub use menu::{Dropdown, MenuItem, context_menu, menu_heading, menu_separator, popup_frame};
pub use numeric::{NumericField, NumericResponse};
pub use property_row::{LABEL_WIDTH, control_row, pair, property_row};
pub use section::{divider, padded, section, section_header};
pub use segmented::{segmented, tabs};
pub use slider::{Slider, slider_value, slider_with_value};
pub use text_field::{TextField, TextFieldResponse, search_field};
pub use tooltip::tooltip;

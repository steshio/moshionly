//! The design tokens: every colour, spacing step, radius, size, type size and
//! shadow the interface uses, for the light and the dark theme.
//!
//! Change a value here and the whole app follows. Nothing outside the `theme`
//! module contains a literal colour or size that belongs to the look.

use egui::{Color32, epaint::Shadow};

/// A colour from a `0xRRGGBB` literal.
const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// A translucent colour from a `0xRRGGBB` literal and an alpha from 0 to 255.
const fn rgba(hex: u32, alpha: u8) -> Color32 {
    let a = alpha as u32;
    let r = ((hex >> 16) & 0xFF) * a / 255;
    let g = ((hex >> 8) & 0xFF) * a / 255;
    let b = (hex & 0xFF) * a / 255;
    Color32::from_rgba_premultiplied(r as u8, g as u8, b as u8, alpha)
}

/// A named colour family used to tell kinds of things apart, for example the
/// kinds of action bars in the timeline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tone {
    /// Grey. Structure and summaries.
    #[default]
    Neutral,
    /// The app's accent colour.
    Accent,
    /// Green-blue.
    Teal,
    /// Warm yellow-orange.
    Amber,
    /// Pink-red.
    Rose,
    /// Purple: main components and their instances.
    Violet,
    /// Light blue: video and sound clips.
    Sky,
}

impl Tone {
    /// Every tone, in display order.
    pub const ALL: [Self; 7] = [
        Self::Neutral,
        Self::Accent,
        Self::Teal,
        Self::Amber,
        Self::Rose,
        Self::Violet,
        Self::Sky,
    ];
}

/// The four colours a [`Tone`] resolves to in one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneColors {
    /// Strong fill, used when the thing is selected. Text on it is `on_solid`.
    pub solid: Color32,
    /// Quiet fill for the resting state.
    pub soft: Color32,
    /// Hairline around the quiet fill.
    pub outline: Color32,
    /// Text and icons on the quiet fill.
    pub text: Color32,
    /// Text and icons on the strong fill.
    pub on_solid: Color32,
}

/// Every colour of one theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// Panels and bars.
    pub surface: Color32,
    /// The canvas behind the artboards.
    pub canvas: Color32,
    /// Things that float above panels: the tool bar, menus, dialogs.
    pub raised: Color32,
    /// The resting fill of fields and the hover fill of quiet buttons.
    pub well: Color32,
    /// A field or quiet button under the pointer.
    pub well_hover: Color32,
    /// A field or quiet button being pressed.
    pub well_pressed: Color32,

    /// Hairlines between panels and sections.
    pub border: Color32,
    /// Outlines that must read against a well, and scroll handles.
    pub border_strong: Color32,

    /// Primary text and icons.
    pub text: Color32,
    /// Labels, units and inactive icons.
    pub text_secondary: Color32,
    /// Placeholders and disabled things.
    pub text_tertiary: Color32,
    /// Text and icons on an accent fill.
    pub text_on_accent: Color32,

    /// The accent: primary buttons, the active tool, the playhead, selection.
    pub accent: Color32,
    /// The accent under the pointer.
    pub accent_hover: Color32,
    /// The accent while pressed.
    pub accent_pressed: Color32,
    /// Accent-coloured text and icons on a neutral surface.
    pub accent_text: Color32,
    /// The pale accent fill behind a selected row.
    pub accent_soft: Color32,
    /// The focus ring around a focused field.
    pub focus_ring: Color32,

    /// Destructive actions and errors.
    pub danger: Color32,
    /// Warnings.
    pub warning: Color32,
    /// Success.
    pub success: Color32,

    /// The dimming layer behind a dialog.
    pub scrim: Color32,
    /// The translucent fill of a rubber-band selection.
    pub marquee_fill: Color32,
    /// Snapping guides on the canvas and in the timeline.
    pub guide: Color32,
    /// Distance measurements on the canvas.
    pub measure: Color32,
    /// The fill of selection handles.
    pub handle_fill: Color32,
    /// The tint over the part of the timeline after the scene ends.
    pub timeline_outside: Color32,
    /// Light square of the transparency checkerboard.
    pub checker_light: Color32,
    /// Dark square of the transparency checkerboard.
    pub checker_dark: Color32,

    /// The tone table, indexed by [`Tone`] in declaration order.
    pub tones: [ToneColors; 7],
}

impl Palette {
    /// The colours of one tone.
    pub fn tone(&self, tone: Tone) -> &ToneColors {
        &self.tones[tone as usize]
    }
}

/// The spacing scale, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    /// 2
    pub xxs: f32,
    /// 4
    pub xs: f32,
    /// 6
    pub sm: f32,
    /// 8
    pub md: f32,
    /// 12
    pub lg: f32,
    /// 16, the side padding of panels
    pub xl: f32,
    /// 24
    pub xxl: f32,
}

/// Corner radii, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radius {
    /// Tiny things: swatches inside fields, bar grips.
    pub xs: f32,
    /// Controls: buttons, fields, rows, timeline bars.
    pub sm: f32,
    /// Menus and popups.
    pub md: f32,
    /// The tool bar and dialogs.
    pub lg: f32,
}

/// Fixed sizes of interface parts, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    /// Height of a field, a button, a dropdown.
    pub control: f32,
    /// Vertical pitch of property rows in the inspector.
    pub row: f32,
    /// Height of a list row (layers).
    pub list_row: f32,
    /// Indent per nesting level in a list.
    pub list_indent: f32,
    /// Height of a section header.
    pub section_header: f32,
    /// The standard icon box.
    pub icon: f32,
    /// Icons on the tool bar.
    pub icon_large: f32,
    /// Height of the top bar.
    pub top_bar: f32,
    /// Height of the floating tool bar.
    pub tool_bar: f32,
    /// Side of a tool button.
    pub tool_button: f32,
    /// Gap between the tool bar and the bottom of the canvas.
    pub tool_bar_inset: f32,
    /// Default width of the layers panel.
    pub left_panel: f32,
    /// Default width of the inspector.
    pub right_panel: f32,
    /// Default width of the chat panel.
    pub chat_panel: f32,
    /// Narrowest a side panel can be dragged.
    pub panel_min: f32,
    /// Widest a side panel can be dragged.
    pub panel_max: f32,
    /// Default height of the timeline.
    pub timeline: f32,
    /// Lowest the timeline can be dragged.
    pub timeline_min: f32,
    /// Width of the invisible grab area of a splitter.
    pub splitter_grab: f32,
    /// Height of the timeline header (transport and ruler).
    pub timeline_header: f32,
    /// Height of the tick band at the top of the ruler.
    pub ruler_ticks: f32,
    /// Vertical pitch of timeline rows.
    pub timeline_row: f32,
    /// Height of an action bar.
    pub timeline_bar: f32,
    /// Default width of the timeline's label column.
    pub timeline_labels: f32,
    /// Width of the grab area at each end of a bar.
    pub bar_edge_grab: f32,
    /// Side of a selection handle on the canvas.
    pub handle: f32,
    /// How far outside a corner the rotation zone reaches.
    pub rotate_zone: f32,
    /// Width of menus and dropdown lists.
    pub menu_width: f32,
    /// Width of the command palette.
    pub palette_width: f32,
    /// Width of a toast.
    pub toast_width: f32,
}

/// Type sizes, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    /// Badges and ruler numbers.
    pub caption: f32,
    /// Secondary labels.
    pub small: f32,
    /// Nearly everything.
    pub body: f32,
    /// The file name and dialog titles.
    pub title: f32,
    /// Empty-state headlines.
    pub heading: f32,
}

/// Shadows of floating things.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadows {
    /// The tool bar: a tight contact shadow and a wide soft one.
    pub float: [Shadow; 2],
    /// Menus, dropdowns, tooltips.
    pub popup: Shadow,
    /// Dialogs.
    pub modal: Shadow,
    /// The faint drop under an artboard.
    pub artboard: Shadow,
}

pub(super) const SPACE: Space = Space {
    xxs: 2.0,
    xs: 4.0,
    sm: 6.0,
    md: 8.0,
    lg: 12.0,
    xl: 16.0,
    xxl: 24.0,
};

pub(super) const RADIUS: Radius = Radius {
    xs: 3.0,
    sm: 5.0,
    md: 8.0,
    lg: 12.0,
};

pub(super) const SIZE: Size = Size {
    control: 24.0,
    row: 32.0,
    list_row: 28.0,
    list_indent: 16.0,
    section_header: 36.0,
    icon: 16.0,
    icon_large: 20.0,
    top_bar: 44.0,
    tool_bar: 44.0,
    tool_button: 32.0,
    tool_bar_inset: 12.0,
    left_panel: 248.0,
    right_panel: 264.0,
    chat_panel: 320.0,
    panel_min: 200.0,
    panel_max: 480.0,
    timeline: 248.0,
    timeline_min: 120.0,
    splitter_grab: 7.0,
    timeline_header: 44.0,
    ruler_ticks: 12.0,
    timeline_row: 26.0,
    timeline_bar: 20.0,
    timeline_labels: 248.0,
    bar_edge_grab: 7.0,
    handle: 7.0,
    rotate_zone: 14.0,
    menu_width: 220.0,
    palette_width: 520.0,
    toast_width: 320.0,
};

pub(super) const TYPE: TypeScale = TypeScale {
    caption: 10.5,
    small: 11.0,
    body: 12.0,
    title: 13.0,
    heading: 15.0,
};

pub(super) const LIGHT_PALETTE: Palette = Palette {
    surface: rgb(0xFFFFFF),
    canvas: rgb(0xF2F2F4),
    raised: rgb(0xFFFFFF),
    well: rgb(0xF2F2F4),
    well_hover: rgb(0xEAEAED),
    well_pressed: rgb(0xE0E0E5),

    border: rgb(0xE7E7EA),
    border_strong: rgb(0xD0D0D6),

    text: rgb(0x1C1C20),
    text_secondary: rgb(0x74747C),
    text_tertiary: rgb(0xADADB4),
    text_on_accent: rgb(0xFFFFFF),

    accent: rgb(0x5B5FEF),
    accent_hover: rgb(0x4E52E0),
    accent_pressed: rgb(0x4347D0),
    accent_text: rgb(0x4347D9),
    accent_soft: rgb(0xECEDFE),
    focus_ring: rgb(0x5B5FEF),

    danger: rgb(0xE5484D),
    warning: rgb(0xE5932A),
    success: rgb(0x2E9E6B),

    scrim: rgba(0x101014, 96),
    marquee_fill: rgba(0x5B5FEF, 28),
    guide: rgb(0xF0447C),
    measure: rgb(0xF2742B),
    handle_fill: rgb(0xFFFFFF),
    timeline_outside: rgb(0xF5F5F7),
    checker_light: rgb(0xFFFFFF),
    checker_dark: rgb(0xE9E9EC),

    tones: [
        ToneColors {
            solid: rgb(0x6F6F78),
            soft: rgb(0xEDEDF0),
            outline: rgb(0xD2D2D8),
            text: rgb(0x55555C),
            on_solid: rgb(0xFFFFFF),
        },
        ToneColors {
            solid: rgb(0x5B5FEF),
            soft: rgb(0xECEDFE),
            outline: rgb(0xB5B8F7),
            text: rgb(0x4347D9),
            on_solid: rgb(0xFFFFFF),
        },
        ToneColors {
            solid: rgb(0x0E9F86),
            soft: rgb(0xE0F5F0),
            outline: rgb(0x99DACC),
            text: rgb(0x0B7F6B),
            on_solid: rgb(0xFFFFFF),
        },
        ToneColors {
            solid: rgb(0xBD7414),
            soft: rgb(0xFCF0DD),
            outline: rgb(0xEFCB90),
            text: rgb(0xA9650F),
            on_solid: rgb(0xFFFFFF),
        },
        ToneColors {
            solid: rgb(0xE0508A),
            soft: rgb(0xFDE8F0),
            outline: rgb(0xF3B1CB),
            text: rgb(0xBC316A),
            on_solid: rgb(0xFFFFFF),
        },
        ToneColors {
            solid: rgb(0x9747FF),
            soft: rgb(0xF3EBFF),
            outline: rgb(0xD3B5FF),
            text: rgb(0x7B2BE0),
            on_solid: rgb(0xFFFFFF),
        },
        ToneColors {
            solid: rgb(0x1A8FD8),
            soft: rgb(0xE3F3FD),
            outline: rgb(0x9CCFF0),
            text: rgb(0x0A6BA8),
            on_solid: rgb(0xFFFFFF),
        },
    ],
};

pub(super) const DARK_PALETTE: Palette = Palette {
    surface: rgb(0x2A2A2D),
    canvas: rgb(0x1B1B1E),
    raised: rgb(0x303034),
    well: rgb(0x38383C),
    well_hover: rgb(0x424247),
    well_pressed: rgb(0x4C4C52),

    border: rgb(0x3C3C41),
    border_strong: rgb(0x55555C),

    text: rgb(0xF4F4F6),
    text_secondary: rgb(0xA6A6AF),
    text_tertiary: rgb(0x6C6C75),
    text_on_accent: rgb(0xFFFFFF),

    accent: rgb(0x6E72FF),
    accent_hover: rgb(0x8286FF),
    accent_pressed: rgb(0x5B5FEF),
    accent_text: rgb(0xAEB1FF),
    accent_soft: rgb(0x34366A),
    focus_ring: rgb(0x8286FF),

    danger: rgb(0xF2686C),
    warning: rgb(0xF0A94A),
    success: rgb(0x4CC38A),

    scrim: rgba(0x000000, 140),
    marquee_fill: rgba(0x6E72FF, 40),
    guide: rgb(0xFF5C93),
    measure: rgb(0xFF8E4A),
    handle_fill: rgb(0xFFFFFF),
    timeline_outside: rgb(0x242427),
    checker_light: rgb(0x3A3A3E),
    checker_dark: rgb(0x303034),

    tones: [
        ToneColors {
            solid: rgb(0x8E8E98),
            soft: rgb(0x3B3B40),
            outline: rgb(0x57575E),
            text: rgb(0xC9C9D0),
            on_solid: rgb(0x16161A),
        },
        ToneColors {
            solid: rgb(0x6E72FF),
            soft: rgb(0x31335F),
            outline: rgb(0x5357B8),
            text: rgb(0xB8BAFF),
            on_solid: rgb(0xFFFFFF),
        },
        ToneColors {
            solid: rgb(0x1DB89C),
            soft: rgb(0x1E3F39),
            outline: rgb(0x2C7568),
            text: rgb(0x7FE0CD),
            on_solid: rgb(0x06211C),
        },
        ToneColors {
            solid: rgb(0xE39A3B),
            soft: rgb(0x453620),
            outline: rgb(0x86632A),
            text: rgb(0xF2C583),
            on_solid: rgb(0x2A1B05),
        },
        ToneColors {
            solid: rgb(0xEA6A9E),
            soft: rgb(0x482738),
            outline: rgb(0x8C4263),
            text: rgb(0xF5A9C8),
            on_solid: rgb(0x2E0B1B),
        },
        ToneColors {
            solid: rgb(0xA66BFF),
            soft: rgb(0x3B2C55),
            outline: rgb(0x6C4CA6),
            text: rgb(0xCDB2FF),
            on_solid: rgb(0x1C0B33),
        },
        ToneColors {
            solid: rgb(0x3FA9F5),
            soft: rgb(0x1D3A4F),
            outline: rgb(0x2D6B94),
            text: rgb(0x8ED0FF),
            on_solid: rgb(0x04223A),
        },
    ],
};

pub(super) const LIGHT_SHADOWS: Shadows = Shadows {
    float: [
        Shadow {
            offset: [0, 1],
            blur: 3,
            spread: 0,
            color: rgba(0x101014, 26),
        },
        Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: rgba(0x101014, 30),
        },
    ],
    popup: Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: rgba(0x101014, 46),
    },
    modal: Shadow {
        offset: [0, 16],
        blur: 48,
        spread: 0,
        color: rgba(0x101014, 70),
    },
    artboard: Shadow {
        offset: [0, 1],
        blur: 4,
        spread: 0,
        color: rgba(0x101014, 18),
    },
};

pub(super) const DARK_SHADOWS: Shadows = Shadows {
    float: [
        Shadow {
            offset: [0, 1],
            blur: 3,
            spread: 0,
            color: rgba(0x000000, 90),
        },
        Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: rgba(0x000000, 110),
        },
    ],
    popup: Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: rgba(0x000000, 130),
    },
    modal: Shadow {
        offset: [0, 16],
        blur: 48,
        spread: 0,
        color: rgba(0x000000, 160),
    },
    artboard: Shadow {
        offset: [0, 1],
        blur: 4,
        spread: 0,
        color: rgba(0x000000, 70),
    },
};

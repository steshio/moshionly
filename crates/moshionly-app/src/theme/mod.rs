//! The look of Moshionly, in one place.
//!
//! [`Theme`] holds the design tokens (see [`tokens`]) for a light and a dark
//! theme. [`install`] registers the fonts and translates the tokens into egui's
//! own [`egui::Style`] for both themes, so built-in egui widgets match the
//! custom-painted ones. Widgets fetch the tokens with [`Theme::of`].
//!
//! To change the look of the app, edit this module and nothing else.

pub mod fonts;
pub mod icons;
pub mod tokens;

use egui::{
    Color32, Context, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Ui, Vec2,
    style::{ScrollStyle, Selection, WidgetVisuals, Widgets},
};
use serde::{Deserialize, Serialize};

pub use tokens::{Palette, Radius, Shadows, Size, Space, Tone, ToneColors, TypeScale};

/// Which theme the app shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThemeMode {
    /// Follow the operating system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl ThemeMode {
    /// Every mode, in menu order.
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    /// The name shown in menus.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

/// The complete set of design tokens for one theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Whether this is the dark theme.
    pub dark: bool,
    /// Colours.
    pub color: Palette,
    /// The spacing scale.
    pub space: Space,
    /// Corner radii.
    pub radius: Radius,
    /// Fixed sizes of interface parts.
    pub size: Size,
    /// Type sizes.
    pub type_scale: TypeScale,
    /// Shadows.
    pub shadow: Shadows,
}

static LIGHT: Theme = Theme {
    dark: false,
    color: tokens::LIGHT_PALETTE,
    space: tokens::SPACE,
    radius: tokens::RADIUS,
    size: tokens::SIZE,
    type_scale: tokens::TYPE,
    shadow: tokens::LIGHT_SHADOWS,
};

static DARK: Theme = Theme {
    dark: true,
    color: tokens::DARK_PALETTE,
    space: tokens::SPACE,
    radius: tokens::RADIUS,
    size: tokens::SIZE,
    type_scale: tokens::TYPE,
    shadow: tokens::DARK_SHADOWS,
};

impl Theme {
    /// The light theme.
    pub fn light() -> &'static Self {
        &LIGHT
    }

    /// The dark theme.
    pub fn dark() -> &'static Self {
        &DARK
    }

    /// The theme for a light or dark surface.
    pub fn for_dark_mode(dark: bool) -> &'static Self {
        if dark { &DARK } else { &LIGHT }
    }

    /// The theme in effect for `ui`.
    pub fn of(ui: &Ui) -> &'static Self {
        Self::for_dark_mode(ui.visuals().dark_mode)
    }

    /// The theme in effect for the whole window.
    pub fn of_ctx(ctx: &Context) -> &'static Self {
        Self::for_dark_mode(ctx.theme() == egui::Theme::Dark)
    }

    /// Regular interface text.
    pub fn body(&self) -> FontId {
        FontId::new(self.type_scale.body, FontFamily::Proportional)
    }

    /// Interface text with a little more weight: button labels, active tabs.
    pub fn body_medium(&self) -> FontId {
        FontId::new(self.type_scale.body, medium())
    }

    /// Section titles and other emphasised text.
    pub fn body_strong(&self) -> FontId {
        FontId::new(self.type_scale.body, semibold())
    }

    /// Secondary labels.
    pub fn small(&self) -> FontId {
        FontId::new(self.type_scale.small, FontFamily::Proportional)
    }

    /// Secondary labels with a little more weight.
    pub fn small_medium(&self) -> FontId {
        FontId::new(self.type_scale.small, medium())
    }

    /// Badges and ruler numbers.
    pub fn caption(&self) -> FontId {
        FontId::new(self.type_scale.caption, FontFamily::Proportional)
    }

    /// The file name and dialog titles.
    pub fn title(&self) -> FontId {
        FontId::new(self.type_scale.title, semibold())
    }

    /// Empty-state headlines.
    pub fn heading(&self) -> FontId {
        FontId::new(self.type_scale.heading, semibold())
    }

    /// Numbers that must keep their width: times and readouts.
    pub fn mono(&self) -> FontId {
        FontId::new(self.type_scale.small, FontFamily::Monospace)
    }

    /// A one-pixel hairline in the border colour.
    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.color.border)
    }
}

fn medium() -> FontFamily {
    FontFamily::Name(fonts::MEDIUM_FAMILY.into())
}

fn semibold() -> FontFamily {
    FontFamily::Name(fonts::SEMIBOLD_FAMILY.into())
}

/// Registers the fonts and both themes with egui. Call once at startup.
pub fn install(ctx: &Context) {
    ctx.set_fonts(fonts::definitions());
    ctx.set_style_of(egui::Theme::Light, egui_style(&LIGHT));
    ctx.set_style_of(egui::Theme::Dark, egui_style(&DARK));
}

/// Switches between following the system, light and dark.
pub fn set_mode(ctx: &Context, mode: ThemeMode) {
    ctx.set_theme(match mode {
        ThemeMode::System => egui::ThemePreference::System,
        ThemeMode::Light => egui::ThemePreference::Light,
        ThemeMode::Dark => egui::ThemePreference::Dark,
    });
}

/// Translates a [`Theme`] into the style egui's built-in widgets use.
fn egui_style(theme: &Theme) -> egui::Style {
    let c = &theme.color;
    let mut style = egui::Style {
        text_styles: [
            (TextStyle::Small, theme.small()),
            (TextStyle::Body, theme.body()),
            (TextStyle::Button, theme.body()),
            (TextStyle::Heading, theme.heading()),
            (TextStyle::Monospace, theme.mono()),
        ]
        .into(),
        drag_value_text_style: TextStyle::Body,
        animation_time: 0.1,
        explanation_tooltips: false,
        ..egui::Style::default()
    };
    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = 0.5;

    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::new(theme.space.md, theme.space.xs);
    spacing.button_padding = Vec2::new(theme.space.md, theme.space.xs);
    spacing.interact_size = Vec2::new(theme.size.control, theme.size.control);
    spacing.indent = theme.size.list_indent;
    spacing.icon_width = theme.size.icon;
    spacing.icon_width_inner = theme.size.icon - 6.0;
    spacing.icon_spacing = theme.space.sm;
    spacing.menu_margin = Margin::same(theme.space.xs as i8);
    spacing.menu_width = theme.size.menu_width;
    spacing.window_margin = Margin::same(theme.space.lg as i8);
    spacing.tooltip_width = 280.0;
    spacing.scroll = ScrollStyle {
        bar_width: 6.0,
        floating_width: 4.0,
        floating_allocated_width: 0.0,
        bar_inner_margin: 2.0,
        bar_outer_margin: 2.0,
        ..ScrollStyle::floating()
    };

    let visuals = &mut style.visuals;
    *visuals = if theme.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = c.surface;
    visuals.window_fill = c.raised;
    visuals.window_stroke = Stroke::new(1.0, c.border);
    visuals.window_corner_radius = corner(theme.radius.lg);
    visuals.window_shadow = theme.shadow.modal;
    visuals.menu_corner_radius = corner(theme.radius.md);
    visuals.popup_shadow = theme.shadow.popup;
    visuals.extreme_bg_color = c.well;
    visuals.text_edit_bg_color = Some(c.well);
    visuals.faint_bg_color = c.well;
    visuals.code_bg_color = c.well;
    visuals.hyperlink_color = c.accent_text;
    visuals.warn_fg_color = c.warning;
    visuals.error_fg_color = c.danger;
    visuals.override_text_color = None;
    visuals.weak_text_color = Some(c.text_secondary);
    visuals.button_frame = true;
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = false;
    visuals.striped = false;
    visuals.slider_trailing_fill = true;
    visuals.text_cursor.stroke = Stroke::new(1.5, c.accent);
    visuals.selection = Selection {
        bg_fill: c.accent.gamma_multiply(0.3),
        stroke: Stroke::new(1.0, c.accent),
    };
    visuals.widgets = widget_visuals(theme);

    style
}

fn widget_visuals(theme: &Theme) -> Widgets {
    let c = &theme.color;
    let radius = corner(theme.radius.sm);
    let state = |fill: Color32, outline: Color32, text: Color32| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, outline),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    };
    Widgets {
        noninteractive: state(c.surface, c.border, c.text),
        inactive: state(c.well, Color32::TRANSPARENT, c.text),
        hovered: state(c.well_hover, Color32::TRANSPARENT, c.text),
        active: state(c.well_pressed, Color32::TRANSPARENT, c.text),
        open: state(c.well_hover, Color32::TRANSPARENT, c.text),
    }
}

/// An egui corner radius from a token.
pub fn corner(radius: f32) -> CornerRadius {
    CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance of a colour, per WCAG.
    fn luminance(color: Color32) -> f32 {
        let channel = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn text_is_readable_on_its_surfaces() {
        for theme in [Theme::light(), Theme::dark()] {
            let c = &theme.color;
            assert!(contrast(c.text, c.surface) >= 7.0);
            assert!(contrast(c.text, c.well) >= 7.0);
            assert!(contrast(c.text_secondary, c.surface) >= 4.0);
            assert!(contrast(c.text_on_accent, c.accent) >= 3.0);
            assert!(contrast(c.accent_text, c.surface) >= 4.0);
            assert!(contrast(c.text, c.accent_soft) >= 7.0);
        }
    }

    #[test]
    fn tone_text_is_readable_on_tone_fills() {
        for theme in [Theme::light(), Theme::dark()] {
            for tone in Tone::ALL {
                let t = theme.color.tone(tone);
                assert!(
                    contrast(t.text, t.soft) >= 4.0,
                    "{tone:?} text on soft, dark={}",
                    theme.dark
                );
                assert!(
                    contrast(t.on_solid, t.solid) >= 3.0,
                    "{tone:?} text on solid, dark={}",
                    theme.dark
                );
            }
        }
    }

    #[test]
    fn themes_differ_only_where_they_should() {
        assert_eq!(Theme::light().space, Theme::dark().space);
        assert_eq!(Theme::light().size, Theme::dark().size);
        assert_ne!(Theme::light().color, Theme::dark().color);
        assert!(Theme::for_dark_mode(true).dark);
        assert!(!Theme::for_dark_mode(false).dark);
    }

    #[test]
    fn installs_into_a_context() {
        let ctx = Context::default();
        install(&ctx);
        set_mode(&ctx, ThemeMode::Dark);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            Theme::dark().color.surface
        );
        assert_eq!(
            ctx.style_of(egui::Theme::Light).visuals.panel_fill,
            Theme::light().color.surface
        );
    }
}

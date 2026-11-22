//! Headless screenshots of the editor's interface.
//!
//! Each test renders a piece of the interface with the real theme, without
//! opening a window, in light and dark at 1x and 2x, and writes PNG files (see
//! [`common::output_dir`]). Look at them when you change the theme or a
//! widget. The tests only assert that rendering worked; they do not compare
//! pixels, because graphics adapters differ.

mod common;

use common::{Variant, assert_rendered, can_render, snapshot};
use egui::vec2;
use moshionly_app::{
    key::Key,
    settings::Settings,
    shell::{InspectorTab, Shell, sample::Sample},
    theme::{Theme, icons},
    timeline::{Timeline, TimelineState},
    viewport::{Artboard, Camera, Measurement, OrientedBox, Overlays, Viewport},
    widgets::*,
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Design,
    Animate,
}

fn controls_gallery(ui: &mut egui::Ui) {
    let theme = Theme::of(ui);
    let mut mode = Mode::Design;
    let mut open = true;
    let mut closed = false;
    let mut name = String::from("Hero card");
    let mut query = String::new();
    let mut visible = true;

    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(theme.space.md);
    padded(ui, |ui| {
        segmented(
            ui,
            &mut mode,
            &[(Mode::Design, "Design"), (Mode::Animate, "Animate")],
        );
        ui.add_space(theme.space.md);
    });

    section(
        ui,
        "Buttons",
        &mut open,
        |ui| {
            ui.add(IconButton::new(icons::PLUS).tooltip("Add"));
            ui.add(IconButton::new(icons::SLIDERS_HORIZONTAL).dimmed(true));
        },
        |ui| {
            control_row(ui, |ui| {
                ui.add(TextButton::primary("Export").icon(icons::EXPORT));
                ui.add(TextButton::new("Secondary"));
                ui.add(TextButton::new("Quiet").kind(ButtonKind::Quiet));
            });
            control_row(ui, |ui| {
                ui.add(IconButton::new(icons::ARROW_U_UP_LEFT));
                ui.add(IconButton::new(icons::MAGNET).selected(true));
                icon_toggle(
                    ui,
                    &mut visible,
                    icons::EYE,
                    icons::EYE_SLASH,
                    "Show or hide",
                );
                ui.add(ToolButton::new(icons::CURSOR, "Select").selected(true));
                ui.add(ToolButton::new(icons::RECTANGLE, "Rectangle"));
            });
        },
    );
    section(
        ui,
        "Text",
        &mut open,
        |_| {},
        |ui| {
            property_row(ui, "Name", |ui| {
                TextField::new(&mut name).show(ui);
            });
            control_row(ui, |ui| {
                search_field(ui, &mut query);
            });
        },
    );
    section(ui, "Collapsed", &mut closed, |_| {}, |_| {});
    divider(ui);

    ListRow::new("Scene 1")
        .icon(icons::FRAME_CORNERS)
        .expanded(true)
        .strong(true)
        .show(ui, |_, _| {});
    ListRow::new("Card")
        .icon(icons::RECTANGLE)
        .depth(1)
        .selected(true)
        .show(ui, |ui, hovered| {
            let mut locked = true;
            let mut hidden = false;
            row_toggle(ui, &mut hidden, icons::EYE_SLASH, icons::EYE, hovered);
            row_toggle(
                ui,
                &mut locked,
                icons::LOCK_SIMPLE,
                icons::LOCK_SIMPLE_OPEN,
                hovered,
            );
        });
    ListRow::new("Title with a long name that will not fit")
        .icon(icons::TEXT_T)
        .depth(1)
        .show(ui, |_, _| {});
    ListRow::new("Hidden photo")
        .icon(icons::IMAGE)
        .depth(1)
        .dimmed(true)
        .show(ui, |_, _| {});
    divider(ui);
    empty_state(
        ui,
        icons::LIGHTNING,
        "No actions yet",
        "Select a layer and add an action to make it move.",
        Some("Add action"),
    );
}

#[derive(Clone, Copy, PartialEq)]
enum Easing {
    Smooth,
    Natural,
    Bounce,
}

fn inputs_gallery(ui: &mut egui::Ui) {
    let theme = Theme::of(ui);
    let mut open = true;
    let (mut x, mut y, mut w, mut h) = (240.0, 340.0, 600.0, 400.0);
    let (mut rotation, mut radius, mut opacity) = (0.0, 32.0, 80.0);
    let (mut start, mut duration) = (0.0, 600.0);
    let mut fill = Rgba::from_hex("#5B5FEF").unwrap();
    let mut stroke = Rgba::from_hex("#1C1C2080").unwrap();
    let mut easing = Easing::Smooth;
    let mut intensity = 50.0;
    let mut tab = 0;

    ui.spacing_mut().item_spacing.y = 0.0;
    padded(ui, |ui| {
        tabs(ui, &mut tab, &[(0, "Layers"), (1, "Assets")]);
    });
    section(
        ui,
        "Position and size",
        &mut open,
        |_| {},
        |ui| {
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut x, "X").label("X").show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut y, "Y").label("Y").show(ui);
                    },
                );
            });
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut w, "Width").label("W").show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut h, "Height").label("H").show(ui);
                    },
                );
            });
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut rotation, "Rotation")
                            .icon(icons::ANGLE)
                            .unit("°")
                            .show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut radius, "Corner radius")
                            .icon(icons::CORNERS_OUT)
                            .mixed(true)
                            .show(ui);
                    },
                );
            });
        },
    );
    section(
        ui,
        "Fill and stroke",
        &mut open,
        |ui| {
            ui.add(IconButton::new(icons::PLUS).tooltip("Add fill"));
        },
        |ui| {
            control_row(ui, |ui| {
                ColorField::new(&mut fill, "Fill").show(ui);
            });
            control_row(ui, |ui| {
                ColorField::new(&mut stroke, "Stroke").show(ui);
            });
            property_row(ui, "Opacity", |ui| {
                slider_with_value(ui, &mut opacity, "Opacity", 0.0..=100.0, "%");
            });
        },
    );
    section(
        ui,
        "Timing",
        &mut open,
        |_| {},
        |ui| {
            control_row(ui, |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut start, "Start")
                            .icon(icons::TIMER)
                            .unit("ms")
                            .show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut duration, "Duration")
                            .icon(icons::ARROWS_HORIZONTAL)
                            .unit("ms")
                            .show(ui);
                    },
                );
            });
            property_row(ui, "Easing", |ui| {
                Dropdown::new(
                    &mut easing,
                    "Easing",
                    &[
                        (Easing::Smooth, "Smooth"),
                        (Easing::Natural, "Natural"),
                        (Easing::Bounce, "Bounce"),
                    ],
                )
                .icon(icons::WAVE_SINE)
                .show(ui);
            });
            property_row(ui, "Intensity", |ui| {
                slider_with_value(ui, &mut intensity, "Intensity", 0.0..=100.0, "");
            });
        },
    );
    divider(ui);
    ui.add_space(theme.space.lg);

    // Popups, drawn in place so they show up in a still image.
    padded(ui, |ui| {
        popup_frame(ui).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
            ui.set_width(theme.size.menu_width - 8.0);
            menu_heading(ui, "Layer");
            ui.add(MenuItem::new("Duplicate").icon(icons::COPY).shortcut("⌘D"));
            ui.add(
                MenuItem::new("Group selection")
                    .icon(icons::FOLDER_SIMPLE)
                    .shortcut("⌘G")
                    .highlighted(true),
            );
            ui.add(MenuItem::new("Snap to grid").checked(true));
            menu_separator(ui);
            ui.add(
                MenuItem::new("Delete")
                    .icon(icons::TRASH)
                    .shortcut("⌫")
                    .danger(true),
            );
        });
        ui.add_space(theme.space.lg);
        popup_frame(ui)
            .inner_margin(theme.space.lg as i8)
            .show(ui, |ui| {
                color_picker(ui, egui::Id::unique("gallery-picker"), &mut fill);
            });
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Mp4,
    Gif,
    Png,
}

/// A dialog and a stack of toasts over a dimmed backdrop.
fn overlays_gallery() -> impl FnMut(&mut egui::Ui) {
    let mut toasts = Toasts::default();
    toasts.push(Toast::success("Exported launch-teaser.mp4"));
    toasts.push(Toast::error("Could not read hero.png"));
    move |ui| {
        let mut format = Format::Mp4;
        let (mut width, mut height, mut fps) = (1080.0, 1080.0, 60.0);
        let anchor = ui.max_rect().center_bottom() - vec2(0.0, 16.0);
        toasts.show(ui.ctx(), anchor);
        let _ = ModalFrame::new("Export").width(360.0).show(ui.ctx(), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            property_row(ui, "Format", |ui| {
                Dropdown::new(
                    &mut format,
                    "Format",
                    &[
                        (Format::Mp4, "MP4 video"),
                        (Format::Gif, "Animated GIF"),
                        (Format::Png, "PNG sequence"),
                    ],
                )
                .show(ui);
            });
            property_row(ui, "Size", |ui| {
                pair(
                    ui,
                    |ui| {
                        NumericField::new(&mut width, "Width").label("W").show(ui);
                    },
                    |ui| {
                        NumericField::new(&mut height, "Height").label("H").show(ui);
                    },
                );
            });
            property_row(ui, "Frame rate", |ui| {
                NumericField::new(&mut fps, "Frame rate")
                    .unit("fps")
                    .width(96.0)
                    .show(ui);
            });
            modal_footer(ui, |ui| {
                ui.add(TextButton::primary("Export").icon(icons::EXPORT));
                ui.add(TextButton::new("Cancel"));
            });
        });
    }
}

#[test]
fn controls() {
    if !can_render() {
        return;
    }
    let size = vec2(280.0, 760.0);
    for variant in Variant::ALL {
        let (path, image) = snapshot("controls", size, variant, controls_gallery);
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

#[test]
fn inputs() {
    if !can_render() {
        return;
    }
    let size = vec2(280.0, 960.0);
    for variant in Variant::ALL {
        let (path, image) = snapshot("inputs", size, variant, inputs_gallery);
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

#[test]
fn overlays() {
    if !can_render() {
        return;
    }
    let size = vec2(560.0, 400.0);
    for variant in Variant::ALL {
        let (path, image) = snapshot("overlays", size, variant, overlays_gallery());
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The whole editor window with the sample scene.
#[test]
fn shell() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = Shell::new(Settings::default());
        let (path, image) = snapshot("shell", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The window with the chat panel open and the Animate tab showing.
#[test]
fn shell_with_chat() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = Shell::new(Settings::default());
        shell.panels_mut().chat_open = true;
        shell.inspector_mut().tab = InspectorTab::Animate;
        let (path, image) = snapshot("shell-chat", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The canvas on its own: two artboards without pictures, and one of every
/// overlay.
#[test]
fn viewport() {
    if !can_render() {
        return;
    }
    let size = vec2(640.0, 400.0);
    for variant in Variant::ALL {
        let mut camera = Camera {
            center: egui::pos2(700.0, 500.0),
            zoom: 0.3,
        };
        let (path, image) = snapshot("viewport", size, variant, move |ui| {
            let rect = ui.ctx().content_rect();
            let artboards = [
                Artboard {
                    key: Key::new("one"),
                    name: "Scene 1",
                    rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1080.0, 1080.0)),
                    texture: None,
                    fill: egui::Color32::WHITE,
                    selected: true,
                },
                Artboard {
                    key: Key::new("two"),
                    name: "A second scene with a name too long to fit above it",
                    rect: egui::Rect::from_min_size(egui::pos2(1240.0, 0.0), vec2(400.0, 1080.0)),
                    texture: None,
                    fill: egui::Color32::from_rgb(0x18, 0x18, 0x1E),
                    selected: false,
                },
            ];
            let rect_at = |x: f32, y: f32, w: f32, h: f32| {
                egui::Rect::from_min_size(egui::pos2(x, y), vec2(w, h))
            };
            let overlays = Overlays {
                outlines: vec![OrientedBox::from_rect(rect_at(120.0, 700.0, 160.0, 160.0))],
                selection: Some(OrientedBox::new(rect_at(320.0, 180.0, 560.0, 320.0), -12.0)),
                show_size: true,
                hover: Some(OrientedBox::from_rect(rect_at(1300.0, 140.0, 280.0, 200.0))),
                marquee: Some(rect_at(60.0, 640.0, 420.0, 320.0)),
                guides: vec![
                    [egui::pos2(540.0, -80.0), egui::pos2(540.0, 1160.0)],
                    [egui::pos2(-80.0, 540.0), egui::pos2(1160.0, 540.0)],
                ],
                measurements: vec![Measurement {
                    from: egui::pos2(1080.0, 800.0),
                    to: egui::pos2(1240.0, 800.0),
                    label: "160".to_owned(),
                }],
            };
            Viewport::new(&mut camera)
                .artboards(&artboards)
                .overlays(&overlays)
                .show(ui, rect);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The timeline on its own, with the sample scene's actions.
#[test]
fn timeline() {
    if !can_render() {
        return;
    }
    let size = vec2(1100.0, 280.0);
    for variant in Variant::ALL {
        let sample = Sample::default();
        let mut groups = sample.timeline_groups();
        groups[1].collapsed = true;
        groups[2].hidden = true;
        let selection = [Key::new("a-card-move")];
        let mut state = TimelineState::default();
        let (path, image) = snapshot("timeline", size, variant, move |ui| {
            let rect = ui.ctx().content_rect();
            Timeline::new(&mut state, &groups)
                .selection(&selection)
                .selected_group(Some(&Key::new("card")))
                .playhead(1200.0)
                .duration(sample.duration)
                .looping(true)
                .show(ui, rect);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

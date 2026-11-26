//! Headless screenshots of the editor's interface.
//!
//! Each test renders a piece of the interface with the real theme, without
//! opening a window, in light and dark at 1x and 2x, and writes PNG files (see
//! [`common::output_dir`]). Look at them when you change the theme or a
//! widget. The tests only assert that rendering worked; they do not compare
//! pixels, because graphics adapters differ.
//!
//! The two pictures of the editor in `docs/images` come from here:
//! `editor-light.png` is `shell-light@2x.png` and `editor-dark.png` is
//! `shell-chat-dark@2x.png`. To refresh them after changing the look, run
//!
//! ```text
//! MOSHIONLY_SNAPSHOT_DIR=/tmp/shots cargo test -p moshionly-app --test snapshots
//! ```
//!
//! and copy those two files over.

mod common;

use common::{Variant, assert_rendered, can_render, snapshot};
use egui::vec2;
use moshionly_app::{
    key::Key,
    settings::Settings,
    shell::{InspectorTab, LayersTab, Shell},
    theme::{Theme, icons},
    timeline::{Timeline, TimelineState},
    viewport::{Artboard, Camera, Measurement, OrientedBox, Overlays, PenOverlay, Viewport},
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

/// The demo project that comes with the engine.
const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../moshionly-core/examples/demo.moshi"
);

/// An editor on the demo project that draws with the bundled fonts on the
/// test's own thread, so every frame shows finished pictures.
fn demo_shell() -> Shell {
    let session = moshionly_agent::SessionHandle::open(DEMO).expect("the demo file opens");
    let renderer = moshionly_render::Renderer::new(
        moshionly_render::FontLibrary::bundled(),
        moshionly_render::AssetLoader::default(),
    );
    Shell::headless(Settings::default(), session, renderer)
}

/// A chat whose only AI is a scripted stand-in for Claude Code, already
/// checked, with keys in memory: nothing reaches a real AI.
fn scripted_chat() -> moshionly_app::chat::Chat {
    use moshionly_app::chat::testing::{ScriptedProvider, ScriptedSource, status, statuses};
    let mut ready = statuses(&[]);
    ready[0] = status(
        "claude-code",
        moshionly_ai::ReadyState::Ready,
        "Claude Code is installed and signed in.",
    )
    .unwrap();
    let source = ScriptedSource::new(ready).with_provider(
        ScriptedProvider::new("claude-code", Vec::new())
            .with_models(vec![moshionly_ai::ModelInfo::new("sonnet", "Sonnet")]),
    );
    let registry = moshionly_ai::ProviderRegistry::new(
        moshionly_ai::AiSettings::default(),
        std::sync::Arc::new(moshionly_ai::MemoryKeyStore::new()),
    );
    let mut chat =
        moshionly_app::chat::Chat::with_source(registry, std::sync::Arc::new(source), None);
    chat.controller_mut().ensure_checked();
    assert!(
        chat.controller_mut()
            .settle(std::time::Duration::from_secs(10)),
        "the stand-in AI was not checked"
    );
    chat
}

/// The whole editor window with the demo project.
#[test]
fn shell() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = demo_shell();
        shell
            .editor_mut()
            .selection
            .select(moshionly_core::NodeId::from("title"));
        let (path, image) = snapshot("shell", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The window with the chat panel open and the Animate tab showing the
/// card in the middle of its animation.
#[test]
fn shell_with_chat() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = demo_shell();
        shell.set_chat(scripted_chat());
        // Agents can connect, as in the app; any free port will do.
        shell.serve_agents(0);
        shell.panels_mut().chat_open = true;
        shell.inspector_mut().tab = InspectorTab::Animate;
        let editor = shell.editor_mut();
        editor
            .selection
            .select(moshionly_core::NodeId::from("card"));
        // The amount is counting up and the dot popping in.
        editor.set_playhead(1700.0);
        let (path, image) = snapshot("shell-chat", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The Animate tab with the title's letter-by-letter action selected: its
/// timing, easing curve and settings, with the timeline below and the
/// canvas in the middle of the animation.
#[test]
fn animate_tab() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = demo_shell();
        shell.inspector_mut().tab = InspectorTab::Animate;
        let editor = shell.editor_mut();
        editor
            .selection
            .select(moshionly_core::NodeId::from("title"));
        editor.set_actions([moshionly_core::NodeId::from("titlein")]);
        editor.set_playhead(900.0);
        let (path, image) = snapshot("animate", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// A move made on the canvas: its path drawn over the card, and its values
/// and overshooting easing in the Animate tab.
#[test]
fn animate_move() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = demo_shell();
        shell.inspector_mut().tab = InspectorTab::Animate;
        let editor = shell.editor_mut();
        editor.selection.select(moshionly_core::NodeId::from("dot"));
        editor.set_playhead(2000.0);
        let added = editor
            .add_actions(moshionly_core::NodeType::Move, 0.0)
            .expect("a move is added");
        let id = added[0].clone();
        editor
            .apply(
                "Set the move",
                &[
                    moshionly_core::Edit::set(
                        id.clone(),
                        "to",
                        serde_json::json!({"x": -380, "y": -200}),
                    ),
                    moshionly_core::Edit::set(
                        id.clone(),
                        "easing",
                        serde_json::json!({"type": "overshoot", "intensity": 60}),
                    ),
                ],
            )
            .expect("the move is set");
        editor.set_playhead(2400.0);
        let (path, image) = snapshot("animate-move", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// Components: the demo's card made a main component, two instances of it
/// on a second artboard (one smaller and playing later), the Components
/// list in the layers panel, and the selected instance's Component section
/// with its bar on the timeline.
#[test]
fn components() {
    if !can_render() {
        return;
    }
    use moshionly_core::{Edit, NodeId, Point, build};
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = demo_shell();
        let editor = shell.editor_mut();
        let card = NodeId::from("card");
        let document = editor.document().clone();
        let (_, edits) = build::make_component(&document, std::slice::from_ref(&card))
            .expect("the card becomes a component");
        editor.apply("Create component", &edits).unwrap();
        let document = editor.document().clone();
        let (copies, edits) = build::add_artboard(&document, "Copies", 1080.0, 1080.0).unwrap();
        editor.apply("Add artboard", &edits).unwrap();
        let mut place = |at: Point| {
            let document = editor.document().clone();
            let (instance, edits) =
                build::add_instance(&document, &card, &copies, &build::Place::Last, at).unwrap();
            editor.apply("Insert instance", &edits).unwrap();
            instance
        };
        place(Point::new(240.0, 120.0));
        let small = place(Point::new(390.0, 620.0));
        editor
            .apply(
                "Resize and delay",
                &[Edit::update(
                    small.clone(),
                    [
                        ("width", serde_json::json!(300)),
                        ("height", serde_json::json!(200)),
                        ("time_offset", serde_json::json!(300)),
                    ],
                )],
            )
            .unwrap();
        editor.selection.select(small);
        editor.follow_selection();
        shell.set_layers_tab(LayersTab::Components);
        let (path, image) = snapshot("components", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// Made-up video frames, so the snapshot needs no `ffmpeg`: an evening
/// gradient whose light drifts with the moment of the file.
struct FakeFrames;

impl moshionly_render::VideoFrames for FakeFrames {
    fn frame(
        &self,
        request: &moshionly_render::VideoRequest<'_>,
    ) -> Result<moshionly_render::skia_safe::Image, String> {
        let (width, height) = (96u32, 54u32);
        let drift = request.time_ms / 4000.0;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let across = x as f64 / f64::from(width) + drift;
                let down = f64::from(y) / f64::from(height);
                let sun = (1.0 - ((across - 0.6).powi(2) + (down - 0.55).powi(2)).sqrt() * 2.5)
                    .clamp(0.0, 1.0);
                let channel = |base: f64, glow: f64| ((base + glow * sun) * 255.0).min(255.0) as u8;
                rgba.extend([
                    channel(0.35 + 0.3 * down, 0.6),
                    channel(0.25 + 0.15 * down, 0.5),
                    channel(0.55 - 0.25 * down, 0.2),
                    255,
                ]);
            }
        }
        moshionly_render::video::image_from_rgba(width, height, rgba)
    }
}

/// Video and sound: a reel with a video layer drawn from made-up frames,
/// selected with its Video section in the inspector, a title that fades
/// in over it, and on the timeline the video's clip and the scene's sound.
#[test]
fn media() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    let reel = r##"{"format":"moshionly","version":3,"name":"Beach reel","nodes":[
        {"id":"reel","parent":null,"index":"a0","type":"artboard","name":"Reel",
         "width":1080,"height":1080,"fill":"#14161C","duration":6000},
        {"id":"beach","parent":"reel","index":"a0","type":"video","name":"Beach at dusk",
         "source":"/fake/Beach at dusk.mp4","x":60,"y":200,"width":960,"height":540,
         "corner_radius":32,"start":500,"end":5000,"trim_start":1200,"volume":0.8},
        {"id":"title","parent":"reel","index":"a1","type":"text","content":"Summer reel",
         "x":60,"y":800,"font_size":96,"fill":"#FFFFFF"},
        {"id":"fade","parent":"reel","index":"a0","type":"fade_in","target":"title",
         "start":1000,"end":1800},
        {"id":"music","parent":"reel","index":"a1","type":"audio","name":"Music",
         "source":"/fake/music.mp3","start":0,"end":6000},
        {"id":"waves","parent":"reel","index":"a2","type":"audio",
         "source":"/fake/waves.wav","start":2500,"end":5500,"volume":0.5}
    ]}"##;
    for variant in Variant::ALL {
        let document = moshionly_core::io::parse(reel).expect("the reel parses");
        let renderer = moshionly_render::Renderer::new(
            moshionly_render::FontLibrary::bundled(),
            moshionly_render::AssetLoader::default().with_video(std::sync::Arc::new(FakeFrames)),
        );
        let mut shell = Shell::headless(
            Settings::default(),
            moshionly_agent::SessionHandle::from_document(document),
            renderer,
        );
        shell
            .editor_mut()
            .selection
            .select(moshionly_core::NodeId::from("beach"));
        let (path, image) = snapshot("media", size, variant, move |ui| {
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
                    texture_rect: None,
                    fill: egui::Color32::WHITE,
                    selected: true,
                },
                Artboard {
                    key: Key::new("two"),
                    name: "A second scene with a name too long to fit above it",
                    rect: egui::Rect::from_min_size(egui::pos2(1240.0, 0.0), vec2(400.0, 1080.0)),
                    texture: None,
                    texture_rect: None,
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
                pen: Some(PenOverlay {
                    path: vec![
                        egui::pos2(1340.0, 900.0),
                        egui::pos2(1440.0, 760.0),
                        egui::pos2(1540.0, 940.0),
                    ],
                    anchors: vec![egui::pos2(1340.0, 900.0), egui::pos2(1440.0, 760.0)],
                    handles: vec![[egui::pos2(1440.0, 760.0), egui::pos2(1500.0, 740.0)]],
                    closing: false,
                }),
                editing: None,
                paths: vec![(
                    (0..=24)
                        .map(|n| {
                            let t = n as f32 / 24.0;
                            egui::pos2(160.0 + 420.0 * t, 960.0 - 300.0 * t * (2.0 - t))
                        })
                        .collect(),
                    vec![egui::pos2(160.0, 960.0), egui::pos2(580.0, 660.0)],
                )],
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

/// The timeline on its own, with the demo project's actions.
#[test]
fn timeline() {
    if !can_render() {
        return;
    }
    let size = vec2(1100.0, 280.0);
    for variant in Variant::ALL {
        let document = moshionly_core::io::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moshionly-core/examples/demo.moshi"
        ))
        .expect("the demo file loads");
        let artboard = document.artboards()[0].clone();
        let duration = moshionly_core::eval::duration_of(&document, &artboard).unwrap();
        let mut groups = moshionly_app::editor::timeline::timeline_groups(
            &document,
            &artboard,
            &Default::default(),
        );
        groups[1].collapsed = true;
        groups[2].hidden = true;
        let selection = [groups[0].bars[0].key.clone()];
        let selected_group = groups[0].key.clone();
        let mut state = TimelineState::default();
        let (path, image) = snapshot("timeline", size, variant, move |ui| {
            let rect = ui.ctx().content_rect();
            Timeline::new(&mut state, &groups)
                .selection(&selection)
                .selected_group(Some(&selected_group))
                .playhead(1200.0)
                .duration(duration)
                .looping(true)
                .show(ui, rect);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The command palette over the window, with something typed.
#[test]
fn command_palette() {
    if !can_render() {
        return;
    }
    let size = vec2(1440.0, 900.0);
    for variant in Variant::ALL {
        let mut shell = demo_shell();
        shell.palette_mut().open();
        let (path, image) = snapshot("palette", size, variant, move |ui| {
            shell.show(ui);
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The dialogs of round 3 over the window: Export and Connect an agent (with
/// the server on a free port).
#[test]
fn connection_dialogs() {
    if !can_render() {
        return;
    }
    let size = vec2(1280.0, 820.0);
    for (name, export) in [("dialog-export", true), ("dialog-connect", false)] {
        for variant in Variant::ALL {
            let mut shell = demo_shell();
            if export {
                shell.open_export_dialog();
            } else {
                shell.serve_agents(0);
                shell.open_connect_dialog();
            }
            let (path, image) = snapshot(name, size, variant, move |ui| {
                shell.show(ui);
            });
            assert_rendered(&image, size, variant);
            println!("wrote {}", path.display());
        }
    }
}

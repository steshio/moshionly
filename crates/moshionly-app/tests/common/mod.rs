//! Shared helpers for the headless rendering tests.

use std::path::PathBuf;

use egui_kittest::Harness;
use moshionly_app::theme;

/// One rendering configuration: theme and pixel density.
#[derive(Clone, Copy, Debug)]
pub struct Variant {
    /// Render the dark theme.
    pub dark: bool,
    /// Physical pixels per point: 1.0 for standard screens, 2.0 for Retina.
    pub pixels_per_point: f32,
}

impl Variant {
    /// Light and dark, each at 1x and 2x.
    pub const ALL: [Self; 4] = [
        Self::new(false, 1.0),
        Self::new(false, 2.0),
        Self::new(true, 1.0),
        Self::new(true, 2.0),
    ];

    const fn new(dark: bool, pixels_per_point: f32) -> Self {
        Self {
            dark,
            pixels_per_point,
        }
    }

    /// The file name suffix for this variant, for example `dark@2x`.
    pub fn suffix(self) -> String {
        let theme = if self.dark { "dark" } else { "light" };
        let scale = if self.pixels_per_point >= 2.0 {
            "@2x"
        } else {
            ""
        };
        format!("{theme}{scale}")
    }
}

/// True when this machine can render headless. Without a graphics adapter
/// (some CI runners) the snapshot tests print a note and pass.
pub fn can_render() -> bool {
    use eframe::wgpu;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    if adapters.is_empty() {
        eprintln!("no graphics adapter found: skipping snapshot rendering");
    }
    !adapters.is_empty()
}

/// Where snapshots are written.
///
/// By default this is a `snapshots` folder inside cargo's temporary directory
/// for this crate. Set `MOSHIONLY_SNAPSHOT_DIR` to write somewhere else.
pub fn output_dir() -> PathBuf {
    let dir = std::env::var_os("MOSHIONLY_SNAPSHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("snapshots"));
    std::fs::create_dir_all(&dir).expect("create snapshot directory");
    dir
}

/// Renders `ui` headless at `size` points and returns the image.
///
/// The Moshionly theme is installed first, so the result looks exactly like
/// the app. The closure runs for a few frames so layouts that need a second
/// pass settle.
pub fn render(
    size: egui::Vec2,
    variant: Variant,
    mut ui: impl FnMut(&mut egui::Ui),
) -> image::RgbaImage {
    let egui_theme = if variant.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(variant.pixels_per_point)
        .with_theme(egui_theme)
        .with_context_setup(theme::install)
        .wgpu()
        .build_ui(move |root| {
            // The harness insets its root; the app's window does not. Use the
            // whole surface so snapshots match what the window shows.
            let full = root.ctx().content_rect();
            let mut window = root.new_child(egui::UiBuilder::new().max_rect(full));
            window.set_clip_rect(full);
            let fill = theme::Theme::of(&window).color.surface;
            window.painter().rect_filled(full, 0.0, fill);
            ui(&mut window);
        });
    harness.run_steps(4);
    harness.render().expect("render snapshot")
}

/// Renders `ui` and writes it as `<name>-<variant>.png` in [`output_dir`].
/// Returns the path and the image.
pub fn snapshot(
    name: &str,
    size: egui::Vec2,
    variant: Variant,
    ui: impl FnMut(&mut egui::Ui),
) -> (PathBuf, image::RgbaImage) {
    let image = render(size, variant, ui);
    let path = output_dir().join(format!("{name}-{}.png", variant.suffix()));
    image.save(&path).expect("write snapshot");
    (path, image)
}

/// Checks that an image has the expected pixel size and is not one flat
/// colour, which is what a failed or empty render looks like.
pub fn assert_rendered(image: &image::RgbaImage, size: egui::Vec2, variant: Variant) {
    let expected = (
        (size.x * variant.pixels_per_point).round() as u32,
        (size.y * variant.pixels_per_point).round() as u32,
    );
    assert_eq!(image.dimensions(), expected, "snapshot has the wrong size");
    let first = *image.get_pixel(0, 0);
    assert!(
        image.pixels().any(|p| *p != first),
        "snapshot is a single flat colour"
    );
}

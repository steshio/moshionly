//! Measures how long the canvas takes to draw one frame of a project, the
//! way its worker threads do: evaluate the artboard, then render it.
//!
//! ```text
//! cargo run --release -p moshionly-app --example frame_time -- crates/moshionly-core/examples/demo.moshi
//! ```
//!
//! Prints the mean time per frame at rest and in the middle of the
//! animation, at 1x and 2x density, with the bundled fonts, and how it
//! splits between evaluating, rendering and preparing the texture.

use std::time::{Duration, Instant};

use egui::{Rect, Vec2, pos2};
use moshionly_app::canvas::{RenderKey, plan};
use moshionly_core::{eval::duration_of, io};
use moshionly_render::{AssetLoader, FontLibrary, RenderOptions, Renderer};

/// Frames drawn for each measurement, after a few to warm up.
const FRAMES: u32 = 60;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("give the path of a .moshi file")?;
    let document = io::load(&path)?;
    let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::new(Some(path.into())));
    for artboard in document.artboards() {
        let node = document.get(artboard).ok_or("an artboard went missing")?;
        let layer = node.layer().ok_or("an artboard without a box")?;
        let size = Vec2::new(layer.width as f32, layer.height as f32);
        let middle = duration_of(&document, artboard).unwrap_or(0.0) / 2.0;
        println!("{} ({} × {})", node.name(), layer.width, layer.height);
        for density in [1.0, 2.0] {
            let visible = Rect::from_min_size(pos2(0.0, 0.0), size);
            let plan = plan(size, visible, density, 1.0, None);
            for time in [None, Some(middle)] {
                let key = RenderKey {
                    revision: 0,
                    time,
                    plan,
                    skip: None,
                };
                let [evaluate, render, texture] = measure(&renderer, &document, artboard, &key)?;
                let ms = |d: Duration| d.as_secs_f64() * 1000.0;
                println!(
                    "  {}x, {}: {:.2} ms per frame (evaluate {:.2}, render {:.2}, texture {:.2})",
                    density,
                    time.map_or("at rest".to_owned(), |t| format!("at {t:.0} ms")),
                    ms(evaluate + render + texture),
                    ms(evaluate),
                    ms(render),
                    ms(texture),
                );
            }
        }
    }
    Ok(())
}

/// The mean time to evaluate, render and prepare the texture of one frame.
fn measure(
    renderer: &Renderer,
    document: &moshionly_core::Document,
    artboard: &moshionly_core::NodeId,
    key: &RenderKey,
) -> Result<[Duration; 3], Box<dyn std::error::Error>> {
    let options = RenderOptions::new().with_scale(f64::from(key.plan.scale));
    let mut spent = [Duration::ZERO; 3];
    for round in 0..FRAMES + 5 {
        let started = Instant::now();
        let scene = moshionly_app::canvas::scene_of(renderer, document, artboard, key.time)?;
        let evaluated = Instant::now();
        let pixels = renderer.render(&scene, &options)?;
        let rendered = Instant::now();
        // Turned into what the window uploads, as the workers do.
        let image = egui::ColorImage::from_rgba_premultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.premultiplied(),
        );
        std::hint::black_box(image);
        let done = Instant::now();
        // The first few frames warm up caches and are not counted.
        if round >= 5 {
            spent[0] += evaluated - started;
            spent[1] += rendered - evaluated;
            spent[2] += done - rendered;
        }
    }
    Ok(spent.map(|total| total / FRAMES))
}

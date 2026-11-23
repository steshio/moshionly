//! Measures how many different frames a second the canvas shows while a
//! project plays, the way the editor plays it: a new picture is asked for
//! sixty times a second, the worker threads draw what they can, and frames
//! they cannot keep up with are dropped.
//!
//! ```text
//! cargo run --release -p moshionly-app --example playback_rate -- crates/moshionly-core/examples/demo.moshi [zoom] [pixels_per_point]
//! ```
//!
//! `zoom` defaults to 0.45, about what a 1080 by 1080 artboard gets when it
//! is fitted into the canvas of a 1440 by 900 window, and
//! `pixels_per_point` to 2, a Retina screen. The first artboard plays for
//! four seconds at full resolution, then four seconds with the resolution
//! lowered while renders are slow, as the editor does.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Rect, Vec2, pos2};
use moshionly_app::canvas::{CanvasRenderer, Quality, RenderKey, plan};
use moshionly_core::{eval::duration_of, io};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};

/// How long each run plays.
const PLAY: Duration = Duration::from_secs(4);

/// How often the window asks for a picture.
const TICK: Duration = Duration::from_micros(16_667);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let path = arguments.next().ok_or("give the path of a .moshi file")?;
    let zoom: f32 = arguments.next().map_or(Ok(0.45), |text| text.parse())?;
    let ppp: f32 = arguments.next().map_or(Ok(2.0), |text| text.parse())?;
    let document = Arc::new(io::load(&path)?);
    let artboard = document
        .artboards()
        .first()
        .cloned()
        .ok_or("the project has no artboards")?;
    let layer = document
        .get(&artboard)
        .and_then(|node| node.layer())
        .ok_or("an artboard without a box")?;
    let size = Vec2::new(layer.width as f32, layer.height as f32);
    let duration = duration_of(&document, &artboard).unwrap_or(1000.0).max(1.0);
    let ctx = egui::Context::default();
    println!(
        "{} × {} at zoom {zoom} on a {ppp}x screen",
        layer.width, layer.height
    );
    for adaptive in [false, true] {
        let renderer = Renderer::new(
            FontLibrary::bundled(),
            AssetLoader::new(Some(path.clone().into())),
        );
        let mut canvas = CanvasRenderer::new(renderer);
        let mut quality = Quality::new();
        let mut shown = 0u32;
        let mut last_time: Option<Option<f64>> = None;
        let mut factors = Vec::new();
        let started = Instant::now();
        let mut next = started;
        while started.elapsed() < PLAY {
            canvas.poll(&ctx);
            if adaptive {
                let times = canvas.times();
                quality.observe_renders(times.finished(), times.last());
            }
            if let Some(picture) = canvas.picture(&artboard)
                && last_time != Some(picture.key.time)
            {
                last_time = Some(picture.key.time);
                shown += 1;
            }
            let factor = if adaptive { quality.factor() } else { 1.0 };
            factors.push(factor);
            let time = (started.elapsed().as_secs_f64() * 1000.0) % duration;
            let visible = Rect::from_min_size(pos2(0.0, 0.0), size);
            let previous = canvas.requested(&artboard).map(|key| key.plan);
            let key = RenderKey {
                revision: 0,
                time: Some(time),
                plan: plan(size, visible, zoom * factor, ppp, previous.as_ref()),
                skip: None,
            };
            canvas.request(&artboard, &document, key);
            next += TICK;
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        let seconds = started.elapsed().as_secs_f64();
        let average = canvas
            .times()
            .average()
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let lowest = factors.iter().copied().fold(1.0, f32::min);
        println!(
            "  {}: {:.1} frames a second shown, {:.1} ms a render, resolution down to {:.0}%",
            if adaptive {
                "lowered when slow"
            } else {
                "full resolution"
            },
            f64::from(shown) / seconds,
            average,
            lowest * 100.0
        );
    }
    Ok(())
}

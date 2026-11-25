//! Lottie: the artboard as vector shapes and keyframes, in a JSON file that
//! apps and websites play at any size.
//!
//! Lottie export converts the scene model, not pixels. Each layer becomes a
//! Lottie layer, groups keep their structure, and every animated property
//! becomes keyframes with Bezier easing that land on the values Moshionly
//! shows at every frame. What Lottie cannot express exactly is drawn as
//! closely as it can be and listed as a warning that names the layer.
//!
//! | Module | What it does |
//! |---|---|
//! | `fit` | Keyframes that follow a value over time within a tolerance |
//! | `timeline`, `scenes`, `clock` | Values over time, where they change, and frames |
//! | `layers` | The layer tree as Lottie layers; shapes and their paint |
//! | `groups` | Null layers, precompositions, clips, masks and track mattes |
//! | `text`, `svg` | Letters as outlines from the renderer, and pieces that move |
//! | `poses` | How the pieces of a text move, worked out once per moment |
//! | `images` | Pictures inside the file or beside it |
//! | `instances` | Instances of components, detached on a copy first |
//! | `effects` | Drop shadows and blurs |
//! | `shapes`, `paint` | Outlines; fills and strokes in colours and gradients |
//! | `warnings` | What could not be exported exactly, by layer |
//! | `props`, `json` | Writing properties and numbers compactly |

mod clock;
mod effects;
mod fit;
mod groups;
mod images;
mod instances;
mod json;
mod layers;
mod paint;
mod poses;
mod props;
mod scenes;
mod shapes;
mod svg;
mod text;
mod timeline;
mod warnings;

use std::fs;
use std::path::Path;

use moshionly_core::Family;
use moshionly_core::model::Paint;
use moshionly_render::Background;
use serde_json::Value;

use crate::error::ExportError;
use crate::job::Context;
use crate::progress::Phase;
use clock::Clock;
use fit::Animated;
use images::Images;
use json::{num, object, places};
use layers::{Comp, Converter, Step};
use props::Transform;
use scenes::Scenes;
use timeline::Timeline;

/// The Lottie format version the files follow.
const VERSION: &str = "5.12.0";

/// Where a Lottie file's images go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LottieImages {
    /// Inside the JSON file, as base64 PNG or JPEG: one file that works
    /// anywhere. The default.
    #[default]
    Embedded,
    /// As image files in a folder beside the JSON file, named after it
    /// (`launch.json` gets `launch_images/`): a smaller JSON file, for
    /// players that load images from a folder.
    Files,
}

pub(crate) use images::ImageFiles;

/// Writes the plan's artboard as a Lottie file at `path`.
pub(crate) fn write(cx: &mut Context<'_>, path: &Path) -> Result<Option<ImageFiles>, ExportError> {
    let Some(times) = cx.plan.moments.frames() else {
        // A plan for Lottie always has frames; it is an animated format.
        return Ok(None);
    };
    let clock = Clock::new(times);
    let (document, mut notes) = instances::expanded(cx.source.document, cx.source.artboard);
    let document: &moshionly_core::Document = &document;
    let artboard = document
        .get(cx.source.artboard)
        .ok_or_else(|| ExportError::Internal {
            frame: 0,
            message: "the planned artboard is missing".into(),
        })?;
    let scenes = Scenes::new(document, &artboard.id, cx.source.renderer.fonts(), clock);
    let cancel = cx.reporter.cancel_token().clone();
    let timeline = Timeline {
        document,
        clock,
        scenes: &scenes,
        cancel: &cancel,
    };
    cx.reporter.report(Phase::Rendering, 0);

    let total = document
        .descendants(&artboard.id, Family::Layer)
        .len()
        .max(1) as u64;
    let frames = cx.plan.frame_count();
    let mut converted = 0;
    let mut reported = 0;
    let (warnings, images) = {
        let reporter = &mut cx.reporter;
        // Frames stand for the part of the layers converted, counting the
        // part done of the layer being converted.
        let progress = Box::new(move |step: Step| {
            let done = match step {
                Step::Layer => {
                    converted += 1;
                    converted as f64
                }
                Step::Part(part) => converted as f64 + part.clamp(0.0, 1.0),
            };
            let frames_done = (frames as f64 * done.min(total as f64) / total as f64) as u64;
            if frames_done != reported {
                reported = frames_done;
                reporter.report(Phase::Rendering, frames_done);
            }
            reporter.check()
        });
        let images = Images::new(cx.plan.lottie_images, cx.destination, cx.project_file);
        let mut converter = Converter::new(timeline, cx.source.renderer, images, progress);
        let mut comp = Comp::default();

        // A size other than the artboard's own scales everything from one
        // layer that draws nothing.
        let scale = cx.plan.scale;
        let root = ((scale - 1.0).abs() > 1e-9).then(|| comp.index());
        converter.add_children(&artboard.id, &mut comp, root, [0.0, 0.0])?;
        let over = match cx.plan.render.background {
            Background::Transparent => None,
            Background::Artboard => Some(None),
            Background::Over(color) => Some(Some(color)),
        };
        if let Some(over) = over {
            // A colour behind an artboard that is opaque throughout is never
            // seen.
            let opaque = converter.survey.iter().all(|frame| {
                matches!(
                    timeline.fill(&artboard.id, *frame),
                    Some(Paint::Solid(color)) if color.a == 255
                )
            });
            converter.background(artboard, &mut comp, root, over.filter(|_| !opaque));
        }
        if let Some(root) = root {
            let transform = Transform {
                scale: Animated::Static(vec![scale * 100.0]),
                ..Transform::default()
            };
            let mut layer = converter.layer(artboard, 3, root, None, &transform);
            layer.insert("nm".into(), Value::from("Size"));
            comp.layers.push(Value::Object(layer));
        }
        let animation = object([
            ("v", Value::from(VERSION)),
            ("fr", num(clock.fps, 3)),
            ("ip", Value::from(0)),
            ("op", num(clock.out_point(), places::TIME)),
            ("w", Value::from(cx.plan.width)),
            ("h", Value::from(cx.plan.height)),
            ("nm", Value::from(cx.plan.artboard_name.as_str())),
            ("ddd", Value::from(0)),
            (
                "assets",
                Value::Array(std::mem::take(&mut converter.assets)),
            ),
            ("layers", Value::Array(comp.layers)),
            ("meta", object([("g", Value::from("Moshionly"))])),
        ]);
        let text = serde_json::to_string(&animation).map_err(|error| ExportError::Internal {
            frame: 0,
            message: error.to_string(),
        })?;
        fs::write(path, text).map_err(|error| ExportError::io("written", path, error))?;
        converter.finish()
    };
    notes.extend(warnings.into_vec());
    for warning in notes {
        cx.reporter.note(warning);
    }
    cx.reporter.check()?;
    cx.reporter.report(Phase::Encoding, cx.plan.frame_count());
    Ok(images.into_files())
}

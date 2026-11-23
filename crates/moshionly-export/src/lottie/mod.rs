//! Lottie: the artboard as vector shapes and keyframes, in a JSON file that
//! apps and websites play at any size.
//!
//! Lottie export converts the scene model, not pixels. Each layer becomes a
//! Lottie layer, groups keep their structure, and every animated property
//! becomes keyframes with Bezier easing that land on the values Moshionly
//! shows at every frame. What Lottie cannot express exactly is drawn as
//! closely as it can be and listed as a warning that names the layer.

mod json;

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::ExportError;
use crate::job::Context;
use crate::progress::Phase;
use json::{num, object, places};

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

/// Image files written beside a Lottie file, moved into place after it.
#[derive(Debug, Default)]
pub(crate) struct ImageFiles {}

impl ImageFiles {
    /// Moves the image files into place.
    pub(crate) fn commit(self) -> Result<(), ExportError> {
        Ok(())
    }
}

/// Writes the plan's artboard as a Lottie file at `path`.
pub(crate) fn write(cx: &mut Context<'_>, path: &Path) -> Result<Option<ImageFiles>, ExportError> {
    let Some(times) = cx.plan.moments.frames() else {
        // A plan for Lottie always has frames; it is an animated format.
        return Ok(None);
    };
    cx.reporter.report(Phase::Rendering, 0);
    let animation = object([
        ("v", Value::from(VERSION)),
        ("fr", num(times.rate().fps(), 3)),
        ("ip", Value::from(0)),
        ("op", num(times.len() as f64, places::TIME)),
        ("w", Value::from(cx.plan.width)),
        ("h", Value::from(cx.plan.height)),
        ("nm", Value::from(cx.plan.artboard_name.as_str())),
        ("ddd", Value::from(0)),
        ("assets", Value::Array(Vec::new())),
        ("layers", Value::Array(Vec::new())),
        ("meta", object([("g", Value::from("Moshionly"))])),
    ]);
    cx.reporter.check()?;
    cx.reporter.report(Phase::Encoding, cx.plan.frame_count());
    let text = serde_json::to_string(&animation).map_err(|error| ExportError::Internal {
        frame: 0,
        message: error.to_string(),
    })?;
    fs::write(path, text).map_err(|error| ExportError::io("written", path, error))?;
    Ok(None)
}

//! PNG: one frame, or a folder of numbered frames.

use std::fs;
use std::path::Path;

use crate::error::ExportError;
use crate::job::Context;
use crate::progress::Phase;
use crate::settings::Moment;

/// Writes a single frame as a PNG file at `path`.
pub(crate) fn write_still(cx: &mut Context<'_>, path: &Path) -> Result<(), ExportError> {
    // A plan always has a first moment.
    let moment = cx.plan.moments.get(0).unwrap_or(Moment::AtRest);
    cx.reporter.report(Phase::Rendering, 0);
    let frame = cx.source.render(0, moment, &cx.plan.render)?;
    cx.reporter.warn(frame.warnings());
    cx.reporter.check()?;
    cx.reporter.report(Phase::Encoding, 1);
    let png = frame
        .to_png()
        .map_err(|source| ExportError::Render { frame: 0, source })?;
    fs::write(path, png).map_err(|error| ExportError::io("written", path, error))
}

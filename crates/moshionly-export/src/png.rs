//! PNG: one frame, or a folder of numbered frames.

use std::fs;
use std::path::Path;

use crate::error::ExportError;
use crate::frames::{self, Pass};
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

/// Writes every frame as a PNG file in the folder at `dir`, named
/// `frame_0000.png`, `frame_0001.png` and so on: zero-padded to four digits,
/// or more when there are 10 000 frames or more, so the names sort in
/// order.
pub(crate) fn write_sequence(cx: &mut Context<'_>, dir: &Path) -> Result<(), ExportError> {
    let digits = sequence_digits(cx.plan.frame_count());
    let pass = Pass {
        moments: cx.plan.moments,
        options: cx.plan.render,
        frame_bytes: cx.plan.width as usize * cx.plan.height as usize * 5,
        report_progress: true,
    };
    // Compressing a PNG costs about as much as drawing it, so it happens on
    // the worker threads too.
    let prepare = |index: u64, frame: moshionly_render::Frame| {
        frame.to_png().map_err(|source| ExportError::Render {
            frame: index,
            source,
        })
    };
    let mut consume = |index: u64, png: Vec<u8>| {
        let path = dir.join(sequence_name(index, digits));
        fs::write(&path, png).map_err(|error| ExportError::io("written", path, error))
    };
    cx.reporter.report(Phase::Rendering, 0);
    frames::render_in_order(cx.source, &pass, &mut cx.reporter, &prepare, &mut consume)
}

/// How many digits frame numbers are padded to.
fn sequence_digits(count: u64) -> usize {
    count.saturating_sub(1).to_string().len().max(4)
}

/// The file name of frame `index`.
fn sequence_name(index: u64, digits: usize) -> String {
    format!("frame_{index:0digits$}.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_in_frame_order() {
        assert_eq!(sequence_digits(1), 4);
        assert_eq!(sequence_digits(10_000), 4);
        assert_eq!(sequence_digits(10_001), 5);
        assert_eq!(sequence_name(7, 4), "frame_0007.png");
        assert_eq!(sequence_name(12_345, 5), "frame_12345.png");
    }
}

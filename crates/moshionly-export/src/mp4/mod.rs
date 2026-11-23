//! MP4 video with H.264, encoded in-process.
//!
//! Frames are converted to BT.709 Y'CbCr on the worker threads (see
//! [`yuv`]), encoded in order with OpenH264 (see [`h264`]) and written into
//! an MP4 file whose header comes first (see [`mux`]). MP4 cannot be
//! transparent: the plan always puts a colour behind the frames.

mod h264;
mod mux;
mod yuv;

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use moshionly_render::Frame;

use crate::error::ExportError;
use crate::frames::{self, Pass};
use crate::job::Context;
use crate::progress::Phase;
use crate::settings::Moment;
use crate::timing::FrameRate;
use h264::H264Encoder;
use mux::Mp4Writer;
use yuv::Yuv420;

/// Writes the MP4 video of `cx.plan` to `path`.
pub(crate) fn write(cx: &mut Context<'_>, path: &Path) -> Result<(), ExportError> {
    let plan = cx.plan;
    let rate = plan
        .moments
        .frames()
        .map_or(FrameRate::default(), |times| times.rate());
    let (width, height) = (plan.width as usize, plan.height as usize);
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut encoder = H264Encoder::new(plan.width, plan.height, rate, plan.quality)?;
    let mut muxer = Mp4Writer::new(dir, plan.width, plan.height, rate)
        .map_err(|error| ExportError::io("created", dir, error))?;

    let pass = Pass {
        moments: plan.moments,
        options: plan.render,
        frame_bytes: width * height * 6,
        report_progress: true,
    };
    let prepare =
        |_: u64, frame: Frame| Ok(Yuv420::from_rgba(frame.premultiplied(), width, height));
    let mut consume = |index: u64, picture: Yuv420| {
        let time_ms = match plan.moments.get(index) {
            Some(Moment::At(time)) => time,
            Some(Moment::AtRest) | None => 0.0,
        };
        let encoded = encoder.encode(&picture, time_ms)?;
        muxer
            .add(&encoded.data, encoded.key)
            .map_err(|error| ExportError::io("written", dir, error))
    };
    cx.reporter.report(Phase::Rendering, 0);
    frames::render_in_order(cx.source, &pass, &mut cx.reporter, &prepare, &mut consume)?;

    cx.reporter.report(Phase::Encoding, plan.frame_count());
    let (sps, pps) = encoder.parameter_sets()?;
    let file = File::create(path).map_err(|error| ExportError::io("created", path, error))?;
    muxer
        .finish(&mut BufWriter::new(file), sps, pps)
        .map_err(|error| ExportError::io("written", path, error))
}

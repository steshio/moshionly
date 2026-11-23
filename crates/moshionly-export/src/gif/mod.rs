//! Animated GIF.
//!
//! A GIF is written in two passes. The first renders every frame, small,
//! and counts their colours to choose one palette that suits the whole
//! animation (see [`palette`]); a colour that shows for a single frame is
//! counted like any other. The second renders every frame at full size,
//! maps it onto the palette (see [`dither`]) and stores only what changed:
//!
//! - a frame that looks exactly like the one before is not stored; the
//!   frame before is shown for longer instead;
//! - otherwise only the rectangle around the pixels that changed is
//!   stored, and inside it the pixels that did not change are marked
//!   see-through, so the picture below shows and the compressor sees long
//!   runs of one value.
//!
//! A transparent GIF cannot draw over the frame before, since a pixel that
//! turns see-through would keep its old colour. Each frame is cleared after
//! it is shown instead, and stores the rectangle around its visible pixels.
//!
//! Frame delays are whole hundredths of a second with the rounding carried
//! from frame to frame (see [`crate::FrameTimes::centisecond_delays`]), so the GIF
//! lasts as long as the animation.

mod dither;
mod palette;

use std::borrow::Cow;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use ::gif::{DisposalMethod, Encoder, Repeat};
use moshionly_render::Frame;

use crate::error::ExportError;
use crate::frames::{self, Pass};
use crate::job::Context;
use crate::progress::Phase;
use crate::settings::{Loop, Plan, Quality};
use dither::Indexer;
use palette::{ColorCounts, Palette};

/// How the palette is chosen at each quality.
///
/// Every frame is counted, each at the same reduced size. The size shares
/// `total_pixels` out between the frames, so a short animation is counted
/// in detail and a long one stays quick, within limits: never so small that
/// a small shape loses every pixel of its exact colour, and never larger
/// than needed.
struct Effort {
    /// Palette entries for colours; one more is kept for transparency.
    colors: usize,
    /// Pixels counted over the whole animation, when the limits allow.
    total_pixels: f64,
    /// Pixels each frame is counted at, at least.
    min_frame_pixels: f64,
    /// Pixels each frame is counted at, at most.
    max_frame_pixels: f64,
}

fn effort(quality: Quality) -> Effort {
    match quality {
        Quality::Draft => Effort {
            colors: 128,
            total_pixels: 2_000_000.0,
            min_frame_pixels: 60_000.0,
            max_frame_pixels: 160_000.0,
        },
        Quality::Standard => Effort {
            colors: 255,
            total_pixels: 9_000_000.0,
            min_frame_pixels: 160_000.0,
            max_frame_pixels: 360_000.0,
        },
        Quality::High => Effort {
            colors: 255,
            total_pixels: 48_000_000.0,
            min_frame_pixels: 360_000.0,
            max_frame_pixels: 1_000_000.0,
        },
    }
}

/// Writes the animated GIF of `cx.plan` to `path`.
pub(crate) fn write(cx: &mut Context<'_>, path: &Path) -> Result<(), ExportError> {
    let plan = cx.plan;
    let palette = choose_palette(cx)?;
    let transparent = palette.len() as u8;

    let file = File::create(path).map_err(|error| ExportError::io("created", path, error))?;
    let mut table: Vec<u8> = palette.colors().iter().flatten().copied().collect();
    // The entry after the colours is the see-through one.
    table.extend([0, 0, 0]);
    let encoder = Encoder::new(
        BufWriter::new(file),
        plan.width as u16,
        plan.height as u16,
        &table,
    )
    .map_err(gif_error)?;
    let mut writer = FrameWriter::new(encoder, plan, transparent)?;

    let indexer = Indexer {
        palette: &palette,
        dither: plan.dither,
        transparent,
    };
    let width = plan.width as usize;
    let pass = Pass {
        moments: plan.moments,
        options: plan.render,
        frame_bytes: width * plan.height as usize * 5,
        report_progress: true,
    };
    let prepare = |_: u64, frame: Frame| Ok(indexer.index(frame.premultiplied(), width));
    let mut consume = |index: u64, indices: Vec<u8>| {
        let delay = plan
            .moments
            .frames()
            .and_then(|times| times.centisecond_delay(index));
        writer.push(indices, delay.unwrap_or(0))
    };
    cx.reporter.report(Phase::Rendering, 0);
    frames::render_in_order(cx.source, &pass, &mut cx.reporter, &prepare, &mut consume)?;
    cx.reporter.report(Phase::Encoding, plan.frame_count());
    writer
        .finish()
        .map_err(|error| ExportError::io("written", path, error))
}

/// Renders every frame small and chooses the palette from their colours.
///
/// The counts of every frame are added up, which does not depend on the
/// order the frames are drawn in, and the palette is chosen from the total,
/// so the same animation always gets the same palette.
fn choose_palette(cx: &mut Context<'_>) -> Result<Palette, ExportError> {
    let plan = cx.plan;
    let effort = effort(plan.quality);
    let pixels = f64::from(plan.width) * f64::from(plan.height);
    let frame_pixels = (effort.total_pixels / plan.frame_count().max(1) as f64)
        .clamp(effort.min_frame_pixels, effort.max_frame_pixels);
    let shrink = (frame_pixels / pixels).sqrt().min(1.0);
    let options = plan.render.with_scale(plan.scale * shrink);
    let pass = Pass {
        moments: plan.moments,
        options,
        frame_bytes: (pixels * shrink * shrink) as usize * 8,
        report_progress: false,
    };
    let mut counts = ColorCounts::default();
    let prepare = |_: u64, frame: Frame| {
        let mut counts = ColorCounts::default();
        counts.add_premultiplied(frame.premultiplied());
        Ok(counts)
    };
    let mut consume = |_: u64, frame_counts: ColorCounts| {
        counts.merge(frame_counts);
        Ok(())
    };
    frames::render_in_order(cx.source, &pass, &mut cx.reporter, &prepare, &mut consume)?;
    Ok(Palette::from_counts(&counts, effort.colors))
}

fn gif_error(error: gif::EncodingError) -> ExportError {
    match error {
        gif::EncodingError::Io(error) => ExportError::io("written", "the GIF", error),
        other => ExportError::Gif(other.to_string()),
    }
}

/// Decides what each frame stores and writes it, one frame behind so that
/// frames that look the same can be merged into a longer delay.
struct FrameWriter<W: Write> {
    encoder: Encoder<W>,
    width: usize,
    height: usize,
    /// The see-through palette index.
    transparent: u8,
    /// Whether the GIF is transparent: every frame is cleared after it is
    /// shown.
    clear: bool,
    /// The last frame pushed, in full.
    shown: Option<Vec<u8>>,
    /// The last frame's stored part, waiting for its final delay.
    pending: Option<gif::Frame<'static>>,
    /// The delay of the pending frame, in hundredths of a second.
    pending_delay: u32,
}

impl<W: Write> FrameWriter<W> {
    fn new(mut encoder: Encoder<W>, plan: &Plan, transparent: u8) -> Result<Self, ExportError> {
        let repeat = match plan.gif_loop {
            Loop::Forever => Some(Repeat::Infinite),
            Loop::Once | Loop::Times(0 | 1) => None,
            // The GIF field counts repeats after the first play.
            Loop::Times(times) => Some(Repeat::Finite(times - 1)),
        };
        if let Some(repeat) = repeat {
            encoder.set_repeat(repeat).map_err(gif_error)?;
        }
        Ok(Self {
            encoder,
            width: plan.width as usize,
            height: plan.height as usize,
            transparent,
            clear: plan.transparent,
            shown: None,
            pending: None,
            pending_delay: 0,
        })
    }

    /// Adds the next frame, `indices` for every pixel, shown for `delay`
    /// hundredths of a second.
    fn push(&mut self, indices: Vec<u8>, delay: u32) -> Result<(), ExportError> {
        if self.shown.as_ref() == Some(&indices) {
            self.pending_delay += delay;
            return Ok(());
        }
        self.flush().map_err(gif_error)?;
        let frame = match (&self.shown, self.clear) {
            (_, true) => self.visible_part(&indices),
            (None, false) => self.part(&indices, Rect::whole(self.width, self.height), None),
            (Some(shown), false) => self.changed_part(shown, &indices),
        };
        self.pending = Some(frame);
        self.pending_delay = delay;
        self.shown = Some(indices);
        Ok(())
    }

    /// Writes the last frame and the end of the file.
    fn finish(mut self) -> std::io::Result<()> {
        self.flush().map_err(std::io::Error::other)?;
        let mut writer = self.encoder.into_inner().map_err(std::io::Error::other)?;
        writer.flush()
    }

    fn flush(&mut self) -> Result<(), gif::EncodingError> {
        let Some(mut frame) = self.pending.take() else {
            return Ok(());
        };
        frame.delay = self.pending_delay.min(u32::from(u16::MAX)) as u16;
        self.encoder.write_frame(&frame)
    }

    /// For a transparent GIF: the rectangle around the visible pixels,
    /// cleared once shown.
    fn visible_part(&self, indices: &[u8]) -> gif::Frame<'static> {
        let t = self.transparent;
        let area = Rect::around(self.width, self.height, |x, y| {
            indices[y * self.width + x] != t
        })
        .unwrap_or(Rect::pixel());
        let mut frame = self.part(indices, area, None);
        frame.transparent = Some(t);
        frame.dispose = DisposalMethod::Background;
        frame
    }

    /// For an opaque GIF: the rectangle around what changed, with what did
    /// not change left see-through so the frame below shows.
    fn changed_part(&self, shown: &[u8], indices: &[u8]) -> gif::Frame<'static> {
        let area = Rect::around(self.width, self.height, |x, y| {
            let at = y * self.width + x;
            indices[at] != shown[at]
        })
        .unwrap_or(Rect::pixel());
        let mut frame = self.part(indices, area, Some(shown));
        frame.transparent = Some(self.transparent);
        frame
    }

    /// The pixels of `area`, with pixels equal to `shown` made see-through
    /// when it is given.
    fn part(&self, indices: &[u8], area: Rect, shown: Option<&[u8]>) -> gif::Frame<'static> {
        let mut buffer = Vec::with_capacity(area.width * area.height);
        for y in area.y..area.y + area.height {
            let row = y * self.width + area.x..y * self.width + area.x + area.width;
            match shown {
                None => buffer.extend_from_slice(&indices[row]),
                Some(shown) => buffer.extend(indices[row.clone()].iter().zip(&shown[row]).map(
                    |(now, before)| {
                        if now == before {
                            self.transparent
                        } else {
                            *now
                        }
                    },
                )),
            }
        }
        gif::Frame {
            left: area.x as u16,
            top: area.y as u16,
            width: area.width as u16,
            height: area.height as u16,
            buffer: Cow::Owned(buffer),
            dispose: DisposalMethod::Keep,
            ..gif::Frame::default()
        }
    }
}

/// A rectangle of pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Rect {
    fn whole(width: usize, height: usize) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// The top-left pixel, for a frame that stores nothing.
    fn pixel() -> Rect {
        Rect::whole(1, 1)
    }

    /// The smallest rectangle around the pixels for which `inside` is
    /// true, or `None` when there are none.
    fn around(width: usize, height: usize, inside: impl Fn(usize, usize) -> bool) -> Option<Rect> {
        let rows: Vec<usize> = (0..height)
            .filter(|&y| (0..width).any(|x| inside(x, y)))
            .collect();
        let (&top, &bottom) = (rows.first()?, rows.last()?);
        let left = rows
            .iter()
            .filter_map(|&y| (0..width).find(|&x| inside(x, y)))
            .min()?;
        let right = rows
            .iter()
            .filter_map(|&y| (0..width).rev().find(|&x| inside(x, y)))
            .max()?;
        Some(Rect {
            x: left,
            y: top,
            width: right - left + 1,
            height: bottom - top + 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use moshionly_core::{Document, NodeId, io};

    fn plan(transparent: bool, gif_loop: Loop) -> Plan {
        let document: Document = io::parse(
            r#"{"format":"moshionly","version":1,"name":"T","nodes":[
                {"id":"board","parent":null,"index":"a0","type":"artboard","width":4,"height":3}
            ]}"#,
        )
        .unwrap();
        let mut plan = Settings::default()
            .with_gif_loop(gif_loop)
            .plan(crate::Format::Gif, &document, &NodeId::from("board"))
            .unwrap();
        plan.transparent = transparent;
        plan
    }

    fn written(transparent: bool, frames: &[(&[u8], u32)]) -> Vec<gif::Frame<'static>> {
        let plan = plan(transparent, Loop::Forever);
        let table = [0, 0, 0, 255, 255, 255, 255, 0, 0, 9, 9, 9];
        let encoder = Encoder::new(Vec::new(), 4, 3, &table).unwrap();
        let mut writer = FrameWriter::new(encoder, &plan, 3).unwrap();
        for (indices, delay) in frames {
            writer.push(indices.to_vec(), *delay).unwrap();
        }
        writer.flush().unwrap();
        let bytes = writer.encoder.into_inner().unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(bytes.as_slice()).unwrap();
        let mut out = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            out.push(frame.clone());
        }
        out
    }

    #[rustfmt::skip]
    const FIRST: [u8; 12] = [
        0, 0, 0, 0,
        0, 1, 1, 0,
        0, 0, 0, 0,
    ];
    #[rustfmt::skip]
    const SECOND: [u8; 12] = [
        0, 0, 0, 0,
        0, 1, 2, 0,
        0, 0, 2, 0,
    ];

    #[test]
    fn only_what_changed_is_stored() {
        let frames = written(false, &[(&FIRST, 3), (&FIRST, 4), (&SECOND, 3)]);
        // The repeated frame is merged into a longer first frame.
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].delay, 7);
        assert_eq!((frames[0].width, frames[0].height), (4, 3));
        assert_eq!(frames[0].transparent, None);
        // The second frame is the 1 by 2 rectangle that changed.
        let second = &frames[1];
        assert_eq!(
            (second.left, second.top, second.width, second.height),
            (2, 1, 1, 2)
        );
        assert_eq!(&*second.buffer, [2, 2]);
        assert_eq!(second.transparent, Some(3));
        assert_eq!(second.dispose, DisposalMethod::Keep);
        assert_eq!(second.delay, 3);
    }

    #[test]
    fn unchanged_pixels_inside_the_rectangle_are_see_through() {
        #[rustfmt::skip]
        let corners: [u8; 12] = [
            1, 0, 0, 0,
            0, 1, 1, 0,
            0, 0, 0, 1,
        ];
        let frames = written(false, &[(&FIRST, 2), (&corners, 2)]);
        assert_eq!((frames[1].width, frames[1].height), (4, 3));
        assert_eq!(&*frames[1].buffer, [1, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 1]);
    }

    #[test]
    fn transparent_gifs_clear_each_frame() {
        #[rustfmt::skip]
        let shape: [u8; 12] = [
            3, 3, 3, 3,
            3, 1, 2, 3,
            3, 3, 3, 3,
        ];
        let empty = [3; 12];
        let frames = written(true, &[(&shape, 2), (&empty, 2), (&shape, 2)]);
        assert_eq!(frames.len(), 3);
        assert_eq!(
            (
                frames[0].left,
                frames[0].top,
                frames[0].width,
                frames[0].height
            ),
            (1, 1, 2, 1)
        );
        assert!(
            frames
                .iter()
                .all(|frame| frame.dispose == DisposalMethod::Background)
        );
        assert!(frames.iter().all(|frame| frame.transparent == Some(3)));
        // A frame with nothing visible is one see-through pixel.
        assert_eq!((frames[1].width, frames[1].height), (1, 1));
        assert_eq!(&*frames[1].buffer, [3]);
    }

    #[test]
    fn loops_are_written_as_repeats() {
        let encoder = |gif_loop| {
            let plan = plan(false, gif_loop);
            let encoder = Encoder::new(Vec::new(), 4, 3, &[0, 0, 0]).unwrap();
            let mut writer = FrameWriter::new(encoder, &plan, 1).unwrap();
            writer.push(vec![0; 12], 10).unwrap();
            writer.flush().unwrap();
            let bytes = writer.encoder.into_inner().unwrap();
            let decoder = gif::DecodeOptions::new()
                .read_info(bytes.as_slice())
                .unwrap();
            decoder.repeat()
        };
        assert_eq!(encoder(Loop::Forever), Repeat::Infinite);
        assert_eq!(encoder(Loop::Times(3)), Repeat::Finite(2));
        // No loop extension at all: every viewer plays it once.
        assert_eq!(encoder(Loop::Once), Repeat::Finite(0));
    }

    #[test]
    fn rectangles_fit_tightly() {
        let inside = |x: usize, y: usize| (x, y) == (1, 2) || (x, y) == (3, 1);
        let rect = Rect::around(5, 4, inside).unwrap();
        assert_eq!(
            rect,
            Rect {
                x: 1,
                y: 1,
                width: 3,
                height: 2
            }
        );
        assert_eq!(Rect::around(5, 4, |_, _| false), None);
    }
}

//! One `ffmpeg` that reads a video's frames forward from a moment of it.
//!
//! The frames come out of its standard output as raw RGBA, one after the
//! other at the size asked for. Raw pixels carry no time, so `ffmpeg` is
//! also asked to describe each frame on its standard error (the
//! `showinfo` filter), and a thread reads those lines and passes on each
//! frame's timestamp in the order the frames come. Timestamps are kept as
//! they are in the file (`-copyts`), so a frame has the same time however
//! far before it reading started, and the time is exact: whole ticks of
//! the stream's time base.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use moshionly_render::skia_safe as sk;
use moshionly_render::video::image_from_rgba;

use super::probe::input;
use super::{MediaTools, seconds};
use crate::ffmpeg::last_words;

/// How much of what `ffmpeg` prints besides frame descriptions is kept for
/// error messages.
const MESSAGE_BYTES: usize = 2000;

/// What to say when `ffmpeg` is older than 5.1, which added `-fps_mode`.
const TOO_OLD: &str = "reading video needs ffmpeg 5.1 or newer, and this one is older; update \
     ffmpeg (on a Mac with Homebrew: brew upgrade ffmpeg)";

/// A running `ffmpeg` reading frames. Dropping it stops the program.
pub(crate) struct Stream {
    child: Child,
    stdout: ChildStdout,
    /// When each frame starts, in microseconds of media time, in order.
    times: Receiver<Result<i64, String>>,
    /// The end of what `ffmpeg` printed besides frame descriptions.
    said: Arc<Mutex<String>>,
    reader: Option<JoinHandle<()>>,
    width: u32,
    height: u32,
    /// Where reading started, in microseconds of media time.
    pub seek_us: i64,
    /// How many frames have been read.
    pub frames: u64,
    /// The frame before the last one read, and when it starts.
    pub prev: Option<(i64, sk::Image)>,
    /// The last frame read, and when it starts.
    pub last: Option<(i64, sk::Image)>,
    /// Whether the file has no more frames.
    pub ended: bool,
    /// When the stream was last used, for choosing one to replace.
    pub used: u64,
}

impl Stream {
    /// Starts reading the file at `path` at `seek_us` microseconds into it
    /// (0 is the start of the file's timeline, `start_us` in its
    /// timestamps), as `width` by `height` frames.
    pub fn start(
        tools: &MediaTools,
        path: &Path,
        start_us: i64,
        (width, height): (u32, u32),
        seek_us: i64,
    ) -> Result<Stream, String> {
        let mut command = Command::new(tools.ffmpeg());
        command.args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"]);
        // Keep the file's own timestamps, so each frame is known by the
        // same time whichever moment reading starts at.
        command.arg("-copyts");
        if seek_us > 0 {
            command.args(["-ss", &seconds(seek_us)]);
        }
        command.arg("-i").arg(input(path));
        // The first video that is not cover art, and nothing else.
        command.args(["-map", "0:V:0", "-an", "-sn", "-dn"]);
        // One frame out for every frame decoded, described as it passes.
        let filter = format!("showinfo,scale={width}:{height},format=rgba");
        command.args(["-vf", &filter, "-fps_mode", "passthrough"]);
        command.args(["-f", "rawvideo", "pipe:1"]);
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                format!(
                    "{} could not be started ({error})",
                    tools.ffmpeg().display()
                )
            })?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("ffmpeg could not be talked to".into());
        };
        let (sender, times) = mpsc::channel();
        let said = Arc::new(Mutex::new(String::new()));
        let reader = {
            let said = Arc::clone(&said);
            thread::spawn(move || read_descriptions(stderr, start_us, &sender, &said))
        };
        Ok(Stream {
            child,
            stdout,
            times,
            said,
            reader: Some(reader),
            width,
            height,
            seek_us,
            frames: 0,
            prev: None,
            last: None,
            ended: false,
            used: 0,
        })
    }

    /// Reads the next frame and when it starts, or `None` when the file
    /// has no more.
    pub fn next_frame(&mut self) -> Result<Option<(i64, sk::Image)>, String> {
        let mut pixels = vec![0; self.width as usize * self.height as usize * 4];
        let mut filled = 0;
        while filled < pixels.len() {
            match self.stdout.read(&mut pixels[filled..]) {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(self.failure(&error.to_string())),
            }
        }
        if filled == 0 {
            // The end, or ffmpeg failed before its first frame. Once it has
            // stopped, everything it said has been read.
            let status = self.child.wait();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
            let failed = status.map_or(true, |status| !status.success());
            if failed && self.frames == 0 {
                return Err(self.failure("ffmpeg stopped without giving a frame"));
            }
            return Ok(None);
        }
        if filled < pixels.len() {
            return Err(self.failure("ffmpeg stopped in the middle of a frame"));
        }
        let start = match self.times.recv() {
            Ok(Ok(start)) => start,
            Ok(Err(problem)) => return Err(problem),
            Err(_) => return Err(self.failure("ffmpeg did not say when a frame is")),
        };
        let image = image_from_rgba(self.width, self.height, pixels)?;
        Ok(Some((start, image)))
    }

    /// What ffmpeg said, or `fallback` when it said nothing.
    fn failure(&self, fallback: &str) -> String {
        explained(self.said(), fallback)
    }

    fn said(&self) -> String {
        let said = self.said.lock().unwrap_or_else(PoisonError::into_inner);
        last_words(&said, MESSAGE_BYTES).to_owned()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Why `ffmpeg` failed: what it `said`, put plainly when it is too old to
/// read frames this way, or `fallback` when it said nothing.
fn explained(said: String, fallback: &str) -> String {
    if said.contains("Unrecognized option 'fps_mode'") {
        TOO_OLD.to_owned()
    } else if said.is_empty() {
        fallback.to_owned()
    } else {
        said
    }
}

/// Reads what `ffmpeg` prints: frame descriptions become start times sent
/// to `times`, the rest is kept in `said`.
fn read_descriptions(
    mut stderr: impl Read,
    start_us: i64,
    times: &mpsc::Sender<Result<i64, String>>,
    said: &Mutex<String>,
) {
    let mut time_base = None;
    let mut pending = Vec::new();
    let mut buffer = [0; 8192];
    while let Ok(read @ 1..) = stderr.read(&mut buffer) {
        pending.extend_from_slice(&buffer[..read]);
        while let Some(end) = pending
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r')
        {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            match describe(&line) {
                Line::TimeBase(base) => time_base = Some(base),
                Line::Frame(pts) => {
                    let start = match (pts, time_base) {
                        (Some(pts), Some((num, den))) if den != 0 => {
                            Ok(micros_of(pts, num, den) - start_us)
                        }
                        _ => Err("ffmpeg gave a frame without a time".to_owned()),
                    };
                    if times.send(start).is_err() {
                        return;
                    }
                }
                Line::Other if !line.trim().is_empty() => {
                    let mut said = said.lock().unwrap_or_else(PoisonError::into_inner);
                    said.push_str(line.trim_end());
                    said.push('\n');
                    if said.len() > 4 * MESSAGE_BYTES {
                        let cut = said.len() - MESSAGE_BYTES;
                        let cut = (cut..said.len())
                            .find(|at| said.is_char_boundary(*at))
                            .unwrap_or(said.len());
                        said.drain(..cut);
                    }
                }
                Line::Other => {}
            }
        }
    }
}

/// A line `ffmpeg` printed while reading frames.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// The time base of the frames' timestamps: numerator, denominator.
    TimeBase((i64, i64)),
    /// A frame went past, with its timestamp in the time base (`None`
    /// when it has none).
    Frame(Option<i64>),
    /// Anything else.
    Other,
}

/// Reads one line of `ffmpeg`'s output.
fn describe(line: &str) -> Line {
    if !line.contains("Parsed_showinfo") {
        return Line::Other;
    }
    if let Some((_, rest)) = line.split_once("config in time_base:") {
        let base = rest.split([',', ' ']).find(|part| !part.is_empty());
        if let Some((num, den)) = base.and_then(|base| base.split_once('/'))
            && let (Ok(num), Ok(den)) = (num.parse(), den.parse())
        {
            return Line::TimeBase((num, den));
        }
        return Line::Other;
    }
    if line.contains(" n:")
        && let Some((_, rest)) = line.split_once(" pts:")
    {
        let pts = rest
            .split_whitespace()
            .next()
            .and_then(|pts| pts.parse().ok());
        return Line::Frame(pts);
    }
    Line::Other
}

/// A timestamp of `pts` ticks of `num / den` seconds, in whole
/// microseconds, rounded to the nearest.
fn micros_of(pts: i64, num: i64, den: i64) -> i64 {
    let scaled = i128::from(pts) * i128::from(num) * 1_000_000;
    let (scaled, den) = if den < 0 {
        (-scaled, -i128::from(den))
    } else {
        (scaled, i128::from(den))
    };
    // Halves round away from zero.
    let rounded = if scaled >= 0 {
        (scaled + den / 2) / den
    } else {
        -((-scaled + den / 2) / den)
    };
    rounded.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_descriptions_give_timestamps() {
        assert_eq!(
            describe(
                "[Parsed_showinfo_0 @ 0x76630395c0] config in time_base: 1/10240, frame_rate: 10/1"
            ),
            Line::TimeBase((1, 10240))
        );
        assert_eq!(
            describe(
                "[Parsed_showinfo_0 @ 0x76630395c0] n:   0 pts:  21504 pts_time:2.1     \
                 duration:   1024 duration_time:0.1     fmt:yuv420p"
            ),
            Line::Frame(Some(21504))
        );
        assert_eq!(
            describe("[Parsed_showinfo_0 @ 0x1] n:  12 pts: NOPTS pts_time:NOPTS"),
            Line::Frame(None)
        );
        assert_eq!(
            describe("[Parsed_showinfo_0 @ 0x1] color_range:unknown color_space:unknown"),
            Line::Other
        );
        assert_eq!(
            describe("Stream #0:0 -> #0:0 (mpeg4 (native) -> rawvideo (native))"),
            Line::Other
        );
    }

    #[test]
    fn an_ffmpeg_too_old_to_read_frames_is_said_to_be() {
        // What ffmpeg 5.0 and older say to -fps_mode.
        let said = "Unrecognized option 'fps_mode'.\n\
                    Error splitting the argument list: Option not found";
        let message = explained(said.to_owned(), "no frame");
        assert!(message.contains("needs ffmpeg 5.1 or newer"), "{message}");
        assert_eq!(
            explained("Invalid data".to_owned(), "no frame"),
            "Invalid data"
        );
        assert_eq!(explained(String::new(), "no frame"), "no frame");
    }

    #[test]
    fn timestamps_become_exact_microseconds() {
        assert_eq!(micros_of(21504, 1, 10240), 2_100_000);
        assert_eq!(micros_of(1001, 1, 30000), 33_367);
        assert_eq!(micros_of(-1024, 1, 10240), -100_000);
        assert_eq!(micros_of(1, 1, 3), 333_333);
        assert_eq!(micros_of(2, 1, 3), 666_667);
    }

    #[test]
    fn descriptions_are_read_from_a_stream_of_text() {
        let text = "Input #0, mov\n\
            [Parsed_showinfo_0 @ 0x1] config in time_base: 1/1000, frame_rate: 25/1\n\
            [Parsed_showinfo_0 @ 0x1] n:   0 pts:   1540 pts_time:1.54\r\n\
            [Parsed_showinfo_0 @ 0x1] n:   1 pts:   1580 pts_time:1.58\n\
            Something went wrong\n";
        let (sender, times) = mpsc::channel();
        let said = Mutex::new(String::new());
        read_descriptions(text.as_bytes(), 1_500_000, &sender, &said);
        drop(sender);
        let times: Vec<_> = times.iter().collect();
        assert_eq!(times, [Ok(40_000), Ok(80_000)]);
        let said = said.into_inner().unwrap();
        assert!(said.contains("Something went wrong"), "{said}");
        assert!(!said.contains("pts:"), "{said}");
    }
}

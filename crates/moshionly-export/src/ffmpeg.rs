//! WebM (VP9) and ProRes 4444 (QuickTime), written by `ffmpeg`.
//!
//! These formats need encoders that are too large to build in, so an
//! installed `ffmpeg` writes them. Frames are rendered as usual and piped
//! to its standard input as raw straight-alpha RGBA; `ffmpeg` converts them
//! to BT.709 Y'CbCr, encodes them and writes the file. Both formats keep
//! transparency when the plan asks for it.
//!
//! `ffmpeg` runs as a separate program, so its licence does not extend to
//! Moshionly. It is found on `PATH` and in the places package managers put
//! it, since an app started from the Finder does not see the shell's
//! `PATH`.

use std::env;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use moshionly_render::Frame;

use crate::error::ExportError;
use crate::format::Format;
use crate::frames::{self, Pass};
use crate::job::Context;
use crate::progress::{CancelToken, Phase};
use crate::settings::{Plan, Quality};

/// Where `ffmpeg` is commonly installed, besides the folders on `PATH`.
const USUAL_PLACES: [&str; 5] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/bin",
    "/snap/bin",
];

/// How much of what `ffmpeg` prints on failure is kept for the message.
const MESSAGE_BYTES: usize = 2000;

/// How much of what a broken `ffmpeg` prints is kept when it cannot even
/// list its encoders, for a menu to show beside a greyed-out format.
const BROKEN_BYTES: usize = 400;

/// An installed `ffmpeg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    path: PathBuf,
}

impl Ffmpeg {
    /// Finds `ffmpeg` in the folders on `PATH`, then in the usual install
    /// places. `None` when it is not installed.
    pub fn locate() -> Option<Ffmpeg> {
        find_program("ffmpeg").map(|path| Ffmpeg { path })
    }

    /// The `ffmpeg` at `path`, for example one the person chose in the
    /// app's settings. Whether it works is checked when it is used.
    pub fn at(path: impl Into<PathBuf>) -> Ffmpeg {
        Ffmpeg { path: path.into() }
    }

    /// Where the program is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether this `ffmpeg` can write `format`, or why not.
    pub fn check(&self, format: Format) -> Result<(), String> {
        self.encoders()
            .and_then(|encoders| encoders.check(&self.path, format))
    }

    /// The encoders this `ffmpeg` was built with, or why it could not say:
    /// it cannot be started, or it starts and fails, for example because an
    /// upgrade removed a library it needs.
    pub(crate) fn encoders(&self) -> Result<Encoders, String> {
        let output = Command::new(&self.path)
            .args(["-hide_banner", "-encoders"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                format!(
                    "{} could not be started ({error}); {}",
                    self.path.display(),
                    INSTALL_HINT
                )
            })?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            let said = last_words(&said, BROKEN_BYTES);
            let said = if said.is_empty() {
                String::new()
            } else {
                format!(": {said}")
            };
            return Err(format!(
                "{} failed to run ({}){said}; it may be damaged, so reinstall ffmpeg (on a Mac \
                 with Homebrew: brew reinstall ffmpeg)",
                self.path.display(),
                output.status
            ));
        }
        let listing = String::from_utf8_lossy(&output.stdout);
        Ok(Encoders(
            listing
                .lines()
                .filter_map(|line| line.split_whitespace().nth(1))
                .map(str::to_owned)
                .collect(),
        ))
    }
}

/// The names of the encoders an `ffmpeg` has.
pub(crate) struct Encoders(Vec<String>);

impl Encoders {
    /// Whether these encoders can write `format`, or why not.
    pub(crate) fn check(&self, ffmpeg: &Path, format: Format) -> Result<(), String> {
        let Some(encoder) = encoder_for(format) else {
            return Err(format!("{format} is not written with ffmpeg"));
        };
        if self.0.iter().any(|name| name == encoder) {
            Ok(())
        } else {
            Err(format!(
                "the ffmpeg at {} was built without the {encoder} encoder; install a full \
                 build of ffmpeg",
                ffmpeg.display()
            ))
        }
    }
}

/// How to get `ffmpeg`.
pub(crate) const INSTALL_HINT: &str = "install ffmpeg from https://ffmpeg.org, or on a Mac \
     with Homebrew: brew install ffmpeg";

/// The `ffmpeg` encoder a format needs.
fn encoder_for(format: Format) -> Option<&'static str> {
    match format {
        Format::WebM => Some("libvpx-vp9"),
        Format::ProRes => Some("prores_ks"),
        _ => None,
    }
}

/// Finds the program `name` (`ffmpeg`, `ffprobe`) in the folders on
/// `PATH`, then in the usual install places.
pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let path_dirs = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path_dirs)
        .chain(USUAL_PLACES.iter().map(PathBuf::from))
        .map(|dir| dir.join(&file))
        .find(|path| is_program(path))
}

/// Whether `path` is a file this process may run.
pub(crate) fn is_program(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// The `ffmpeg` command line that reads frames from standard input and
/// writes `plan`'s video to `out`.
fn arguments(plan: &Plan, out: &Path) -> Vec<OsString> {
    let rate = plan
        .moments
        .frames()
        .map(|times| times.rate())
        .unwrap_or_default();
    let mut args: Vec<OsString> = Vec::new();
    let mut add = |values: &[&str]| args.extend(values.iter().map(OsString::from));
    add(&["-hide_banner", "-loglevel", "error", "-nostats", "-y"]);
    let size = format!("{}x{}", plan.width, plan.height);
    let fps = format!("{}/{}", rate.numerator(), rate.denominator());
    add(&[
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &size,
        "-framerate",
        &fps,
    ]);
    add(&["-i", "pipe:0"]);
    // Options after the input are for the output. No random ids or version
    // strings, so the same export gives the same file.
    add(&["-fflags", "+bitexact", "-flags:v", "+bitexact"]);
    let key_interval = (2.0 * rate.fps()).round().max(1.0).to_string();
    let pixel_format = match (plan.format, plan.transparent) {
        (Format::ProRes, true) => "yuva444p10le",
        (Format::ProRes, false) => "yuv444p10le",
        (_, true) => "yuva420p",
        (_, false) => "yuv420p",
    };
    // Convert with the BT.709 matrix, and say so in the file: primaries,
    // transfer and matrix, so players show the colours as rendered.
    let filter = format!(
        "scale=out_color_matrix=bt709:out_range=tv,format={pixel_format},\
         setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv"
    );
    add(&["-vf", &filter, "-pix_fmt", pixel_format]);
    add(&[
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-colorspace",
        "bt709",
    ]);
    add(&["-color_range", "tv"]);
    match plan.format {
        Format::ProRes => {
            let (profile, quantiser) = match plan.quality {
                Quality::Draft => ("4444", Some("11")),
                Quality::Standard => ("4444", None),
                Quality::High => ("4444xq", None),
            };
            add(&[
                "-c:v",
                "prores_ks",
                "-profile:v",
                profile,
                "-vendor",
                "apl0",
            ]);
            if let Some(quantiser) = quantiser {
                add(&["-qscale:v", quantiser]);
            }
            if plan.transparent {
                add(&["-alpha_bits", "16"]);
            }
            add(&["-movflags", "+faststart", "-f", "mov"]);
        }
        _ => {
            let (crf, speed) = match plan.quality {
                Quality::Draft => ("40", "5"),
                Quality::Standard => ("30", "3"),
                Quality::High => ("20", "1"),
            };
            add(&["-c:v", "libvpx-vp9", "-crf", crf, "-b:v", "0"]);
            add(&["-deadline", "good", "-cpu-used", speed, "-row-mt", "1"]);
            add(&["-g", &key_interval]);
            if plan.transparent {
                // Alternate reference frames do not carry transparency.
                add(&["-auto-alt-ref", "0"]);
            }
            add(&["-f", "webm"]);
        }
    }
    args.push(out.as_os_str().to_owned());
    args
}

/// Writes the WebM or ProRes video of `cx.plan` to `path` with `ffmpeg`.
pub(crate) fn write(cx: &mut Context<'_>, path: &Path, ffmpeg: &Ffmpeg) -> Result<(), ExportError> {
    let plan = cx.plan;
    let format = plan.format;
    let cancel = cx.reporter.cancel_token().clone();
    let mut child = Running::start(ffmpeg, &arguments(plan, path), format, cancel)?;

    let pass = Pass {
        moments: plan.moments,
        options: plan.render,
        frame_bytes: plan.width as usize * plan.height as usize * 8,
        report_progress: true,
    };
    let prepare = |_: u64, frame: Frame| Ok(frame.to_unpremultiplied());
    let mut consume = |_: u64, pixels: Vec<u8>| child.feed(&pixels);
    cx.reporter.report(Phase::Rendering, 0);
    frames::render_in_order(cx.source, &pass, &mut cx.reporter, &prepare, &mut consume)?;
    cx.reporter.report(Phase::Encoding, plan.frame_count());
    child.finish()
}

/// How often a running `ffmpeg` is checked on: whether the export was
/// cancelled, and whether it has finished.
const CHECK_EVERY: Duration = Duration::from_millis(20);

/// A running `ffmpeg`. Dropping it stops the program, so an export that
/// fails or is cancelled never leaves one behind.
///
/// Cancelling stops it at once, whatever the export is doing at the time:
/// a watcher thread stops the program as soon as the export is cancelled,
/// which also ends a write that is waiting for `ffmpeg` to take a frame,
/// and waiting for `ffmpeg` to finish the file gives up on cancellation.
struct Running {
    /// Shared with the watcher, which stops it on cancellation.
    child: Arc<Mutex<Child>>,
    stdin: Option<ChildStdin>,
    errors: Option<JoinHandle<String>>,
    watcher: Option<Watcher>,
    cancel: CancelToken,
    format: Format,
}

/// The thread that stops `ffmpeg` when the export is cancelled.
struct Watcher {
    /// Dropping it tells the thread to end.
    done: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

impl Running {
    fn start(
        ffmpeg: &Ffmpeg,
        args: &[OsString],
        format: Format,
        cancel: CancelToken,
    ) -> Result<Self, ExportError> {
        let mut child = Command::new(ffmpeg.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| ExportError::FfmpegMissing {
                format,
                reason: format!(
                    "{} could not be started ({error}); {INSTALL_HINT}",
                    ffmpeg.path().display()
                ),
            })?;
        let stdin = child.stdin.take();
        // Read what ffmpeg says as it goes, so it never blocks on a full
        // pipe, and keep the end of it for the error message.
        let errors = child.stderr.take().map(|mut stderr| {
            thread::spawn(move || {
                let mut text = Vec::new();
                let mut buffer = [0; 4096];
                while let Ok(read @ 1..) = stderr.read(&mut buffer) {
                    text.extend_from_slice(&buffer[..read]);
                    if text.len() > 4 * MESSAGE_BYTES {
                        text.drain(..text.len() - MESSAGE_BYTES);
                    }
                }
                String::from_utf8_lossy(&text).into_owned()
            })
        });
        let child = Arc::new(Mutex::new(child));
        let watcher = Watcher::start(Arc::clone(&child), cancel.clone());
        Ok(Self {
            child,
            stdin,
            errors,
            watcher: Some(watcher),
            cancel,
            format,
        })
    }

    /// Hands one frame to ffmpeg.
    fn feed(&mut self, pixels: &[u8]) -> Result<(), ExportError> {
        let written = match &mut self.stdin {
            Some(stdin) => stdin.write_all(pixels),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        };
        match written {
            Ok(()) => Ok(()),
            // The watcher stopped ffmpeg, which is why the write failed.
            Err(_) if self.cancel.is_cancelled() => Err(ExportError::Cancelled),
            // ffmpeg stopped on its own; its own message says why.
            Err(_) => Err(self.failure()),
        }
    }

    /// Lets ffmpeg finish the file and checks that it did.
    fn finish(mut self) -> Result<(), ExportError> {
        drop(self.stdin.take());
        if self.wait()?.success() {
            Ok(())
        } else {
            Err(self.failure())
        }
    }

    /// Waits for ffmpeg to end, giving up as soon as the export is
    /// cancelled; dropping `self` then stops it.
    fn wait(&self) -> Result<ExitStatus, ExportError> {
        loop {
            if self.cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            match lock(&self.child).try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => thread::sleep(CHECK_EVERY),
                Err(error) => {
                    return Err(ExportError::Ffmpeg {
                        format: self.format,
                        message: error.to_string(),
                    });
                }
            }
        }
    }

    /// Stops ffmpeg and says why it failed, in its own words.
    fn failure(&mut self) -> ExportError {
        drop(self.stdin.take());
        let status = match self.wait() {
            Ok(status) => Some(status),
            Err(ExportError::Cancelled) => return ExportError::Cancelled,
            Err(_) => None,
        };
        let said = self
            .errors
            .take()
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        let said = last_words(&said, MESSAGE_BYTES);
        let message = if !said.is_empty() {
            said.to_owned()
        } else {
            match status.and_then(|status| status.code()) {
                Some(code) => format!("ffmpeg stopped with exit code {code}"),
                None => "ffmpeg stopped unexpectedly".to_owned(),
            }
        };
        ExportError::Ffmpeg {
            format: self.format,
            message,
        }
    }
}

impl Watcher {
    /// Watches `cancel` and stops `child` when it is cancelled, until
    /// told to end.
    fn start(child: Arc<Mutex<Child>>, cancel: CancelToken) -> Self {
        let (done, ended) = mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            while let Err(RecvTimeoutError::Timeout) = ended.recv_timeout(CHECK_EVERY) {
                if cancel.is_cancelled() {
                    let _ = lock(&child).kill();
                    return;
                }
            }
        });
        Self { done, thread }
    }

    /// Ends the thread and waits for it.
    fn end(self) {
        drop(self.done);
        let _ = self.thread.join();
    }
}

/// The child, even if a thread panicked while holding it.
fn lock(child: &Mutex<Child>) -> MutexGuard<'_, Child> {
    child.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The end of what a program printed, at most `limit` bytes, without
/// surrounding blank space.
pub(crate) fn last_words(said: &str, limit: usize) -> &str {
    let said = said.trim();
    let start = said.len().saturating_sub(limit);
    let start = (start..said.len())
        .find(|at| said.is_char_boundary(*at))
        .unwrap_or(said.len());
    said[start..].trim_start()
}

impl Drop for Running {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Some(watcher) = self.watcher.take() {
            watcher.end();
        }
        let mut child = lock(&self.child);
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use moshionly_core::{NodeId, io};

    fn plan(format: Format, settings: Settings) -> Plan {
        let document = io::parse(
            r#"{"format":"moshionly","version":1,"name":"T","nodes":[
                {"id":"board","parent":null,"index":"a0","type":"artboard","width":200,
                 "height":100,"duration":1000}
            ]}"#,
        )
        .unwrap();
        settings
            .plan(format, &document, &NodeId::from("board"))
            .unwrap()
    }

    fn joined(args: &[OsString]) -> String {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn webm_keeps_alpha_when_transparent() {
        let args = joined(&arguments(
            &plan(Format::WebM, Settings::default()),
            Path::new("/out/clip.part"),
        ));
        assert!(
            args.contains("-s 200x100 -framerate 30/1 -i pipe:0"),
            "{args}"
        );
        assert!(args.contains("-c:v libvpx-vp9 -crf 30"), "{args}");
        assert!(args.contains("setparams=color_primaries=bt709"), "{args}");
        assert!(args.contains("-fflags +bitexact"), "{args}");
        assert!(args.contains("format=yuva420p"), "{args}");
        assert!(args.contains("-auto-alt-ref 0"), "{args}");
        assert!(args.contains("-colorspace bt709"), "{args}");
        assert!(args.ends_with("-f webm /out/clip.part"), "{args}");
    }

    #[test]
    fn prores_follows_quality_and_transparency() {
        let opaque = Settings::default()
            .with_background(moshionly_render::Background::Over(
                moshionly_core::Color::BLACK,
            ))
            .with_quality(Quality::High);
        let args = joined(&arguments(&plan(Format::ProRes, opaque), Path::new("x")));
        assert!(args.contains("-profile:v 4444xq"), "{args}");
        assert!(args.contains("-pix_fmt yuv444p10le"), "{args}");
        assert!(!args.contains("alpha_bits"), "{args}");
        let clear = Settings::default().with_frame_rate(crate::FrameRate::FPS_29_97);
        let args = joined(&arguments(&plan(Format::ProRes, clear), Path::new("x")));
        assert!(args.contains("-framerate 30000/1001"), "{args}");
        assert!(args.contains("-pix_fmt yuva444p10le"), "{args}");
        assert!(args.contains("-alpha_bits 16"), "{args}");
        assert!(args.contains("-f mov"), "{args}");
    }

    #[test]
    fn only_the_last_words_are_kept() {
        assert_eq!(last_words("  all is well \n", 100), "all is well");
        assert_eq!(last_words("first line\nlast line", 9), "last line");
        // Never in the middle of a character: "é" is two bytes.
        assert_eq!(last_words("café", 1), "");
        assert_eq!(last_words("café", 2), "é");
        assert_eq!(last_words("café", 3), "fé");
        assert_eq!(last_words("", 10), "");
    }

    #[test]
    fn a_missing_program_is_explained() {
        let missing = Ffmpeg::at("/nowhere/ffmpeg");
        let reason = missing.check(Format::WebM).unwrap_err();
        assert!(reason.contains("brew install ffmpeg"), "{reason}");
        assert!(missing.check(Format::Mp4).is_err());
        assert!(!is_program(Path::new("/nowhere/ffmpeg")));
    }
}

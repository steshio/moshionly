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
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::{self, JoinHandle};

use moshionly_render::Frame;

use crate::error::ExportError;
use crate::format::Format;
use crate::frames::{self, Pass};
use crate::job::Context;
use crate::progress::Phase;
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

/// An installed `ffmpeg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    path: PathBuf,
}

impl Ffmpeg {
    /// Finds `ffmpeg` in the folders on `PATH`, then in the usual install
    /// places. `None` when it is not installed.
    pub fn locate() -> Option<Ffmpeg> {
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let path_dirs = env::var_os("PATH").unwrap_or_default();
        env::split_paths(&path_dirs)
            .chain(USUAL_PLACES.iter().map(PathBuf::from))
            .map(|dir| dir.join(name))
            .find(|path| is_program(path))
            .map(|path| Ffmpeg { path })
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

    /// The encoders this `ffmpeg` was built with.
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

/// Whether `path` is a file this process may run.
fn is_program(path: &Path) -> bool {
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
    let mut child = Running::start(ffmpeg, &arguments(plan, path), format)?;

    let moments = plan.moments.to_vec();
    let pass = Pass {
        moments: &moments,
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

/// A running `ffmpeg`. Dropping it stops the program, so an export that
/// fails or is cancelled never leaves one behind.
struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    errors: Option<JoinHandle<String>>,
    format: Format,
}

impl Running {
    fn start(ffmpeg: &Ffmpeg, args: &[OsString], format: Format) -> Result<Self, ExportError> {
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
        Ok(Self {
            child,
            stdin,
            errors,
            format,
        })
    }

    /// Hands one frame to ffmpeg.
    fn feed(&mut self, pixels: &[u8]) -> Result<(), ExportError> {
        let written = match &mut self.stdin {
            Some(stdin) => stdin.write_all(pixels),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        };
        // A write fails when ffmpeg has stopped; its own message says why.
        written.map_err(|_| self.failure())
    }

    /// Lets ffmpeg finish the file and checks that it did.
    fn finish(mut self) -> Result<(), ExportError> {
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|error| ExportError::Ffmpeg {
            format: self.format,
            message: error.to_string(),
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(self.failure())
        }
    }

    /// Stops ffmpeg and says why it failed, in its own words.
    fn failure(&mut self) -> ExportError {
        drop(self.stdin.take());
        let status = self.child.wait().ok();
        let said = self
            .errors
            .take()
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        let said = said.trim();
        let message = if !said.is_empty() {
            let start = said.len().saturating_sub(MESSAGE_BYTES);
            let start = (start..said.len())
                .find(|at| said.is_char_boundary(*at))
                .unwrap_or(said.len());
            said[start..].to_owned()
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

impl Drop for Running {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
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
    fn a_missing_program_is_explained() {
        let missing = Ffmpeg::at("/nowhere/ffmpeg");
        let reason = missing.check(Format::WebM).unwrap_err();
        assert!(reason.contains("brew install ffmpeg"), "{reason}");
        assert!(missing.check(Format::Mp4).is_err());
        assert!(!is_program(Path::new("/nowhere/ffmpeg")));
    }
}

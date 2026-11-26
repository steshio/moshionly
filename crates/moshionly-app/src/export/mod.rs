//! Exporting from the editor: what the Export dialog's choices mean as an
//! export job, and running that job off the UI thread.
//!
//! - [`ExportChoices`]: what the person picked, remembered between runs.
//! - [`ExportSetup`]: those choices for one artboard, turned into
//!   `moshionly-export` [`Settings`] and an [`ExportJob`].
//! - [`run`]: the job on a thread of its own, with progress and Cancel.
//!
//! The dialog itself is drawn by the shell.

pub mod run;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use moshionly_core::{Document, NodeId};
use moshionly_export::{Background, ExportJob, Format, FrameRate, Loop, Quality, Settings, Size};
use serde::{Deserialize, Serialize};

pub use run::{ExportRun, RunOutcome};

/// The sizes offered, as multiples of the artboard's size.
pub const SCALES: [(f64, &str); 5] = [
    (0.5, "0.5×"),
    (1.0, "1×"),
    (2.0, "2×"),
    (3.0, "3×"),
    (4.0, "4×"),
];

/// The frame rates offered, in frames per second.
pub const FRAME_RATES: [(f64, &str); 7] = [
    (12.0, "12 fps"),
    (15.0, "15 fps"),
    (24.0, "24 fps"),
    (25.0, "25 fps"),
    (30.0, "30 fps"),
    (50.0, "50 fps"),
    (60.0, "60 fps"),
];

/// How many times a GIF plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GifLoop {
    /// Over and over.
    #[default]
    Forever,
    /// Once.
    Once,
    /// This many times.
    Times(u16),
}

impl GifLoop {
    /// The loops offered.
    pub const OFFERED: [(Self, &'static str); 4] = [
        (Self::Forever, "Forever"),
        (Self::Once, "Once"),
        (Self::Times(2), "Twice"),
        (Self::Times(3), "3 times"),
    ];

    /// The exporter's loop.
    pub fn to_loop(self) -> Loop {
        match self {
            Self::Forever => Loop::Forever,
            Self::Once => Loop::Once,
            Self::Times(times) => Loop::Times(times.max(1)),
        }
    }
}

/// What the person chose in the Export dialog. Kept with the settings, so
/// the dialog opens as it was left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportChoices {
    /// The format's short name, such as `mp4`.
    pub format: String,
    /// The size, as a multiple of the artboard's size.
    pub scale: f64,
    /// Frames per second.
    pub fps: f64,
    /// `draft`, `standard` or `high`.
    pub quality: String,
    /// Leave the background out, for formats that can.
    pub transparent: bool,
    /// How many times a GIF plays.
    pub gif_loop: GifLoop,
    /// A PNG captures the playhead's moment rather than the design at rest.
    pub still_at_playhead: bool,
    /// MP4, WebM and ProRes files carry the artboard's sound, when it has
    /// any.
    pub sound: bool,
}

impl Default for ExportChoices {
    /// An MP4 at the artboard's size, 30 frames per second, standard
    /// quality, on the artboard's background, with its sound.
    fn default() -> Self {
        Self {
            format: Format::Mp4.as_str().to_owned(),
            scale: 1.0,
            fps: 30.0,
            quality: Quality::Standard.as_str().to_owned(),
            transparent: false,
            gif_loop: GifLoop::Forever,
            still_at_playhead: false,
            sound: true,
        }
    }
}

impl ExportChoices {
    /// The chosen format; MP4 when what was remembered is not a format.
    pub fn format(&self) -> Format {
        Format::parse(&self.format).unwrap_or(Format::Mp4)
    }

    /// Chooses the format.
    pub fn set_format(&mut self, format: Format) {
        format.as_str().clone_into(&mut self.format);
    }

    /// The chosen quality; standard when what was remembered is not one.
    pub fn quality(&self) -> Quality {
        Quality::parse(&self.quality).unwrap_or(Quality::Standard)
    }

    /// Chooses the quality.
    pub fn set_quality(&mut self, quality: Quality) {
        quality.as_str().clone_into(&mut self.quality);
    }

    /// The chosen frame rate; 30 when what was remembered is not usable.
    pub fn frame_rate(&self) -> FrameRate {
        FrameRate::from_fps(self.fps).unwrap_or(FrameRate::FPS_30)
    }

    /// The chosen scale; 1 when what was remembered is not usable.
    pub fn scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        }
    }
}

/// The Export dialog's choices for one artboard at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSetup {
    /// What the person chose.
    pub choices: ExportChoices,
    /// The artboard to export.
    pub artboard: NodeId,
    /// Only this part of the scene, in milliseconds; `None` for all of it.
    pub range: Option<(f64, f64)>,
    /// The playhead, in milliseconds, for a still of the current moment.
    pub playhead: f64,
}

impl ExportSetup {
    /// The exporter's settings for these choices.
    pub fn settings(&self) -> Settings {
        let choices = &self.choices;
        let format = choices.format();
        let mut settings = Settings::default()
            .with_size(Size::Scale(choices.scale()))
            .with_frame_rate(choices.frame_rate())
            .with_quality(choices.quality())
            .with_gif_loop(choices.gif_loop.to_loop())
            .with_sound(choices.sound);
        if choices.transparent && format.supports_transparency() {
            settings = settings.with_background(Background::Transparent);
        }
        if format.is_animated() {
            if let Some((start, end)) = self.range {
                settings = settings.with_range(Some(start), Some(end));
            }
        } else {
            settings = settings.with_still(choices.still_at_playhead.then_some(self.playhead));
        }
        settings
    }

    /// The export job writing `document` to `destination`. The caller adds
    /// the project file and the renderer.
    pub fn job(&self, document: Arc<Document>, destination: PathBuf) -> ExportJob {
        ExportJob::new(
            document,
            self.artboard.clone(),
            self.choices.format(),
            destination,
        )
        .with_settings(self.settings())
    }
}

/// A size in bytes as people read it: `980 bytes`, `14 KB`, `1.2 MB`.
pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return if bytes == 1 {
            "1 byte".to_owned()
        } else {
            format!("{bytes} bytes")
        };
    }
    let mut size = bytes as f64 / 1000.0;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < UNITS.len() {
        size /= 1000.0;
        unit += 1;
    }
    if size < 10.0 {
        format!("{size:.1} {}", UNITS[unit])
    } else {
        format!("{size:.0} {}", UNITS[unit])
    }
}

/// What the file manager is called here, for "Show in …".
pub fn file_manager_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(target_os = "windows") {
        "Explorer"
    } else {
        "folder"
    }
}

/// Shows `path` in the system's file manager: selected in the Finder on a
/// Mac or in Explorer on Windows, and its folder opened elsewhere.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("explorer");
        command.arg(format!("/select,{}", path.display()));
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    };
    command.spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use moshionly_export::Moment;

    use super::*;

    fn setup(choices: ExportChoices) -> ExportSetup {
        ExportSetup {
            choices,
            artboard: NodeId::from("scene"),
            range: None,
            playhead: 1500.0,
        }
    }

    #[test]
    fn the_dialogs_choices_become_the_export_settings() {
        let mut choices = ExportChoices {
            scale: 2.0,
            fps: 60.0,
            transparent: true,
            gif_loop: GifLoop::Times(3),
            ..ExportChoices::default()
        };
        choices.set_format(Format::Gif);
        choices.set_quality(Quality::High);
        let mut setup = setup(choices);
        setup.range = Some((500.0, 2500.0));
        let settings = setup.settings();
        assert_eq!(settings.size, Size::Scale(2.0));
        assert_eq!(settings.frame_rate, FrameRate::FPS_60);
        assert_eq!(settings.quality, Quality::High);
        assert_eq!(settings.background, Background::Transparent);
        assert_eq!(settings.gif_loop, Loop::Times(3));
        assert_eq!(
            (settings.start_ms, settings.end_ms),
            (Some(500.0), Some(2500.0))
        );
        assert_eq!(settings.still_ms, None);
        assert!(settings.sound, "sound is on unless left out");
        setup.choices.sound = false;
        assert!(!setup.settings().sound);
    }

    #[test]
    fn choices_remembered_before_sound_keep_it() {
        let old: ExportChoices = serde_json::from_str(r#"{"format":"webm","fps":24}"#).unwrap();
        assert!(old.sound);
        assert_eq!(old.format(), Format::WebM);
    }

    #[test]
    fn mp4_keeps_its_background_and_a_png_takes_one_moment() {
        let mut choices = ExportChoices {
            transparent: true,
            ..ExportChoices::default()
        };
        let settings = setup(choices.clone()).settings();
        assert_eq!(
            settings.background,
            Background::Artboard,
            "MP4 cannot be clear"
        );

        choices.set_format(Format::Png);
        let mut png = setup(choices.clone());
        png.range = Some((0.0, 1000.0));
        let settings = png.settings();
        assert_eq!(settings.background, Background::Transparent);
        assert_eq!(settings.still_ms, None, "the design at rest");
        assert_eq!((settings.start_ms, settings.end_ms), (None, None));

        choices.still_at_playhead = true;
        assert_eq!(setup(choices).settings().still_ms, Some(1500.0));
    }

    #[test]
    fn the_job_exports_the_chosen_artboard_and_format_to_the_chosen_file() {
        let document = Arc::new(
            moshionly_core::io::load(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moshionly-core/examples/demo.moshi"
            ))
            .unwrap(),
        );
        let mut choices = ExportChoices::default();
        choices.set_format(Format::Png);
        choices.still_at_playhead = true;
        let job = setup(choices).job(document, PathBuf::from("/tmp/out.png"));
        assert_eq!(job.format, Format::Png);
        assert_eq!(job.artboard, NodeId::from("scene"));
        assert_eq!(job.destination, PathBuf::from("/tmp/out.png"));
        let plan = job.plan().unwrap();
        assert_eq!(plan.moments.to_vec(), vec![Moment::At(1500.0)]);
    }

    #[test]
    fn remembered_choices_that_make_no_sense_fall_back() {
        let choices = ExportChoices {
            format: "avi".into(),
            quality: "best".into(),
            fps: f64::NAN,
            scale: -2.0,
            ..ExportChoices::default()
        };
        assert_eq!(choices.format(), Format::Mp4);
        assert_eq!(choices.quality(), Quality::Standard);
        assert_eq!(choices.frame_rate(), FrameRate::FPS_30);
        assert_eq!(choices.scale(), 1.0);
        assert_eq!(GifLoop::Times(0).to_loop(), Loop::Times(1));
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(file_size(1), "1 byte");
        assert_eq!(file_size(980), "980 bytes");
        assert_eq!(file_size(1_234), "1.2 KB");
        assert_eq!(file_size(45_600), "46 KB");
        assert_eq!(file_size(3_400_000), "3.4 MB");
        assert_eq!(file_size(2_000_000_000), "2.0 GB");
        assert!(!file_manager_name().is_empty());
    }
}

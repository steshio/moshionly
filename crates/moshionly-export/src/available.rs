//! Which formats can be written right now.

use crate::ffmpeg::{Ffmpeg, INSTALL_HINT};
use crate::format::Format;

/// Whether a format can be written right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    /// The format.
    pub format: Format,
    /// `None` when the format can be written. Otherwise why not, and what
    /// to do about it, for a menu to show beside the greyed-out format.
    pub missing: Option<String>,
}

impl Availability {
    /// Whether the format can be written.
    pub fn is_available(&self) -> bool {
        self.missing.is_none()
    }
}

/// Every format, and whether it can be written right now.
///
/// PNG, GIF and MP4 are built in and always available. WebM and ProRes need
/// an `ffmpeg` with their encoders. It is looked for on every call, since
/// it may be installed while the app runs; that runs `ffmpeg` once, which
/// takes a few tens of milliseconds, so call this when a menu opens rather
/// than every frame.
pub fn availability() -> Vec<Availability> {
    availability_with(Ffmpeg::locate().as_ref())
}

/// [`availability`] with a given `ffmpeg`, or none.
pub fn availability_with(ffmpeg: Option<&Ffmpeg>) -> Vec<Availability> {
    let encoders = ffmpeg.map(|ffmpeg| (ffmpeg, ffmpeg.encoders()));
    Format::ALL
        .into_iter()
        .map(|format| {
            let missing = if !format.needs_ffmpeg() {
                None
            } else {
                match &encoders {
                    None => Some(format!(
                        "{format} needs ffmpeg, which was not found; {INSTALL_HINT}"
                    )),
                    Some((_, Err(reason))) => Some(format!("{format} needs ffmpeg: {reason}")),
                    Some((ffmpeg, Ok(encoders))) => encoders
                        .check(ffmpeg.path(), format)
                        .err()
                        .map(|reason| format!("{format} needs ffmpeg: {reason}")),
                }
            };
            Availability { format, missing }
        })
        .collect()
}

/// The formats that can be written right now. See [`availability`] for
/// why one is missing.
pub fn available_formats() -> Vec<Format> {
    availability()
        .into_iter()
        .filter(Availability::is_available)
        .map(|availability| availability.format)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_formats_need_nothing() {
        let without = availability_with(None);
        assert_eq!(without.len(), Format::ALL.len());
        for entry in &without {
            assert_eq!(
                entry.is_available(),
                !entry.format.needs_ffmpeg(),
                "{entry:?}"
            );
        }
        let webm = without
            .iter()
            .find(|entry| entry.format == Format::WebM)
            .unwrap();
        let reason = webm.missing.as_deref().unwrap();
        assert!(
            reason.contains("WebM needs ffmpeg") && reason.contains("brew install"),
            "{reason}"
        );
    }

    #[test]
    fn a_broken_ffmpeg_is_reported() {
        let broken = Ffmpeg::at("/nowhere/ffmpeg");
        let entries = availability_with(Some(&broken));
        let prores = entries
            .iter()
            .find(|entry| entry.format == Format::ProRes)
            .unwrap();
        let reason = prores.missing.as_deref().unwrap();
        assert!(
            reason.contains("/nowhere/ffmpeg could not be started"),
            "{reason}"
        );
        assert!(
            entries
                .iter()
                .find(|e| e.format == Format::Mp4)
                .unwrap()
                .is_available()
        );
    }

    #[test]
    fn an_installed_ffmpeg_makes_every_format_available() {
        let Some(ffmpeg) = Ffmpeg::locate() else {
            println!("skipped: ffmpeg is not installed");
            return;
        };
        let entries = availability_with(Some(&ffmpeg));
        if let Some(missing) = entries.iter().find_map(|entry| entry.missing.clone()) {
            println!("skipped: this ffmpeg lacks an encoder: {missing}");
            return;
        }
        assert_eq!(available_formats(), Format::ALL);
    }
}

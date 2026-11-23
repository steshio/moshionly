//! H.264 encoding with OpenH264.
//!
//! OpenH264 hands back each frame as NAL units in "Annex B" form, each
//! behind a start code. MP4 wants them differently: the sequence and
//! picture parameter sets once, in the file's track description, and each
//! frame's other units behind a four-byte length. This module does that
//! translation.

use openh264::OpenH264API;
use openh264::Timestamp;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate as EncoderRate, FrameType,
    IntraFramePeriod, Profile, QpRange, RateControlMode, UsageType, VuiConfig,
};

use openh264_sys2::{ENCODER_OPTION_TRACE_LEVEL, WELS_LOG_QUIET};

use super::yuv::Yuv420;
use crate::error::ExportError;
use crate::settings::Quality;
use crate::timing::FrameRate;

/// NAL unit types (ITU-T H.264 table 7-1) this module looks at.
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;
const NAL_DELIMITER: u8 = 9;
const NAL_SLICE: u8 = 1;
const NAL_IDR_SLICE: u8 = 5;

/// One encoded frame, ready for an MP4 sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedFrame {
    /// The frame's NAL units, each behind its length as four big-endian
    /// bytes.
    pub data: Vec<u8>,
    /// Whether playback can start at this frame: a key frame.
    pub key: bool,
}

/// Encodes frames of one size into H.264.
pub(crate) struct H264Encoder {
    encoder: Encoder,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

impl H264Encoder {
    /// An encoder for `width` by `height` frames at `rate`.
    pub fn new(
        width: u32,
        height: u32,
        rate: FrameRate,
        quality: Quality,
    ) -> Result<Self, ExportError> {
        let fps = rate.fps();
        let settings = settings(quality);
        let pixels_per_second = f64::from(width) * f64::from(height) * fps;
        let bitrate = (pixels_per_second * settings.bits_per_pixel).clamp(100_000.0, 200e6) as u32;
        // A key frame every two seconds, so players can seek quickly.
        let key_interval = (2.0 * fps).round().max(1.0) as u32;
        let config = EncoderConfig::new()
            // The only general-purpose mode this OpenH264 accepts; "real
            // time" only means it never waits for frames that come later.
            .usage_type(UsageType::CameraVideoRealTime)
            .rate_control_mode(RateControlMode::Quality)
            .bitrate(BitRate::from_bps(bitrate))
            .max_frame_rate(EncoderRate::from_hz(fps as f32))
            .skip_frames(false)
            .profile(Profile::High)
            .complexity(settings.complexity)
            .qp(QpRange::new(settings.qp.0, settings.qp.1))
            .intra_frame_period(IntraFramePeriod::from_num_frames(key_interval))
            .vui(VuiConfig::bt709());
        let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|error| ExportError::H264(error.to_string()))?;
        // OpenH264 prints warnings to the terminal while it starts, before
        // the wrapper turns its log off, so turn it off first.
        let mut quiet = WELS_LOG_QUIET as u32;
        // SAFETY: the encoder is alive for the call. OpenH264 accepts the
        // trace level before it is initialised (it only touches the trace
        // object made with the encoder), and reads the level as a `u32`
        // from the pointer during the call, which points at `quiet`.
        unsafe {
            encoder
                .raw_api()
                .set_option(ENCODER_OPTION_TRACE_LEVEL, (&raw mut quiet).cast());
        }
        Ok(Self {
            encoder,
            sps: None,
            pps: None,
        })
    }

    /// Encodes the next frame, which shows the moment `time_ms`.
    pub fn encode(&mut self, picture: &Yuv420, time_ms: f64) -> Result<EncodedFrame, ExportError> {
        let timestamp = Timestamp::from_millis(time_ms.max(0.0).round() as u64);
        let stream = self
            .encoder
            .encode_at(picture, timestamp)
            .map_err(|error| ExportError::H264(error.to_string()))?;
        let key = stream.frame_type() == FrameType::IDR;
        let mut data = Vec::new();
        let mut slices = 0;
        for layer in (0..stream.num_layers()).filter_map(|index| stream.layer(index)) {
            for unit in (0..layer.nal_count()).filter_map(|index| layer.nal_unit(index)) {
                let unit = without_start_code(unit);
                let Some(&header) = unit.first() else {
                    continue;
                };
                match header & 0x1f {
                    NAL_SPS => {
                        self.sps.get_or_insert_with(|| unit.to_vec());
                    }
                    NAL_PPS => {
                        self.pps.get_or_insert_with(|| unit.to_vec());
                    }
                    NAL_DELIMITER => {}
                    kind => {
                        slices += usize::from(kind == NAL_SLICE || kind == NAL_IDR_SLICE);
                        data.extend((unit.len() as u32).to_be_bytes());
                        data.extend_from_slice(unit);
                    }
                }
            }
        }
        if slices == 0 {
            return Err(ExportError::H264(
                "the encoder skipped a frame; try a higher quality".into(),
            ));
        }
        Ok(EncodedFrame { data, key })
    }

    /// The sequence and picture parameter sets, which the MP4 track
    /// description holds. Known once the first frame is encoded.
    pub fn parameter_sets(&self) -> Result<(&[u8], &[u8]), ExportError> {
        match (&self.sps, &self.pps) {
            (Some(sps), Some(pps)) if sps.len() >= 4 => Ok((sps, pps)),
            _ => Err(ExportError::H264(
                "the encoder did not describe the video stream".into(),
            )),
        }
    }
}

/// How the encoder works at each quality.
struct Settings {
    /// Target bits for each pixel of each frame. Flat motion graphics need
    /// far less than camera video at the same size.
    bits_per_pixel: f64,
    /// The range the quantiser may use: lower is sharper and larger.
    qp: (u8, u8),
    complexity: Complexity,
}

fn settings(quality: Quality) -> Settings {
    match quality {
        Quality::Draft => Settings {
            bits_per_pixel: 0.04,
            qp: (24, 40),
            complexity: Complexity::Low,
        },
        Quality::Standard => Settings {
            bits_per_pixel: 0.1,
            qp: (16, 32),
            complexity: Complexity::Medium,
        },
        Quality::High => Settings {
            bits_per_pixel: 0.25,
            qp: (10, 26),
            complexity: Complexity::High,
        },
    }
}

/// A NAL unit without the `00 00 01` or `00 00 00 01` before it.
fn without_start_code(unit: &[u8]) -> &[u8] {
    let zeros = unit.iter().take_while(|byte| **byte == 0).count();
    match unit.get(zeros) {
        Some(1) if zeros >= 2 => &unit[zeros + 1..],
        _ => unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_codes_are_removed() {
        assert_eq!(without_start_code(&[0, 0, 0, 1, 0x67, 1]), [0x67, 1]);
        assert_eq!(without_start_code(&[0, 0, 1, 0x68]), [0x68]);
        assert_eq!(without_start_code(&[0x65, 0, 0]), [0x65, 0, 0]);
        assert_eq!(without_start_code(&[]), [] as [u8; 0]);
    }

    #[test]
    fn frames_come_out_as_length_prefixed_units() {
        let (width, height) = (64, 48);
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| [(i % 256) as u8, (i / 13 % 256) as u8, 90, 255])
            .collect();
        let picture = Yuv420::from_rgba(&pixels, width, height);
        let mut encoder = H264Encoder::new(
            width as u32,
            height as u32,
            FrameRate::FPS_30,
            Quality::Draft,
        )
        .unwrap();
        let first = encoder.encode(&picture, 0.0).unwrap();
        assert!(first.key, "the first frame is a key frame");
        let second = encoder.encode(&picture, 33.3).unwrap();
        assert!(!second.key);
        for frame in [&first, &second] {
            // Walk the units by their lengths: they cover the data exactly.
            let mut at = 0;
            while at < frame.data.len() {
                let length = u32::from_be_bytes(frame.data[at..at + 4].try_into().unwrap());
                at += 4 + length as usize;
            }
            assert_eq!(at, frame.data.len());
        }
        let (sps, pps) = encoder.parameter_sets().unwrap();
        assert_eq!(sps[0] & 0x1f, NAL_SPS);
        assert_eq!(pps[0] & 0x1f, NAL_PPS);
    }
}

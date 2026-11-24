//! Which H.264 level an MP4 video declares.
//!
//! A level tells a player how much work a stream takes to decode: how many
//! macroblocks (squares of 16 by 16 pixels) a frame may have, and how many
//! may be decoded each second. Hardware decoders in phones and televisions
//! read it to decide whether they can play a file, so a level that is too
//! low makes them try a video they cannot keep up with.
//!
//! OpenH264 works a level out by itself, but it treats any frame rate above
//! 60 as 60 while doing so. The level is therefore worked out here from the
//! true rate and handed to the encoder, which keeps it unless its own is
//! higher. The limits are those of the H.264 standard (ITU-T H.264, Annex A,
//! table A-1), up to level 5.2, the highest OpenH264 writes.

use openh264::encoder::Level;

/// The most frames per second H.264 allows at any level up to 5.2, whatever
/// the size: a frame may follow the one before no sooner than 1/172 second.
pub(crate) const MAX_FPS: f64 = 172.0;

/// The limits of one level.
#[derive(Debug, Clone, Copy)]
pub(crate) struct H264Level {
    /// Macroblocks decoded each second, at most.
    max_per_second: u32,
    /// Macroblocks in a frame, at most.
    max_per_frame: u32,
    /// The level as the encoder takes it.
    pub encoder: Level,
}

const fn level(max_per_second: u32, max_per_frame: u32, encoder: Level) -> H264Level {
    H264Level {
        max_per_second,
        max_per_frame,
        encoder,
    }
}

/// The levels from lowest to highest. Level 1b, a variant of level 1 for
/// tiny low-rate streams, is left out.
const LEVELS: [H264Level; 16] = [
    level(1_485, 99, Level::Level_1_0),
    level(3_000, 396, Level::Level_1_1),
    level(6_000, 396, Level::Level_1_2),
    level(11_880, 396, Level::Level_1_3),
    level(11_880, 396, Level::Level_2_0),
    level(19_800, 792, Level::Level_2_1),
    level(20_250, 1_620, Level::Level_2_2),
    level(40_500, 1_620, Level::Level_3_0),
    level(108_000, 3_600, Level::Level_3_1),
    level(216_000, 5_120, Level::Level_3_2),
    level(245_760, 8_192, Level::Level_4_0),
    level(245_760, 8_192, Level::Level_4_1),
    level(522_240, 8_704, Level::Level_4_2),
    level(589_824, 22_080, Level::Level_5_0),
    level(983_040, 36_864, Level::Level_5_1),
    level(2_073_600, 36_864, Level::Level_5_2),
];

/// The lowest level that holds frames of `width` by `height` pixels at
/// `fps` frames per second, or `None` when even level 5.2 cannot.
///
/// Only the picture's size and rate are considered; the encoder raises the
/// level further if the bit rate needs it.
pub(crate) fn level_for(width: u32, height: u32, fps: f64) -> Option<H264Level> {
    if !(fps > 0.0 && fps <= MAX_FPS) {
        return None;
    }
    let (across, down) = (macroblocks(width), macroblocks(height));
    LEVELS.into_iter().find(|level| {
        fits_frame(level, across, down)
            && (across * down) as f64 * fps <= f64::from(level.max_per_second)
    })
}

/// The most frames per second an MP4 of `width` by `height` pixels can
/// have, rounded down to a hundredth: 172, or fewer when the frames are so
/// large that level 5.2 runs out, such as 64 at 3840 by 2160.
pub(crate) fn max_fps(width: u32, height: u32) -> f64 {
    let (across, down) = (macroblocks(width), macroblocks(height));
    let top = LEVELS[LEVELS.len() - 1];
    if !fits_frame(&top, across, down) {
        return 0.0;
    }
    let by_rate = f64::from(top.max_per_second) / (across * down) as f64;
    ((by_rate * 100.0).floor() / 100.0).min(MAX_FPS)
}

/// Whether a frame `across` by `down` macroblocks is within the level's
/// frame size, which also limits each side to the square root of eight
/// times it.
fn fits_frame(level: &H264Level, across: u64, down: u64) -> bool {
    let most = u64::from(level.max_per_frame);
    across * down <= most && across * across <= 8 * most && down * down <= 8 * most
}

/// Macroblocks needed to cover `pixels` pixels.
fn macroblocks(pixels: u32) -> u64 {
    u64::from(pixels).div_ceil(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The level as H.264 numbers it: 51 for 5.1.
    fn idc(width: u32, height: u32, fps: f64) -> Option<u8> {
        let level = level_for(width, height, fps)?;
        let name = format!("{:?}", level.encoder);
        name.trim_start_matches("Level_")
            .replace('_', "")
            .parse()
            .ok()
    }

    #[test]
    fn levels_follow_size_and_rate() {
        assert_eq!(idc(640, 360, 30.0), Some(30));
        assert_eq!(idc(1280, 720, 30.0), Some(31));
        assert_eq!(idc(1280, 720, 60.0), Some(32));
        assert_eq!(idc(1920, 1080, 30.0), Some(40));
        assert_eq!(idc(1080, 1920, 30.0), Some(40), "portrait");
        assert_eq!(idc(1920, 1080, 60.0), Some(42));
        assert_eq!(idc(1920, 1080, 120.0), Some(51));
        assert_eq!(idc(1920, 1080, 172.0), Some(52));
        assert_eq!(idc(3840, 2160, 30.0), Some(51));
        assert_eq!(idc(3840, 2160, 60.0), Some(52));
        // A large frame needs a high level however slow it is.
        assert_eq!(idc(1920, 1080, 1.0), Some(40));
        // So does a very long side.
        assert_eq!(idc(3840, 16, 1.0), Some(40));
        // Beyond level 5.2.
        assert_eq!(idc(3840, 2160, 65.0), None);
        assert_eq!(idc(320, 240, 173.0), None);
        assert_eq!(idc(320, 240, 0.0), None);
    }

    #[test]
    fn the_most_frames_per_second_shrinks_for_large_frames() {
        assert_eq!(max_fps(1920, 1080), MAX_FPS);
        assert_eq!(max_fps(2560, 1440), 144.0);
        assert_eq!(max_fps(3840, 2160), 64.0);
        assert_eq!(max_fps(2160, 3840), 64.0);
        // Rounded down, so the rate it gives always has a level.
        let most = max_fps(3840, 2048);
        assert_eq!(most, 67.5);
        assert!(level_for(3840, 2048, most).is_some());
        assert!(level_for(3000, 2000, max_fps(3000, 2000)).is_some());
        assert_eq!(max_fps(8192, 8192), 0.0, "larger than any level");
    }
}

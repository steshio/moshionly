//! Writing an MP4 file with one H.264 video track.
//!
//! An MP4 file is a list of boxes: a type, a header describing the track
//! and where every frame is (`moov`), and the frames themselves (`mdat`).
//! The header is written **before** the frames, so a player that downloads
//! the file can start playing before it has all of it. The header's size
//! depends only on the number of frames, but where the frames are depends
//! on the header's size, so the frames are first collected in an unnamed
//! temporary file and copied after the header at the end.
//!
//! The track says its colours are BT.709 (a `colr` box), which is how the
//! frames were converted, so every player shows the same colours.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use crate::timing::FrameRate;

/// The movie's own clock: milliseconds.
const MOVIE_TIMESCALE: u32 = 1000;

/// Collects H.264 samples and writes them as an MP4 file.
pub(crate) struct Mp4Writer {
    width: u32,
    height: u32,
    /// Ticks per second of the track's clock.
    timescale: u32,
    /// Ticks each frame lasts.
    frame_ticks: u32,
    /// Frames per chunk: about a second's worth.
    chunk_frames: usize,
    samples: BufWriter<File>,
    sizes: Vec<u32>,
    /// Key frames, counting from 1.
    keys: Vec<u32>,
}

impl Mp4Writer {
    /// A writer for `width` by `height` video at `rate`, keeping its frames
    /// in a temporary file in `dir` until the end.
    pub fn new(dir: &Path, width: u32, height: u32, rate: FrameRate) -> io::Result<Self> {
        let (timescale, frame_ticks) = track_clock(rate);
        Ok(Self {
            width,
            height,
            timescale,
            frame_ticks,
            chunk_frames: rate.fps().round().max(1.0) as usize,
            samples: BufWriter::new(tempfile::tempfile_in(dir)?),
            sizes: Vec::new(),
            keys: Vec::new(),
        })
    }

    /// Adds the next frame: length-prefixed NAL units, and whether it is a
    /// key frame.
    pub fn add(&mut self, sample: &[u8], key: bool) -> io::Result<()> {
        let size = u32::try_from(sample.len())
            .map_err(|_| io::Error::other("a video frame is larger than 4 GB"))?;
        self.samples.write_all(sample)?;
        self.sizes.push(size);
        if key {
            self.keys.push(self.sizes.len() as u32);
        }
        Ok(())
    }

    /// Writes the whole file to `out`: type, header, then the frames.
    /// `sps` and `pps` are the H.264 parameter sets.
    pub fn finish(self, out: &mut impl Write, sps: &[u8], pps: &[u8]) -> io::Result<()> {
        let mut samples = self
            .samples
            .into_inner()
            .map_err(|error| error.into_error())?;
        let payload: u64 = self.sizes.iter().map(|size| u64::from(*size)).sum();
        let track = Track {
            width: self.width,
            height: self.height,
            timescale: self.timescale,
            frame_ticks: self.frame_ticks,
            chunk_frames: self.chunk_frames,
            sizes: &self.sizes,
            keys: &self.keys,
            sps,
            pps,
        };
        let file_type = file_type();
        let mdat_header = if payload + 8 > u64::from(u32::MAX) {
            16
        } else {
            8
        };
        // Offsets do not change the header's size, so measure it first.
        let mut wide = false;
        let measured = track.movie(0, false).len() as u64;
        if file_type.len() as u64 + measured + mdat_header + payload > u64::from(u32::MAX) {
            wide = true;
        }
        let header_size = track.movie(0, wide).len() as u64;
        let first_sample = file_type.len() as u64 + header_size + mdat_header;
        let movie = track.movie(first_sample, wide);

        out.write_all(&file_type)?;
        out.write_all(&movie)?;
        if mdat_header == 16 {
            out.write_all(&1u32.to_be_bytes())?;
            out.write_all(b"mdat")?;
            out.write_all(&(payload + 16).to_be_bytes())?;
        } else {
            out.write_all(&(payload as u32 + 8).to_be_bytes())?;
            out.write_all(b"mdat")?;
        }
        samples.seek(SeekFrom::Start(0))?;
        let copied = io::copy(&mut samples, out)?;
        if copied != payload {
            return Err(io::Error::other("the video frames could not be read back"));
        }
        out.flush()
    }
}

/// The track's clock: ticks per second, and ticks per frame. Whole numbers
/// for any frame rate, so 29.97 fps is exactly 1001 ticks of 30000.
fn track_clock(rate: FrameRate) -> (u32, u32) {
    let mut scale = 1;
    // Fine enough that editors which round to the clock lose nothing.
    while rate.numerator().saturating_mul(scale) < 10_000 && scale < 1_000_000 {
        scale *= 10;
    }
    (
        rate.numerator().saturating_mul(scale),
        rate.denominator().saturating_mul(scale),
    )
}

/// `ftyp`: the file is ISO base media with H.264.
fn file_type() -> Vec<u8> {
    boxed(b"ftyp", |b| {
        b.extend(b"isom");
        b.extend(0x200u32.to_be_bytes());
        for brand in [b"isom", b"iso2", b"avc1", b"mp41"] {
            b.extend(brand);
        }
    })
}

/// A box: its size, its type, then what `body` writes.
fn boxed(kind: &[u8; 4], body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut content = Vec::new();
    body(&mut content);
    let mut out = Vec::with_capacity(content.len() + 8);
    out.extend((content.len() as u32 + 8).to_be_bytes());
    out.extend(kind);
    out.extend(content);
    out
}

/// A "full box": a box whose body starts with a version and flags.
fn full_box(kind: &[u8; 4], version: u8, flags: u32, body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    boxed(kind, |b| {
        b.extend(((u32::from(version) << 24) | (flags & 0xff_ffff)).to_be_bytes());
        body(b);
    })
}

/// The identity transform, as MP4 headers write it.
fn unity_matrix(b: &mut Vec<u8>) {
    for value in [0x1_0000u32, 0, 0, 0, 0x1_0000, 0, 0, 0, 0x4000_0000] {
        b.extend(value.to_be_bytes());
    }
}

/// What the header describes.
struct Track<'a> {
    width: u32,
    height: u32,
    timescale: u32,
    frame_ticks: u32,
    chunk_frames: usize,
    sizes: &'a [u32],
    keys: &'a [u32],
    sps: &'a [u8],
    pps: &'a [u8],
}

impl Track<'_> {
    /// Track length in its own ticks.
    fn ticks(&self) -> u64 {
        self.sizes.len() as u64 * u64::from(self.frame_ticks)
    }

    /// Track length in movie ticks (milliseconds), rounded.
    fn movie_ticks(&self) -> u64 {
        let ticks = u128::from(self.ticks()) * u128::from(MOVIE_TIMESCALE);
        ((ticks + u128::from(self.timescale) / 2) / u128::from(self.timescale)) as u64
    }

    /// `moov`, with the first frame at byte `first_sample` of the file.
    /// `wide` writes 64-bit chunk offsets.
    fn movie(&self, first_sample: u64, wide: bool) -> Vec<u8> {
        boxed(b"moov", |b| {
            b.extend(self.movie_header());
            b.extend(self.track(first_sample, wide));
        })
    }

    fn movie_header(&self) -> Vec<u8> {
        let duration = self.movie_ticks();
        let long = duration > u64::from(u32::MAX);
        full_box(b"mvhd", u8::from(long), 0, |b| {
            if long {
                b.extend([0; 16]); // created, modified
                b.extend(MOVIE_TIMESCALE.to_be_bytes());
                b.extend(duration.to_be_bytes());
            } else {
                b.extend([0; 8]);
                b.extend(MOVIE_TIMESCALE.to_be_bytes());
                b.extend((duration as u32).to_be_bytes());
            }
            b.extend(0x1_0000u32.to_be_bytes()); // normal speed
            b.extend(0x100u16.to_be_bytes()); // full volume
            b.extend([0; 10]);
            unity_matrix(b);
            b.extend([0; 24]);
            b.extend(2u32.to_be_bytes()); // the next track would be 2
        })
    }

    fn track(&self, first_sample: u64, wide: bool) -> Vec<u8> {
        boxed(b"trak", |b| {
            b.extend(self.track_header());
            b.extend(boxed(b"mdia", |b| {
                b.extend(self.media_header());
                b.extend(full_box(b"hdlr", 0, 0, |b| {
                    b.extend([0; 4]);
                    b.extend(b"vide");
                    b.extend([0; 12]);
                    b.extend(b"VideoHandler\0");
                }));
                b.extend(boxed(b"minf", |b| {
                    b.extend(full_box(b"vmhd", 0, 1, |b| b.extend([0; 8])));
                    b.extend(boxed(b"dinf", |b| {
                        b.extend(full_box(b"dref", 0, 0, |b| {
                            b.extend(1u32.to_be_bytes());
                            // Flag 1: the frames are in this file.
                            b.extend(full_box(b"url ", 0, 1, |_| {}));
                        }));
                    }));
                    b.extend(self.sample_table(first_sample, wide));
                }));
            }));
        })
    }

    fn track_header(&self) -> Vec<u8> {
        let duration = self.movie_ticks();
        let long = duration > u64::from(u32::MAX);
        // Flags: the track is enabled and part of the movie.
        full_box(b"tkhd", u8::from(long), 0x3, |b| {
            if long {
                b.extend([0; 16]);
                b.extend(1u32.to_be_bytes());
                b.extend([0; 4]);
                b.extend(duration.to_be_bytes());
            } else {
                b.extend([0; 8]);
                b.extend(1u32.to_be_bytes());
                b.extend([0; 4]);
                b.extend((duration as u32).to_be_bytes());
            }
            b.extend([0; 8]);
            b.extend([0; 4]); // layer, alternate group
            b.extend([0; 4]); // volume: none for video
            unity_matrix(b);
            b.extend((self.width << 16).to_be_bytes());
            b.extend((self.height << 16).to_be_bytes());
        })
    }

    fn media_header(&self) -> Vec<u8> {
        let duration = self.ticks();
        let long = duration > u64::from(u32::MAX);
        full_box(b"mdhd", u8::from(long), 0, |b| {
            if long {
                b.extend([0; 16]);
                b.extend(self.timescale.to_be_bytes());
                b.extend(duration.to_be_bytes());
            } else {
                b.extend([0; 8]);
                b.extend(self.timescale.to_be_bytes());
                b.extend((duration as u32).to_be_bytes());
            }
            // Language "und" (undetermined), packed into 15 bits.
            let code = |c: u8| u16::from(c - 0x60);
            b.extend(((code(b'u') << 10) | (code(b'n') << 5) | code(b'd')).to_be_bytes());
            b.extend([0; 2]);
        })
    }

    fn sample_table(&self, first_sample: u64, wide: bool) -> Vec<u8> {
        let count = self.sizes.len() as u32;
        boxed(b"stbl", |b| {
            b.extend(full_box(b"stsd", 0, 0, |b| {
                b.extend(1u32.to_be_bytes());
                b.extend(self.sample_entry());
            }));
            // Every frame lasts the same.
            b.extend(full_box(b"stts", 0, 0, |b| {
                b.extend(1u32.to_be_bytes());
                b.extend(count.to_be_bytes());
                b.extend(self.frame_ticks.to_be_bytes());
            }));
            if self.keys.len() < self.sizes.len() {
                b.extend(full_box(b"stss", 0, 0, |b| {
                    b.extend((self.keys.len() as u32).to_be_bytes());
                    for key in self.keys {
                        b.extend(key.to_be_bytes());
                    }
                }));
            }
            let chunks = self.sizes.chunks(self.chunk_frames.max(1));
            let last = chunks.len();
            b.extend(full_box(b"stsc", 0, 0, |b| {
                let full = self.chunk_frames.min(self.sizes.len());
                let rest = self.sizes.len() % self.chunk_frames.max(1);
                let mut runs = vec![(1u32, full as u32)];
                if rest != 0 && last > 1 {
                    runs.push((last as u32, rest as u32));
                }
                b.extend((runs.len() as u32).to_be_bytes());
                for (first_chunk, frames) in runs {
                    b.extend(first_chunk.to_be_bytes());
                    b.extend(frames.to_be_bytes());
                    b.extend(1u32.to_be_bytes()); // the one sample description
                }
            }));
            b.extend(full_box(b"stsz", 0, 0, |b| {
                b.extend(0u32.to_be_bytes()); // sizes differ
                b.extend(count.to_be_bytes());
                for size in self.sizes {
                    b.extend(size.to_be_bytes());
                }
            }));
            let mut offset = first_sample;
            let offsets: Vec<u64> = self
                .sizes
                .chunks(self.chunk_frames.max(1))
                .map(|chunk| {
                    let start = offset;
                    offset += chunk.iter().map(|size| u64::from(*size)).sum::<u64>();
                    start
                })
                .collect();
            let kind = if wide { b"co64" } else { b"stco" };
            b.extend(full_box(kind, 0, 0, |b| {
                b.extend((offsets.len() as u32).to_be_bytes());
                for offset in offsets {
                    if wide {
                        b.extend(offset.to_be_bytes());
                    } else {
                        b.extend((offset as u32).to_be_bytes());
                    }
                }
            }));
        })
    }

    /// `avc1`: the frame size and how to decode the frames.
    fn sample_entry(&self) -> Vec<u8> {
        boxed(b"avc1", |b| {
            b.extend([0; 6]);
            b.extend(1u16.to_be_bytes()); // data reference 1: this file
            b.extend([0; 16]);
            b.extend((self.width as u16).to_be_bytes());
            b.extend((self.height as u16).to_be_bytes());
            b.extend(0x0048_0000u32.to_be_bytes()); // 72 dpi
            b.extend(0x0048_0000u32.to_be_bytes());
            b.extend([0; 4]);
            b.extend(1u16.to_be_bytes()); // one frame per sample
            b.extend([0; 32]); // no compressor name
            b.extend(0x18u16.to_be_bytes()); // colour, no alpha
            b.extend((-1i16).to_be_bytes());
            b.extend(self.decoder_configuration());
            // BT.709 primaries, transfer and matrix, limited range.
            b.extend(boxed(b"colr", |b| {
                b.extend(b"nclx");
                b.extend(1u16.to_be_bytes());
                b.extend(1u16.to_be_bytes());
                b.extend(1u16.to_be_bytes());
                b.push(0);
            }));
            // Square pixels.
            b.extend(boxed(b"pasp", |b| {
                b.extend(1u32.to_be_bytes());
                b.extend(1u32.to_be_bytes());
            }));
        })
    }

    /// `avcC`: the H.264 parameter sets.
    fn decoder_configuration(&self) -> Vec<u8> {
        let (profile, compatibility, level) = (self.sps[1], self.sps[2], self.sps[3]);
        boxed(b"avcC", |b| {
            b.extend([1, profile, compatibility, level]);
            b.push(0xfc | 3); // lengths are 4 bytes
            b.push(0xe0 | 1); // one sequence parameter set
            b.extend((self.sps.len() as u16).to_be_bytes());
            b.extend(self.sps);
            b.push(1); // one picture parameter set
            b.extend((self.pps.len() as u16).to_be_bytes());
            b.extend(self.pps);
            // High profiles also say how colour is sampled: 4:2:0, 8 bits.
            if matches!(profile, 100 | 110 | 122 | 144) {
                b.extend([0xfc | 1, 0xf8, 0xf8, 0]);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_count_whole_ticks() {
        assert_eq!(track_clock(FrameRate::FPS_30), (30_000, 1000));
        assert_eq!(track_clock(FrameRate::FPS_29_97), (30_000, 1001));
        assert_eq!(track_clock(FrameRate::FPS_23_976), (24_000, 1001));
        assert_eq!(track_clock(FrameRate::new(25, 2).unwrap()), (25_000, 2000));
        assert_eq!(track_clock(FrameRate::FPS_120), (12_000, 100));
    }

    fn top_level_boxes(file: &[u8]) -> Vec<String> {
        let mut kinds = Vec::new();
        let mut at = 0;
        while at + 8 <= file.len() {
            let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
            kinds.push(String::from_utf8_lossy(&file[at + 4..at + 8]).into_owned());
            at += size.max(8);
        }
        kinds
    }

    #[test]
    fn the_header_comes_before_the_frames_and_points_at_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = Mp4Writer::new(dir.path(), 64, 48, FrameRate::FPS_30).unwrap();
        // 70 small frames, a key frame every 30.
        for index in 0..70u32 {
            let sample = vec![index as u8; 10 + index as usize];
            writer.add(&sample, index % 30 == 0).unwrap();
        }
        let sps = [0x67, 100, 0, 30, 1, 2];
        let pps = [0x68, 3];
        let mut file = Vec::new();
        writer.finish(&mut file, &sps, &pps).unwrap();
        assert_eq!(top_level_boxes(&file), ["ftyp", "moov", "mdat"]);

        let reader =
            mp4::Mp4Reader::read_header(std::io::Cursor::new(&file), file.len() as u64).unwrap();
        assert_eq!(reader.timescale(), 1000);
        assert_eq!(reader.duration().as_millis(), 2333);
        let track = reader.tracks().get(&1).unwrap();
        assert_eq!((track.width(), track.height()), (64, 48));
        assert_eq!(track.sample_count(), 70);
        assert_eq!(track.timescale(), 30_000);
        assert_eq!(track.frame_rate(), 30.0);
        let mut reader = reader;
        for index in [0u32, 29, 30, 31, 69] {
            let sample = reader.read_sample(1, index + 1).unwrap().unwrap();
            assert_eq!(&sample.bytes[..], vec![index as u8; 10 + index as usize]);
            assert_eq!(sample.is_sync, index % 30 == 0, "frame {index}");
            assert_eq!(sample.start_time, u64::from(index) * 1000);
        }
    }

    #[test]
    fn television_rates_have_exact_durations() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = Mp4Writer::new(dir.path(), 16, 16, FrameRate::FPS_29_97).unwrap();
        for _ in 0..30 {
            writer.add(&[0, 0, 0, 1, 0x65], true).unwrap();
        }
        let mut file = Vec::new();
        writer
            .finish(&mut file, &[0x67, 66, 0, 30], &[0x68])
            .unwrap();
        let reader =
            mp4::Mp4Reader::read_header(std::io::Cursor::new(&file), file.len() as u64).unwrap();
        let track = reader.tracks().get(&1).unwrap();
        assert_eq!(track.timescale(), 30_000);
        assert_eq!(track.duration().as_micros(), 1_001_000);
        // Every frame is a key frame, so there is no list of key frames.
        assert!(track.trak.mdia.minf.stbl.stss.is_none());
    }
}

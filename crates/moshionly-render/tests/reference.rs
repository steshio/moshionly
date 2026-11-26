//! Reference images: scenes rendered and compared with PNGs committed under
//! `tests/reference/`.
//!
//! Each scene is a small project file under `tests/scenes/`, evaluated by
//! core with the renderer's own text measurer and drawn with the bundled
//! fonts only, so the result does not depend on what the machine has
//! installed.
//!
//! To write the references again after an intended change, run
//!
//! ```text
//! MOSHIONLY_UPDATE_REFERENCES=1 cargo test -p moshionly-render --test reference
//! ```
//!
//! and look at every image that changed before committing it. When a
//! comparison fails, the image that was drawn is written next to the test
//! binary's temporary files and its path is printed.
//!
//! # How images are compared
//!
//! Text is rasterised by CoreText on macOS and FreeType on Linux, so even
//! with the same font file the edges of glyphs differ slightly, and Skia's
//! CPU code can round a level differently on different processors. Both
//! images are therefore averaged over each 3 by 3 neighbourhood first: that
//! evens out a difference in how much of an edge pixel is covered, while a
//! glyph or shape that moved, went missing or changed size still differs
//! strongly. Then a pixel counts as different when a channel differs by more
//! than the tolerance's `pixel` levels, and the test fails when more than
//! `share` of the pixels differ or when the mean difference over all
//! channels of the unaveraged images passes `mean`.

use std::path::{Path, PathBuf};

use moshionly_core::{NodeId, evaluate, io};
use moshionly_render::{AssetLoader, FontLibrary, Frame, RenderOptions, RenderWarning, Renderer};

/// Set to write the references instead of comparing with them.
const UPDATE: &str = "MOSHIONLY_UPDATE_REFERENCES";

/// How different a rendered image may be from its reference.
#[derive(Debug, Clone, Copy)]
struct Tolerance {
    /// Levels (of 255) a channel of the averaged images may differ by.
    pixel: u8,
    /// Share of pixels, 0 to 1, that may differ by more than `pixel`.
    share: f64,
    /// Mean difference over every channel of the unaveraged images.
    mean: f64,
}

/// Shapes, gradients and effects: these are drawn by Skia's own code on
/// every platform, so only rounding may differ.
const SHAPES: Tolerance = Tolerance {
    pixel: 6,
    share: 0.002,
    mean: 0.5,
};

/// Anything with text: glyph edges are rasterised by the platform.
const TEXT: Tolerance = Tolerance {
    pixel: 24,
    share: 0.004,
    mean: 1.5,
};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Renders the first artboard of `file` at `time_ms` and compares it with
/// the reference `name`. Returns the frame for further checks.
fn check(name: &str, file: &Path, time_ms: f64, scale: f64, tolerance: Tolerance) -> Frame {
    let document = io::load(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    let artboard: NodeId = document.artboards()[0].clone();
    let fonts = FontLibrary::bundled();
    let renderer = Renderer::new(fonts.clone(), AssetLoader::new(Some(file.to_owned())));
    let scene = evaluate(&document, &artboard, time_ms, &fonts).unwrap();
    let frame = renderer
        .render(&scene, &RenderOptions::new().with_scale(scale))
        .unwrap();
    let png = frame.to_png().unwrap();
    let reference = crate_dir()
        .join("tests/reference")
        .join(format!("{name}.png"));

    if std::env::var_os(UPDATE).is_some() {
        std::fs::write(&reference, &png).unwrap();
        println!("wrote {}", reference.display());
        return frame;
    }
    let expected = match std::fs::read(&reference) {
        Ok(bytes) => image::load_from_memory(&bytes).unwrap().into_rgba8(),
        Err(error) => panic!(
            "{}: {error}; run with {UPDATE}=1 to write it",
            reference.display()
        ),
    };
    // Compare what a file holds on both sides: unpremultiplied RGBA.
    let actual = image::load_from_memory(&png).unwrap().into_rgba8();
    let verdict = compare(&actual, &expected, tolerance);
    if let Err(problem) = verdict {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.actual.png"));
        std::fs::write(&out, &png).unwrap();
        panic!(
            "{name} differs from {}: {problem}\nwhat was drawn: {}",
            reference.display(),
            out.display()
        );
    }
    frame
}

/// Checks `actual` against `expected` as the module docs describe.
fn compare(
    actual: &image::RgbaImage,
    expected: &image::RgbaImage,
    tolerance: Tolerance,
) -> Result<(), String> {
    if actual.dimensions() != expected.dimensions() {
        return Err(format!(
            "the sizes differ: {:?} drawn, {:?} expected",
            actual.dimensions(),
            expected.dimensions()
        ));
    }
    let (width, height) = actual.dimensions();
    let channels = (width * height * 4) as f64;
    let mean = actual
        .as_raw()
        .iter()
        .zip(expected.as_raw())
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / channels;

    let (soft_actual, soft_expected) = (average(actual), average(expected));
    let mut differing = 0usize;
    let mut worst = (0u8, 0, 0);
    let (soft_actual, _) = soft_actual.as_chunks::<4>();
    let (soft_expected, _) = soft_expected.as_chunks::<4>();
    for (index, (a, b)) in soft_actual.iter().zip(soft_expected).enumerate() {
        let apart = a
            .iter()
            .zip(b)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        if apart > tolerance.pixel {
            differing += 1;
        }
        if apart > worst.0 {
            worst = (apart, index as u32 % width, index as u32 / width);
        }
    }
    let share = differing as f64 / f64::from(width * height);
    let summary = format!(
        "{:.3}% of pixels differ by more than {} (allowed {:.3}%), the worst by {} at ({}, {}); \
         mean difference {mean:.3} (allowed {})",
        share * 100.0,
        tolerance.pixel,
        tolerance.share * 100.0,
        worst.0,
        worst.1,
        worst.2,
        tolerance.mean
    );
    if share > tolerance.share || mean > tolerance.mean {
        Err(summary)
    } else {
        Ok(())
    }
}

/// Each channel averaged with its 3 by 3 neighbourhood (edges use what is
/// there).
fn average(image: &image::RgbaImage) -> Vec<u8> {
    let (width, height) = image.dimensions();
    let pixels = image.as_raw();
    let mut out = vec![0u8; pixels.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0u32; 4];
            let mut count = 0;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let at = ((ny * width + nx) * 4) as usize;
                    for (total, value) in sum.iter_mut().zip(&pixels[at..at + 4]) {
                        *total += u32::from(*value);
                    }
                    count += 1;
                }
            }
            let at = ((y * width + x) * 4) as usize;
            for (channel, total) in out[at..at + 4].iter_mut().zip(sum) {
                *channel = ((total + count / 2) / count) as u8;
            }
        }
    }
    out
}

fn scene(name: &str) -> PathBuf {
    crate_dir()
        .join("tests/scenes")
        .join(format!("{name}.moshi"))
}

fn demo() -> PathBuf {
    crate_dir().join("../moshionly-core/examples/demo.moshi")
}

#[test]
fn gradients() {
    check("gradients", &scene("gradients"), 0.0, 1.0, SHAPES);
}

#[test]
fn stroke_positions_caps_and_joins() {
    check("strokes", &scene("strokes"), 0.0, 1.0, SHAPES);
}

#[test]
fn arcs_and_rings() {
    check("arcs", &scene("arcs"), 0.0, 1.0, SHAPES);
}

#[test]
fn stars() {
    check("stars", &scene("stars"), 0.0, 1.0, SHAPES);
}

#[test]
fn pen_paths_and_fill_rules() {
    check("paths", &scene("paths"), 0.0, 1.0, SHAPES);
}

#[test]
fn masks() {
    check("masks", &scene("masks"), 0.0, 1.0, SHAPES);
}

#[test]
fn clips() {
    check("clips", &scene("clips"), 0.0, 1.0, SHAPES);
}

#[test]
fn blend_modes() {
    check("blend", &scene("blend"), 0.0, 1.0, SHAPES);
}

#[test]
fn shadows() {
    check("shadow", &scene("shadow"), 0.0, 1.0, TEXT);
}

#[test]
fn layer_blur() {
    check("blur", &scene("blur"), 0.0, 1.0, SHAPES);
}

#[test]
fn glows_with_spread_and_a_shadow() {
    check("glow", &scene("glow"), 0.0, 1.0, SHAPES);
}

#[test]
fn noise_still_and_moving() {
    let first = check("noise_0", &scene("noise"), 0.0, 1.0, SHAPES);
    let later = check("noise_500", &scene("noise"), 500.0, 1.0, SHAPES);
    // The pixels of a region of the frame, as drawn.
    let region = |frame: &Frame, x0: u32, x1: u32| -> Vec<[u8; 4]> {
        (20..120)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| frame.premultiplied_pixel(x, y).unwrap())
            .collect()
    };
    // "Moving" (left) has new grain at another time; "Still" keeps its grain.
    assert_ne!(region(&first, 20, 120), region(&later, 20, 120));
    assert_eq!(region(&first, 140, 240), region(&later, 140, 240));
    // Drawing the same moment again gives the same grain.
    let again = check("noise_0", &scene("noise"), 0.0, 1.0, SHAPES);
    assert_eq!(first.premultiplied(), again.premultiplied());
}

#[test]
fn directional_blur_across_down_and_slanted() {
    check(
        "directional_blur",
        &scene("directional_blur"),
        0.0,
        1.0,
        SHAPES,
    );
}

#[test]
fn text_alignment_spacing_and_case() {
    let frame = check("text_align", &scene("text_align"), 0.0, 1.0, TEXT);
    assert!(frame.warnings().is_empty(), "{:?}", frame.warnings());
}

#[test]
fn text_animated_unit_by_unit() {
    check("text_letters_400", &scene("text_letters"), 400.0, 1.0, TEXT);
    check("text_letters_700", &scene("text_letters"), 700.0, 1.0, TEXT);
    check(
        "text_letters_rest",
        &scene("text_letters"),
        2000.0,
        1.0,
        TEXT,
    );
}

#[test]
fn images_fit_and_a_missing_one_shows() {
    let frame = check("images", &scene("images"), 0.0, 1.0, SHAPES);
    match frame.warnings() {
        [RenderWarning::MissingImage { node, source, .. }] => {
            assert_eq!(node, &NodeId::from("missing"));
            assert_eq!(source, "missing.png");
        }
        other => panic!("expected one missing image, got {other:?}"),
    }
}

#[test]
fn the_core_demo_across_its_timeline() {
    for time in [300.0, 900.0, 1600.0, 4000.0] {
        check(&format!("demo_{time}"), &demo(), time, 0.25, TEXT);
    }
}

#[test]
fn the_comparison_tolerates_edges_and_catches_moves() {
    // A small square with soft edges, and the same with every edge pixel a
    // little lighter or darker: allowed.
    let square = |shift: u32, edge: u8| {
        image::RgbaImage::from_fn(60, 60, |x, y| {
            let inside = (20 + shift..40 + shift).contains(&x) && (20..40).contains(&y);
            let on_edge = (x == 19 + shift || x == 40 + shift) && (20..40).contains(&y);
            image::Rgba(if inside {
                [0, 0, 0, 255]
            } else if on_edge {
                [edge, edge, edge, 255]
            } else {
                [255, 255, 255, 255]
            })
        })
    };
    let reference = square(0, 128);
    assert!(compare(&square(0, 128), &reference, TEXT).is_ok());
    assert!(compare(&square(0, 90), &reference, TEXT).is_ok());
    // Moved by two pixels: caught, even with the text tolerance.
    assert!(compare(&square(2, 128), &reference, TEXT).is_err());
    let smaller = image::RgbaImage::new(59, 60);
    assert!(compare(&smaller, &reference, SHAPES).is_err());
}

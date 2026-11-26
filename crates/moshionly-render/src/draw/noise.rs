//! Film grain: an SkSL runtime shader that lightens and darkens a node's
//! pixels inside their alpha.
//!
//! The shader gives each grain cell a value between -½ and ½ and turns it
//! into white (above 0) or black (below), with an alpha of twice the value
//! times the noise's amount. The painter draws that over the node's
//! content, in the node's own layer, with the `SrcATop` blend mode: each
//! pixel moves toward white or black by that alpha and keeps its own alpha,
//! so the grain stays inside the shape and never clips. Drawing it this way
//! needs no reading back of the content, which as an image filter costs
//! about as much again as the grain itself.
//!
//! The grain must be the same on every machine and every time a frame is
//! drawn, because exports promise the same bytes for the same project and
//! reference images are compared across macOS and Linux. So:
//!
//! - it is seeded from the node's id and, when it moves, the scene's time,
//!   never from a random source or the clock;
//! - the hash uses no `sin` (whose last bits differ between CPUs) and no
//!   floating-point rounding at all: every step is integer arithmetic on
//!   values below 2^24, which `float` holds exactly, so fused or separate
//!   multiply-adds give the same result everywhere.
//!
//! The hash works modulo the prime 4093: the grain cell's coordinates and
//! a seed are combined, then squared twice, each time with a coordinate
//! added. A square of a number below 4093 plus two more stays below 2^24.
//! Simpler permutation hashes leave diagonal streaks in the grain; this one
//! does not, and costs about 20 ms on a whole 1920 by 1080 layer on an
//! Apple M-series CPU (the benchmark example's `effects`). The pattern
//! repeats every 4093 cells.
//!
//! `RuntimeEffect` is not `Send`, so each thread compiles the program once
//! and keeps it.

use moshionly_core::NodeId;
use moshionly_core::model::Noise;
use skia_safe as sk;

/// The grain program. `size` is the side of a grain cell in the node's
/// units; `seed` picks one of the many patterns.
const PROGRAM: &str = r#"
uniform float amount;
uniform float size;
uniform float2 seed;

half4 main(float2 coord) {
    float2 cell = mod(floor(coord / size), 4093.0);
    // Modulo the prime 4093 every value stays below 4093, so a square plus
    // two coordinates stays under 2^24 and is exact. Each square mixes in
    // one more coordinate.
    float h = mod(cell.x * 1531.0 + cell.y + seed.x, 4093.0);
    h = mod(h * h + cell.x + seed.y, 4093.0);
    h = mod(h * h + cell.y, 4093.0);
    float grain = h / 4093.0 - 0.5;
    half strength = half(abs(grain) * 2.0 * amount);
    // Premultiplied white or black.
    return grain > 0.0 ? half4(strength) : half4(0.0, 0.0, 0.0, strength);
}
"#;

thread_local! {
    static EFFECT: Option<sk::RuntimeEffect> =
        sk::RuntimeEffect::make_for_shader(PROGRAM, None).ok();
}

/// Grain cells smaller than this are drawn at this size, so that a cell
/// count stays a number the hash can work with.
const MIN_SIZE: f64 = 0.01;

/// The paint that draws `noise`'s grain over a node's content, or `None`
/// when the grain cannot be seen. Draw it over what the node drew, in a
/// layer of its own.
///
/// `time_ms` is the scene's time; it only changes the grain when the noise
/// is animated.
pub(crate) fn paint(noise: &Noise, id: &NodeId, time_ms: f64) -> Option<sk::Paint> {
    let amount = noise.amount.clamp(0.0, 1.0);
    if !(amount > 0.0 && noise.size.is_finite()) {
        return None;
    }
    let effect = EFFECT.with(Clone::clone)?;
    let [seed_a, seed_b] = seed(id, noise.animated.then_some(time_ms));
    let mut builder = sk::runtime_effect::RuntimeShaderBuilder::new(effect);
    builder.set_uniform_float("amount", &[amount as f32]).ok()?;
    builder
        .set_uniform_float("size", &[noise.size.max(MIN_SIZE) as f32])
        .ok()?;
    builder
        .set_uniform_float("seed", &[seed_a as f32, seed_b as f32])
        .ok()?;
    let mut paint = sk::Paint::default();
    paint.set_shader(builder.make_shader(&sk::Matrix::new_identity())?);
    paint.set_blend_mode(sk::BlendMode::SrcATop);
    Some(paint)
}

/// The two seeds of the grain pattern, each below 4093: a hash of the
/// node's id and, for moving grain, of the time to the microsecond.
pub(crate) fn seed(id: &NodeId, time_ms: Option<f64>) -> [u32; 2] {
    // FNV-1a over the id, then the time.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(id.as_str().as_bytes());
    if let Some(time) = time_ms.filter(|time| time.is_finite()) {
        feed(&[0xff]);
        feed(&((time * 1000.0).round() as i64).to_le_bytes());
    }
    // Fold the high bits in, so both seeds depend on every byte.
    let mixed = hash ^ (hash >> 29);
    [(mixed % 4093) as u32, ((mixed >> 32) % 4093) as u32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_compiles() {
        sk::RuntimeEffect::make_for_shader(PROGRAM, None).unwrap();
        assert!(EFFECT.with(Option::is_some));
    }

    #[test]
    fn seeds_depend_on_the_id_and_a_moving_grains_time_only() {
        let id = NodeId::from("grain");
        assert_eq!(seed(&id, Some(40.0)), seed(&id, Some(40.0)));
        assert_ne!(seed(&id, Some(40.0)), seed(&id, Some(80.0)));
        assert_ne!(seed(&id, None), seed(&NodeId::from("other"), None));
        let [a, b] = seed(&id, Some(1234.5));
        assert!(a < 4093 && b < 4093);
        // Frames at 30 frames per second all get their own grain.
        let frames: std::collections::HashSet<[u32; 2]> = (0..90)
            .map(|frame| seed(&id, Some(f64::from(frame) * 1000.0 / 30.0)))
            .collect();
        assert_eq!(frames.len(), 90);
    }

    #[test]
    fn grain_that_cannot_be_seen_has_no_paint() {
        let id = NodeId::from("n");
        let none = Noise {
            amount: 0.0,
            ..Noise::default()
        };
        assert!(paint(&none, &id, 0.0).is_none());
        assert!(paint(&Noise::default(), &id, 0.0).is_some());
        let broken = Noise {
            size: f64::NAN,
            ..Noise::default()
        };
        assert!(paint(&broken, &id, 0.0).is_none());
    }
}

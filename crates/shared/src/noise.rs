//! Tileable gradient noise. Every lattice wraps with a period that divides the world size, so the
//! terrain repeats seamlessly at the world edges. Only `+` and `*` on f32 are used, so native and
//! wasm builds produce bit-identical heights.

/// 16 unit gradients around the circle (cos/sin of k * 22.5 degrees), spelled out so no trig runs.
#[allow(clippy::approx_constant)]
const GRADIENTS: [(f32, f32); 16] = [
    (1.0, 0.0),
    (0.92388, 0.38268),
    (0.70711, 0.70711),
    (0.38268, 0.92388),
    (0.0, 1.0),
    (-0.38268, 0.92388),
    (-0.70711, 0.70711),
    (-0.92388, 0.38268),
    (-1.0, 0.0),
    (-0.92388, -0.38268),
    (-0.70711, -0.70711),
    (-0.38268, -0.92388),
    (0.0, -1.0),
    (0.38268, -0.92388),
    (0.70711, -0.70711),
    (0.92388, -0.38268),
];

/// SplitMix64-style integer hash.
pub fn hash64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

pub fn hash3(seed: u64, a: u64, b: u64) -> u64 {
    hash64(seed ^ hash64(a.wrapping_mul(0x632B_E59B_D9B4_E019) ^ hash64(b)))
}

/// Uniform float in [0, 1) from a hash.
pub fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Perlin noise in roughly [-1, 1], with `period` lattice cells before it repeats.
/// `x`/`z` are in lattice units.
pub fn perlin(seed: u64, x: f32, z: f32, period: i64) -> f32 {
    let xf = x.floor();
    let zf = z.floor();
    let (ix, iz) = (xf as i64, zf as i64);
    let (fx, fz) = (x - xf, z - zf);
    let grad = |cx: i64, cz: i64, dx: f32, dz: f32| {
        let h = hash3(
            seed,
            cx.rem_euclid(period) as u64,
            cz.rem_euclid(period) as u64,
        );
        let (gx, gz) = GRADIENTS[(h & 15) as usize];
        gx * dx + gz * dz
    };
    let n00 = grad(ix, iz, fx, fz);
    let n10 = grad(ix + 1, iz, fx - 1.0, fz);
    let n01 = grad(ix, iz + 1, fx, fz - 1.0);
    let n11 = grad(ix + 1, iz + 1, fx - 1.0, fz - 1.0);
    let u = fade(fx);
    let v = fade(fz);
    let a = n00 + (n10 - n00) * u;
    let b = n01 + (n11 - n01) * u;
    (a + (b - a) * v) * 1.414
}

/// Fractal noise over world coordinates. `cell` is the size of the coarsest octave in metres and
/// must divide `world`; each octave halves the cell.
pub fn fbm(seed: u64, x: f32, z: f32, world: f32, cell: f32, octaves: u32, gain: f32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    let mut cell = cell;
    for o in 0..octaves {
        let period = (world / cell) as i64;
        sum += perlin(
            hash64(seed.wrapping_add(o as u64)),
            x / cell,
            z / cell,
            period,
        ) * amp;
        norm += amp;
        amp *= gain;
        cell *= 0.5;
    }
    sum / norm
}

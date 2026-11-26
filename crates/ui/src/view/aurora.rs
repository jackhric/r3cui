//! The voice overlay's aurora: light rising from the bottom edge and up the sides, flowing with the voice. Drawn on
//! the CPU at a low resolution, since Slint has no shaders, and scaled up smooth, which is the blur.

use slint::{Rgba8Pixel, SharedPixelBuffer};

pub const W: u32 = 120;
pub const H: u32 = 200;

const BAYER: [[f32; 4]; 4] = [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];
/// Where the four bands swell along the bottom edge, left to right.
const LOBES: [f32; 4] = [0.18, 0.4, 0.62, 0.84];
/// Past this many thicknesses from its edge the light is below one step of 255: nothing there is drawn.
const REACH_OUT: f32 = 6.0;
/// The most the noise widens the light, and the most a band's lobe adds.
const SHAPE_MAX: f32 = 1.4;
const LOBE: f32 = 0.045;
/// Hue steps from the accent, in degrees: a neighbour, a cool counterpoint, a warm tint.
const HUES: [f32; 4] = [0.0, 35.0, 190.0, -40.0];

/// One frame's worth of what the aurora shows.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub seed: u32,
    /// Seconds since it began; the flow runs faster as the voice gets louder.
    pub flow: f32,
    /// 0 to 1: how far in it has come.
    pub reach: f32,
    pub level: f32,
    pub bands: [f32; 4],
    /// 0 to 1: settling while the answer is worked out.
    pub dim: f32,
}

/// Four colours around the accent, as sRGB 0..1, bright enough to glow on black.
pub fn palette(accent: (u8, u8, u8)) -> [[f32; 3]; 4] {
    let (l, c, h) = oklch(accent);
    HUES.map(|step| from_oklch(l.max(0.74), c.max(0.15), h + step))
}

pub fn render(look: &Look, palette: &[[f32; 3]; 4]) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(W, H);
    let aspect = W as f32 / H as f32;
    let seed = look.seed;
    let t = look.flow;
    // How high up the sides it climbs, as a share of the height.
    let rise = look.reach * (0.26 + 0.22 * look.level);
    let strength = 1.0 - 0.55 * look.dim;
    let loudest_band = look.bands.iter().copied().fold(0.0, f32::max);
    let bottom_most = look.reach * (0.04 + 0.05 * look.level + LOBE * loudest_band) * SHAPE_MAX * REACH_OUT;
    let side_most = look.reach * (0.022 + 0.028 * look.level) * SHAPE_MAX * REACH_OUT;
    // The band lobes depend only on the column.
    let lobes: Vec<f32> = (0..W)
        .map(|i| {
            let u = (i as f32 + 0.5) / W as f32;
            LOBES.iter().zip(look.bands).map(|(&c, b)| b * LOBE * (-((u - c) / 0.12).powi(2)).exp()).sum()
        })
        .collect();
    let px = buf.make_mut_slice();
    for j in 0..H {
        let v = (j as f32 + 0.5) / H as f32;
        let up = 1.0 - v;
        let climb = smoothstep(rise, 0.0, up);
        for i in 0..W {
            let u = (i as f32 + 0.5) / W as f32;
            // Most of the screen is far from every edge: left clear, without the noise.
            if up > bottom_most && (climb <= 0.0 || u.min(1.0 - u) * aspect > side_most) {
                continue;
            }
            let (x, y) = (u * aspect, v);
            let warp = fbm(x * 1.6, y * 1.6 + t * 0.05, seed);
            let n = fbm(x * 2.6 + warp * 1.3 + t * 0.12, y * 2.6 - t * 0.17, seed.wrapping_add(1));
            let shape = 0.6 + 0.8 * n;

            let bottom = look.reach * (0.04 + 0.05 * look.level + lobes[i as usize]) * shape;
            let side = look.reach * (0.022 + 0.028 * look.level) * shape * climb;
            let edge = |d: f32, thick: f32| if thick > 1e-4 { (-d / thick).exp() } else { 0.0 };
            let fb = edge(up, bottom);
            let fs = edge(u.min(1.0 - u) * aspect, side);
            // Screen-blended, so the corners join instead of doubling.
            let field = 1.0 - (1.0 - fb) * (1.0 - fs);
            let core = field.powi(4);
            let alpha = ((0.9 * field + 0.5 * core) * strength).clamp(0.0, 1.0);

            // Colour runs along the edge, round the corners and up, and drifts, warped by the same flow.
            let along = u + (2.0 * u - 1.0) * up * 0.6;
            let p = (along * 0.85 + n * 0.45 + t * 0.025 + (seed % 97) as f32 / 97.0).rem_euclid(1.0) * 4.0;
            let (k, f) = (p.floor() as usize % 4, smoothstep(0.0, 1.0, p.fract()));
            let (a, b) = (palette[k], palette[(k + 1) % 4]);
            let hot = core * (0.2 + 0.55 * look.level);
            let dither = (BAYER[(j % 4) as usize][(i % 4) as usize] / 16.0 - 0.5) / 255.0;
            let channel = |c: usize| {
                let base = a[c] + (b[c] - a[c]) * f;
                ((base + (1.0 - base) * hot + dither) * 255.0).round().clamp(0.0, 255.0) as u8
            };
            px[(j * W + i) as usize] = Rgba8Pixel { r: channel(0), g: channel(1), b: channel(2), a: ((alpha + dither) * 255.0).round().clamp(0.0, 255.0) as u8 };
        }
    }
    buf
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (xf, yf) = (x - x.floor(), y - y.floor());
    let (sx, sy) = (xf * xf * (3.0 - 2.0 * xf), yf * yf * (3.0 - 2.0 * yf));
    let (a, b) = (hash(xi, yi, seed), hash(xi + 1, yi, seed));
    let (c, d) = (hash(xi, yi + 1, seed), hash(xi + 1, yi + 1, seed));
    let (top, bottom) = (a + (b - a) * sx, c + (d - c) * sx);
    top + (bottom - top) * sy
}

/// Two octaves, 0 to 1.
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    (noise(x, y, seed) * 0.67 + noise(x * 2.03 + 17.1, y * 2.03 - 9.4, seed ^ 0x9e37) * 0.33).clamp(0.0, 1.0)
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn oklch((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let [r, g, b] = [r, g, b].map(|c| to_linear(f32::from(c) / 255.0));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let (ll, aa, bb) = (
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    );
    (ll, aa.hypot(bb), bb.atan2(aa).to_degrees())
}

/// Back to sRGB, giving up chroma until the colour fits.
fn from_oklch(l: f32, c: f32, h: f32) -> [f32; 3] {
    let (sin, cos) = h.to_radians().sin_cos();
    let mut c = c;
    loop {
        let (a, b) = (c * cos, c * sin);
        let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
        let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
        let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
        let rgb = [
            4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
            -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
            -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
        ];
        if rgb.iter().all(|v| (-0.001..=1.001).contains(v)) || c < 0.01 {
            return rgb.map(to_srgb);
        }
        c *= 0.92;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accent_survives_the_round_trip_and_the_middle_stays_clear() {
        let red = palette((0xE8, 0x34, 0x3A))[0];
        assert!(red[0] > red[1] && red[0] > red[2], "{red:?}");
        let look = Look { seed: 7, flow: 1.0, reach: 1.0, level: 0.6, bands: [0.4; 4], dim: 0.0 };
        let buf = render(&look, &palette((0xE8, 0x34, 0x3A)));
        let at = |u: u32, v: u32| buf.as_slice()[(v * W + u) as usize].a;
        assert!(at(W / 2, H - 1) > 150, "bright along the bottom");
        assert!(at(W / 2, H / 3) < 8, "clear in the middle");
    }
}

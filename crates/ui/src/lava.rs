//! The menu's background: slow lava-lamp blobs in three colours drawn from the accent, ordered-
//! dithered at a low resolution and scaled up with hard pixels. Drawn on the CPU because Slint
//! has no custom shaders; at 120x200 a frame is cheap even on the remote.

use slint::{Rgb8Pixel, SharedPixelBuffer};

pub const W: u32 = 120;
pub const H: u32 = 200;

const BAYER: [[f32; 4]; 4] = [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];

/// (x speed, y speed, phase, radius). Few and far larger than the screen is wide, so what shows
/// is slow drifting fields of colour rather than shapes.
const BLOBS: [(f32, f32, f32, f32); 3] = [(0.031, 0.023, 0.0, 0.62), (0.019, 0.029, 2.1, 0.55), (0.025, 0.017, 4.2, 0.58)];

/// Three colours: black and two dim steps of the accent. `strength` 0..1 sets how faded the whole thing is.
pub fn palette(accent: (u8, u8, u8), strength: f32) -> [[u8; 3]; 3] {
    let base = [accent.0 as f32, accent.1 as f32, accent.2 as f32];
        let scale = |c: [f32; 3], k: f32| c.map(|v| (v * k * strength).round().clamp(0.0, 255.0) as u8);
    [[0, 0, 0], scale(base, 0.07), scale(base, 0.12)]
}

pub fn render(t: f32, colours: [[u8; 3]; 3]) -> SharedPixelBuffer<Rgb8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(W, H);
    let aspect = W as f32 / H as f32;
    let centres: Vec<(f32, f32, f32)> = BLOBS
        .iter()
        .map(|&(sx, sy, p, r)| {
            // Blobs rise and sink like wax, drifting sideways a little.
            let x = 0.5 + 0.6 * (t * sx + p).sin();
            let y = 0.5 + 0.7 * (t * sy + p * 1.3).sin();
            (x, y, r * r)
        })
        .collect();
    let px = buf.make_mut_slice();
    for j in 0..H {
        let v = j as f32 / H as f32;
        for i in 0..W {
            let u = i as f32 / W as f32;
            // A gentle wobble of the sample point keeps the edges from looking like plain circles.
            let wu = u + 0.025 * (v * 9.0 + t * 0.6).sin();
            let wv = v + 0.02 * (u * 7.0 - t * 0.5).cos();
            let field: f32 = centres
                .iter()
                .map(|&(x, y, r2)| {
                    let dx = (wu - x) * aspect;
                    let dy = wv - y;
                    r2 * aspect / (dx * dx + dy * dy + 1e-4)
                })
                .sum();
            // 0 below the edge of the wax, rising to 2 at the cores.
            let level = ((field - 1.6) * 0.9).clamp(0.0, 2.0);
            let threshold = (BAYER[(j % 4) as usize][(i % 4) as usize] + 0.5) / 16.0;
            let index = (level.floor() as usize + usize::from(level.fract() > threshold)).min(2);
            let c = colours[index];
            px[(j * W + i) as usize] = Rgb8Pixel { r: c[0], g: c[1], b: c[2] };
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_only_the_three_colours_and_all_of_them() {
        let colours = palette((232, 52, 58), 1.0);
        let frame = render(3.0, colours);
        let mut seen = [false; 3];
        for p in frame.as_slice() {
            let i = colours.iter().position(|c| *c == [p.r, p.g, p.b]).expect("a palette colour");
            seen[i] = true;
        }
        assert_eq!(seen, [true; 3]);
    }
}

//! The most vibrant colour in a poster, for tinting the now-playing screen and the keys.

use image::DynamicImage;

const BINS: usize = 24;

/// `None` for posters with no real colour in them (black and white, washed out), so the caller
/// keeps its usual accent.
pub fn accent(img: &DynamicImage) -> Option<(u8, u8, u8)> {
    let small = img.resize_exact(40, 60, image::imageops::FilterType::Triangle).to_rgb8();
    let mut weight = [0f32; BINS];
    let mut sum = [[0f32; 3]; BINS];
    let total = (small.width() * small.height()) as f32;
    for p in small.pixels() {
        let [r, g, b] = p.0.map(|c| c as f32 / 255.0);
        let (h, s, v) = hsv(r, g, b);
        if s < 0.35 || v < 0.25 {
            continue;
        }
        // Saturation counts most: a small vivid area should beat a large muddy one.
        let w = s * s * v;
        let bin = ((h / 360.0 * BINS as f32) as usize).min(BINS - 1);
        weight[bin] += w;
        for (acc, c) in sum[bin].iter_mut().zip([r, g, b]) {
            *acc += c * w;
        }
    }
    // Hues near a bin edge split across two bins; judge each bin together with its neighbours.
    let near = |i: usize| weight[(i + BINS - 1) % BINS] + weight[i] + weight[(i + 1) % BINS];
    let best = (0..BINS).max_by(|&a, &b| near(a).total_cmp(&near(b)))?;
    if near(best) < total * 0.015 {
        return None;
    }
    let mut rgb = [0f32; 3];
    let mut w = 0.0;
    for i in [(best + BINS - 1) % BINS, best, (best + 1) % BINS] {
        w += weight[i];
        for (acc, c) in rgb.iter_mut().zip(sum[i]) {
            *acc += c;
        }
    }
    let (h, s, _) = hsv(rgb[0] / w, rgb[1] / w, rgb[2] / w);
    // Lifted to read as an accent on black and on the key LEDs: always bright, never pastel.
    let (r, g, b) = rgb_of(h, s.clamp(0.6, 0.92), 0.95);
    Some(((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8))
}

fn hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn rgb_of(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn a_small_vivid_patch_beats_a_large_dull_one() {
        let mut img = RgbImage::from_pixel(40, 60, Rgb([90, 80, 70]));
        for y in 0..12 {
            for x in 0..12 {
                img.put_pixel(x, y, Rgb([20, 90, 230]));
            }
        }
        let (r, g, b) = accent(&DynamicImage::ImageRgb8(img)).expect("a colour");
        assert!(b > 200 && r < 100 && g < 160, "{r} {g} {b}");
    }

    #[test]
    fn grey_posters_have_no_accent() {
        let img = RgbImage::from_fn(40, 60, |x, _| Rgb([(x * 6) as u8; 3]));
        assert_eq!(accent(&DynamicImage::ImageRgb8(img)), None);
    }
}

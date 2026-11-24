//! Preview only: draws the UI into a photo of the Remote 3 and lights the keys by backlight zone,
//! so stills and recordings show the whole device. Not part of the on-device build path.

use crate::backlight::Keys;
use anyhow::Context;
use image::RgbImage;
use std::path::Path;

pub const WIDTH: u32 = 544;
pub const HEIGHT: u32 = 1800;
const SCREEN: (u32, u32) = (32, 25);
const SCREEN_SIZE: (u32, u32) = (480, 800);
/// The photo is the 462px-wide product shot scaled so its screen is 480px wide.
const SCALE: f32 = 544.0 / 462.0;
const PATCH: i32 = 36;
const GLYPH_RADIUS: f32 = 24.0;
const FADE_SECS: f32 = 0.22;

/// Key glyph centres in the original photo, with the backlight zone each belongs to.
/// Key glyph centres in the original photo, and whether the key is in the power zone.
const KEYS: &[(bool, i32, i32)] = &[
    (false, 230, 843),
    (false, 63, 845),
    (true, 395, 845),
    (false, 64, 954),
    (true, 394, 953),
    (false, 229, 996),
    (false, 103, 1121),
    (false, 355, 1121),
    (false, 229, 1247),
    (false, 64, 1286),
    (false, 394, 1286),
    (false, 63, 1397),
    (false, 176, 1397),
    (false, 286, 1398),
    (false, 396, 1396),
    (false, 63, 1480),
    (false, 175, 1480),
    (false, 287, 1480),
    (false, 397, 1480),
];

struct Key {
    power: bool,
    x0: i32,
    y0: i32,
    /// How much of each pixel is printed glyph, and a blurred copy of that for the glow.
    glyph: Vec<f32>,
    glow: Vec<f32>,
}

pub struct Device {
    base: RgbImage,
    keys: Vec<Key>,
    /// The colours showing now (the rest, then the power zone), easing toward those asked for.
    level: [[f32; 3]; 2],
}

impl Device {
    pub fn load_opt(path: Option<&String>) -> Option<Self> {
        path.and_then(|p| Self::load(p.as_ref()).map_err(|e| tracing::warn!("{e:#}")).ok())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let base = image::open(path).with_context(|| format!("opening {}", path.display()))?.to_rgb8();
        anyhow::ensure!(
            base.dimensions() == (WIDTH, HEIGHT),
            "{} must be {WIDTH}x{HEIGHT}, got {:?}",
            path.display(),
            base.dimensions()
        );
        let side = (2 * PATCH + 1) as usize;
        let keys = KEYS
            .iter()
            .map(|&(power, cx, cy)| {
                let (cx, cy) = ((cx as f32 * SCALE).round() as i32, (cy as f32 * SCALE).round() as i32);
                let (x0, y0) = (cx - PATCH, cy - PATCH);
                let mut glyph = vec![0.0f32; side * side];
                for dy in -PATCH..=PATCH {
                    for dx in -PATCH..=PATCH {
                        if ((dx * dx + dy * dy) as f32).sqrt() > GLYPH_RADIUS {
                            continue;
                        }
                        let p = base.get_pixel((cx + dx) as u32, (cy + dy) as u32).0;
                        let luma = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
                        glyph[((dy + PATCH) as usize) * side + (dx + PATCH) as usize] =
                            ((luma - 80.0) / 130.0).clamp(0.0, 1.0);
                    }
                }
                let glow = blur(&blur(&glyph, side, 5), side, 5);
                Key { power, x0, y0, glyph, glow }
            })
            .collect();
        Ok(Self { base, keys, level: [[0.0; 3]; 2] })
    }

    /// One picture: the photo, the keys lit as `keys` asks, and `screen` (RGBA) on the panel.
    /// `dt` is the time since the previous picture, for the key fade; pass `f32::MAX` for a still.
    pub fn compose(&mut self, screen: &[u8], screen_size: (u32, u32), keys: Keys, dt: f32) -> RgbImage {
        let step = (dt / FADE_SECS).min(1.0) * 255.0;
        for (level, want) in self.level.iter_mut().zip([keys.rest, keys.power]) {
            for (now, target) in level.iter_mut().zip([want.0, want.1, want.2]) {
                *now += (target as f32 - *now).clamp(-step, step);
            }
        }

        let mut out = self.base.clone();
        let side = 2 * PATCH + 1;
        for key in &self.keys {
            let colour = self.level[key.power as usize];
            let lvl = colour.iter().cloned().fold(0.0, f32::max) / 255.0;
            for py in 0..side {
                for px in 0..side {
                    let i = (py * side + px) as usize;
                    let (m, g) = (key.glyph[i], key.glow[i]);
                    if m == 0.0 && g == 0.0 {
                        continue;
                    }
                    let p = out.get_pixel_mut((key.x0 + px) as u32, (key.y0 + py) as u32);
                    for c in 0..3 {
                        let b = p.0[c] as f32;
                        let unlit = b * (1.0 - 0.7 * m);
                        let lit = b * (1.0 - m) + colour[c] * m;
                        let v = unlit + (lit - unlit) * lvl.min(1.0).ceil() + colour[c] * g * 0.5;
                        p.0[c] = v.clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }

        let panel = if screen_size == SCREEN_SIZE {
            image::RgbaImage::from_raw(screen_size.0, screen_size.1, screen.to_vec())
        } else {
            image::RgbaImage::from_raw(screen_size.0, screen_size.1, screen.to_vec()).map(|full| {
                image::imageops::resize(&full, SCREEN_SIZE.0, SCREEN_SIZE.1, image::imageops::FilterType::Triangle)
            })
        };
        if let Some(panel) = panel {
            for (x, y, p) in panel.enumerate_pixels() {
                out.put_pixel(SCREEN.0 + x, SCREEN.1 + y, image::Rgb([p.0[0], p.0[1], p.0[2]]));
            }
        }
        out
    }
}

fn blur(src: &[f32], side: usize, r: i32) -> Vec<f32> {
    let pass = |src: &[f32], horizontal: bool| {
        let mut out = vec![0.0f32; src.len()];
        for y in 0..side as i32 {
            for x in 0..side as i32 {
                let (mut sum, mut n) = (0.0, 0.0);
                for d in -r..=r {
                    let (sx, sy) = if horizontal { (x + d, y) } else { (x, y + d) };
                    if sx >= 0 && sy >= 0 && sx < side as i32 && sy < side as i32 {
                        sum += src[sy as usize * side + sx as usize];
                        n += 1.0;
                    }
                }
                out[y as usize * side + x as usize] = sum / n;
            }
        }
        out
    };
    pass(&pass(src, true), false)
}

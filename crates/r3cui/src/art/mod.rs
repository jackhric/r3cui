//! Artwork: fetching it, and turning it into the poster, backdrop and grain the screen shows.

pub(crate) mod vibrant;

use std::time::Duration;

const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_IMAGE_BYTES: u64 = 24 << 20;
const POSTER_SIZE: (u32, u32) = (384, 576);
const WALL_POSTER_SIZE: (u32, u32) = (136, 204);
const BACKDROP_SIZE: (u32, u32) = (120, 200);
const GRAIN_SIZE: (u32, u32) = (480, 800);

/// Plain HTTP only: Kodi serves its artwork from its own web server on the LAN.
pub(crate) fn fetch_image(url: &str) -> anyhow::Result<image::DynamicImage> {
    use std::io::Read;
    let mut bytes = Vec::new();
    ureq::get(url).timeout(FETCH_TIMEOUT).call()?.into_reader().take(MAX_IMAGE_BYTES).read_to_end(&mut bytes)?;
    Ok(image::load_from_memory(&bytes)?)
}
pub(crate) type Pixels = slint::SharedPixelBuffer<slint::Rgba8Pixel>;
pub(crate) fn pixels(img: image::RgbaImage) -> Pixels {
    Pixels::clone_from_slice(img.as_raw(), img.width(), img.height())
}
pub(crate) fn poster_pixels(img: &image::DynamicImage) -> Pixels {
    pixels(img.resize_to_fill(POSTER_SIZE.0, POSTER_SIZE.1, image::imageops::FilterType::Triangle).to_rgba8())
}
/// A poster at the size the library wall draws it; the wall holds many, so no alpha channel.
pub(crate) fn wall_poster_pixels(img: &image::DynamicImage) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    let (w, h) = WALL_POSTER_SIZE;
    let small = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgb8();
    slint::SharedPixelBuffer::clone_from_slice(small.as_raw(), w, h)
}
/// Monochrome film grain to lay over the backdrop: white and black specks, mostly transparent.
/// Drawn once; a fixed seed keeps it the same from run to run.
pub(crate) fn grain() -> slint::Image {
    let (w, h) = GRAIN_SIZE;
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    let mut x: u32 = 0x9E37_79B9;
    for p in buf.make_mut_slice() {
        // xorshift32
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let v = (x & 0xFF) as u8;
        let a = ((x >> 8) & 0x1F) as u8;
        *p = slint::Rgba8Pixel { r: v, g: v, b: v, a };
    }
    slint::Image::from_rgba8(buf)
}
/// Small, blurred, darker and a little greyer, so it sits behind the controls. The window scales
/// it up smoothly, which softens it further.
pub(crate) fn backdrop_pixels(img: &image::DynamicImage) -> Pixels {
    let small = img.resize_to_fill(BACKDROP_SIZE.0, BACKDROP_SIZE.1, image::imageops::FilterType::Triangle).to_rgba8();
    let mut soft = image::imageops::blur(&small, 3.2);
    for p in soft.pixels_mut() {
        let [r, g, b, _] = p.0;
        let grey = 0.3 * r as f32 + 0.59 * g as f32 + 0.11 * b as f32;
        let mix = |c: u8| ((c as f32 * 0.5 + grey * 0.5) * 0.62) as u8;
        p.0 = [mix(r), mix(g), mix(b), 255];
    }
    pixels(soft)
}

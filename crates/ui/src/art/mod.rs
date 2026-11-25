//! Artwork: fetching it, and turning it into the poster, backdrop and grain the screen shows.

pub(crate) mod vibrant;

use std::time::Duration;

/// Plain HTTP only: Kodi serves its artwork from its own web server on the LAN.
pub(crate) fn fetch_image(url: &str) -> anyhow::Result<image::DynamicImage> {
    use std::io::Read;
    let mut bytes = Vec::new();
    ureq::get(url).timeout(Duration::from_secs(8)).call()?.into_reader().take(24 << 20).read_to_end(&mut bytes)?;
    Ok(image::load_from_memory(&bytes)?)
}
pub(crate) type Pixels = slint::SharedPixelBuffer<slint::Rgba8Pixel>;
pub(crate) fn pixels(img: image::RgbaImage) -> Pixels {
    Pixels::clone_from_slice(img.as_raw(), img.width(), img.height())
}
pub(crate) fn poster_pixels(img: &image::DynamicImage) -> Pixels {
    pixels(img.resize_to_fill(384, 576, image::imageops::FilterType::Triangle).to_rgba8())
}
/// Monochrome film grain to lay over the backdrop: white and black specks, mostly transparent.
/// Drawn once; a fixed seed keeps it the same from run to run.
pub(crate) fn grain() -> slint::Image {
    let (w, h) = (480u32, 800u32);
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
    let small = img.resize_to_fill(120, 200, image::imageops::FilterType::Triangle).to_rgba8();
    let mut soft = image::imageops::blur(&small, 3.2);
    for p in soft.pixels_mut() {
        let [r, g, b, _] = p.0;
        let grey = 0.3 * r as f32 + 0.59 * g as f32 + 0.11 * b as f32;
        let mix = |c: u8| ((c as f32 * 0.5 + grey * 0.5) * 0.62) as u8;
        p.0 = [mix(r), mix(g), mix(b), 255];
    }
    pixels(soft)
}

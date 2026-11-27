//! QR codes drawn for the black screen: the modules white, the rest black, so the code is part of the page.

use qrcode::{Color, QrCode};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Empty modules around the code, as the spec asks for.
const QUIET_MODULES: usize = 2;
const MODULE: Rgba8Pixel = Rgba8Pixel { r: 0xF1, g: 0xF3, b: 0xF3, a: 0xFF };
const GROUND: Rgba8Pixel = Rgba8Pixel { r: 0, g: 0, b: 0, a: 0xFF };

/// The code for `text`, each module a whole number of pixels, as large as fits in `target` pixels a side.
pub(crate) fn image(text: &str, target: u32) -> Option<Image> {
    let code = QrCode::new(text.as_bytes()).map_err(|e| tracing::warn!("QR code for {text:?}: {e}")).ok()?;
    let (modules, side) = (code.to_colors(), code.width());
    let cells = side + 2 * QUIET_MODULES;
    let scale = (target as usize / cells).max(1);
    let size = cells * scale;
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(size as u32, size as u32);
    let out = pixels.make_mut_slice();
    for (y, row) in out.chunks_mut(size).enumerate() {
        for (x, pixel) in row.iter_mut().enumerate() {
            let (mx, my) = (x / scale, y / scale);
            let inside = (QUIET_MODULES..QUIET_MODULES + side).contains(&mx) && (QUIET_MODULES..QUIET_MODULES + side).contains(&my);
            let dark = inside && modules[(my - QUIET_MODULES) * side + (mx - QUIET_MODULES)] == Color::Dark;
            *pixel = if dark { MODULE } else { GROUND };
        }
    }
    Some(Image::from_rgba8(pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_whole_pixels_per_module_white_on_black() {
        let text = "https://github.com/jackhric/r3cui/releases/tag/v0.2.0";
        let cells = QrCode::new(text.as_bytes()).unwrap().width() + 2 * QUIET_MODULES;
        let image = image(text, 168).unwrap();
        let size = image.size().width as usize;
        assert_eq!((size % cells, size <= 168), (0, true));
        let scale = size / cells;
        let buffer = image.to_rgba8().unwrap();
        let pixels = buffer.as_slice();
        assert_eq!(pixels[0], GROUND, "the quiet zone is black");
        let finder = QUIET_MODULES * scale;
        assert_eq!(pixels[finder * size + finder], MODULE, "the finder pattern's corner is white");
    }

    #[test]
    fn a_target_too_small_still_makes_a_code() {
        assert!(image("x", 1).is_some());
    }
}

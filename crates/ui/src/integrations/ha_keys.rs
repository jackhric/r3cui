//! The key backlight as Home Assistant asks for it: a light entity whose colour the keys take, and a select helper
//! choosing between following the remote, that colour, or off. Their entity-id endings come from settings.

use crate::app::entity::{number, text};
use crate::app::settings;
use crate::hardware::backlight::{Ha, Mode, Rgb};
use coreapi::{Attr, Entity, MediaState};

/// What the helpers ask for. Without the mode helper the light alone decides: on means its colour, off means
/// follow the remote.
pub(crate) fn keys(entities: &[Entity]) -> Ha {
    let cfg = settings::get();
    let ending = |suffix: &str| entities.iter().find(|e| !suffix.is_empty() && e.entity_id.ends_with(suffix));
    let colour = ending(&cfg.keys_light).filter(|l| l.state() == MediaState::On).map(|light| {
        let at = |a| number(Some(light), a);
        hsv(at(Attr::Hue), at(Attr::Saturation) / 255.0, at(Attr::Brightness) / 255.0)
    });
    let mode = match text(ending(&cfg.keys_mode), Attr::CurrentOption).to_lowercase().as_str() {
        "off" => Mode::Off,
        "custom" => Mode::Custom,
        "" if colour.is_some() => Mode::Custom,
        _ => Mode::Follow,
    };
    Ha { mode, colour }
}

/// Hue in degrees, saturation and value 0 to 1 (the light's brightness as the value).
fn hsv(h: f64, s: f64, v: f64) -> Rgb {
    let f = |n: f64| {
        let k = (n + h / 60.0) % 6.0;
        ((v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    Rgb(f(5.0), f(3.0), f(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_to_rgb() {
        let rgb = |h, s, v| {
            let Rgb(r, g, b) = hsv(h, s, v);
            (r, g, b)
        };
        assert_eq!(rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(rgb(240.0, 1.0, 0.5), (0, 0, 128));
        assert_eq!(rgb(0.0, 0.0, 1.0), (255, 255, 255));
    }
}

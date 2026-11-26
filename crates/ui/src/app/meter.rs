//! How loud the voice coming in is and where its energy sits, which the voice overlay's aurora moves with. Measured
//! per audio chunk on the session's thread; read every frame on the UI's.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Centre frequencies of the four bands, low to high: voicing, the first and second formants, sibilance.
const BANDS: [f32; 4] = [250.0, 700.0, 1800.0, 4000.0];
/// The quietest a voice is taken to be, and where it counts as loud, in dB below full scale.
const FLOOR_DB: f32 = -55.0;
const LOUD_DB: f32 = -12.0;
/// A reading older than this is silence: the chunks have stopped.
const STALE: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Heard {
    /// 0 silent to 1 loud.
    pub level: f32,
    /// Each band's share of the voice, 0 to 1.
    pub bands: [f32; 4],
}

static LATEST: Mutex<Option<(Heard, Instant)>> = Mutex::new(None);

pub(crate) fn hear(heard: Heard) {
    *LATEST.lock().unwrap() = Some((heard, Instant::now()));
}

/// What was last heard, or silence once the chunks stop.
pub(crate) fn latest() -> Heard {
    match *LATEST.lock().unwrap() {
        Some((h, at)) if at.elapsed() < STALE => h,
        _ => Heard::default(),
    }
}

fn loudness(power: f32) -> f32 {
    let db = 10.0 * power.max(1e-12).log10();
    ((db - FLOOR_DB) / (LOUD_DB - FLOOR_DB)).clamp(0.0, 1.0)
}

/// Signed 16-bit little-endian mono PCM at `rate` Hz.
pub(crate) fn measure(pcm: &[u8], rate: u32) -> Heard {
    let samples: Vec<f32> = pcm.as_chunks::<2>().0.iter().map(|&b| f32::from(i16::from_le_bytes(b)) / 32768.0).collect();
    if samples.is_empty() {
        return Heard::default();
    }
    let n = samples.len() as f32;
    let level = loudness(samples.iter().map(|s| s * s).sum::<f32>() / n);
    // Goertzel: one frequency's energy without a whole FFT.
    let band = |freq: f32| {
        let coeff = 2.0 * (std::f32::consts::TAU * freq / rate as f32).cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &x in &samples {
            let s0 = x + coeff * s1 - s2;
            (s2, s1) = (s1, s0);
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (n * n)
    };
    let power = BANDS.map(band);
    let total: f32 = power.iter().sum::<f32>().max(1e-12);
    let bands = power.map(|p| (p / total * level * 2.0).clamp(0.0, 1.0));
    Heard { level, bands }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, amplitude: f32) -> Vec<u8> {
        (0..1600).flat_map(|i| (((i as f32 / 16000.0 * freq * std::f32::consts::TAU).sin() * amplitude * 32767.0) as i16).to_le_bytes()).collect()
    }

    #[test]
    fn silence_is_quiet_and_a_tone_lands_in_its_band() {
        assert_eq!(measure(&vec![0; 3200], 16000).level, 0.0);
        let h = measure(&tone(700.0, 0.5), 16000);
        assert!(h.level > 0.8, "{h:?}");
        let loudest = (0..4).max_by(|&a, &b| h.bands[a].total_cmp(&h.bands[b])).unwrap();
        assert_eq!(loudest, 1, "{h:?}");
        assert!(measure(&tone(700.0, 0.01), 16000).level < h.level);
    }
}

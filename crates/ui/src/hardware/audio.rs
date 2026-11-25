//! Cue sounds. The stock UI plays its own WAVs through Qt Multimedia; whether this sandboxed
//! process may open the sound card is not known, so the device build only probes `/dev/snd` and
//! logs what it finds. The desktop build pipes the WAV to `aplay`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Confirm,
    Success,
    Error,
}

impl Sound {
    #[cfg_attr(feature = "device", allow(dead_code))]
    fn wav(self) -> &'static [u8] {
        match self {
            Self::Confirm => include_bytes!("../../assets/sounds/confirm.wav"),
            Self::Success => include_bytes!("../../assets/sounds/success.wav"),
            Self::Error => include_bytes!("../../assets/sounds/error.wav"),
        }
    }
}

pub trait Backend: Send {
    /// Returns at once; `volume` is 0 to 1.
    fn play(&mut self, sound: Sound, volume: f32);
}

/// The WAV with every 16-bit sample scaled by `volume`.
#[cfg_attr(feature = "device", allow(dead_code))]
pub fn scaled(wav: &[u8], volume: f32) -> Vec<u8> {
    let mut out = wav.to_vec();
    let Some(at) = wav.windows(4).position(|w| w == b"data") else { return out };
    let start = at + 8;
    for pair in out.get_mut(start..).unwrap_or_default().as_chunks_mut::<2>().0 {
        let v = i16::from_le_bytes(*pair);
        pair.copy_from_slice(&((f32::from(v) * volume.clamp(0.0, 1.0)) as i16).to_le_bytes());
    }
    out
}

#[cfg(not(feature = "device"))]
struct Aplay {
    missing: bool,
}

#[cfg(not(feature = "device"))]
impl Backend for Aplay {
    fn play(&mut self, sound: Sound, volume: f32) {
        use std::io::Write;
        use std::process::{Command, Stdio};
        if self.missing || volume <= 0.0 {
            return;
        }
        let child = Command::new("aplay").args(["-q", "-t", "wav", "-"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("no aplay, cue sounds are off: {e}");
                self.missing = true;
                return;
            }
        };
        let wav = scaled(sound.wav(), volume);
        std::thread::spawn(move || {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&wav);
            }
            let _ = child.wait();
        });
    }
}

#[cfg(feature = "device")]
struct Silent;

#[cfg(feature = "device")]
impl Backend for Silent {
    fn play(&mut self, sound: Sound, _volume: f32) {
        tracing::debug!(?sound, "no audio backend on the device");
    }
}

#[cfg(feature = "device")]
fn probe() {
    match std::fs::read_dir("/dev/snd") {
        Ok(dir) => {
            let nodes: Vec<String> = dir.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            let writable = nodes.iter().filter(|n| n.starts_with("pcm") && n.ends_with('p')).any(|n| {
                std::fs::OpenOptions::new().write(true).open(format!("/dev/snd/{n}")).is_ok()
            });
            tracing::info!(?nodes, writable, "audio: /dev/snd");
        }
        Err(e) => tracing::info!("audio: no /dev/snd: {e}"),
    }
}

pub fn backend() -> Box<dyn Backend> {
    #[cfg(feature = "device")]
    {
        probe();
        Box::new(Silent)
    }
    #[cfg(not(feature = "device"))]
    {
        Box::new(Aplay { missing: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_scales_samples_not_the_header() {
        let wav = Sound::Confirm.wav();
        let half = scaled(wav, 0.5);
        assert_eq!(wav.len(), half.len());
        assert_eq!(&wav[..44], &half[..44]);
        let peak = |w: &[u8]| w[44..].as_chunks::<2>().0.iter().map(|p| i16::from_le_bytes(*p).unsigned_abs()).max().unwrap();
        assert!(peak(&half) <= peak(wav) / 2 + 1 && peak(&half) > 0);
        assert_eq!(peak(&scaled(wav, 0.0)), 0);
    }
}

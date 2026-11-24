#!/usr/bin/env python3
"""Writes the UI's short cue sounds (16 kHz mono 16-bit WAV) into crates/ui/assets/sounds."""
import math, struct, sys, wave
from pathlib import Path

RATE = 16000
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent / "crates/ui/assets/sounds"


def tone(parts, peak=0.6):
    samples = []
    for freq, ms in parts:
        n = RATE * ms // 1000
        for i in range(n):
            env = min(1.0, i / 80) * (1 - i / n) ** 2
            samples.append(peak * env * math.sin(2 * math.pi * freq * i / RATE))
    return samples


def save(name, samples):
    OUT.mkdir(parents=True, exist_ok=True)
    with wave.open(str(OUT / f"{name}.wav"), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, s)) * 32767)) for s in samples))


save("confirm", tone([(1320, 45)]))
save("success", tone([(988, 55), (1319, 90)]))
save("error", tone([(330, 70), (247, 120)], peak=0.7))

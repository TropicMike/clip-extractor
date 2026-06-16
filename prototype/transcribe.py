"""Throwaway prototype: transcribe a media file with faster-whisper.

Reference only. The real app uses whisper-rs (Rust). Kept to validate output.

Usage:
    pip install -r requirements.txt
    python transcribe.py ../fixtures/sample.mp4
"""
import sys
from faster_whisper import WhisperModel

src = sys.argv[1] if len(sys.argv) > 1 else r"../fixtures/sample.mp4"

model = WhisperModel("base", device="cpu", compute_type="int8")
segments, info = model.transcribe(src, beam_size=5)

print(f"[lang={info.language} p={info.language_probability:.2f}]")
for s in segments:
    print(f"[{s.start:.2f}-{s.end:.2f}] {s.text.strip()}")

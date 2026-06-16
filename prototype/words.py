"""Throwaway prototype: word-level timestamps with faster-whisper.

Reference only. Used to pick precise clip in/out points.

Usage:
    pip install -r requirements.txt
    python words.py ../fixtures/sample.mp4
"""
import sys
from faster_whisper import WhisperModel

src = sys.argv[1] if len(sys.argv) > 1 else r"../fixtures/sample.mp4"

model = WhisperModel("base", device="cpu", compute_type="int8")
segments, info = model.transcribe(src, beam_size=5, word_timestamps=True)

for s in segments:
    for w in s.words:
        print(f"{w.start:.2f}-{w.end:.2f}  {w.word}")

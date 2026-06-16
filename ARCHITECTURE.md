# Clipper — Architecture Plan

A cross-platform desktop app: transcribe a video/audio file, show the transcript,
let the user pick clip in/out points, and export an audio (or video) clip.

## Decision summary

Chosen direction based on requirements:
- **Fully native UI, no web/webview**
- **Rust is acceptable** (and preferred — lets us ship a single self-contained binary with no Python runtime)

## Stack

### GUI framework — `egui`
- Immediate-mode Rust GUI (renders via `wgpu`/`glow`), cross-platform (Windows/macOS/Linux).
- Drawing a waveform and draggable in/out handles is a few lines on egui's `Painter`.
- Alternatives considered:
  - **Slint** — most polished/native look, declarative `.slint` markup. Pick this if visual polish > build speed.
  - **Iced** — Elm-style, retained-mode, has a built-in `Canvas`. Good for larger apps.

### Transcription — `whisper-rs`
- Rust bindings to whisper.cpp. No Python. GPU-optional. Provides word-level timestamps.
- Ship a ggml model file (e.g. `ggml-base.en.bin`, ~150 MB) or download on first run.

### Audio decode / waveform data — `symphonia`
- Pure-Rust decoder; produces samples to render the waveform.

### Playback (preview) — `rodio`
- Simple cross-platform audio playback for previewing the selected region.

### Clip export — `ffmpeg-sidecar`
- Downloads/bundles a static ffmpeg binary and runs it.
- Same command we validated manually:
  `ffmpeg -y -ss <start> -to <end> -i <input> -vn -q:a 2 <out.mp3>`
  (drop `-vn` and choose a video codec for a video clip.)

## UI layout (sketch)

```
+-- egui window ------------------------------------+
|  Transcript (clickable words) ...... whisper-rs   |
|  -------------------------------------------------|
|  Waveform + draggable in/out handles . symphonia  |
|  [>] preview ........................ rodio       |
|  [Export clip] ...................... ffmpeg      |
+---------------------------------------------------+
```

## Interaction model
- Open file -> decode audio (symphonia) -> render waveform.
- Run whisper-rs -> populate transcript with per-word timestamps.
- Click a word -> seek playhead / set selection edge.
- Drag in/out handles on the waveform -> sets clip range (snaps to word boundaries optionally).
- Preview plays just the selected range (rodio).
- Export -> ffmpeg-sidecar cuts the clip.

## Validated reference values (from the prototype)
- Sample file: `fixtures/sample.mp4` (Joe Pesci green-screen meme, 8s).
- Transcript: "What the fuck is this piece of shit?"
- Word timestamps (whisper base model):
  - What 4.94-5.54 / the 5.54-5.82 / fuck 5.82-6.22 / is 6.22-6.52 /
    this 6.52-6.80 / piece 6.80-7.10 / of 7.10-7.36 / shit 7.36-7.78
- Best tight soundbite cut: **4.8 -> 8.0** (see `fixtures/sample_soundbite.mp3`).

## Suggested crate list (Cargo.toml)
```toml
[dependencies]
eframe = "0.28"          # egui + window/app shell
egui = "0.28"
whisper-rs = "0.12"      # whisper.cpp bindings
symphonia = { version = "0.5", features = ["all"] }
rodio = "0.19"
ffmpeg-sidecar = "1"
rfd = "0.14"             # native open/save file dialogs
```

## Build milestones
1. Window opens, file picker (rfd), decode + draw static waveform.
2. Run whisper-rs on the file, render transcript words.
3. Draggable in/out selection + live range readout.
4. Preview playback of the selection (rodio).
5. Export via ffmpeg-sidecar; match the prototype command.
6. Package per-OS (cargo-bundle / cargo-wix / .app).

## Prototype reference
The `prototype/` folder holds the throwaway Python (faster-whisper) scripts used to
validate transcription + word timestamps. They are reference only — the real app
uses whisper-rs and does not need Python.

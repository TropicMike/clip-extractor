# Clipper

Cross-platform desktop app to transcribe a video/audio file, browse the transcript,
select clip in/out points, and export the clip.

- **Plan / architecture:** see [ARCHITECTURE.md](ARCHITECTURE.md)
- **Stack:** Rust + egui (native UI), whisper-rs (transcription), symphonia (decode),
  rodio (preview), ffmpeg-sidecar (export). No Python at runtime.

## Layout

```
.
├── Cargo.toml
├── README.md
├── ARCHITECTURE.md          # the design plan
├── .gitignore
├── src/
│   └── main.rs              # egui skeleton (milestone 1)
├── prototype/               # reference-only Python scripts (faster-whisper)
│   ├── transcribe.py
│   ├── words.py
│   └── requirements.txt
└── fixtures/                # sample assets for testing
    ├── sample.mp4
    ├── sample_soundbite.mp3
    └── transcript.txt
```

## Build & run

Requires Rust (https://rustup.rs) and `ffmpeg` on PATH (export uses it).

```powershell
winget install Rustlang.Rustup   # if you don't have Rust yet, then restart shell
cargo run
```

## Status — milestone 1 (skeleton)
Working: window, file picker, transcript list (demo data), waveform panel with
draggable green (in) / red (out) handles, and **Export** that shells out to
`ffmpeg` to cut the selected range to MP3.

Placeholders, wired up at later milestones (see ARCHITECTURE.md):
- Waveform is synthetic until `symphonia` decoding is added.
- Transcript is demo data until `whisper-rs` is added.
- Export uses system `ffmpeg`; moves to `ffmpeg-sidecar` for bundling.

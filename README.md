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

### Required tools

| Tool | Why | Download |
|------|-----|----------|
| Rust | builds the app | https://rustup.rs |
| CMake | build-time: compiles whisper.cpp | https://cmake.org/download/ |
| LLVM / libclang | build-time: bindgen FFI for whisper-rs | https://releases.llvm.org/ |
| ffmpeg | runtime: clip export + download merge | https://ffmpeg.org/download.html (Windows static: https://www.gyan.dev/ffmpeg/builds/) |
| yt-dlp | runtime: URL / YouTube download | https://github.com/yt-dlp/yt-dlp/releases |

The base whisper model downloads automatically on first use. ffmpeg and yt-dlp
are only needed on `PATH` for a default `cargo run`; the
`--features embed-assets` release bundles them into the binary instead.

```powershell
winget install Rustlang.Rustup   # if you don't have Rust yet, then restart shell
cargo run
```

### macOS (build from source)

> Not yet tested on macOS — developed and verified on Windows. The code has no
> hard Windows dependencies and CI builds macOS (Intel + Apple Silicon), but
> expect possible first-build tweaks.

`whisper-rs` compiles whisper.cpp, so the build machine needs **CMake** and
**libclang/LLVM** (bindgen); a default run also needs **ffmpeg** and **yt-dlp**
on `PATH` (export + URL download). The base model downloads on first use.

```bash
brew install cmake llvm ffmpeg yt-dlp        # rustup separately if needed
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"   # so bindgen finds libclang
cargo run
```

The single self-contained binary (`cargo build --release --features embed-assets`)
embeds ffmpeg + yt-dlp + the base model. Those live in `vendor/<target-triple>/`
and `models/`, which are **gitignored — not in the repo**: either run the release
workflow (`.github/workflows/release.yml`, which stages them per-arch) or place
them there manually first. A plain `cargo run` does not need them.

## Status — milestone 1 (skeleton)
Working: window, file picker, transcript list (demo data), waveform panel with
draggable green (in) / red (out) handles, and **Export** that shells out to
`ffmpeg` to cut the selected range to MP3.

Placeholders, wired up at later milestones (see ARCHITECTURE.md):
- Waveform is synthetic until `symphonia` decoding is added.
- Transcript is demo data until `whisper-rs` is added.
- Export uses system `ffmpeg`; moves to `ffmpeg-sidecar` for bundling.

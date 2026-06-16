# clip-extractor

A cross-platform native desktop app (Windows / macOS / Linux) for turning a video
into a clip. Paste a **URL (e.g. YouTube) or a local file**, it **transcribes the
speech** into a word-level transcript, you **pick the in/out points** by clicking
words or dragging the waveform handles, and **export** the selection as **MP3
(audio) or MP4 (video)**.

Fully native Rust — no web view, and no Python at runtime.

- **Architecture & rationale:** see [ARCHITECTURE.md](ARCHITECTURE.md)
- **Stack:** Rust + [egui](https://github.com/emilk/egui) (native UI),
  [whisper-rs](https://github.com/tazz4843/whisper-rs) (transcription),
  [symphonia](https://github.com/pdeljanov/Symphonia) (audio decode),
  `yt-dlp` (download) and `ffmpeg` (export) as sidecars.

## Features

- **Source:** a YouTube/other URL (downloaded with `yt-dlp`) or any local media file.
- **Transcription:** word-level timestamps via `whisper-rs` (whisper.cpp); choose a
  **base / small / medium** model in the UI for the speed/accuracy trade-off.
- **Waveform:** the real audio waveform (decoded with `symphonia`, with an `ffmpeg`
  fallback for codecs symphonia lacks, e.g. Opus) with draggable **green (in)** /
  **red (out)** handles and a shaded selection.
- **Transcript:** click a word to snap the selection to its time span.
- **Export:** the selected range to **MP3** or **MP4**, written to a chosen output
  directory.
- **Responsive UI:** download, decode, and transcription run on worker threads with
  live progress and a cancel button.

## Using it

1. Launch the app (`cargo run`).
2. **Source** — paste a URL or local path (or *Browse…*).
3. **Output dir** — where exported clips are written (or *Browse…*).
4. Choose the **model** (base/small/medium) and output **format** (MP4/MP3).
5. **Load / Transcribe** — downloads (if a URL), decodes the audio, and transcribes.
   The chosen model is downloaded automatically the first time it's used (the base
   model ships embedded in release builds).
6. Set the range: click a transcript word, or drag the in/out handles on the waveform.
7. **Export clip** — writes `<name>_<in>-<out>.<ext>` to the output directory.

## Build & run

### Required tools

| Tool | Why | Download |
|------|-----|----------|
| Rust | builds the app | https://rustup.rs |
| CMake | build-time: compiles whisper.cpp | https://cmake.org/download/ |
| LLVM / libclang | build-time: bindgen FFI for whisper-rs | https://releases.llvm.org/ |
| ffmpeg | runtime: clip export + download merge | https://ffmpeg.org/download.html (Windows static: https://www.gyan.dev/ffmpeg/builds/) |
| yt-dlp | runtime: URL / YouTube download | https://github.com/yt-dlp/yt-dlp/releases |

The base whisper model downloads automatically on first use. `ffmpeg` and `yt-dlp`
are only needed on `PATH` for a default `cargo run`; the `--features embed-assets`
release bundles them into the binary instead (see below).

### Windows

```powershell
winget install Rustlang.Rustup        # Rust (restart shell afterwards)
winget install LLVM.LLVM              # libclang for bindgen
# CMake: from VS "Desktop development with C++", or: winget install Kitware.CMake
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"   # so bindgen finds libclang
cargo run
```

### macOS

> Not yet tested on macOS — developed and verified on Windows. The code has no hard
> Windows dependencies and CI builds macOS (Intel + Apple Silicon), but expect
> possible first-build tweaks.

```bash
brew install cmake llvm ffmpeg yt-dlp        # rustup separately if needed
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"   # so bindgen finds libclang
cargo run
```

### Linux

```bash
sudo apt-get install -y cmake clang llvm-dev libclang-dev ffmpeg   # Debian/Ubuntu
# yt-dlp: package manager or https://github.com/yt-dlp/yt-dlp/releases
cargo run
```

## Single self-contained binary

`cargo build --release --features embed-assets` produces one executable per platform
with `ffmpeg`, `yt-dlp`, and the base whisper model embedded (via `include_bytes!`)
and extracted to the per-user cache dir at runtime — no system `ffmpeg`/`yt-dlp` and
no model download needed.

The embedded assets are **not** in the repo (they're large and gitignored). Stage
them under `vendor/<target-triple>/` (`ffmpeg`, `yt-dlp`) and `models/`
(`ggml-base.en.bin`) before building, or just push a `v*` tag and let
[`.github/workflows/release.yml`](.github/workflows/release.yml) build all platforms
(it stages the right binaries per-arch). A plain `cargo run` does not need any of this.

## Tests

```bash
cargo test                 # unit/integration tests (decode, transcribe, export, parsing)
cargo test -- --ignored    # also runs network/staged-file tests (download + full pipeline)
```

The transcription test checks output against the validated fixture transcript; the
export test compares an MP3 cut against `fixtures/sample_soundbite.mp3`.

## Project layout

```
src/
├── main.rs          # entry point + module declarations
├── app.rs           # egui UI, app state, worker-message drain, job dispatch
├── model.rs         # shared types: Word, Phase, WorkerMsg, OutputFormat, WhisperModelKind
├── audio.rs         # symphonia decode (+ ffmpeg fallback) -> waveform + 16 kHz PCM
├── transcribe.rs    # whisper-rs transcription -> word-level timestamps
├── download.rs      # yt-dlp download + progress parsing
├── export.rs        # ffmpeg MP3/MP4 export
└── resources.rs     # cache paths, on-demand model download, embedded-asset extraction
build.rs             # stages embedded assets when building with --features embed-assets
.github/workflows/
└── release.yml      # per-OS single-binary release build
fixtures/            # sample.mp4, sample_soundbite.mp3, transcript.txt (test assets)
prototype/           # reference-only Python (faster-whisper) used to validate output
```

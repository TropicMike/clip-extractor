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
- **Preview:** play back the selected range (via `rodio`) before exporting.
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
7. **Preview** the selected range to check the cut before exporting.
8. **Export clip** — writes `<name>_<in>-<out>.<ext>` to the output directory.

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

Builds and runs on macOS (developed on Apple Silicon; CI also builds Intel).

```bash
brew install cmake llvm ffmpeg yt-dlp        # rustup separately if needed
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"   # so bindgen finds libclang
cargo run
```

> If you launch a built `.app` from Finder/Spotlight instead of a terminal, it
> won't inherit your shell `PATH`, so a Homebrew `ffmpeg`/`yt-dlp` in
> `/opt/homebrew/bin` won't be found. Run from a terminal for a dev build, or use
> the `embed-assets` release (below), which bundles both and needs neither on `PATH`.

### Linux

```bash
sudo apt-get install -y cmake clang llvm-dev libclang-dev ffmpeg   # Debian/Ubuntu
# yt-dlp: package manager or https://github.com/yt-dlp/yt-dlp/releases
cargo run
```

## Standalone executable (single self-contained binary)

`cargo build --release --features embed-assets` produces **one executable per platform**
with `ffmpeg`, `yt-dlp`, and the base whisper model embedded (via `include_bytes!`) and
extracted to the per-user cache dir at runtime — the end user needs no system
`ffmpeg`/`yt-dlp` and no model download. whisper.cpp is statically linked, so there's
no runtime library dependency either.

The build machine still needs the build toolchain (Rust + CMake + libclang — see
[Required tools](#required-tools)). Only the *end user* gets a dependency-free binary.

### Easiest: let CI build all platforms

Push a `v*` tag (or run the workflow manually) and
[`.github/workflows/release.yml`](.github/workflows/release.yml) builds every target,
staging the right sidecars per-arch and uploading a `clip-extractor-<platform>` artifact:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

Targets built: `windows-x86_64`, `linux-x86_64`, `macos-x86_64` (Intel),
`macos-aarch64` (Apple Silicon).

### Building one locally

The embedded assets are **not** in the repo (large + gitignored). Stage three files for
your target triple, then build. Replace `<TARGET>` with your triple:

| Platform | Target triple (`<TARGET>`) |
|----------|----------------------------|
| Windows x86_64 | `x86_64-pc-windows-msvc` |
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| macOS Intel | `x86_64-apple-darwin` |
| macOS Apple Silicon | `aarch64-apple-darwin` |

Required layout before building (filenames matter; `.exe` suffix on Windows):

```
vendor/<TARGET>/ffmpeg          # or ffmpeg.exe on Windows
vendor/<TARGET>/yt-dlp          # or yt-dlp.exe  on Windows
models/ggml-base.en.bin
```

**macOS** (Apple Silicon shown; for Intel use target `x86_64-apple-darwin`):

```bash
TARGET=aarch64-apple-darwin
mkdir -p "vendor/$TARGET" models
curl -L https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip -o ff.zip
unzip -o ff.zip -d ff && cp ff/ffmpeg "vendor/$TARGET/ffmpeg"
curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos -o "vendor/$TARGET/yt-dlp"
curl -L https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin -o models/ggml-base.en.bin
chmod +x "vendor/$TARGET/ffmpeg" "vendor/$TARGET/yt-dlp"

export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
cargo build --release --features embed-assets --target "$TARGET"
# -> target/aarch64-apple-darwin/release/clip-extractor
```

**Linux** (`x86_64-unknown-linux-gnu`):

```bash
TARGET=x86_64-unknown-linux-gnu
mkdir -p "vendor/$TARGET" models
curl -L https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz -o ff.tar.xz
tar xf ff.tar.xz && cp ffmpeg-*-static/ffmpeg "vendor/$TARGET/ffmpeg"
curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux -o "vendor/$TARGET/yt-dlp"
curl -L https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin -o models/ggml-base.en.bin
chmod +x "vendor/$TARGET/ffmpeg" "vendor/$TARGET/yt-dlp"

cargo build --release --features embed-assets --target "$TARGET"
# -> target/x86_64-unknown-linux-gnu/release/clip-extractor
```

**Windows** (`x86_64-pc-windows-msvc`, PowerShell):

```powershell
$TARGET = "x86_64-pc-windows-msvc"
New-Item -ItemType Directory -Force -Path "vendor/$TARGET", models | Out-Null
Invoke-WebRequest "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" -OutFile ff.zip
Expand-Archive ff.zip -DestinationPath ff
Copy-Item (Get-ChildItem ff -Recurse -Filter ffmpeg.exe | Select-Object -First 1).FullName "vendor/$TARGET/ffmpeg.exe"
Invoke-WebRequest "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "vendor/$TARGET/yt-dlp.exe"
Invoke-WebRequest "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin" -OutFile "models/ggml-base.en.bin"

$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
cargo build --release --features embed-assets --target $TARGET
# -> target\x86_64-pc-windows-msvc\release\clip-extractor.exe
```

> Cross-compiling (e.g. building the Intel macOS binary on an Apple Silicon Mac) also
> needs `rustup target add <TARGET>` and a matching cross-linker; the simplest path to
> all four binaries is the CI workflow above. A plain `cargo run` needs none of this.

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

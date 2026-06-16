# clip-extractor — project context

Cross-platform desktop app: transcribe a video/audio file, browse the transcript,
select clip in/out points, and export the clip (audio or video).

## Stack decision (fully native, no web; Rust)
- **UI:** `egui` / `eframe` 0.28 — immediate-mode native GUI (Win/macOS/Linux).
- **Transcription:** `whisper-rs` 0.16 (whisper.cpp bindings) — no Python at runtime.
- **Decode → waveform + 16 kHz PCM:** `symphonia` (pure Rust).
- **Download:** `yt-dlp` (sidecar binary) for URL/YouTube input.
- **Export:** `ffmpeg` (sidecar binary) — MP3 or MP4.
- **Single-file distribution:** ffmpeg + yt-dlp + base model embedded via
  `include_bytes!` (the `embed-assets` feature) and extracted to the cache dir.

Full rationale and alternatives (Slint/Iced, NiceGUI, Tauri) are in `ARCHITECTURE.md`.

## Current state — full app built (M0–M10)
Modules under `src/`: `main` (entry), `app` (UI + state + worker-message drain),
`model` (shared types: `Word`, `Phase`, `WorkerMsg`, `OutputFormat`,
`WhisperModelKind`), `audio` (symphonia decode), `transcribe` (whisper-rs),
`download` (yt-dlp), `export` (ffmpeg), `resources` (cache paths, model download,
embedded-asset extraction). `build.rs` stages embedded assets.

Pipeline: paste a URL or local path + output dir → (yt-dlp download) → symphonia
decode → on-demand model fetch → whisper transcription → click words / drag
green(in)/red(out) handles → export MP3 or MP4. Long jobs run on worker threads
that message the UI via a `crossbeam-channel`; a `Phase` enum drives progress +
button enablement; transcription is cancellable.

Verified by `cargo test` (decode, transcribe-vs-fixture, export MP3+MP4, yt-dlp
progress parse, HTTPS download; an `embed-assets`-gated extraction test).

## Build & run
Requires Rust, **CMake**, and **libclang/LLVM** on the build machine (whisper-rs
compiles whisper.cpp via CMake and generates bindings via bindgen/libclang). A
system `ffmpeg`/`yt-dlp` on PATH is convenient for non-embedded dev builds.
```powershell
winget install Rustlang.Rustup        # Rust
winget install LLVM.LLVM              # libclang for bindgen
# CMake: from VS "Desktop development with C++", or `winget install Kitware.CMake`
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"   # so bindgen finds libclang
cargo run                              # dev build: ffmpeg/yt-dlp from PATH, models downloaded
cargo build --release --features embed-assets      # single self-contained binary
```
`embed-assets` requires `vendor/<target-triple>/{ffmpeg,yt-dlp}` and
`models/ggml-base.en.bin` to exist (CI stages these — see
`.github/workflows/release.yml`).

### ⚠️ whisper-rs 0.16 gotcha
`FullParams::set_abort_callback_safe` is **buggy** (double-boxes the closure but
the trampoline reinterprets the pointer as the raw closure type) → encode aborts
with "failed to encode" / `GenericError(-6)`. We wire the abort callback via the
`unsafe` `set_abort_callback` + a hand-written trampoline reading the cancel
flag. Do not switch back to the `_safe` variant. (`set_progress_callback_safe` is
fine.) See `src/transcribe.rs`.

## Milestones — all complete
0. ✅ Toolchain + skeleton compiles (egui 0.28; no API drift needed).
1. ✅ Module split. 2. ✅ Async scaffolding (channels + `Phase`).
3. ✅ Input fields (source/out-dir/model picker/format toggle).
4. ✅ Real decode + waveform (symphonia). 5. ✅ Transcription (whisper-rs).
6. ✅ Export MP3/MP4. 7. ✅ yt-dlp download. 8. ✅ On-demand model download.
9. ✅ Embed sidecars + base model (`embed-assets`). 10. ✅ CI release matrix.
Optional future: rodio preview playback; OS installers (cargo-wix/cargo-bundle).

## Reference assets
- `fixtures/sample.mp4` — Joe Pesci green-screen meme, 8s test clip.
- `fixtures/sample_soundbite.mp3` — validated tight cut 4.8→8.0s.
- `fixtures/transcript.txt` — transcript + per-word timestamps.
- `prototype/` — throwaway Python (faster-whisper) used to validate transcription;
  reference only, not part of the app.

## Validated transcript (sample.mp4)
"What the fuck is this piece of shit?" — words:
What 4.94–5.54 / the 5.54–5.82 / fuck 5.82–6.22 / is 6.22–6.52 /
this 6.52–6.80 / piece 6.80–7.10 / of 7.10–7.36 / shit 7.36–7.78.
Best tight soundbite cut: **4.8 → 8.0**.

## Notes
- `Cargo.lock` is committed (binary app).
- `vendor/` and `models/` are gitignored (large; staged by CI or locally for
  `embed-assets` builds).
- whisper uses English `.en` ggml weights; non-speech tokens (`*Music*`, `[..]`,
  `(..)`) are filtered from the transcript in `transcribe.rs`.
- Default cache locations: models in `<cache>/clip-extractor/models`, extracted
  sidecars in `<cache>/clip-extractor/bin` (`%LOCALAPPDATA%` / `$XDG_CACHE_HOME`).

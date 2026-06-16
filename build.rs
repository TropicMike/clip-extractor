//! Build script: when the `embed-assets` feature is on, stage the per-target
//! sidecar binaries and the base whisper model into `OUT_DIR` so `resources.rs`
//! can `include_bytes!` them. Does nothing otherwise.

use std::path::PathBuf;

fn main() {
    // Only stage assets when the embedding feature is enabled.
    if std::env::var_os("CARGO_FEATURE_EMBED_ASSETS").is_none() {
        return;
    }

    let target = std::env::var("TARGET").expect("TARGET");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let exe = if target.contains("windows") { ".exe" } else { "" };

    let vendor = manifest.join("vendor").join(&target);
    let sources = [
        (vendor.join(format!("ffmpeg{exe}")), "ffmpeg.bin"),
        (vendor.join(format!("yt-dlp{exe}")), "ytdlp.bin"),
        (manifest.join("models").join("ggml-base.en.bin"), "ggml-base.en.bin"),
    ];

    for (src, staged) in sources {
        if !src.exists() {
            panic!(
                "embed-assets: missing required asset {}\n\
                 Populate vendor/{target}/ (ffmpeg{exe}, yt-dlp{exe}) and \
                 models/ggml-base.en.bin before building with --features embed-assets.",
                src.display()
            );
        }
        std::fs::copy(&src, out.join(staged))
            .unwrap_or_else(|e| panic!("failed to stage {}: {e}", src.display()));
        println!("cargo:rerun-if-changed={}", src.display());
    }
}

//! Locations for cached assets (whisper models, and later the extracted
//! sidecar binaries), plus on-demand model download.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use eframe::egui;

use crate::model::{WhisperModelKind, WorkerMsg};

/// Per-user cache root for this app, e.g. `%LOCALAPPDATA%\clip-extractor`.
pub fn cache_root() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("XDG_CACHE_HOME"))
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.cache")))
        .unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(base).join("clip-extractor")
}

/// Directory whisper models are cached in.
pub fn models_dir() -> PathBuf {
    cache_root().join("models")
}

/// Directory extracted sidecar binaries live in.
pub fn bin_dir() -> PathBuf {
    cache_root().join("bin")
}

/// Expected on-disk path of a given model.
pub fn model_path(kind: WhisperModelKind) -> PathBuf {
    models_dir().join(kind.filename())
}

#[cfg_attr(not(feature = "embed-assets"), allow(dead_code))]
fn ffmpeg_exe() -> &'static str {
    if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }
}

fn ytdlp_exe() -> &'static str {
    if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }
}

/// Assets embedded into the binary under the `embed-assets` feature.
#[cfg(feature = "embed-assets")]
mod embedded {
    pub const FFMPEG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ffmpeg.bin"));
    pub const YTDLP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ytdlp.bin"));
    pub const BASE_MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ggml-base.en.bin"));
}

/// Write `bytes` to `dir/name` if missing or a different size (cheap version
/// check), making it executable on Unix. Returns the path.
#[cfg(feature = "embed-assets")]
fn extract(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    let stale = std::fs::metadata(&path)
        .map(|m| m.len() != bytes.len() as u64)
        .unwrap_or(true);
    if stale {
        std::fs::write(&path, bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// Path to the ffmpeg executable: the extracted embedded copy when built with
/// `embed-assets`, otherwise rely on `PATH`.
pub fn ffmpeg_path() -> PathBuf {
    #[cfg(feature = "embed-assets")]
    {
        if let Ok(p) = extract(&bin_dir(), ffmpeg_exe(), embedded::FFMPEG) {
            return p;
        }
    }
    PathBuf::from("ffmpeg")
}

/// Path to the yt-dlp executable: the extracted embedded copy when built with
/// `embed-assets`, else a cached copy under the bin dir, else `PATH`.
pub fn ytdlp_path() -> PathBuf {
    #[cfg(feature = "embed-assets")]
    {
        if let Ok(p) = extract(&bin_dir(), ytdlp_exe(), embedded::YTDLP) {
            return p;
        }
    }
    let cached = bin_dir().join(ytdlp_exe());
    if cached.is_file() {
        cached
    } else {
        PathBuf::from("yt-dlp")
    }
}

/// Ensure `kind`'s model is on disk, downloading it if needed, on a worker
/// thread. Reports `ModelProgress`, then `ModelReady`/`ModelFailed`.
pub fn spawn_ensure_model(
    kind: WhisperModelKind,
    tx: Sender<WorkerMsg>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let path = model_path(kind);
        if path.is_file() {
            let _ = tx.send(WorkerMsg::ModelReady(path));
            ctx.request_repaint();
            return;
        }

        // The base model ships embedded — extract it rather than downloading.
        #[cfg(feature = "embed-assets")]
        if kind.is_embedded() {
            let msg = match extract(&models_dir(), kind.filename(), embedded::BASE_MODEL) {
                Ok(p) => WorkerMsg::ModelReady(p),
                Err(e) => WorkerMsg::ModelFailed(format!("Could not extract model: {e}")),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
            return;
        }

        let progress_tx = tx.clone();
        let progress_ctx = ctx.clone();
        let result = download_file(kind.url(), &path, &cancel, move |pct| {
            let _ = progress_tx.send(WorkerMsg::ModelProgress { pct });
            progress_ctx.request_repaint();
        });
        let msg = match result {
            Ok(true) => WorkerMsg::ModelReady(path),
            Ok(false) => WorkerMsg::ModelFailed("Model download cancelled.".to_owned()),
            Err(e) => WorkerMsg::ModelFailed(format!("Model download failed: {e}")),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
}

/// Stream `url` to `dest` (via a `.partial` temp file, then rename), reporting
/// fractional progress. Returns `Ok(false)` if cancelled. Generic so it can be
/// reused (and tested) independently of model kinds.
pub(crate) fn download_file(
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32),
) -> Result<bool, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let tmp = dest.with_extension("partial");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 1 << 16];
    let mut downloaded: u64 = 0;
    let mut last_pct = -1i32;

    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Ok(false);
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        if total > 0 {
            let pct = (downloaded as f32 / total as f32).clamp(0.0, 1.0);
            let p100 = (pct * 100.0) as i32;
            if p100 != last_pct {
                last_pct = p100;
                on_progress(pct);
            }
        }
    }

    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercise the HTTPS downloader (TLS, redirects, Content-Length progress,
    /// atomic rename) against a small stable file. Network-dependent, so
    /// `#[ignore]`d by default; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "network: downloads a small file over HTTPS"]
    fn downloads_small_file_with_progress() {
        let dest = std::env::temp_dir()
            .join("clip-extractor-dl-test")
            .join("SHA2-256SUMS");
        let _ = std::fs::remove_file(&dest);
        let cancel = AtomicBool::new(false);
        let mut ticks = 0u32;
        let ok = download_file(
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS",
            &dest,
            &cancel,
            |_p| ticks += 1,
        )
        .expect("download should succeed");

        assert!(ok, "should not report cancelled");
        let len = std::fs::metadata(&dest).expect("file exists").len();
        assert!(len > 0, "downloaded file is empty");
        assert!(ticks >= 1, "no progress callbacks fired");
    }

    /// With assets embedded, the sidecar paths must resolve to extracted files
    /// whose sizes match the embedded bytes.
    #[cfg(feature = "embed-assets")]
    #[test]
    fn extracts_embedded_sidecars() {
        let ff = ffmpeg_path();
        assert!(ff.is_absolute() && ff.is_file(), "ffmpeg not extracted: {}", ff.display());
        assert_eq!(
            std::fs::metadata(&ff).unwrap().len() as usize,
            embedded::FFMPEG.len(),
            "extracted ffmpeg size mismatch"
        );

        let yt = ytdlp_path();
        assert!(yt.is_absolute() && yt.is_file(), "yt-dlp not extracted: {}", yt.display());
        assert_eq!(
            std::fs::metadata(&yt).unwrap().len() as usize,
            embedded::YTDLP.len(),
            "extracted yt-dlp size mismatch"
        );
    }
}

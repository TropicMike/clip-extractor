//! Video/audio download via yt-dlp.
//!
//! Currently invokes a system/cached `yt-dlp`; M9 switches to the extracted
//! sidecar binary. yt-dlp uses ffmpeg (also on PATH for now) to merge formats.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use eframe::egui;

use crate::model::WorkerMsg;

/// Download `url` into `out_dir` on a worker thread, streaming progress.
/// Reports `DownloadProgress`, then `DownloadDone`/`DownloadFailed`.
pub fn spawn_download(
    ytdlp: PathBuf,
    url: String,
    out_dir: PathBuf,
    tx: Sender<WorkerMsg>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let msg = match download(&ytdlp, &url, &out_dir, &tx, &ctx, &cancel) {
            Ok(Some(path)) => WorkerMsg::DownloadDone(path),
            Ok(None) => WorkerMsg::DownloadFailed("Download cancelled.".to_owned()),
            Err(e) => WorkerMsg::DownloadFailed(e),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
}

fn download(
    ytdlp: &Path,
    url: &str,
    out_dir: &Path,
    tx: &Sender<WorkerMsg>,
    ctx: &egui::Context,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<PathBuf>, String> {
    let mut cmd = Command::new(ytdlp);
    cmd.args([
        "--newline",
        "--no-playlist",
        // Prefer mp4 video + m4a (AAC) audio so the pure-Rust decoder can read
        // it directly; fall back to whatever's best (decode has an ffmpeg path).
        "-f",
        "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b",
        "--merge-output-format",
        "mp4",
        "-o",
        "%(title)s.%(ext)s",
        "--print",
        "after_move:filepath",
        "--no-simulate",
    ]);

    // yt-dlp needs ffmpeg to merge video+audio. If we have a bundled/extracted
    // ffmpeg (absolute path), point yt-dlp at it; otherwise it uses PATH.
    let ffmpeg = crate::resources::ffmpeg_path();
    if ffmpeg.is_absolute() {
        if let Some(dir) = ffmpeg.parent() {
            cmd.arg("--ffmpeg-location").arg(dir);
        }
    }

    let mut child = cmd
        .arg("-P")
        .arg(out_dir)
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run yt-dlp: {e} (is it installed?)"))?;

    // Drain stderr on a side thread so a full pipe can't deadlock the reader.
    let stderr = child.stderr.take().expect("piped stderr");
    let err_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut buf);
        buf
    });

    let stdout = child.stdout.take().expect("piped stdout");
    let mut final_path: Option<PathBuf> = None;
    for line in BufReader::new(stdout).lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            // yt-dlp status line, e.g. "[download]  45.6% of 10.00MiB at ...".
            if let Some(pct) = parse_progress(trimmed) {
                let _ = tx.send(WorkerMsg::DownloadProgress {
                    pct,
                    eta: String::new(),
                });
                ctx.request_repaint();
            }
        } else {
            // Non-status line: the path printed by `--print after_move:filepath`.
            final_path = Some(PathBuf::from(trimmed));
        }
    }

    let status = child.wait().map_err(|e| format!("yt-dlp wait failed: {e}"))?;
    let err_text = err_handle.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    if !status.success() {
        let last = err_text.lines().last().unwrap_or("unknown error");
        return Err(format!("yt-dlp failed: {last}"));
    }
    match final_path {
        Some(p) if p.exists() => Ok(Some(p)),
        _ => Err("download finished but output file was not found".to_owned()),
    }
}

/// Parse a yt-dlp `[download] NN.N% ...` line into a 0.0..=1.0 fraction.
fn parse_progress(line: &str) -> Option<f32> {
    if !line.starts_with("[download]") {
        return None;
    }
    let pct_idx = line.find('%')?;
    let num = line[..pct_idx]
        .rsplit(char::is_whitespace)
        .next()?
        .trim();
    num.parse::<f32>().ok().map(|p| (p / 100.0).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_download_progress() {
        assert_eq!(
            parse_progress("[download]   0.0% of 10.00MiB at 1.00MiB/s ETA 00:10"),
            Some(0.0)
        );
        let p = parse_progress("[download]  45.6% of 10.00MiB at 1.00MiB/s ETA 00:05").unwrap();
        assert!((p - 0.456).abs() < 1e-4, "got {p}");
        assert_eq!(
            parse_progress("[download] 100% of 10.00MiB in 00:10"),
            Some(1.0)
        );
        // Non-progress status lines yield nothing.
        assert_eq!(parse_progress("[youtube] Extracting URL"), None);
        assert_eq!(parse_progress("[Merger] Merging formats"), None);
    }

    /// Full pipeline against a real, short, stable YouTube video ("Me at the
    /// zoo", ~19s): download -> decode -> transcribe. Network-dependent and
    /// requires the base model present, so `#[ignore]`d by default.
    #[test]
    #[ignore = "network: downloads a YouTube video and transcribes it"]
    fn youtube_end_to_end() {
        let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
        let dir = std::env::temp_dir().join("cx-e2e");
        std::fs::create_dir_all(&dir).unwrap();

        let (tx, _rx) = crossbeam_channel::unbounded();
        let ctx = egui::Context::default();
        let cancel = Arc::new(AtomicBool::new(false));

        let path = download(&crate::resources::ytdlp_path(), url, &dir, &tx, &ctx, &cancel)
            .expect("download failed")
            .expect("download cancelled");
        eprintln!("downloaded: {}", path.display());

        let decoded = crate::audio::decode(&path).expect("decode failed");
        eprintln!(
            "decoded: {:.1}s, {} samples @16k",
            decoded.duration,
            decoded.pcm16k.len()
        );

        let model = crate::resources::model_path(crate::model::WhisperModelKind::Base);
        assert!(model.is_file(), "base model required at {}", model.display());
        let words =
            crate::transcribe::transcribe(&model, &decoded.pcm16k, &cancel, |_p| {})
                .expect("transcribe failed");
        let text: String = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("TRANSCRIPT ({} words): {text}", words.len());
        assert!(!words.is_empty(), "no words transcribed");
    }
}

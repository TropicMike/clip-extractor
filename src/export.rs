//! Clip export via ffmpeg.
//!
//! Currently shells out to a system `ffmpeg`; M9 switches to the extracted
//! sidecar binary.

use std::path::{Path, PathBuf};
use std::process::Command;

use crossbeam_channel::Sender;
use eframe::egui;

use crate::model::{OutputFormat, WorkerMsg};

/// Export the selected range on a worker thread; report `ExportDone`/`ExportFailed`.
pub fn spawn_export(
    input: PathBuf,
    out_dir: PathBuf,
    sel_in: f32,
    sel_out: f32,
    format: OutputFormat,
    tx: Sender<WorkerMsg>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        let msg = match export(&input, &out_dir, sel_in, sel_out, format) {
            Ok(out) => WorkerMsg::ExportDone(out),
            Err(e) => WorkerMsg::ExportFailed(e),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
}

/// Blocking export. Returns the written file path.
fn export(
    input: &Path,
    out_dir: &Path,
    sel_in: f32,
    sel_out: f32,
    format: OutputFormat,
) -> Result<PathBuf, String> {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip");
    let name = format!("{stem}_{sel_in:.1}-{sel_out:.1}.{}", format.ext());
    let out = out_dir.join(name);

    let mut cmd = Command::new(crate::resources::ffmpeg_path());
    cmd.args([
        "-y",
        "-ss",
        &format!("{sel_in:.3}"),
        "-to",
        &format!("{sel_out:.3}"),
        "-i",
    ])
    .arg(input);

    match format {
        // Audio-only, V2 VBR MP3 (matches the validated fixture cut).
        OutputFormat::Mp3 => {
            cmd.args(["-vn", "-q:a", "2"]);
        }
        // Re-encode for a frame-accurate video cut with web-friendly moov atom.
        OutputFormat::Mp4 => {
            cmd.args([
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
            ]);
        }
    }
    cmd.arg(&out);

    let result = cmd
        .output()
        .map_err(|e| format!("Could not run ffmpeg: {e} (is it on PATH?)"))?;

    if result.status.success() {
        Ok(out)
    } else {
        let err = String::from_utf8_lossy(&result.stderr)
            .lines()
            .last()
            .unwrap_or("unknown error")
            .to_owned();
        Err(format!("ffmpeg failed: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Export the validated 4.8→8.0 cut of the fixture as both MP3 and MP4 and
    /// confirm non-empty files are produced. Requires a system ffmpeg with an
    /// H.264 encoder.
    #[test]
    fn exports_mp3_and_mp4() {
        let input = Path::new("fixtures/sample.mp4");
        let dir = std::env::temp_dir().join("clip-extractor-export-test");
        std::fs::create_dir_all(&dir).unwrap();

        for fmt in [OutputFormat::Mp3, OutputFormat::Mp4] {
            let out = export(input, &dir, 4.8, 8.0, fmt)
                .unwrap_or_else(|e| panic!("export .{}: {e}", fmt.ext()));
            let len = std::fs::metadata(&out).expect("output exists").len();
            assert!(len > 0, "empty .{} output", fmt.ext());
        }
    }
}

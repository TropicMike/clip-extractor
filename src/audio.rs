//! Audio decode via symphonia.
//!
//! Produces three things from any container/codec symphonia supports:
//!   - a downsampled waveform *envelope* (peak per bucket) for drawing,
//!   - the total duration in seconds,
//!   - a mono f32 PCM buffer resampled to 16 kHz for whisper.
//!
//! Resampling uses a simple box-average (downsample) / linear (upsample) pass.
//! This is adequate for speech recognition; swap in `rubato` if higher fidelity
//! is ever needed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crossbeam_channel::Sender;
use eframe::egui;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::model::WorkerMsg;

/// Sample rate whisper expects.
const WHISPER_SR: u32 = 16_000;
/// Number of points in the drawn waveform envelope.
const WAVEFORM_BUCKETS: usize = 2000;

pub struct DecodeResult {
    /// Peak-amplitude envelope for drawing (values in [0, 1]-ish).
    pub waveform: Vec<f32>,
    pub duration: f32,
    /// Mono f32 PCM at 16 kHz for transcription.
    pub pcm16k: Vec<f32>,
}

/// Decode `path` on a worker thread; report `DecodeDone`/`DecodeFailed`.
pub fn spawn_decode(path: PathBuf, tx: Sender<WorkerMsg>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let msg = match decode(&path) {
            Ok(r) => WorkerMsg::DecodeDone {
                samples: r.waveform,
                duration: r.duration,
                pcm16k: Arc::new(r.pcm16k),
            },
            Err(e) => WorkerMsg::DecodeFailed(format!("Decode failed: {e}")),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
}

/// Blocking decode. Returns the waveform envelope, duration, and 16 kHz mono PCM.
///
/// Tries the pure-Rust symphonia path first; if that can't handle the codec
/// (e.g. Opus, common in YouTube audio), falls back to ffmpeg.
pub fn decode(path: &Path) -> Result<DecodeResult, Box<dyn std::error::Error>> {
    match decode_symphonia(path) {
        Ok(r) => Ok(r),
        Err(sym_err) => decode_with_ffmpeg(path)
            .map_err(|ff_err| format!("symphonia: {sym_err}; ffmpeg: {ff_err}").into()),
    }
}

fn decode_symphonia(path: &Path) -> Result<DecodeResult, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no decodable audio track found")?;
    let track_id = track.id;
    let src_rate = track.codec_params.sample_rate.unwrap_or(44_100);

    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    // Accumulate the full-rate mono signal.
    let mut mono: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let ch = spec.channels.count().max(1);
                let mut sbuf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                sbuf.copy_interleaved_ref(decoded);
                // Downmix interleaved frames to mono.
                for frame in sbuf.samples().chunks(ch) {
                    let sum: f32 = frame.iter().copied().sum();
                    mono.push(sum / ch as f32);
                }
            }
            Err(SymphoniaError::DecodeError(_)) => continue, // skip a corrupt packet
            Err(e) => return Err(e.into()),
        }
    }

    if mono.is_empty() {
        return Err("decoded zero audio samples".into());
    }

    let duration = mono.len() as f32 / src_rate as f32;
    let waveform = envelope(&mono, WAVEFORM_BUCKETS);
    let pcm16k = resample_to_16k(&mono, src_rate);

    Ok(DecodeResult {
        waveform,
        duration,
        pcm16k,
    })
}

/// Decode via ffmpeg: ask it for 16 kHz mono f32 PCM on stdout. Handles any
/// codec ffmpeg supports (Opus, etc.). The waveform is built from the 16 kHz
/// signal here (sufficient for display).
fn decode_with_ffmpeg(path: &Path) -> Result<DecodeResult, Box<dyn std::error::Error>> {
    let output = Command::new(crate::resources::ffmpeg_path())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg decode failed: {}", err.trim()).into());
    }

    let pcm16k: Vec<f32> = output
        .stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if pcm16k.is_empty() {
        return Err("ffmpeg produced no audio samples".into());
    }

    let duration = pcm16k.len() as f32 / WHISPER_SR as f32;
    let waveform = envelope(&pcm16k, WAVEFORM_BUCKETS);
    Ok(DecodeResult {
        waveform,
        duration,
        pcm16k,
    })
}

/// Peak-per-bucket envelope of `mono`, for drawing.
fn envelope(mono: &[f32], buckets: usize) -> Vec<f32> {
    if mono.is_empty() {
        return Vec::new();
    }
    let buckets = buckets.min(mono.len());
    let chunk = mono.len() / buckets;
    let mut out = Vec::with_capacity(buckets);
    for b in 0..buckets {
        let start = b * chunk;
        let end = if b + 1 == buckets {
            mono.len()
        } else {
            start + chunk
        };
        let peak = mono[start..end]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        out.push(peak);
    }
    out
}

/// Resample mono PCM to 16 kHz. Box-average when downsampling (acts as a crude
/// anti-alias filter), linear interpolation when upsampling.
fn resample_to_16k(mono: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == WHISPER_SR || mono.is_empty() {
        return mono.to_vec();
    }
    let out_len = (((mono.len() as f64) * WHISPER_SR as f64 / src_rate as f64).round() as usize)
        .max(1);
    let mut out = Vec::with_capacity(out_len);

    if src_rate > WHISPER_SR {
        let step = src_rate as f64 / WHISPER_SR as f64; // > 1 input samples per output
        for i in 0..out_len {
            let start = ((i as f64 * step).floor() as usize).min(mono.len() - 1);
            let end = (((i + 1) as f64 * step).floor() as usize).clamp(start + 1, mono.len());
            let slice = &mono[start..end];
            let avg = slice.iter().copied().sum::<f32>() / slice.len() as f32;
            out.push(avg);
        }
    } else {
        let step = src_rate as f64 / WHISPER_SR as f64; // < 1
        let last = mono.len() - 1;
        for i in 0..out_len {
            let pos = i as f64 * step;
            let idx = (pos.floor() as usize).min(last);
            let frac = (pos - idx as f64) as f32;
            let a = mono[idx];
            let b = mono[(idx + 1).min(last)];
            out.push(a + (b - a) * frac);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode the real fixture (Joe Pesci clip, ~8s) and sanity-check the output.
    #[test]
    fn decodes_sample_fixture() {
        let r = decode(Path::new("fixtures/sample.mp4")).expect("decode sample.mp4");

        // The clip is ~8 seconds.
        assert!(
            (r.duration - 8.0).abs() < 1.0,
            "unexpected duration {}",
            r.duration
        );

        // 16 kHz mono PCM length should track the duration.
        let expected = (r.duration * WHISPER_SR as f32) as usize;
        let diff = (r.pcm16k.len() as i64 - expected as i64).abs();
        assert!(
            diff < WHISPER_SR as i64,
            "pcm16k len {} vs expected ~{}",
            r.pcm16k.len(),
            expected
        );

        // Waveform should be non-empty and carry real (non-zero) signal.
        assert!(!r.waveform.is_empty(), "waveform empty");
        assert!(
            r.waveform.iter().any(|&v| v > 0.0),
            "waveform is entirely zero"
        );
    }

    /// Verify the ffmpeg fallback decodes a file symphonia can't (Opus). Uses a
    /// file staged at `%TEMP%/me-at-zoo-opus.mp4`; skips if absent. Network/setup
    /// dependent, so `#[ignore]`d.
    #[test]
    #[ignore = "requires an Opus-audio file staged at %TEMP%/me-at-zoo-opus.mp4"]
    fn ffmpeg_fallback_decodes_opus() {
        let path = std::env::temp_dir().join("me-at-zoo-opus.mp4");
        if !path.is_file() {
            eprintln!("skipping: no Opus file at {}", path.display());
            return;
        }
        // symphonia alone should reject it...
        assert!(
            decode_symphonia(&path).is_err(),
            "expected symphonia to reject Opus"
        );
        // ...but the public decode() falls back to ffmpeg and succeeds.
        let r = decode(&path).expect("ffmpeg fallback decode");
        assert!(r.duration > 1.0, "duration {}", r.duration);
        assert!(!r.pcm16k.is_empty(), "no PCM");
        eprintln!("ffmpeg fallback: {:.1}s, {} samples", r.duration, r.pcm16k.len());
    }
}

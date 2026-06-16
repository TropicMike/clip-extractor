//! Transcription via whisper-rs (whisper.cpp).
//!
//! Feeds 16 kHz mono f32 PCM to whisper and produces word-level `Word`s by
//! enabling per-token timestamps and splitting segments on word boundaries.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use eframe::egui;
use whisper_rs::{
    DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext,
    WhisperContextParameters,
};

use crate::model::{WhisperModelKind, Word, WorkerMsg};

/// Transcribe `pcm16k` with the model at `model_path` on a worker thread.
/// Reports `TranscribeProgress`, then `TranscribeDone`/`TranscribeFailed`.
pub fn spawn_transcribe(
    model_path: PathBuf,
    model_kind: WhisperModelKind,
    pcm16k: Arc<Vec<f32>>,
    tx: Sender<WorkerMsg>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress_ctx = ctx.clone();
        let result = transcribe(&model_path, model_kind, &pcm16k, &cancel, move |pct| {
            let _ = progress_tx.send(WorkerMsg::TranscribeProgress(pct as f32 / 100.0));
            progress_ctx.request_repaint();
        });
        let msg = match result {
            Ok(words) => WorkerMsg::TranscribeDone(words),
            Err(e) => WorkerMsg::TranscribeFailed(format!("Transcription failed: {e}")),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
}

/// Blocking transcription. `progress` is called with a percentage in `0..=100`.
pub fn transcribe(
    model_path: &Path,
    model_kind: WhisperModelKind,
    pcm16k: &[f32],
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(i32) + 'static,
) -> Result<Vec<Word>, Box<dyn std::error::Error>> {
    if pcm16k.is_empty() {
        return Err("no audio to transcribe".into());
    }
    if !pcm16k.iter().all(|s| s.is_finite()) {
        return Err("audio contains non-finite samples".into());
    }

    let model = model_path.to_str().ok_or("model path is not valid UTF-8")?;
    // Enable DTW token-level timestamps. Whisper's default per-token timestamps
    // are a heuristic that drifts badly past the 30 s window boundary (most
    // visibly on the larger models), placing words tens of seconds from where
    // they were actually spoken. DTW aligns each token to the audio via the
    // model's attention heads, which requires a preset matching the model.
    let mut cparams = WhisperContextParameters::default();
    cparams.dtw_parameters(DtwParameters {
        mode: DtwMode::ModelPreset {
            model_preset: dtw_preset(model_kind),
        },
        ..Default::default()
    });
    let ctx = WhisperContext::new_with_params(model, cparams)?;
    let mut state = ctx.create_state()?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8) as i32)
        .unwrap_or(4);
    params.set_n_threads(threads);
    // Word-level granularity: one token per "segment", split on word boundaries.
    params.set_token_timestamps(true);
    params.set_max_len(1);
    params.set_split_on_word(true);
    params.set_progress_callback_safe(progress);

    // whisper-rs 0.16's `set_abort_callback_safe` has a type-erasure bug (it
    // double-boxes the closure but the trampoline reinterprets the pointer as
    // the raw closure type), which makes encode abort immediately. Wire the
    // abort callback ourselves: pass a raw pointer to the cancel flag — kept
    // alive by the caller's `Arc` for the duration of `full` — and read it in a
    // matching trampoline.
    unsafe extern "C" fn abort_trampoline(user_data: *mut std::ffi::c_void) -> bool {
        let flag = &*(user_data as *const AtomicBool);
        flag.load(Ordering::Relaxed)
    }
    unsafe {
        params.set_abort_callback(Some(abort_trampoline));
        params.set_abort_callback_user_data(Arc::as_ptr(cancel) as *mut std::ffi::c_void);
    }

    state.full(params, pcm16k)?;

    // `max_len(1)` + `split_on_word` makes each segment a single word. Take its
    // start from the earliest DTW-aligned token time (centiseconds). DTW only
    // populates per-token `t_dtw`; the segment-level t0/t1 stay heuristic, so we
    // must not use them. `t_dtw` is -1 when unavailable (special tokens, or no
    // DTW) — fall back to the segment start only if a word has no aligned token.
    let mut starts: Vec<(f32, String)> = Vec::new();
    for segment in state.as_iter() {
        let text = segment.to_str()?.trim().to_owned();
        // Skip whisper's non-speech annotations ("*Music*", "[Music]", …) and
        // stray punctuation-only segments the larger models sometimes emit.
        if text.is_empty() || is_non_speech(&text) || !text.chars().any(char::is_alphanumeric) {
            continue;
        }
        let mut start_cs = i64::MAX;
        for i in 0..segment.n_tokens() {
            if let Some(tok) = segment.get_token(i) {
                let t = tok.token_data().t_dtw;
                if t >= 0 {
                    start_cs = start_cs.min(t);
                }
            }
        }
        let start = if start_cs != i64::MAX {
            start_cs as f32 / 100.0
        } else {
            segment.start_timestamp() as f32 / 100.0
        };
        starts.push((start, text));
    }

    // DTW gives one aligned point per token, so a word's end is taken as the
    // next word's start. Cap the span so a word before a long pause doesn't
    // stretch across silence; the last word gets a short default.
    const MAX_WORD: f32 = 1.5;
    const LAST_WORD: f32 = 0.5;
    let audio_dur = pcm16k.len() as f32 / 16_000.0;
    let mut words = Vec::with_capacity(starts.len());
    for i in 0..starts.len() {
        let (start, ref text) = starts[i];
        let end = match starts.get(i + 1) {
            Some((next, _)) => next.min(start + MAX_WORD),
            None => (start + LAST_WORD).min(audio_dur),
        }
        .max(start + 0.01);
        words.push(Word {
            start,
            end,
            text: text.clone(),
        });
    }
    Ok(words)
}

/// DTW alignment preset matching the loaded ggml weights. We use English-only
/// (`.en`) weights, so pick the `*En` presets.
fn dtw_preset(kind: WhisperModelKind) -> DtwModelPreset {
    match kind {
        WhisperModelKind::Base => DtwModelPreset::BaseEn,
        WhisperModelKind::Small => DtwModelPreset::SmallEn,
        WhisperModelKind::Medium => DtwModelPreset::MediumEn,
    }
}

/// Whisper marks non-speech audio with bracketed/asterisked tokens such as
/// `*Music*`, `[Music]`, or `(applause)`. Treat any wholly-enclosed token as
/// non-speech so the transcript shows only spoken words.
fn is_non_speech(text: &str) -> bool {
    matches!(
        (text.chars().next(), text.chars().last()),
        (Some('*'), Some('*')) | (Some('['), Some(']')) | (Some('('), Some(')'))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WhisperModelKind;

    /// Regression test for DTW word timestamps: prepend 50 s of silence to the
    /// fixture so the phrase falls well past whisper's 30 s window boundary, and
    /// assert each cached model places the first word near ~55 s. Before DTW the
    /// heuristic timestamps drifted ~15 s early on the medium model (words found,
    /// positions wrong). Runs for whichever models are cached; skips otherwise.
    #[test]
    fn dtw_timestamps_survive_window_boundary() {
        let decoded =
            crate::audio::decode(Path::new("fixtures/sample.mp4")).expect("decode fixture");
        let mut long = vec![0.0f32; 50 * 16_000];
        long.extend_from_slice(&decoded.pcm16k);

        let mut ran = false;
        for kind in WhisperModelKind::ALL {
            let model = crate::resources::model_path(kind);
            if !model.is_file() {
                continue;
            }
            ran = true;
            let cancel = Arc::new(AtomicBool::new(false));
            let words = transcribe(&model, kind, &long, &cancel, |_p| {}).expect("transcribe");
            let first = words.first().expect("no words produced");
            assert_eq!(
                first.text.to_lowercase(),
                "what",
                "{:?}: first word {:?}",
                model.file_name(),
                first.text
            );
            // The original fixture speech starts ~5 s in, so after 50 s of
            // silence it should land ~55 s — not collapsed back toward 0–40 s.
            assert!(
                (50.0..58.0).contains(&first.start),
                "{:?}: first word at {:.2}s (expected ~55s)",
                model.file_name(),
                first.start
            );
        }
        if !ran {
            eprintln!("skipping: no whisper models cached");
        }
    }

    /// Transcribe the real fixture and check it against the validated ground
    /// truth from CLAUDE.md. Skips (does not fail) if the base model isn't
    /// present in the cache, so the suite still runs without the download.
    #[test]
    fn transcribes_sample_fixture() {
        let model = crate::resources::model_path(WhisperModelKind::Base);
        if !model.is_file() {
            eprintln!("skipping: base model not found at {}", model.display());
            return;
        }

        let decoded =
            crate::audio::decode(Path::new("fixtures/sample.mp4")).expect("decode fixture");
        let cancel = Arc::new(AtomicBool::new(false));
        let words = transcribe(&model, WhisperModelKind::Base, &decoded.pcm16k, &cancel, |_p| {})
            .expect("transcribe");

        assert!(!words.is_empty(), "no words produced");
        let joined = words
            .iter()
            .map(|w| w.text.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        // Expected phrase: "What the fuck is this piece of shit?" (ignore case/punct).
        for expected in ["what", "the", "is", "this", "piece", "of"] {
            assert!(joined.contains(expected), "missing '{expected}' in: {joined}");
        }

        // Non-speech ("*Music*") must be filtered, so the first word is "What"
        // near ~5s and the speech ends near ~8s.
        let first = words.first().unwrap();
        let last = words.last().unwrap();
        assert_eq!(first.text.to_lowercase(), "what", "first word: {}", first.text);
        assert!(
            (4.0..6.0).contains(&first.start),
            "unexpected first start {}",
            first.start
        );
        assert!(
            (7.0..9.0).contains(&last.end),
            "unexpected last end {}",
            last.end
        );
    }
}

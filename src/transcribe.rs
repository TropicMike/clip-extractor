//! Transcription via whisper-rs (whisper.cpp).
//!
//! Feeds 16 kHz mono f32 PCM to whisper and produces word-level `Word`s by
//! enabling per-token timestamps and splitting segments on word boundaries.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use eframe::egui;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::model::{Word, WorkerMsg};

/// Transcribe `pcm16k` with the model at `model_path` on a worker thread.
/// Reports `TranscribeProgress`, then `TranscribeDone`/`TranscribeFailed`.
pub fn spawn_transcribe(
    model_path: PathBuf,
    pcm16k: Arc<Vec<f32>>,
    tx: Sender<WorkerMsg>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress_ctx = ctx.clone();
        let result = transcribe(&model_path, &pcm16k, &cancel, move |pct| {
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
    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())?;
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

    let mut words = Vec::new();
    for segment in state.as_iter() {
        let text = segment.to_str()?.trim().to_owned();
        // Skip whisper's non-speech annotations, e.g. "*Music*", "[Music]", "(applause)".
        if text.is_empty() || is_non_speech(&text) {
            continue;
        }
        // whisper timestamps are centiseconds (10 ms units).
        let start = segment.start_timestamp() as f32 / 100.0;
        let end = segment.end_timestamp() as f32 / 100.0;
        words.push(Word { start, end, text });
    }
    Ok(words)
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
        let words = transcribe(&model, &decoded.pcm16k, &cancel, |_p| {}).expect("transcribe");

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

//! Shared data types used across the app.

use std::path::PathBuf;
use std::sync::Arc;

/// One transcript word with its time span (seconds).
#[derive(Clone)]
pub struct Word {
    pub start: f32,
    pub end: f32,
    pub text: String,
}

/// Which selection handle is being dragged on the waveform.
#[derive(PartialEq)]
pub enum Handle {
    In,
    Out,
}

/// What an exported clip should be.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Mp3,
    Mp4,
}

impl OutputFormat {
    pub fn ext(self) -> &'static str {
        match self {
            OutputFormat::Mp3 => "mp3",
            OutputFormat::Mp4 => "mp4",
        }
    }
}

/// Whisper model the user can choose. `Base` is embedded in the binary; the
/// larger ones are downloaded on demand (M8).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WhisperModelKind {
    Base,
    Small,
    Medium,
}

impl WhisperModelKind {
    pub const ALL: [WhisperModelKind; 3] =
        [WhisperModelKind::Base, WhisperModelKind::Small, WhisperModelKind::Medium];

    /// Human-readable label for the picker.
    pub fn label(self) -> &'static str {
        match self {
            WhisperModelKind::Base => "base — fast (~150 MB)",
            WhisperModelKind::Small => "small — accurate (~500 MB)",
            WhisperModelKind::Medium => "medium — most accurate (~1.5 GB)",
        }
    }

    /// ggml model filename (English-only weights, matching the validated fixture).
    pub fn filename(self) -> &'static str {
        match self {
            WhisperModelKind::Base => "ggml-base.en.bin",
            WhisperModelKind::Small => "ggml-small.en.bin",
            WhisperModelKind::Medium => "ggml-medium.en.bin",
        }
    }

    /// Download URL for on-demand fetch (M8).
    pub fn url(self) -> &'static str {
        match self {
            WhisperModelKind::Base => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
            }
            WhisperModelKind::Small => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin"
            }
            WhisperModelKind::Medium => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin"
            }
        }
    }

    /// Whether this model ships embedded in the binary (M9).
    #[cfg_attr(not(feature = "embed-assets"), allow(dead_code))]
    pub fn is_embedded(self) -> bool {
        matches!(self, WhisperModelKind::Base)
    }
}

/// The current long-running job, if any. Drives the progress widget and
/// disables actions while work is in flight.
pub enum Phase {
    Idle,
    Downloading { pct: f32, label: String },
    Decoding,
    Transcribing { progress: f32 },
    Exporting,
}

/// Messages sent from worker threads back to the UI thread. The UI owns the
/// single `Receiver` and drains it once per frame; each worker holds a cloned
/// `Sender` and calls `ctx.request_repaint()` after sending so the UI wakes.
pub enum WorkerMsg {
    // Download (yt-dlp) — M7
    DownloadProgress { pct: f32, eta: String },
    DownloadDone(PathBuf),
    DownloadFailed(String),

    // Decode (symphonia) -> waveform + duration + 16 kHz mono PCM — M4
    DecodeDone {
        samples: Vec<f32>,
        duration: f32,
        pcm16k: Arc<Vec<f32>>,
    },
    DecodeFailed(String),

    // Model fetch (whisper ggml) — M8
    ModelProgress { pct: f32 },
    ModelReady,
    ModelFailed(String),

    // Transcribe (whisper-rs) — M5
    TranscribeProgress(f32),
    TranscribeDone(Vec<Word>),
    TranscribeFailed(String),

    // Export (ffmpeg) — M6
    ExportDone(PathBuf),
    ExportFailed(String),

    /// Generic status-line update.
    Status(String),
}

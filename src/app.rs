//! The Clip Extractor application: UI, state, and the (still placeholder)
//! load/export behaviour.
//!
//! Placeholders to be replaced at later milestones (see ARCHITECTURE.md):
//!   - Waveform samples are synthetic until `symphonia` decoding is wired in.
//!   - Transcript words are demo data until `whisper-rs` is wired in.
//!   - Export uses system `ffmpeg`; will move to `ffmpeg-sidecar`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;

use crate::model::{Handle, OutputFormat, Phase, WhisperModelKind, Word, WorkerMsg};

pub struct ClipApp {
    file: Option<PathBuf>,
    duration: f32,
    /// Synthetic waveform amplitudes in [0,1]; replaced by real decode later.
    samples: Vec<f32>,
    words: Vec<Word>,
    sel_in: f32,
    sel_out: f32,
    dragging: Option<Handle>,
    status: String,

    // --- inputs / config ---
    /// URL (e.g. YouTube) or local file path.
    source_input: String,
    /// Directory exported clips are written to.
    out_dir: String,
    output_format: OutputFormat,
    model_kind: WhisperModelKind,

    // --- async plumbing ---
    /// Cloned into each worker thread.
    tx: Sender<WorkerMsg>,
    /// Drained once per frame in `update`.
    rx: Receiver<WorkerMsg>,
    /// Captured at startup so workers can wake the UI via `request_repaint`.
    ctx_for_workers: egui::Context,
    /// The current long-running job, if any.
    phase: Phase,
    /// Decoded 16 kHz mono PCM, cached so transcription needn't re-decode.
    pcm16k: Option<Arc<Vec<f32>>>,
    /// Abort flag for the in-flight job (e.g. transcription), if cancellable.
    cancel: Option<Arc<AtomicBool>>,
}

impl ClipApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = unbounded();
        let app = Self {
            file: None,
            duration: 8.0,
            samples: synth_waveform(8.0),
            words: demo_words(),
            sel_in: 4.8,
            sel_out: 8.0,
            dragging: None,
            status: "Open a media file to begin.".to_owned(),
            source_input: String::new(),
            out_dir: default_output_dir(),
            output_format: OutputFormat::Mp4,
            model_kind: WhisperModelKind::Base,
            tx,
            rx,
            ctx_for_workers: cc.egui_ctx.clone(),
            phase: Phase::Idle,
            pcm16k: None,
            cancel: None,
        };

        // Prove the worker -> channel -> repaint -> drain path end-to-end: a
        // background thread delivers a status message shortly after startup.
        app.spawn_job(|tx, ctx| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = tx.send(WorkerMsg::Status(
                "Ready — open a media file to begin.".to_owned(),
            ));
            ctx.request_repaint();
        });

        app
    }

    /// Spawn a worker thread, handing it a cloned `Sender` and the UI `Context`.
    /// Workers communicate results back exclusively via `WorkerMsg`.
    fn spawn_job<F>(&self, job: F)
    where
        F: FnOnce(Sender<WorkerMsg>, egui::Context) + Send + 'static,
    {
        let tx = self.tx.clone();
        let ctx = self.ctx_for_workers.clone();
        std::thread::spawn(move || job(tx, ctx));
    }

    /// Drain all pending worker messages and apply them to UI state. Called at
    /// the top of every frame.
    fn drain_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                WorkerMsg::Status(s) => self.status = s,

                WorkerMsg::DownloadProgress { pct, eta } => {
                    let label = if eta.is_empty() {
                        format!("{:.0}%", pct * 100.0)
                    } else {
                        format!("{:.0}% (ETA {eta})", pct * 100.0)
                    };
                    self.phase = Phase::Downloading { pct, label };
                }
                WorkerMsg::DownloadDone(path) => {
                    self.cancel = None;
                    self.status = format!("Downloaded {}", path.display());
                    self.decode_file(path);
                }
                WorkerMsg::DownloadFailed(e) => {
                    self.status = e;
                    self.phase = Phase::Idle;
                    self.cancel = None;
                }

                WorkerMsg::DecodeDone {
                    samples,
                    duration,
                    pcm16k,
                } => {
                    self.samples = samples;
                    self.duration = duration;
                    self.sel_in = 0.0;
                    self.sel_out = duration;
                    self.pcm16k = Some(pcm16k);
                    self.status = format!("Decoded {duration:.1}s.");
                    self.ensure_model_then_transcribe();
                }
                WorkerMsg::DecodeFailed(e) => {
                    self.status = e;
                    self.phase = Phase::Idle;
                }

                WorkerMsg::ModelProgress { pct } => {
                    self.phase = Phase::Downloading {
                        pct,
                        label: "model".to_owned(),
                    };
                    self.status = format!("Downloading model… {:.0}%", pct * 100.0);
                }
                WorkerMsg::ModelReady(_) => {
                    self.cancel = None;
                    self.start_transcription();
                }
                WorkerMsg::ModelFailed(e) => {
                    self.status = e;
                    self.phase = Phase::Idle;
                    self.cancel = None;
                }

                WorkerMsg::TranscribeProgress(progress) => {
                    self.phase = Phase::Transcribing { progress };
                }
                WorkerMsg::TranscribeDone(words) => {
                    self.status = format!("Transcription complete — {} words.", words.len());
                    self.words = words;
                    self.phase = Phase::Idle;
                    self.cancel = None;
                }
                WorkerMsg::TranscribeFailed(e) => {
                    self.status = e;
                    self.phase = Phase::Idle;
                    self.cancel = None;
                }

                WorkerMsg::ExportDone(path) => {
                    self.status = format!("Exported {}", path.display());
                    self.phase = Phase::Idle;
                }
                WorkerMsg::ExportFailed(e) => {
                    self.status = e;
                    self.phase = Phase::Idle;
                }
            }
        }
    }
}

impl eframe::App for ClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();

        let busy = !matches!(self.phase, Phase::Idle);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            egui::Grid::new("inputs")
                .num_columns(3)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Source:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.source_input)
                            .hint_text("YouTube URL or local file path")
                            .desired_width(520.0),
                    );
                    if ui.add_enabled(!busy, egui::Button::new("Browse…")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Media", &["mp4", "mov", "mkv", "mp3", "wav", "m4a"])
                            .pick_file()
                        {
                            self.source_input = path.display().to_string();
                        }
                    }
                    ui.end_row();

                    ui.label("Output dir:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.out_dir)
                            .hint_text("folder where clips are saved")
                            .desired_width(520.0),
                    );
                    if ui.add_enabled(!busy, egui::Button::new("Browse…")).clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.out_dir = dir.display().to_string();
                        }
                    }
                    ui.end_row();
                });

            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label("Model:");
                egui::ComboBox::from_id_source("model_picker")
                    .selected_text(self.model_kind.label())
                    .show_ui(ui, |ui| {
                        for kind in WhisperModelKind::ALL {
                            ui.selectable_value(&mut self.model_kind, kind, kind.label());
                        }
                    });

                ui.separator();
                ui.label("Output:");
                ui.selectable_value(&mut self.output_format, OutputFormat::Mp4, "MP4 (video)");
                ui.selectable_value(&mut self.output_format, OutputFormat::Mp3, "MP3 (audio)");

                ui.separator();
                let can_load = !busy && !self.source_input.trim().is_empty();
                if ui
                    .add_enabled(can_load, egui::Button::new("▶ Load / Transcribe"))
                    .clicked()
                {
                    self.start_load();
                }
                if busy && ui.button("✖ Cancel").clicked() {
                    if let Some(c) = &self.cancel {
                        c.store(true, Ordering::Relaxed);
                    }
                    self.status = "Cancelling…".to_owned();
                }
            });
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| match &self.phase {
                Phase::Downloading { pct, label } => {
                    ui.add(egui::ProgressBar::new(*pct).desired_width(220.0).text(label.clone()));
                    ui.label(&self.status);
                }
                Phase::Transcribing { progress } => {
                    ui.add(
                        egui::ProgressBar::new(*progress)
                            .desired_width(220.0)
                            .text("Transcribing…"),
                    );
                    ui.label(&self.status);
                }
                Phase::Decoding | Phase::Exporting => {
                    ui.spinner();
                    ui.label(&self.status);
                }
                Phase::Idle => {
                    ui.label(&self.status);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Transcript");
            ui.label("Click a word to set the selection to that word.");
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let words = self.words.clone();
                        for w in &words {
                            let selected = w.start >= self.sel_in && w.end <= self.sel_out;
                            if ui
                                .selectable_label(selected, format!("{} ", w.text))
                                .clicked()
                            {
                                self.sel_in = w.start;
                                self.sel_out = w.end;
                            }
                        }
                    });
                });

            ui.separator();
            ui.heading("Waveform");
            self.waveform(ui);

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Selection: {:.2}s → {:.2}s  ({:.2}s)",
                    self.sel_in,
                    self.sel_out,
                    (self.sel_out - self.sel_in).max(0.0)
                ));
                if ui
                    .add_enabled(
                        self.file.is_some() && !busy,
                        egui::Button::new("✂ Export clip"),
                    )
                    .clicked()
                {
                    self.start_export();
                }
            });
        });
    }
}

impl ClipApp {
    /// Resolve the `source_input` (URL or local path) and begin loading.
    ///
    /// For now this only handles local files (enabling export); URL download is
    /// wired in M7, and real decode/transcription in M4/M5.
    fn start_load(&mut self) {
        let src = self.source_input.trim().to_owned();
        if src.starts_with("http://") || src.starts_with("https://") {
            let Some(out_dir) = self.ensure_out_dir() else { return };
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancel = Some(cancel.clone());
            self.phase = Phase::Downloading {
                pct: 0.0,
                label: "starting…".to_owned(),
            };
            self.status = "Downloading…".to_owned();
            crate::download::spawn_download(
                crate::resources::ytdlp_path(),
                src,
                out_dir,
                self.tx.clone(),
                self.ctx_for_workers.clone(),
                cancel,
            );
            return;
        }

        let path = PathBuf::from(&src);
        if !path.is_file() {
            self.status = format!("File not found: {src}");
            return;
        }
        self.decode_file(path);
    }

    /// Begin decoding a local media file (then transcription chains on completion).
    fn decode_file(&mut self, path: PathBuf) {
        self.status = format!("Decoding {}…", path.display());
        self.phase = Phase::Decoding;
        self.file = Some(path.clone());
        crate::audio::spawn_decode(path, self.tx.clone(), self.ctx_for_workers.clone());
    }

    /// Validate and create the output directory; returns it or sets an error status.
    fn ensure_out_dir(&mut self) -> Option<PathBuf> {
        let dir = self.out_dir.trim();
        if dir.is_empty() {
            self.status = "Set an output directory first.".to_owned();
            return None;
        }
        let out_dir = PathBuf::from(dir);
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            self.status = format!("Cannot create output dir: {e}");
            return None;
        }
        Some(out_dir)
    }

    /// Ensure the selected model is present (downloading on demand), then
    /// transcribe the cached PCM.
    fn ensure_model_then_transcribe(&mut self) {
        if self.pcm16k.is_none() {
            self.phase = Phase::Idle;
            return;
        }
        if crate::resources::model_path(self.model_kind).is_file() {
            self.start_transcription();
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.phase = Phase::Downloading {
            pct: 0.0,
            label: "model".to_owned(),
        };
        self.status = format!("Downloading {} model…", self.model_kind.label());
        crate::resources::spawn_ensure_model(
            self.model_kind,
            self.tx.clone(),
            self.ctx_for_workers.clone(),
            cancel,
        );
    }

    /// Begin transcribing the cached PCM with the selected model, which must
    /// already be present on disk (ensured by `ensure_model_then_transcribe`).
    fn start_transcription(&mut self) {
        let Some(pcm) = self.pcm16k.clone() else {
            self.phase = Phase::Idle;
            return;
        };
        let model = crate::resources::model_path(self.model_kind);
        if !model.is_file() {
            self.status = format!("Model not found at {}.", model.display());
            self.phase = Phase::Idle;
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.phase = Phase::Transcribing { progress: 0.0 };
        crate::transcribe::spawn_transcribe(
            model,
            pcm,
            self.tx.clone(),
            self.ctx_for_workers.clone(),
            cancel,
        );
    }

    /// Draw the waveform and the draggable in/out handles.
    fn waveform(&mut self, ui: &mut egui::Ui) {
        let desired = egui::vec2(ui.available_width(), 140.0);
        let (response, painter) = ui.allocate_painter(desired, egui::Sense::click_and_drag());
        let rect = response.rect;
        let visuals = ui.visuals();

        painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);

        // Waveform bars.
        let n = self.samples.len().max(1);
        let mid = rect.center().y;
        for (i, amp) in self.samples.iter().enumerate() {
            let x = rect.left() + (i as f32 / n as f32) * rect.width();
            let h = amp * (rect.height() * 0.45);
            painter.line_segment(
                [egui::pos2(x, mid - h), egui::pos2(x, mid + h)],
                egui::Stroke::new(1.0, visuals.weak_text_color()),
            );
        }

        let time_to_x =
            |t: f32| rect.left() + (t / self.duration).clamp(0.0, 1.0) * rect.width();
        let x_to_time = |x: f32| {
            ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) * self.duration
        };

        // Shade the selected region.
        let sel_rect = egui::Rect::from_x_y_ranges(
            time_to_x(self.sel_in)..=time_to_x(self.sel_out),
            rect.y_range(),
        );
        painter.rect_filled(
            sel_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(100, 150, 255, 40),
        );

        // Drag handling.
        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() {
                let din = (pos.x - time_to_x(self.sel_in)).abs();
                let dout = (pos.x - time_to_x(self.sel_out)).abs();
                self.dragging = Some(if din <= dout { Handle::In } else { Handle::Out });
            }
            if response.dragged() {
                let t = x_to_time(pos.x);
                match self.dragging {
                    Some(Handle::In) => self.sel_in = t.min(self.sel_out - 0.05).max(0.0),
                    Some(Handle::Out) => {
                        self.sel_out = t.max(self.sel_in + 0.05).min(self.duration)
                    }
                    None => {}
                }
            }
        }
        if response.drag_stopped() {
            self.dragging = None;
        }

        // Draw handles.
        for (t, color) in [
            (self.sel_in, egui::Color32::LIGHT_GREEN),
            (self.sel_out, egui::Color32::LIGHT_RED),
        ] {
            let x = time_to_x(t);
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(2.0, color),
            );
        }
    }

    /// Export the selected range to the configured output directory.
    fn start_export(&mut self) {
        let Some(input) = self.file.clone() else { return };
        let Some(out_dir) = self.ensure_out_dir() else { return };

        self.phase = Phase::Exporting;
        self.status = "Exporting…".to_owned();
        crate::export::spawn_export(
            input,
            out_dir,
            self.sel_in,
            self.sel_out,
            self.output_format,
            self.tx.clone(),
            self.ctx_for_workers.clone(),
        );
    }
}

/// A reasonable default output directory (the user's home, else the cwd).
fn default_output_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_owned())
}

/// Demo transcript matching fixtures/sample.mp4 — replaced by whisper-rs output.
fn demo_words() -> Vec<Word> {
    [
        (4.94, 5.54, "What"),
        (5.54, 5.82, "the"),
        (5.82, 6.22, "fuck"),
        (6.22, 6.52, "is"),
        (6.52, 6.80, "this"),
        (6.80, 7.10, "piece"),
        (7.10, 7.36, "of"),
        (7.36, 7.78, "shit?"),
    ]
    .into_iter()
    .map(|(start, end, text)| Word {
        start,
        end,
        text: text.to_owned(),
    })
    .collect()
}

/// Deterministic placeholder waveform so the panel isn't empty before decoding.
fn synth_waveform(duration: f32) -> Vec<f32> {
    let count = (duration * 100.0) as usize;
    (0..count)
        .map(|i| {
            let t = i as f32 / 20.0;
            let env = ((i as f32 / count as f32) * std::f32::consts::PI).sin();
            (0.2 + 0.8 * (t.sin() * 0.5 + 0.5)) * env
        })
        .collect()
}

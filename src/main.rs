//! Clip Extractor — egui app entry point.
//!
//! Transcribe a media file, browse the transcript, select a clip, and export it.
//! See ARCHITECTURE.md for the stack rationale and the milestone roadmap.

mod app;
mod audio;
mod download;
mod export;
mod model;
mod resources;
mod transcribe;

use eframe::egui;

use app::ClipApp;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Clip Extractor",
        native_options,
        Box::new(|cc| Ok(Box::new(ClipApp::new(cc)))),
    )
}

pub mod accept;
pub mod devices;
pub mod dropzone;
pub mod history;
pub mod queue;
pub mod settings;

#[cfg(test)]
mod render_tests;

use droplink_core::protocol::TransportMode;
use eframe::egui::{self, RichText};

use crate::theme;

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KB {
        format!("{bytes:.0} o")
    } else if bytes < KB * KB {
        format!("{:.1} Ko", bytes / KB)
    } else if bytes < KB * KB * KB {
        format!("{:.1} Mo", bytes / (KB * KB))
    } else {
        format!("{:.2} Go", bytes / (KB * KB * KB))
    }
}

pub fn mode_label(mode: TransportMode) -> &'static str {
    match mode {
        TransportMode::Direct => "LAN",
        TransportMode::Relay => "relay",
    }
}

pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.0)
            .color(theme::MUTED)
            .strong(),
    );
}

pub fn sas_spaced(code: &str) -> String {
    if code.len() == 6 {
        format!("{} {}", &code[..3], &code[3..])
    } else {
        code.to_string()
    }
}

use eframe::egui::{self, RichText};

use crate::state::UiState;
use crate::theme;
use crate::ui::{format_bytes, mode_label};

pub fn show(ui: &mut egui::Ui, state: &UiState) {
    if state.history.is_empty() {
        ui.label(RichText::new("Aucun transfert pour l'instant.").color(theme::MUTED));
        return;
    }

    egui::ScrollArea::vertical()
        .max_height(200.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for entry in &state.history {
                ui.horizontal(|ui| {
                    let (mark, color) = if entry.ok {
                        ("✓", theme::OK)
                    } else {
                        ("✗", theme::WARN)
                    };
                    ui.label(RichText::new(mark).color(color));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&entry.file_name).size(12.0));
                        ui.label(
                            RichText::new(format!(
                                "{} · {} · {}",
                                format_bytes(entry.bytes),
                                mode_label(entry.transport),
                                entry.path.as_deref().unwrap_or("—")
                            ))
                            .size(9.0)
                            .color(theme::MUTED),
                        );
                    });
                });
            }
        });
}

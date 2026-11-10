use eframe::egui::{self, RichText};

use crate::state::{UiAction, UiState};
use crate::theme;
use crate::ui::{format_bytes, mode_label};

pub fn show(ui: &mut egui::Ui, state: &UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if state.queue.is_empty() {
        return actions;
    }

    for row in &state.queue {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&row.file_name).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Annuler").clicked() {
                            actions.push(UiAction::Cancel {
                                transfer_id: row.transfer_id.clone(),
                            });
                        }
                    });
                });
                let fraction = if row.total > 0 {
                    (row.bytes as f32 / row.total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                ui.add(egui::ProgressBar::new(fraction).show_percentage());
                let elapsed = row.started.elapsed().as_secs_f32().max(0.001);
                let speed = row.bytes as f32 / elapsed;
                ui.label(
                    RichText::new(format!(
                        "{} / {} · {}/s · {}",
                        format_bytes(row.bytes),
                        format_bytes(row.total),
                        format_bytes(speed as u64),
                        mode_label(row.transport)
                    ))
                    .size(10.0)
                    .color(theme::MUTED),
                );
            });
        });
        ui.separator();
    }
    actions
}

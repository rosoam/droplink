use eframe::egui;

use crate::state::{UiAction, UiState};
use crate::theme;
use crate::ui::{format_bytes, mode_label};

pub fn show(ui: &mut egui::Ui, state: &UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if state.queue.is_empty() {
        return actions;
    }

    for row in &state.queue {
        let fraction = row.displayed.clamp(0.0, 1.0);
        let elapsed = row.started.elapsed().as_secs_f32().max(0.001);
        let speed = row.bytes as f32 / elapsed;

        ui.horizontal(|ui| {
            ui.label(
                theme::mono(
                    if row.direction == droplink_core::protocol::Direction::Send {
                        "^"
                    } else {
                        "v"
                    },
                    12.0,
                )
                .color(theme::ACCENT),
            );
            let name_width = (ui.available_width() - 34.0).max(40.0);
            theme::truncated(ui, &row.file_name, 12.0, theme::TEXT, name_width);
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::Vec2::new(width, 16.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if theme::muted_button(ui, "x")
                        .on_hover_text("annuler")
                        .clicked()
                    {
                        actions.push(UiAction::Cancel {
                            transfer_id: row.transfer_id.clone(),
                        });
                    }
                },
            );
        });
        ui.horizontal(|ui| {
            ui.label(theme::mono(theme::progress_bar(fraction, 14), 11.0).color(theme::ACCENT));
            ui.label(
                theme::mono(format!("{:>3}%", (fraction * 100.0) as u32), 10.0).color(theme::MUTED),
            );
            ui.label(
                theme::mono(
                    format!(
                        "{}/{}  {}/s  {}",
                        format_bytes(row.bytes),
                        format_bytes(row.total),
                        format_bytes(speed as u64),
                        mode_label(row.transport)
                    ),
                    9.5,
                )
                .color(theme::MUTED),
            );
        });
        ui.add_space(4.0);
    }

    actions
}

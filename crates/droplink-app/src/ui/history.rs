use eframe::egui;

use crate::state::{UiAction, UiState};
use crate::theme;
use crate::ui::{format_bytes, mode_label};

pub fn show(ui: &mut egui::Ui, state: &UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if state.history.is_empty() {
        ui.label(theme::mono("aucun transfert", 10.5).color(theme::MUTED));
        return actions;
    }

    for (index, entry) in state.history.iter().enumerate() {
        let path = entry.path.clone();
        let response = theme::row(ui, egui::Id::new(("history", index)), false, |ui| {
            ui.label(theme::mono("+", 12.0).color(theme::ACCENT));
            ui.add_space(4.0);
            let name_width = (ui.available_width() - 104.0).max(40.0);
            theme::truncated(ui, &entry.file_name, 12.0, theme::TEXT, name_width);
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::Vec2::new(width, 16.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if path.is_some() {
                        ui.label(theme::mono("ouvrir", 10.5).color(theme::ACCENT));
                    }
                    ui.label(
                        theme::mono(
                            format!(
                                "{}  {}",
                                format_bytes(entry.bytes),
                                mode_label(entry.transport)
                            ),
                            9.5,
                        )
                        .color(theme::MUTED),
                    );
                },
            );
        });
        if response.clicked() {
            if let Some(path) = path {
                actions.push(UiAction::Open { path: path.into() });
            }
        }
    }

    actions
}

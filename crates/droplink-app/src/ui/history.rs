use eframe::egui;

use crate::state::{UiAction, UiState};
use crate::theme;
use crate::ui::format_bytes;

pub fn show(ui: &mut egui::Ui, state: &UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if state.history.is_empty() {
        ui.label(theme::mono("aucun transfert", 10.5).color(theme::MUTED));
        return actions;
    }

    for (index, entry) in state.history.iter().enumerate() {
        let path = entry.path.clone();
        let clickable = path.is_some();
        let missing = path
            .as_deref()
            .map(|path| !std::path::Path::new(path).exists())
            .unwrap_or(true);
        let (marker, marker_color, name_color) = if missing {
            // Quiet signal: the row is still listed, but the file is gone.
            ("?", theme::AMBER, theme::AMBER)
        } else if entry.ok {
            ("+", theme::ACCENT, theme::TEXT)
        } else {
            ("!", theme::AMBER, theme::AMBER)
        };
        let size = if missing || entry.bytes == 0 {
            "—".to_string()
        } else {
            format_bytes(entry.bytes)
        };
        let response = theme::row(ui, egui::Id::new(("history", index)), false, |ui| {
            ui.label(theme::mono(marker, 12.0).color(marker_color));
            ui.add_space(4.0);
            let name_width = (ui.available_width() - 100.0).max(40.0);
            theme::truncated(ui, &entry.file_name, 12.0, name_color, name_width);
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::Vec2::new(width, 16.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.label(theme::mono(size.clone(), 9.5).color(theme::MUTED));
                },
            );
        })
        .on_hover_cursor(if clickable {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        });
        // The whole row is the affordance: no trailing button.
        if response.clicked() {
            if let Some(path) = path {
                actions.push(UiAction::Open { path: path.into() });
            }
        }
    }

    actions
}

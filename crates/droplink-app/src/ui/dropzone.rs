use eframe::egui::{self, Color32, RichText, Stroke};

use crate::state::UiState;
use crate::theme;

/// Returns `true` when the user asked for the file picker.
pub fn show(ui: &mut egui::Ui, state: &mut UiState, hovering: bool) -> bool {
    let mut picker = false;
    let selected = state.selected_peer().map(|peer| peer.alias.clone());

    let fill = if hovering {
        Color32::from_rgba_unmultiplied(59, 130, 246, 36)
    } else {
        Color32::TRANSPARENT
    };
    let stroke = Stroke::new(
        if hovering { 2.0 } else { 1.0 },
        if hovering {
            theme::ACCENT
        } else {
            Color32::from_rgb(60, 66, 78)
        },
    );

    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(12)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("🗂").size(26.0));
                match &selected {
                    Some(name) => {
                        ui.label(RichText::new("Dépose un fichier sur").color(theme::MUTED));
                        ui.label(RichText::new(name).strong());
                    }
                    None => {
                        ui.label(
                            RichText::new("Sélectionne un appareil ci-dessus").color(theme::MUTED),
                        );
                    }
                }
                ui.add_space(4.0);
                if ui.button("Choisir un fichier…").clicked() {
                    picker = true;
                }
            });
        });

    picker
}

use eframe::egui::{self, Margin, Stroke};

use crate::state::UiState;
use crate::theme;

/// Returns `true` when the user asked for the file picker.
pub fn show(ui: &mut egui::Ui, state: &mut UiState, hovering: bool) -> bool {
    let mut picker = false;
    let selected = state
        .selected_peer()
        .map(|peer| crate::ui::display_name(&peer.alias));

    egui::Frame::new()
        .fill(if hovering {
            theme::ACCENT_DIM
        } else {
            theme::PANEL
        })
        .stroke(Stroke::new(
            1.0,
            if hovering {
                theme::ACCENT
            } else {
                theme::BORDER
            },
        ))
        .inner_margin(Margin::symmetric(10, 10))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    theme::mono(
                        if theme::is_mobile() {
                            "choisir un fichier"
                        } else {
                            "déposer un fichier"
                        },
                        12.0,
                    )
                    .color(if selected.is_some() {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    }),
                );
                ui.label(
                    theme::mono(
                        match &selected {
                            Some(name) => format!("-> {name}"),
                            None => "aucun appareil — connecte pour envoyer".to_string(),
                        },
                        10.5,
                    )
                    .color(theme::MUTED),
                );
                ui.add_space(6.0);
                let label = if selected.is_some() {
                    "choisir"
                } else {
                    "connecter"
                };
                if theme::action_button(ui, label).clicked() {
                    picker = true;
                }
            });
        });

    picker
}

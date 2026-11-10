use crate::state::{peer_key, UiState};
use crate::theme;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, state: &mut UiState) -> bool {
    let mut clicked = false;
    if state.devices.is_empty() {
        ui.add_space(2.0);
        if state.discovery_available {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::mono("recherche d'appareils...", 10.5).color(theme::MUTED));
            });
        } else {
            ui.label(theme::mono("! découverte indisponible", 11.0).color(theme::WARN));
        }
        return clicked;
    }

    let devices = state.devices.clone();
    for peer in devices {
        let key = peer_key(&peer);
        let selected = state.selected.as_ref() == Some(&key);
        let name = crate::ui::display_name(&peer.alias);

        let response = theme::row(ui, egui::Id::new(("device", &key)), selected, |ui| {
            ui.label(
                theme::mono(if selected { ">" } else { "*" }, 12.0).color(if selected {
                    theme::ACCENT
                } else {
                    theme::MUTED
                }),
            );
            ui.add_space(4.0);
            let width = ui.available_width();
            theme::truncated(
                ui,
                &name,
                12.5,
                if selected { theme::ACCENT } else { theme::TEXT },
                width,
            );
        });
        if response.clicked() {
            state.toggle_selected(key);
            clicked = true;
        }
    }
    clicked
}

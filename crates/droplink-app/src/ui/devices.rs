use crate::state::{peer_key, UiState};
use crate::theme;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, state: &mut UiState) {
    if state.devices.is_empty() {
        ui.add_space(2.0);
        if state.discovery_available {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::mono("recherche...", 11.0).color(theme::MUTED));
            });
        } else {
            ui.label(theme::mono("! découverte indisponible", 11.0).color(theme::WARN));
        }
        return;
    }

    let devices = state.devices.clone();
    for peer in devices {
        let key = peer_key(&peer);
        let selected = state.selected.as_ref() == Some(&key);
        let name = crate::ui::display_name(&peer.alias);
        let direct = peer.reach.direct_addr().is_some();

        let response = theme::row(ui, egui::Id::new(("device", &key)), selected, |ui| {
            ui.label(
                theme::mono(if selected { ">" } else { "*" }, 12.0).color(if selected {
                    theme::ACCENT
                } else {
                    theme::MUTED
                }),
            );
            ui.label(theme::mono(&name, 12.5).color(if selected {
                theme::ACCENT
            } else {
                theme::TEXT
            }));
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::Vec2::new(width, 16.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.label(
                        theme::mono(if direct { "lan" } else { "relay" }, 10.0).color(theme::MUTED),
                    );
                },
            );
        });
        if response.clicked() {
            state.toggle_selected(key);
        }
    }
}

use eframe::egui::{self, RichText};

use crate::state::{peer_key, UiState};
use crate::theme;

pub fn show(ui: &mut egui::Ui, state: &mut UiState) {
    if state.devices.is_empty() {
        ui.add_space(6.0);
        if state.discovery_available {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Recherche d'appareils…").color(theme::MUTED));
            });
        } else {
            ui.colored_label(
                theme::WARN,
                "Découverte LAN indisponible. Active le relay dans les réglages.",
            );
        }
        return;
    }

    let devices = state.devices.clone();
    for peer in devices {
        let key = peer_key(&peer);
        let selected = state.selected.as_ref() == Some(&key);
        let badge = if peer.reach.direct_addr().is_some() {
            "LAN"
        } else {
            "relay"
        };
        let label = format!("{}  ·  {badge}", peer.alias);
        if ui
            .selectable_label(selected, RichText::new(label).size(14.0))
            .clicked()
        {
            state.selected = Some(key);
        }
    }
}

use eframe::egui::{self, RichText};

use crate::state::{UiAction, UiState};
use crate::theme;
use crate::ui::{format_bytes, mode_label, sas_spaced};

/// Draw one blocking modal per pending incoming transfer.
pub fn show(ctx: &egui::Context, state: &UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let ids: Vec<String> = state.incoming.keys().cloned().collect();

    for id in ids {
        let Some(incoming) = state.incoming.get(&id).cloned() else {
            continue;
        };
        let response = egui::Modal::new(egui::Id::new(("accept", id.clone()))).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.label(
                RichText::new("TRANSFERT ENTRANT")
                    .size(10.0)
                    .color(theme::MUTED),
            );
            ui.heading(&incoming.sender_alias);
            ui.add_space(4.0);
            ui.label(RichText::new(&incoming.file_name).strong().size(14.0));
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    format_bytes(incoming.file_size),
                    mode_label(incoming.transport)
                ))
                .size(11.0)
                .color(theme::MUTED),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("Code de sécurité  {}", sas_spaced(&incoming.sas)))
                    .monospace()
                    .color(theme::ACCENT),
            );
            ui.label(
                RichText::new("Compare ce code avec l'expéditeur avant d'accepter.")
                    .size(10.0)
                    .color(theme::MUTED),
            );
            ui.add_space(12.0);
            let mut decision = None;
            ui.horizontal(|ui| {
                if ui.button("Refuser").clicked() {
                    decision = Some(false);
                }
                if ui.button("Accepter").clicked() {
                    decision = Some(true);
                }
            });
            decision
        });

        if let Some(accept) = response.inner {
            actions.push(UiAction::Respond {
                transfer_id: id,
                accept,
            });
        } else if response.should_close() {
            actions.push(UiAction::Respond {
                transfer_id: id,
                accept: false,
            });
        }
    }

    actions
}

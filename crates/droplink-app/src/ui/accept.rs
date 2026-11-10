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
        let sender = crate::ui::display_name(&incoming.sender_alias);
        let response = egui::Modal::new(egui::Id::new(("accept", id.clone()))).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.label(theme::mono("-- entrant", 10.5).color(theme::MUTED));
            ui.add_space(2.0);
            theme::key_value(ui, "de", &sender);
            theme::key_value(
                ui,
                "file",
                &format!(
                    "{} ({})",
                    incoming.file_name,
                    format_bytes(incoming.file_size)
                ),
            );
            theme::key_value(ui, "via", mode_label(incoming.transport));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(theme::mono("sas   ", 11.0).color(theme::MUTED));
                ui.label(
                    RichText::new(sas_spaced(&incoming.sas))
                        .monospace()
                        .size(20.0)
                        .color(theme::ACCENT),
                );
            });
            ui.label(theme::mono("compare ce code avec l'expéditeur", 9.5).color(theme::MUTED));
            ui.add_space(10.0);
            let mut decision = None;
            ui.horizontal(|ui| {
                if theme::muted_button(ui, "refuser").clicked() {
                    decision = Some(false);
                }
                if theme::action_button(ui, "accepter").clicked() {
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

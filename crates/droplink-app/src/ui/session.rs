use crate::state::{SessionView, Toast, UiAction, UiState};
use crate::theme;
use droplink_core::relay::{display_code, normalize_code, now_secs};
use eframe::egui;

fn countdown(session: &SessionView) -> (String, u64) {
    let now = now_secs();
    let left = session.expires_at.saturating_sub(now);
    let minutes = left / 60;
    let seconds = left % 60;
    (format!("{minutes:02}:{seconds:02}"), left)
}

pub fn show(ui: &mut egui::Ui, state: &mut UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();

    match state.session.clone() {
        None => {
            ui.horizontal(|ui| {
                if theme::action_button(ui, "créer un code").clicked() {
                    actions.push(UiAction::SessionCreate);
                }
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut state.join_code)
                        .hint_text("code")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(if theme::is_mobile() { 96.0 } else { 110.0 })
                        .margin(egui::Margin::symmetric(5, 3)),
                );
                if theme::muted_button(ui, "rejoindre").clicked() {
                    match normalize_code(&state.join_code) {
                        Some(code) => {
                            state.join_code.clear();
                            actions.push(UiAction::SessionJoin { code });
                        }
                        None => {
                            state.toast = Some(Toast::error(
                                "code invalide : 8 caractères, sans 0/1/I/L/O/U",
                            ));
                        }
                    }
                }
            });
            ui.label(
                theme::mono(
                    "à distance : l'hôte partage un code valable 5 minutes, l'autre le saisit",
                    9.0,
                )
                .color(theme::MUTED),
            );
        }
        Some(session) => {
            let (left_text, left_secs) = countdown(&session);
            let color = if left_secs == 0 {
                theme::WARN
            } else if left_secs <= 30 {
                theme::AMBER
            } else {
                theme::ACCENT
            };
            ui.horizontal(|ui| {
                ui.label(theme::mono("code", 9.5).color(theme::MUTED));
                ui.label(
                    egui::RichText::new(display_code(&session.code))
                        .monospace()
                        .size(20.0)
                        .strong()
                        .color(theme::ACCENT),
                );
                ui.label(theme::mono(&left_text, 11.0).color(color));
                if theme::muted_button(ui, "copier").clicked() {
                    ui.ctx().copy_text(display_code(&session.code));
                    state.toast = Some(Toast::success("code copié"));
                }
                #[cfg(target_os = "ios")]
                if theme::muted_button(ui, "partager").clicked() {
                    droplink_ios_bridge::share_text(&format!(
                        "DropLink : rejoins ma session avec le code {}",
                        display_code(&session.code)
                    ));
                }
            });
            ui.horizontal(|ui| {
                if session.owner {
                    if theme::muted_button(ui, "nouveau code").clicked() {
                        actions.push(UiAction::SessionRefresh);
                    }
                    if theme::muted_button(ui, "terminer").clicked() {
                        actions.push(UiAction::SessionEnd);
                    }
                    ui.label(
                        theme::mono("les appareils connectés restent connectés", 9.0)
                            .color(theme::MUTED),
                    );
                } else {
                    if theme::muted_button(ui, "quitter").clicked() {
                        actions.push(UiAction::SessionEnd);
                    }
                    ui.label(theme::mono("session invitée", 9.0).color(theme::MUTED));
                }
            });
            if left_secs == 0 && session.owner {
                ui.label(
                    theme::mono("code expiré — génère un nouveau code", 9.5).color(theme::WARN),
                );
            } else if left_secs == 0 {
                ui.label(
                    theme::mono(
                        "code expiré — tu restes connecté, demande un nouveau code à l'hôte",
                        9.5,
                    )
                    .color(theme::AMBER),
                );
            }
            if let Some(error) = session.error.as_deref() {
                ui.label(theme::mono(error, 9.5).color(theme::WARN));
            }
        }
    }

    actions
}

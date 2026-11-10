use droplink_core::config::Config;
use droplink_core::Error;
use eframe::egui::{self, RichText};

use crate::theme;

#[derive(Clone, Debug)]
pub struct SettingsDraft {
    pub device_name: String,
    pub receive_dir: String,
    pub port_pref: String,
    pub relay_enabled: bool,
    pub relay_url: String,
    pub room_code: String,
}

impl SettingsDraft {
    pub fn from_config(config: &Config) -> Self {
        Self {
            device_name: config.device_name.clone(),
            receive_dir: config.receive_dir.to_string_lossy().to_string(),
            port_pref: config
                .port_pref
                .map(|port| port.to_string())
                .unwrap_or_default(),
            relay_enabled: config.relay_enabled,
            relay_url: config.relay_url.clone().unwrap_or_default(),
            room_code: config.room_code.clone().unwrap_or_default(),
        }
    }

    pub fn to_config(&self, base: &Config) -> droplink_core::Result<Config> {
        let mut config = base.clone();
        config.device_name = self.device_name.trim().to_string();
        if config.device_name.is_empty() {
            return Err(Error::Protocol("le nom ne peut pas être vide".into()));
        }
        config.receive_dir = self.receive_dir.trim().into();
        config.port_pref = if self.port_pref.trim().is_empty() {
            None
        } else {
            Some(
                self.port_pref
                    .trim()
                    .parse()
                    .map_err(|_| Error::Protocol("port invalide".into()))?,
            )
        };
        config.relay_enabled = self.relay_enabled;
        config.relay_url = if self.relay_url.trim().is_empty() {
            None
        } else {
            Some(self.relay_url.trim().to_string())
        };
        config.room_code = if self.room_code.trim().is_empty() {
            None
        } else {
            Some(self.room_code.trim().to_string())
        };
        config.validate()?;
        Ok(config)
    }
}

pub enum Outcome {
    Save(Config),
    Close,
}

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    draft: &mut SettingsDraft,
    base: &Config,
    error: &mut Option<String>,
) -> Option<Outcome> {
    if !*open {
        return None;
    }
    let mut outcome = None;
    let mut window_open = *open;

    egui::Window::new("Réglages")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(
                RichText::new("Nom de l'appareil")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            ui.text_edit_singleline(&mut draft.device_name);
            ui.add_space(6.0);
            ui.label(
                RichText::new("Dossier de réception")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            ui.text_edit_singleline(&mut draft.receive_dir);
            ui.add_space(6.0);
            ui.label(
                RichText::new("Port fixe (vide = auto)")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            ui.text_edit_singleline(&mut draft.port_pref);
            ui.add_space(6.0);
            ui.checkbox(&mut draft.relay_enabled, "Activer le relay");
            ui.label(RichText::new("URL du relay").size(11.0).color(theme::MUTED));
            ui.text_edit_singleline(&mut draft.relay_url);
            ui.label(
                RichText::new("Code de room (optionnel)")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            ui.text_edit_singleline(&mut draft.room_code);
            if let Some(message) = error.as_deref() {
                ui.colored_label(theme::WARN, message);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Enregistrer").clicked() {
                    match draft.to_config(base) {
                        Ok(config) => outcome = Some(Outcome::Save(config)),
                        Err(err) => *error = Some(err.to_string()),
                    }
                }
                if ui.button("Fermer").clicked() {
                    outcome = Some(Outcome::Close);
                }
            });
        });

    *open = window_open;
    if outcome.is_none() && !window_open {
        outcome = Some(Outcome::Close);
    }
    outcome
}

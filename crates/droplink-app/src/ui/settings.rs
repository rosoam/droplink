use droplink_core::config::Config;
use droplink_core::Error;
use eframe::egui::{self, RichText};

use crate::theme;

#[derive(Clone, Debug)]
pub struct SettingsDraft {
    pub device_name: String,
    pub receive_dir: String,
    pub port_pref: String,
    pub relay_url: String,
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
            relay_url: config.relay_url.clone().unwrap_or_default(),
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
        config.relay_url = if self.relay_url.trim().is_empty() {
            None
        } else {
            Some(self.relay_url.trim().to_string())
        };
        // `relay_enabled` and `room_code` are kept in the config for migration only; the UI no
        // longer exposes them. Preserve the stored values instead of silently resetting them.
        config.validate()?;
        Ok(config)
    }
}

pub enum Outcome {
    Save(Config),
    Close,
    PickFolder,
    OpenFiles,
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(theme::mono(label, 9.5).color(theme::MUTED));
    ui.add(
        egui::TextEdit::singleline(value)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(6, 6)),
    );
    ui.add_space(6.0);
}

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    draft: &mut SettingsDraft,
    base: &Config,
    error: &mut Option<String>,
    ios_destination: Option<&str>,
) -> Option<Outcome> {
    let mut outcome = None;
    let _ = theme::dialog(ctx, "réglages", 320.0, open, |ui| {
        ui.label(theme::mono("-- appareil", 10.0).color(theme::MUTED));
        ui.add_space(4.0);
        field(ui, "Nom de l'appareil", &mut draft.device_name);

        if theme::is_mobile() {
            ui.label(theme::mono("Destination", 9.5).color(theme::MUTED));
            ui.label(
                theme::mono(ios_destination.unwrap_or("DropLink (Fichiers)"), 11.5)
                    .color(theme::TEXT),
            );
            ui.horizontal(|ui| {
                if theme::compact_button(ui, "choisir").clicked() {
                    outcome = Some(Outcome::PickFolder);
                }
                if theme::compact_button(ui, "fichiers").clicked() {
                    outcome = Some(Outcome::OpenFiles);
                }
            });
            ui.label(
                theme::mono("images/vidéos -> Photos, le reste -> ce dossier", 9.5)
                    .color(theme::MUTED),
            );
            ui.add_space(6.0);
        } else {
            field(ui, "Dossier de réception", &mut draft.receive_dir);
            field(ui, "Port fixe (vide = auto)", &mut draft.port_pref);
        }

        ui.add_space(4.0);
        ui.label(theme::mono("-- réseau", 10.0).color(theme::MUTED));
        ui.add_space(4.0);
        field(ui, "URL du relay", &mut draft.relay_url);
        ui.label(
            theme::mono(
                "utilisé seulement quand tu crées ou rejoins une session",
                9.0,
            )
            .color(theme::MUTED),
        );

        if let Some(message) = error.as_deref() {
            ui.add_space(2.0);
            ui.colored_label(theme::WARN, RichText::new(message).size(11.0));
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if theme::compact_primary(ui, "enregistrer").clicked() {
                match draft.to_config(base) {
                    Ok(config) => outcome = Some(Outcome::Save(config)),
                    Err(err) => *error = Some(err.to_string()),
                }
            }
        });
    });

    if !*open && outcome.is_none() {
        outcome = Some(Outcome::Close);
    }
    outcome
}

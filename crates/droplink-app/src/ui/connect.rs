use crate::state::{ConnectTab, SessionView, Toast, UiAction, UiState};
use crate::theme;
use droplink_core::peer::{Peer, Reach};
use droplink_core::relay::{display_code, normalize_code, now_secs};
use eframe::egui;
use std::net::{IpAddr, SocketAddr};

fn tab(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    if active {
        theme::compact_primary(ui, label)
    } else {
        theme::compact_button(ui, label)
    }
}

fn countdown(session: &SessionView) -> (String, u64) {
    let left = session.expires_at.saturating_sub(now_secs());
    (format!("{:02}:{:02}", left / 60, left % 60), left)
}

pub fn show(ctx: &egui::Context, state: &mut UiState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let mut open = state.connect_open;
    let width = if theme::is_mobile() { 300.0 } else { 340.0 };
    let _ = theme::dialog(ctx, "connexion", width, &mut open, |ui| {
        ui.horizontal(|ui| {
            if tab(ui, "appareils", state.connect_tab == ConnectTab::Pairs).clicked() {
                state.connect_tab = ConnectTab::Pairs;
            }
            if tab(ui, "code", state.connect_tab == ConnectTab::Code).clicked() {
                state.connect_tab = ConnectTab::Code;
            }
            if tab(ui, "adresse", state.connect_tab == ConnectTab::Direct).clicked() {
                state.connect_tab = ConnectTab::Direct;
            }
        });
        ui.add_space(8.0);
        match state.connect_tab {
            ConnectTab::Pairs => pairs_tab(ui, state),
            ConnectTab::Code => code_tab(ui, state, &mut actions),
            ConnectTab::Direct => direct_tab(ui, state, &mut actions),
        }
    });
    state.connect_open = open;
    actions
}

fn pairs_tab(ui: &mut egui::Ui, state: &mut UiState) {
    if crate::ui::devices::show(ui, state) {
        // Clicking a device selects it and gets out of the way.
        state.connect_open = false;
    }
    ui.add_space(4.0);
    ui.label(
        theme::mono(
            if state.devices.is_empty() {
                "crée un code (onglet code) ou saisis une adresse."
            } else {
                "clique un appareil pour l'utiliser."
            },
            9.0,
        )
        .color(theme::MUTED),
    );
}

fn code_tab(ui: &mut egui::Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    match state.session.clone() {
        None => {
            ui.label(
                theme::mono(
                    "un code valable 5 minutes, à partager par message. l'autre le saisit.",
                    9.0,
                )
                .color(theme::MUTED),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if theme::compact_primary(ui, "créer un code").clicked() {
                    actions.push(UiAction::SessionCreate);
                }
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::singleline(&mut state.join_code)
                        .hint_text("code")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(if theme::is_mobile() { 96.0 } else { 110.0 })
                        .margin(egui::Margin::symmetric(6, 6)),
                );
                if theme::compact_button(ui, "rejoindre").clicked() {
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
                        .size(18.0)
                        .strong()
                        .color(theme::ACCENT),
                );
                ui.label(theme::mono(left_text, 10.5).color(color));
                if theme::compact_button(ui, "copier").clicked() {
                    ui.ctx().copy_text(display_code(&session.code));
                    state.toast = Some(Toast::success("code copié"));
                }
                #[cfg(target_os = "ios")]
                if theme::compact_button(ui, "partager").clicked() {
                    droplink_ios_bridge::share_text(&format!(
                        "DropLink : rejoins ma session avec le code {}",
                        display_code(&session.code)
                    ));
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if session.owner {
                    if theme::icon_button(ui, "↻", "nouveau code").clicked() {
                        actions.push(UiAction::SessionRefresh);
                    }
                    if theme::compact_button(ui, "terminer").clicked() {
                        actions.push(UiAction::SessionEnd);
                    }
                } else if theme::compact_button(ui, "quitter").clicked() {
                    actions.push(UiAction::SessionEnd);
                }
                ui.label(
                    theme::mono(
                        if session.owner {
                            "les appareils connectés restent connectés"
                        } else {
                            "session invitée"
                        },
                        9.0,
                    )
                    .color(theme::MUTED),
                );
            });
            if left_secs == 0 {
                ui.label(
                    theme::mono(
                        if session.owner {
                            "code expiré — génère un nouveau code"
                        } else {
                            "code expiré — tu restes connecté, demande un nouveau code"
                        },
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
}

fn direct_tab(ui: &mut egui::Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    if let Some(addr) = state.local_addr.clone() {
        ui.horizontal(|ui| {
            ui.label(theme::mono("cette machine", 9.5).color(theme::MUTED));
            ui.label(theme::mono(addr.clone(), 11.0).color(theme::TEXT));
            if theme::compact_button(ui, "copier").clicked() {
                ui.ctx().copy_text(addr.clone());
                state.toast = Some(Toast::success("adresse copiée"));
            }
        });
        if state.local_hotspot {
            ui.label(
                theme::mono(
                    "partage de connexion : la découverte auto peut être filtrée",
                    8.5,
                )
                .color(theme::AMBER),
            );
        }
    }
    ui.add_space(6.0);
    ui.label(
        theme::mono(
            "saisis l'adresse affichée sur l'autre appareil, puis connecte.",
            9.0,
        )
        .color(theme::MUTED),
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.direct_host)
                .hint_text("ip")
                .font(egui::TextStyle::Monospace)
                .desired_width(if theme::is_mobile() { 110.0 } else { 140.0 })
                .margin(egui::Margin::symmetric(6, 6)),
        );
        ui.add(
            egui::TextEdit::singleline(&mut state.direct_port)
                .hint_text("port")
                .font(egui::TextStyle::Monospace)
                .desired_width(58.0)
                .margin(egui::Margin::symmetric(6, 6)),
        );
        if theme::compact_primary(ui, "connecter").clicked() {
            match parse_target(&state.direct_host, &state.direct_port) {
                Ok((ip, port)) => {
                    let peer = Peer {
                        device_id: None,
                        alias: format!("{ip}:{port}"),
                        reach: Reach::Direct { ip, port },
                    };
                    state.direct_host.clear();
                    state.direct_port.clear();
                    state.connect_open = false;
                    actions.push(UiAction::ConnectDirect(peer));
                }
                Err(error) => {
                    state.toast = Some(Toast::error(error));
                }
            }
        }
    });
}

/// Accept `ip` + `port`, or `ip:port` in the first field.
pub fn parse_target(host: &str, port: &str) -> Result<(IpAddr, u16), String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("adresse manquante".into());
    }
    let port = port.trim();
    let (ip_txt, port_txt) = if !port.is_empty() {
        (host, port)
    } else if let Ok(addr) = host.parse::<SocketAddr>() {
        return Ok((addr.ip(), addr.port()));
    } else if let Some((ip, port)) = host.rsplit_once(':') {
        (ip, port)
    } else {
        return Err("port manquant".into());
    };
    let ip: IpAddr = ip_txt
        .trim()
        .parse()
        .map_err(|_| format!("adresse invalide : {ip_txt}"))?;
    let port: u16 = port_txt
        .trim()
        .parse()
        .map_err(|_| format!("port invalide : {port_txt}"))?;
    if port == 0 {
        return Err("port invalide : 0".into());
    }
    Ok((ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepte_ip_et_port_separes() {
        assert_eq!(
            parse_target("172.20.10.1", "7777"),
            Ok((IpAddr::from([172, 20, 10, 1]), 7777))
        );
    }

    #[test]
    fn accepte_ip_port_dans_un_seul_champ() {
        assert_eq!(
            parse_target("172.20.10.1:7777", ""),
            Ok((IpAddr::from([172, 20, 10, 1]), 7777))
        );
    }

    #[test]
    fn refuse_les_saisies_incompletes() {
        assert!(parse_target("", "7777").is_err());
        assert!(parse_target("172.20.10.1", "").is_err());
        assert!(parse_target("pas-une-ip", "7777").is_err());
        assert!(parse_target("172.20.10.1", "0").is_err());
        assert!(parse_target("172.20.10.1", "99999").is_err());
    }

    #[test]
    fn accepte_ipv6_entre_crochets() {
        assert_eq!(
            parse_target("[::1]:7777", ""),
            Ok((IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]), 7777))
        );
    }
}

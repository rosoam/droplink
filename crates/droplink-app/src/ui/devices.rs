use crate::state::{peer_key, UiState};
use crate::theme;
use droplink_core::peer::{Peer, Reach};
use eframe::egui;
use std::net::{IpAddr, SocketAddr};

pub fn show(ui: &mut egui::Ui, state: &mut UiState) -> Option<Peer> {
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
    } else {
        let devices = state.devices.clone();
        for peer in devices {
            let key = peer_key(&peer);
            let selected = state.selected.as_ref() == Some(&key);
            let name = crate::ui::display_name(&peer.alias);
            let direct = peer.reach.direct_addr().is_some();

            let response = theme::row(ui, egui::Id::new(("device", &key)), selected, |ui| {
                ui.label(
                    theme::mono(if selected { ">" } else { "*" }, 12.0)
                        .color(if selected {
                            theme::ACCENT
                        } else {
                            theme::MUTED
                        })
                        .color(theme::ACCENT),
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
                            theme::mono(if direct { "lan" } else { "relay" }, 10.0)
                                .color(theme::MUTED),
                        );
                    },
                );
            });
            if response.clicked() {
                state.toggle_selected(key);
            }
        }
    }

    ui.add_space(6.0);
    if let Some(addr) = state.local_addr.clone() {
        ui.horizontal(|ui| {
            ui.label(theme::mono("cette machine", 9.5).color(theme::MUTED));
            ui.label(theme::mono(addr.clone(), 11.5).color(theme::TEXT));
            if theme::muted_button(ui, "copier").clicked() {
                ui.ctx().copy_text(addr.clone());
                state.toast = Some(crate::state::Toast::success("adresse copiée"));
            }
        });
        if state.local_hotspot {
            ui.label(
                theme::mono(
                    "partage de connexion : la découverte auto peut être filtrée — connexion directe disponible",
                    9.0,
                )
                .color(theme::AMBER),
            );
        }
    }

    ui.add_space(2.0);
    let mut connected: Option<Peer> = None;
    ui.horizontal(|ui| {
        ui.label(theme::mono("direct", 9.5).color(theme::MUTED));
        ui.add(
            egui::TextEdit::singleline(&mut state.direct_host)
                .hint_text("ip")
                .font(egui::TextStyle::Monospace)
                .desired_width(if theme::is_mobile() { 110.0 } else { 140.0 })
                .margin(egui::Margin::symmetric(5, 3)),
        );
        ui.add(
            egui::TextEdit::singleline(&mut state.direct_port)
                .hint_text("port")
                .font(egui::TextStyle::Monospace)
                .desired_width(58.0)
                .margin(egui::Margin::symmetric(5, 3)),
        );
        if theme::action_button(ui, "connecter").clicked() {
            match parse_target(&state.direct_host, &state.direct_port) {
                Ok((ip, port)) => {
                    let peer = Peer {
                        device_id: None,
                        alias: format!("{ip}:{port}"),
                        reach: Reach::Direct { ip, port },
                    };
                    state.direct_host.clear();
                    state.direct_port.clear();
                    debug_assert!(peer.validate().is_ok());
                    connected = Some(peer);
                }
                Err(erreur) => {
                    state.toast = Some(crate::state::Toast::error(erreur));
                }
            }
        }
    });
    ui.label(
        theme::mono(
            "l'autre appareil affiche son adresse dans « cette machine »",
            9.0,
        )
        .color(theme::MUTED),
    );
    connected
}

/// Accepte `ip` + `port`, ou `ip:port` dans le premier champ (le second devient optionnel).
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

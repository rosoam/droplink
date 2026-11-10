//! Headless egui rendering tests: they drive every screen with a populated state. No window is
//! opened: `egui::Context::run` executes the layout and widget code for real.

use crate::state::{HistoryEntry, TransferRow, UiState};
use crate::ui;
use droplink_core::config::{generate_device_id, Config};
use droplink_core::peer::{Peer, PeerKey, Reach};
use droplink_core::protocol::{Direction, TransportMode};
use droplink_net::engine::IncomingTransfer;
use eframe::egui::{self, RawInput};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Instant;

fn config() -> Config {
    Config::new(
        generate_device_id(),
        "Test".into(),
        PathBuf::from("/tmp/droplink-test"),
    )
}

fn populated_state() -> UiState {
    let mut state = UiState::new(50);
    let local = Peer {
        device_id: Some(generate_device_id()),
        alias: "MacBook".into(),
        reach: Reach::Direct {
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            port: 7777,
        },
    };
    let remote = Peer {
        device_id: None,
        alias: "Téléphone".into(),
        reach: Reach::Relay {
            peer_id: "peer-1".into(),
        },
    };
    state.devices = vec![local.clone(), remote.clone()];
    state.selected = Some(PeerKey::Direct(local.device_id.clone().unwrap()));
    state.queue = vec![
        TransferRow {
            transfer_id: "t1".into(),
            file_name: "photo.jpg".into(),
            direction: Direction::Send,
            bytes: 512,
            total: 1024,
            transport: TransportMode::Direct,
            started: Instant::now(),
            displayed: 0.5,
        },
        TransferRow {
            transfer_id: "t2".into(),
            file_name: "video.mp4".into(),
            direction: Direction::Receive,
            bytes: 0,
            total: 0,
            transport: TransportMode::Relay,
            started: Instant::now(),
            displayed: 0.0,
        },
    ];
    state.incoming.insert(
        "t2".into(),
        IncomingTransfer {
            transfer_id: "t2".into(),
            sender_alias: "Téléphone".into(),
            file_name: "video.mp4".into(),
            file_size: 2048,
            transport: TransportMode::Relay,
            sas: "123456".into(),
        },
    );
    state.history = vec![
        HistoryEntry {
            file_name: "ok.bin".into(),
            direction: Direction::Send,
            ok: true,
            path: Some("/tmp/ok.bin".into()),
            transport: TransportMode::Direct,
            bytes: 1024,
        },
        HistoryEntry {
            file_name: "relay.bin".into(),
            direction: Direction::Receive,
            ok: true,
            path: Some("/tmp/relay.bin".into()),
            transport: TransportMode::Relay,
            bytes: 2048,
        },
    ];
    state.toast = Some(crate::state::Toast::error("exemple d'erreur"));
    state
}

#[test]
fn renders_main_screen() {
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let mut state = populated_state();
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            crate::theme::section_header(ui, "Appareils", None);
            ui::devices::show(ui, &mut state);
            crate::theme::section_header(ui, "Envoyer", None);
            let picker = ui::dropzone::show(ui, &mut state, true);
            assert!(!picker);
            crate::theme::section_header(ui, "Transferts", None);
            let _ = ui::queue::show(ui, &state);
            crate::theme::section_header(ui, "Historique", None);
            let _ = ui::history::show(ui, &state);
        });
    });
    assert!(!output.shapes.is_empty());
}

#[test]
fn renders_empty_state() {
    let ctx = egui::Context::default();
    let mut state = UiState::new(10);
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui::devices::show(ui, &mut state);
            ui::dropzone::show(ui, &mut state, false);
            let _ = ui::history::show(ui, &state);
        });
    });
    assert!(!output.shapes.is_empty());
}

#[test]
fn renders_unavailable_discovery() {
    let ctx = egui::Context::default();
    let mut state = UiState::new(10);
    state.discovery_available = false;
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui::devices::show(ui, &mut state);
        });
    });
    assert!(!output.shapes.is_empty());
}

#[test]
fn renders_accept_modal_and_refuses_on_backdrop() {
    let ctx = egui::Context::default();
    let state = populated_state();
    let actions = {
        let mut actions = Vec::new();
        let output = ctx.run(RawInput::default(), |ctx| {
            actions = ui::accept::show(ctx, &state);
        });
        assert!(!output.shapes.is_empty());
        actions
    };
    // Without any click the modal stays open; no action is emitted.
    assert!(actions.is_empty());
}

#[test]
fn renders_settings_and_validates() {
    let ctx = egui::Context::default();
    let base = config();
    let mut draft = ui::settings::SettingsDraft::from_config(&base);
    let mut open = true;
    let mut error = None;
    let output = ctx.run(RawInput::default(), |ctx| {
        let _ = ui::settings::show(ctx, &mut open, &mut draft, &base, &mut error, None);
    });
    assert!(!output.shapes.is_empty());

    // Round-trip through the typed config.
    let config = draft.to_config(&base).unwrap();
    assert_eq!(config.device_name, "Test");

    draft.device_name = "  ".into();
    assert!(draft.to_config(&base).is_err());
    draft.device_name = "Nouveau".into();
    draft.port_pref = "abc".into();
    assert!(draft.to_config(&base).is_err());
    draft.port_pref = "70000".into();
    assert!(draft.to_config(&base).is_err());
    draft.port_pref = "7788".into();
    let updated = draft.to_config(&base).unwrap();
    assert_eq!(updated.port_pref, Some(7788));
}

#[test]
fn renders_toast_and_helpers() {
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let output = ctx.run(RawInput::default(), |ctx| {
        crate::theme::toast(ctx, "erreur", true);
        crate::theme::toast(ctx, "ok", false);
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = crate::theme::progress_bar(0.5, 10);
            crate::theme::truncated(ui, "nom très long", 12.0, crate::theme::TEXT, 40.0);
            crate::theme::key_value(ui, "fichier", "x.bin");
        });
    });
    assert!(!output.shapes.is_empty());
    assert_eq!(crate::theme::progress_bar(0.0, 4), "[····]");
    assert_eq!(crate::theme::progress_bar(1.0, 4), "[████]");
}

#[test]
fn formats_bytes_across_units() {
    assert_eq!(ui::format_bytes(10), "10 o");
    assert_eq!(ui::format_bytes(2048), "2.0 Ko");
    assert_eq!(ui::format_bytes(3 * 1024 * 1024), "3.0 Mo");
    assert!(ui::format_bytes(5 * 1024 * 1024 * 1024).ends_with("Go"));
    assert_eq!(ui::sas_spaced("123456"), "123 456");
    assert_eq!(ui::sas_spaced("12"), "12");
    assert_eq!(ui::mode_label(TransportMode::Direct), "LAN");
    assert_eq!(ui::mode_label(TransportMode::Relay), "relay");
}

use crate::state::{Toast, ToastKind, UiAction, UiState};
use crate::store::{self, Paths};
use crate::theme;
use crate::ui;
use droplink_core::config::Config;
use droplink_core::peer::Source;
use droplink_net::discovery::{advert_for, DiscoveryEvent, DiscoveryHandle};
use droplink_net::engine::{spawn, Command, EngineConfig, EngineHandle, Event};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct App {
    _rt: tokio::runtime::Runtime,
    handle: EngineHandle,
    events: mpsc::Receiver<Event>,
    pub state: UiState,
    config: Config,
    paths: Paths,
    settings: ui::settings::SettingsDraft,
    settings_error: Option<String>,
    picker: Option<std_mpsc::Receiver<Option<PathBuf>>>,
    _discovery: Option<DiscoveryHandle>,
    #[cfg(target_os = "ios")]
    ios_port: u16,
    #[cfg(target_os = "ios")]
    ios_started: bool,
    theme_applied: bool,
}

impl App {
    pub fn new() -> anyhow::Result<Self> {
        let (config, paths) = store::load_or_create();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;

        let mut engine_cfg = EngineConfig::new(
            config.device_id.clone(),
            config.device_name.clone(),
            config.receive_dir.clone(),
        );
        engine_cfg.port_pref = config.port_pref;
        engine_cfg.max_parallel = config.max_parallel;
        engine_cfg.accept_timeout = Duration::from_secs(config.accept_timeout_secs);
        engine_cfg.relay_url = config.relay_url.clone();

        let (handle, bound, events) = rt.block_on(async { spawn(engine_cfg) })?;

        let mut state = UiState::new(config.history_len);
        state.history = store::load_history(&paths);

        let discovery = {
            let (dtx, mut drx) = mpsc::channel(64);
            let spec = advert_for(&config.device_id, &config.device_name, bound.port);
            if let Some(spec) = &spec {
                state.local_addr = Some(format!("{}:{}", spec.ip, bound.port));
                state.local_hotspot = droplink_net::discovery::is_personal_hotspot(&spec.ip);
                if state.local_hotspot {
                    tracing::info!(
                        addr = %state.local_addr.clone().unwrap_or_default(),
                        "personal hotspot: mDNS may be filtered, direct address available"
                    );
                }
            } else {
                state.discovery_available = false;
                tracing::warn!("no advertisable IPv4, mDNS disabled");
            }
            let discovery = rt
                .block_on(async { DiscoveryHandle::start(spec, dtx) })
                .ok();
            if discovery.is_none() {
                state.discovery_available = false;
                tracing::warn!("mdns unavailable, discovery disabled");
            }
            if discovery.is_some() {
                let handle = handle.clone();
                rt.spawn(async move {
                    while let Some(event) = drx.recv().await {
                        match event {
                            DiscoveryEvent::Found(peer) => {
                                tracing::info!(alias = %peer.alias, "peer discovered");
                                handle.command(Command::PeerSeen {
                                    peer,
                                    source: Source::Mdns,
                                })
                            }
                            DiscoveryEvent::Lost(key) => {
                                tracing::info!(?key, "peer lost");
                                handle.command(Command::PeerGone {
                                    key,
                                    source: Source::Mdns,
                                })
                            }
                            DiscoveryEvent::Unavailable => {
                                tracing::warn!("discovery unavailable");
                            }
                        }
                    }
                });
            }
            discovery
        };

        #[cfg(target_os = "ios")]
        {
            use droplink_core::peer::{Peer, PeerKey, Reach};
            let commands = handle.command_sender();
            droplink_ios_bridge::set_peer_handler(move |event| match event {
                droplink_ios_bridge::PeerEvent::Found {
                    device_id,
                    alias,
                    ip,
                    port,
                } => {
                    if let Ok(ip) = ip.parse() {
                        tracing::info!(%alias, %ip, port, "ios peer discovered");
                        let peer = Peer {
                            device_id: Some(device_id),
                            alias,
                            reach: Reach::Direct { ip, port },
                        };
                        let _ = commands.try_send(Command::PeerSeen {
                            peer,
                            source: Source::Mdns,
                        });
                    }
                }
                droplink_ios_bridge::PeerEvent::Lost { device_id } => {
                    let _ = commands.try_send(Command::PeerGone {
                        key: PeerKey::Direct(device_id),
                        source: Source::Mdns,
                    });
                }
            });
            tracing::info!(port = bound.port, "ios bonjour configured");
        }

        tracing::info!(
            port = bound.port,
            receive_dir = %config.receive_dir.display(),
            discovery = discovery.is_some(),
            relay_on_demand = config.relay_url.is_some(),
            "droplink started"
        );

        let settings = ui::settings::SettingsDraft::from_config(&config);

        Ok(Self {
            _rt: rt,
            handle,
            events,
            state,
            config,
            paths,
            settings,
            settings_error: None,
            picker: None,
            _discovery: discovery,
            #[cfg(target_os = "ios")]
            ios_port: bound.port,
            #[cfg(target_os = "ios")]
            ios_started: false,
            theme_applied: false,
        })
    }

    fn dispatch(&mut self, action: UiAction) {
        match action {
            UiAction::Send { peer, path } => {
                tracing::info!(
                    alias = %peer.alias,
                    path = %path.display(),
                    "send requested"
                );
                self.handle.command(Command::Send { peer, path })
            }
            UiAction::Respond {
                transfer_id,
                accept,
            } => self.handle.command(Command::Respond {
                transfer_id,
                accept,
            }),
            UiAction::Cancel { transfer_id } => {
                self.handle.command(Command::Cancel { transfer_id })
            }
            UiAction::SessionCreate => self.handle.command(Command::SessionCreate),
            UiAction::SessionJoin { code } => self.handle.command(Command::SessionJoin { code }),
            UiAction::SessionRefresh => self.handle.command(Command::SessionRefresh),
            UiAction::SessionEnd => self.handle.command(Command::SessionEnd),
            UiAction::Open { path } => {
                tracing::info!(path = %path.display(), "open requested");
                if path.exists() {
                    open_path(&path);
                } else {
                    self.state.toast = Some(Toast::error("fichier introuvable"));
                }
            }
        }
    }

    #[cfg(not(target_os = "ios"))]
    fn open_picker(&mut self) {
        let (tx, rx) = std_mpsc::channel();
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new().pick_file();
            let _ = tx.send(file);
        });
        self.picker = Some(rx);
    }

    #[cfg(target_os = "ios")]
    fn open_picker(&mut self) {
        tracing::info!("opening document picker");
        droplink_ios_bridge::pick_file();
    }

    fn poll_picker(&mut self) {
        let Some(rx) = self.picker.as_ref() else {
            return;
        };
        let received = rx.try_recv();
        match received {
            Ok(Some(path)) => {
                if let Some(peer) = self.state.selected_peer().cloned() {
                    self.dispatch(UiAction::Send { peer, path });
                } else {
                    self.state.toast = Some(Toast::error("Sélectionne un appareil d'abord"));
                }
                self.picker = None;
            }
            Ok(None) => self.picker = None,
            Err(std_mpsc::TryRecvError::Empty) => {}
            Err(std_mpsc::TryRecvError::Disconnected) => self.picker = None,
        }
    }

    fn drain_events(&mut self) {
        let mut history_dirty = false;
        for _ in 0..2000 {
            match self.events.try_recv() {
                Ok(event) => {
                    #[cfg(target_os = "ios")]
                    if let Event::Completed {
                        direction: droplink_core::protocol::Direction::Receive,
                        path: Some(path),
                        ..
                    } = &event
                    {
                        droplink_ios_bridge::export_received(path);
                    }
                    match &event {
                        Event::Queued { file_name, .. } => {
                            tracing::info!(%file_name, "queued")
                        }
                        Event::Incoming(incoming) => tracing::info!(
                            alias = %incoming.sender_alias,
                            file = %incoming.file_name,
                            "incoming request"
                        ),
                        Event::Completed {
                            file_name,
                            transport,
                            ..
                        } => {
                            tracing::info!(%file_name, ?transport, "transfer completed")
                        }
                        Event::Failed {
                            file_name, reason, ..
                        } => {
                            tracing::warn!(%file_name, %reason, "transfer failed")
                        }
                        Event::Sas { code, .. } => tracing::info!(%code, "sas"),
                        Event::DevicesChanged(peers) => {
                            tracing::info!(count = peers.len(), "devices changed")
                        }
                        _ => {}
                    }
                    let touches_history =
                        matches!(event, Event::Completed { .. } | Event::Failed { .. });
                    self.state.apply(event);
                    history_dirty |= touches_history;
                }
                Err(_) => break,
            }
        }
        if history_dirty {
            store::save_history(&self.paths, &self.state.history);
        }
    }

    fn status(&self) -> (String, egui::Color32) {
        if !self.state.discovery_available {
            ("Découverte indisponible".to_string(), theme::WARN)
        } else if self.state.devices.is_empty() {
            ("Recherche d'appareils…".to_string(), theme::AMBER)
        } else {
            let count = self.state.devices.len();
            (
                format!(
                    "{count} appareil{} · prêt",
                    if count == 1 { "" } else { "s" }
                ),
                theme::ACCENT,
            )
        }
    }
}

#[cfg(not(target_os = "ios"))]
fn open_path(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

#[cfg(target_os = "ios")]
fn open_path(path: &std::path::Path) {
    droplink_ios_bridge::open_file(path);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            theme::apply(ctx);
            self.theme_applied = true;
        }
        self.state.tick();
        let dt = ctx.input(|input| input.stable_dt);
        self.state.animate(dt);
        self.drain_events();
        self.poll_picker();

        #[cfg(target_os = "ios")]
        {
            if !self.ios_started {
                let ip = droplink_net::discovery::select_advertisable_ip(
                    &droplink_net::discovery::system_interfaces(),
                )
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "127.0.0.1".to_string());
                droplink_ios_bridge::start_bonjour(
                    self.ios_port,
                    &self.config.device_id,
                    &self.config.device_name,
                    &ip,
                );
                self.ios_started = true;
            }
            let picked = droplink_ios_bridge::take_picked_files();
            if !picked.is_empty() {
                tracing::info!(count = picked.len(), "ios files picked");
                match self.state.selected_peer().cloned() {
                    Some(peer) => {
                        for path in picked {
                            self.dispatch(UiAction::Send {
                                peer: peer.clone(),
                                path,
                            });
                        }
                    }
                    None => {
                        self.state.toast =
                            Some(Toast::error("Sélectionne un appareil avant d'envoyer"));
                    }
                }
            }
        }

        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            if self.state.selected_peer().is_none() {
                self.state.toast = Some(Toast::error("Sélectionne un appareil avant de déposer"));
            } else {
                for action in self.state.on_files_dropped(dropped) {
                    self.dispatch(action);
                }
            }
        }
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());

        let mut actions: Vec<UiAction> = Vec::new();
        let mut manual_peer: Option<droplink_core::peer::Peer> = None;
        let mut picker_requested = false;
        let (status, status_color) = self.status();

        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(theme::page_pad(), 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("droplink")
                            .monospace()
                            .size(15.0)
                            .strong()
                            .color(theme::ACCENT),
                    );
                    ui.label(theme::mono("v1.0.0", 9.5).color(theme::MUTED));
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        egui::Vec2::new(width, 22.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if theme::action_button(ui, "cfg").clicked() {
                                self.state.settings_open = true;
                            }
                            ui.label(
                                egui::RichText::new(format!("* {status}"))
                                    .monospace()
                                    .size(9.5)
                                    .color(status_color),
                            );
                        },
                    );
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(theme::page_pad(), 0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        theme::section_header(
                            ui,
                            "pairs",
                            Some(&format!("{}", self.state.devices.len())),
                        );
                        ui.add_space(6.0);
                        manual_peer = ui::devices::show(ui, &mut self.state);

                        ui.add_space(10.0);
                        theme::section_header(ui, "envoyer", None);
                        ui.add_space(6.0);
                        if ui::dropzone::show(ui, &mut self.state, hovering) {
                            picker_requested = true;
                        }

                        if !self.state.queue.is_empty() || self.state.outcome.is_some() {
                            ui.add_space(10.0);
                            theme::section_header(ui, "transferts", None);
                            ui.add_space(6.0);
                            if let Some(outcome) = self.state.outcome.clone() {
                                let color = if outcome.ok {
                                    theme::ACCENT
                                } else {
                                    theme::WARN
                                };
                                let blink = (ctx.input(|input| input.time) * 5.0).sin() > -0.5;
                                ui.horizontal(|ui| {
                                    ui.label(
                                        theme::mono(
                                            if blink {
                                                if outcome.ok {
                                                    "+"
                                                } else {
                                                    "!"
                                                }
                                            } else {
                                                " "
                                            },
                                            12.0,
                                        )
                                        .color(color),
                                    );
                                    ui.add_space(4.0);
                                    let width = ui.available_width();
                                    theme::truncated(ui, &outcome.text, 12.0, color, width);
                                });
                            }
                            actions.extend(ui::queue::show(ui, &self.state));
                        }

                        ui.add_space(10.0);
                        theme::section_header(ui, "historique", None);
                        ui.add_space(6.0);
                        actions.extend(ui::history::show(ui, &self.state));
                        ui.add_space(18.0);
                    });
            });

        if let Some(peer) = manual_peer {
            tracing::info!(alias = %peer.alias, "manual peer added");
            self.handle.command(Command::PeerSeen {
                peer,
                source: Source::Manual,
            });
        }

        actions.extend(ui::accept::show(ctx, &self.state));

        #[cfg(target_os = "ios")]
        let ios_destination = droplink_ios_bridge::destination();
        #[cfg(not(target_os = "ios"))]
        let ios_destination: Option<String> = None;

        if let Some(outcome) = ui::settings::show(
            ctx,
            &mut self.state.settings_open,
            &mut self.settings,
            &self.config,
            &mut self.settings_error,
            ios_destination.as_deref(),
        ) {
            match outcome {
                ui::settings::Outcome::Save(config) => {
                    if let Err(err) = store::save_config(&self.paths, &config) {
                        self.settings_error = Some(err.to_string());
                    } else {
                        self.config = config;
                        self.state.settings_open = false;
                        self.state.toast = Some(Toast::error(
                            "Réglages enregistrés — redémarre pour les appliquer",
                        ));
                    }
                }
                ui::settings::Outcome::Close => self.state.settings_open = false,
                ui::settings::Outcome::PickFolder => {
                    #[cfg(target_os = "ios")]
                    droplink_ios_bridge::pick_folder();
                }
                ui::settings::Outcome::OpenFiles => {
                    #[cfg(target_os = "ios")]
                    droplink_ios_bridge::open_files();
                }
            }
        }

        for action in actions {
            self.dispatch(action);
        }
        if picker_requested {
            if self.state.selected_peer().is_none() {
                self.state.toast = Some(Toast::error("Sélectionne d'abord un appareil"));
            } else {
                self.open_picker();
            }
        }

        if let Some(toast) = &self.state.toast {
            theme::toast(ctx, &toast.text, matches!(toast.kind, ToastKind::Error));
        }

        let active = !self.state.queue.is_empty();
        if active || self.state.outcome.is_some() || self.state.toast.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

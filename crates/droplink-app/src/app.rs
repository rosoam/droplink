use crate::state::{UiAction, UiState};
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
        engine_cfg.relay_enabled = config.relay_enabled;
        engine_cfg.relay_url = config.relay_url.clone();
        engine_cfg.room_code = config.room_code.clone();

        let (handle, bound, events) = rt.block_on(async { spawn(engine_cfg) })?;

        let mut state = UiState::new(config.history_len);
        state.history = store::load_history(&paths);

        let (dtx, mut drx) = mpsc::channel(64);
        let spec = advert_for(&config.device_id, &config.device_name, bound.port);
        if spec.is_none() {
            state.discovery_available = false;
            tracing::warn!("no advertisable IPv4, mDNS disabled");
        }
        let discovery = rt
            .block_on(async { DiscoveryHandle::start(spec, dtx) })
            .ok();
        if discovery.is_some() {
            let handle = handle.clone();
            rt.spawn(async move {
                while let Some(event) = drx.recv().await {
                    match event {
                        DiscoveryEvent::Found(peer) => handle.command(Command::PeerSeen {
                            peer,
                            source: Source::Mdns,
                        }),
                        DiscoveryEvent::Lost(key) => handle.command(Command::PeerGone {
                            key,
                            source: Source::Mdns,
                        }),
                        DiscoveryEvent::Unavailable => {}
                    }
                }
            });
        }

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
            theme_applied: false,
        })
    }

    fn dispatch(&self, action: UiAction) {
        match action {
            UiAction::Send { peer, path } => self.handle.command(Command::Send { peer, path }),
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
        }
    }

    fn open_picker(&mut self) {
        let (tx, rx) = std_mpsc::channel();
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new().pick_file();
            let _ = tx.send(file);
        });
        self.picker = Some(rx);
    }

    fn poll_picker(&mut self) {
        let Some(rx) = &self.picker else {
            return;
        };
        match rx.try_recv() {
            Ok(Some(path)) => {
                if let Some(peer) = self.state.selected_peer().cloned() {
                    self.dispatch(UiAction::Send { peer, path });
                } else {
                    self.state.last_error = Some("Sélectionne un appareil d'abord".into());
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
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            theme::apply(ctx);
            self.theme_applied = true;
        }
        self.drain_events();
        self.poll_picker();

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
                self.state.last_error =
                    Some("Sélectionne un appareil avant de déposer un fichier".into());
            } else {
                for action in self.state.on_files_dropped(dropped) {
                    self.dispatch(action);
                }
            }
        }
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());

        let mut actions: Vec<UiAction> = Vec::new();
        let mut picker_requested = false;

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("DropLink").size(18.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⚙").on_hover_text("Réglages").clicked() {
                        self.state.settings_open = true;
                    }
                    let count = self.state.devices.len();
                    ui.label(
                        egui::RichText::new(format!(
                            "{count} appareil{}",
                            if count == 1 { "" } else { "s" }
                        ))
                        .size(11.0)
                        .color(theme::MUTED),
                    );
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui::section_title(ui, "À proximité");
                    ui::devices::show(ui, &mut self.state);
                    ui.add_space(6.0);
                    ui::section_title(ui, "Envoyer");
                    if ui::dropzone::show(ui, &mut self.state, hovering) {
                        picker_requested = true;
                    }
                    if !self.state.queue.is_empty() {
                        ui.add_space(8.0);
                        ui::section_title(ui, "Transferts");
                        actions.extend(ui::queue::show(ui, &self.state));
                    }
                    if let Some(error) = self.state.last_error.clone() {
                        ui.add_space(6.0);
                        ui.colored_label(theme::WARN, format!("✗ {error}"));
                    }
                    ui.add_space(10.0);
                    ui::section_title(ui, "Historique");
                    ui::history::show(ui, &self.state);
                });
        });

        actions.extend(ui::accept::show(ctx, &self.state));

        if let Some(outcome) = ui::settings::show(
            ctx,
            &mut self.state.settings_open,
            &mut self.settings,
            &self.config,
            &mut self.settings_error,
        ) {
            match outcome {
                ui::settings::Outcome::Save(config) => {
                    if let Err(err) = store::save_config(&self.paths, &config) {
                        self.settings_error = Some(err.to_string());
                    } else {
                        self.config = config;
                        self.state.settings_open = false;
                        self.state.last_error =
                            Some("Réglages enregistrés — redémarre pour les appliquer.".into());
                    }
                }
                ui::settings::Outcome::Close => self.state.settings_open = false,
            }
        }

        for action in actions {
            self.dispatch(action);
        }
        if picker_requested {
            self.open_picker();
        }

        let active = !self.state.queue.is_empty();
        ctx.request_repaint_after(if active {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(500)
        });
    }
}

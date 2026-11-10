#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod logging;
mod state;
mod store;
mod theme;
mod ui;

use eframe::egui;

fn main() {
    let started = std::time::Instant::now();
    let paths = store::default_paths();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("droplink=info")),
        )
        .with_writer(logging::LogWriter::open(&paths.log_file()))
        .init();

    let smoke = std::env::args().any(|arg| arg == "--smoke");
    let application = match app::App::new() {
        Ok(application) => application,
        Err(err) => {
            eprintln!("droplink: startup failed: {err:#}");
            std::process::exit(1);
        }
    };

    if smoke {
        println!("startup_ms={}", started.elapsed().as_millis());
        println!("smoke ok");
        return;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([380.0, 560.0])
            .with_min_inner_size([380.0, 560.0])
            .with_resizable(false)
            .with_title("DropLink"),
        ..Default::default()
    };

    if let Err(err) = eframe::run_native(
        "DropLink",
        options,
        Box::new(move |_cc| Ok(Box::new(application))),
    ) {
        eprintln!("droplink: {err}");
        std::process::exit(1);
    }
}

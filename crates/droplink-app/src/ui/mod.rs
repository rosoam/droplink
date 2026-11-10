pub mod accept;
pub mod connect;
pub mod devices;
pub mod dropzone;
pub mod history;
pub mod queue;
pub mod settings;

#[cfg(test)]
mod render_tests;

use droplink_core::protocol::TransportMode;

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KB {
        format!("{bytes:.0} o")
    } else if bytes < KB * KB {
        format!("{:.1} Ko", bytes / KB)
    } else if bytes < KB * KB * KB {
        format!("{:.1} Mo", bytes / (KB * KB))
    } else {
        format!("{:.2} Go", bytes / (KB * KB * KB))
    }
}

pub fn mode_label(mode: TransportMode) -> &'static str {
    match mode {
        TransportMode::Direct => "LAN",
        TransportMode::Relay => "relay",
    }
}

pub fn sas_spaced(code: &str) -> String {
    if code.len() == 6 {
        format!("{} {}", &code[..3], &code[3..])
    } else {
        code.to_string()
    }
}

pub fn display_name(alias: &str) -> String {
    alias
        .strip_suffix(".local")
        .or_else(|| alias.strip_suffix(".local."))
        .unwrap_or(alias)
        .to_string()
}

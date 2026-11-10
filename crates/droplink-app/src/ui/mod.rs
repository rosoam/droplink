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
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} o")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
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

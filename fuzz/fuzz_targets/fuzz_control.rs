#![no_main]

use droplink_core::protocol::Control;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(control) = serde_json::from_slice::<Control>(data) {
        // Validation must never panic, whatever the payload claims.
        let _ = control.validate();
    }
});

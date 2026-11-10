#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Must never panic on arbitrary bytes, and must never consume more than the buffer.
    if let Ok(Some((_frame, consumed))) = droplink_core::frame::decode(data) {
        assert!(consumed <= data.len());
    }
});

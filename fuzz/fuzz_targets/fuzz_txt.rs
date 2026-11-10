#![no_main]

use droplink_net::discovery::mdns::parse_peer_txt;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '|');
    let id = parts.next();
    let alias = parts.next();
    let version = parts.next();
    let _ = parse_peer_txt(id, alias, version, "own-device");
});

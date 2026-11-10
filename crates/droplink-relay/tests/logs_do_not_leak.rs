//! Proves that a full session cycle never writes a secret to the relay logs.
//!
//! The relay handles opaque frames: it must never log the session code, a token, a room id or a
//! file name. This test installs a capturing subscriber, drives create → join → route → refresh
//! → sweep, then scans the captured output. It fails the day someone adds `tracing::info!(%code)`
//! or logs the payload.

use droplink_core::relay::{encode_binary, generate_code, generate_token};
use droplink_relay::room::{Outbound, Rooms};
use rand_core::OsRng;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn a_full_session_cycle_never_logs_code_token_room_or_file_name() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();

    let code = generate_code(&mut OsRng);
    let token = generate_token(&mut OsRng);
    let guest_token = generate_token(&mut OsRng);
    let file_name = "confidentiel-dossier-2026.bin";
    let payload = format!("ciphertext-of-{file_name}").into_bytes();

    tracing::subscriber::with_default(subscriber, || {
        let mut rooms = Rooms::new();
        let (tx_a, _rx_a) = mpsc::channel(64);
        let (peer_a, _, _) = rooms
            .create_session(
                "aaaaaaaa-1111-4111-8111-111111111111".into(),
                "A".into(),
                tx_a,
                1_000,
                code.clone(),
                token.clone(),
            )
            .unwrap();

        let (tx_b, _rx_b) = mpsc::channel(64);
        let (peer_b, _, _) = rooms
            .join_session(
                &code,
                "bbbbbbbb-1111-4111-8111-111111111111".into(),
                "B".into(),
                tx_b,
                1_001,
                guest_token.clone(),
            )
            .unwrap();

        let circuit = rooms.connect(&peer_a, &peer_b).unwrap();
        let frame = encode_binary(&circuit, &payload).unwrap();
        let route = rooms.prepare_route(&peer_a, &frame).unwrap();
        route.tx.try_send(Outbound::Binary(frame)).unwrap();

        rooms
            .refresh_session(&token, 1_002, generate_code(&mut OsRng))
            .unwrap();
        let _ = rooms.sweep(10_000);
    });

    let logs = capture.text();
    assert!(
        logs.contains("session created"),
        "capture vide : le test ne prouverait rien\n{logs}"
    );
    assert!(
        !logs.contains(&code),
        "code de session dans les logs\n{logs}"
    );
    assert!(
        !logs.contains(&token),
        "token propriétaire dans les logs\n{logs}"
    );
    assert!(
        !logs.contains(&guest_token),
        "token invité dans les logs\n{logs}"
    );
    assert!(
        !logs.contains(file_name),
        "nom de fichier dans les logs\n{logs}"
    );
    assert!(
        !logs.contains("ciphertext-of"),
        "payload dans les logs\n{logs}"
    );
    assert!(!logs.contains("session:"), "salle dans les logs\n{logs}");
}

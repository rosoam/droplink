//! Best-effort real mDNS test: two daemons on the same machine must discover each other.
//!
//! Ignored by default because multicast is blocked on some CI networks. Run with:
//! `cargo test -p droplink-net --test mdns_loopback -- --ignored --nocapture`

use droplink_net::discovery::mdns::{advert_for, DiscoveryEvent, DiscoveryHandle};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn two_daemons_discover_each_other() {
    let id_a = Uuid::new_v4().to_string();
    let id_b = Uuid::new_v4().to_string();

    let Some(spec_a) = advert_for(&id_a, "node-a", 17771) else {
        eprintln!("no advertisable interface, skipping");
        return;
    };
    let Some(spec_b) = advert_for(&id_b, "node-b", 17772) else {
        eprintln!("no advertisable interface, skipping");
        return;
    };

    let (tx_a, mut rx_a) = mpsc::channel(32);
    let (tx_b, mut rx_b) = mpsc::channel(32);
    let a = DiscoveryHandle::start(Some(spec_a), tx_a).unwrap();
    let b = DiscoveryHandle::start(Some(spec_b), tx_b).unwrap();

    let found_b = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(event) = rx_a.recv().await {
            if let DiscoveryEvent::Found(peer) = event {
                if peer.device_id.as_deref() == Some(id_b.as_str()) {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);

    let found_a = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(event) = rx_b.recv().await {
            if let DiscoveryEvent::Found(peer) = event {
                if peer.device_id.as_deref() == Some(id_a.as_str()) {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);

    a.shutdown();
    b.shutdown();

    assert!(found_b, "node-a did not discover node-b");
    assert!(found_a, "node-b did not discover node-a");
}

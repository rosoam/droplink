use crate::discovery::interfaces::{select_advertisable_ip, system_interfaces};
use droplink_core::peer::{Peer, PeerKey, Reach};
use droplink_core::protocol::{DeviceId, PROTOCOL_VERSION, SERVICE_TYPE};
use droplink_core::{Error, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap as Map;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Found(Peer),
    Lost(PeerKey),
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct AdvertSpec {
    pub device_id: DeviceId,
    pub alias: String,
    pub ip: Ipv4Addr,
    pub port: u16,
}

pub struct DiscoveryHandle {
    daemon: ServiceDaemon,
    fullname: String,
    current_ip: Arc<Mutex<Ipv4Addr>>,
    publish_spec: AdvertSpec,
}

impl DiscoveryHandle {
    /// Advertise this device and browse for peers.
    ///
    /// If no advertisable IPv4 exists, mDNS is disabled and `DiscoveryEvent::Unavailable` is sent:
    /// we never advertise loopback.
    pub fn start(spec: Option<AdvertSpec>, events: mpsc::Sender<DiscoveryEvent>) -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(|e| Error::Protocol(e.to_string()))?;

        let Some(spec) = spec else {
            let _ = events.try_send(DiscoveryEvent::Unavailable);
            return Ok(Self {
                daemon,
                fullname: String::new(),
                current_ip: Arc::new(Mutex::new(Ipv4Addr::UNSPECIFIED)),
                publish_spec: AdvertSpec {
                    device_id: String::new(),
                    alias: String::new(),
                    ip: Ipv4Addr::UNSPECIFIED,
                    port: 0,
                },
            });
        };

        let (fullname, service) = build_service(&spec)?;
        daemon
            .register(service)
            .map_err(|e| Error::Protocol(e.to_string()))?;
        tracing::info!(port = spec.port, "mdns service registered");

        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| Error::Protocol(e.to_string()))?;
        tracing::info!("mdns browse started");

        let own_id = spec.device_id.clone();
        let event_tx = events.clone();
        let fullname_keys: Arc<Mutex<Map<String, PeerKey>>> = Arc::new(Mutex::new(Map::new()));
        let keys = fullname_keys.clone();
        let _task = tokio::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    ServiceEvent::ServiceResolved(resolved) => {
                        let Some((id, alias)) = parse_peer_txt(
                            resolved.get_property_val_str("id"),
                            resolved.get_property_val_str("alias"),
                            resolved.get_property_val_str("v"),
                            &own_id,
                        ) else {
                            continue;
                        };
                        let Some(ip) = resolved.get_addresses_v4().into_iter().next() else {
                            continue;
                        };
                        let peer = Peer {
                            device_id: Some(id.clone()),
                            alias,
                            reach: Reach::Direct {
                                ip: std::net::IpAddr::V4(ip),
                                port: resolved.get_port(),
                            },
                        };
                        if let Some(device_id) = peer.device_id.clone() {
                            if let Ok(mut map) = keys.lock() {
                                map.insert(
                                    resolved.get_fullname().to_string(),
                                    PeerKey::Direct(device_id),
                                );
                            }
                        }
                        if event_tx.try_send(DiscoveryEvent::Found(peer)).is_err() {
                            tracing::debug!("discovery event channel full");
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        let key = keys.lock().ok().and_then(|mut map| map.remove(&fullname));
                        if let Some(key) = key {
                            let _ = event_tx.try_send(DiscoveryEvent::Lost(key));
                        }
                    }
                    _ => {}
                }
            }
        });

        let current_ip = Arc::new(Mutex::new(spec.ip));
        let republish_ip = current_ip.clone();
        let republish_spec = spec.clone();
        let republish_daemon = daemon.clone();
        let republish_fullname = fullname.clone();
        let _republish = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            loop {
                ticker.tick().await;
                let Some(ip) = select_advertisable_ip(&system_interfaces()) else {
                    continue;
                };
                let changed = {
                    let mut current = match republish_ip.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    if *current == ip {
                        false
                    } else {
                        *current = ip;
                        true
                    }
                };
                if changed {
                    let _ = republish_daemon.unregister(&republish_fullname);
                    let updated = AdvertSpec {
                        ip,
                        ..republish_spec.clone()
                    };
                    if let Ok((_, service)) = build_service(&updated) {
                        let _ = republish_daemon.register(service);
                    }
                }
            }
        });

        Ok(Self {
            daemon,
            fullname,
            current_ip,
            publish_spec: spec,
        })
    }

    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    pub fn advertised_ip(&self) -> Ipv4Addr {
        self.current_ip
            .lock()
            .map(|ip| *ip)
            .unwrap_or(self.publish_spec.ip)
    }

    pub fn shutdown(&self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Validate the identity advertised in a TXT record.
///
/// Returns `None` for our own device, an unknown protocol version, a missing or invalid UUID.
pub fn parse_peer_txt(
    id: Option<&str>,
    alias: Option<&str>,
    version: Option<&str>,
    own_id: &str,
) -> Option<(String, String)> {
    let id = id?;
    if id == own_id {
        return None;
    }
    let version = version.unwrap_or("1");
    if version != PROTOCOL_VERSION.to_string() {
        return None;
    }
    uuid::Uuid::parse_str(id).ok()?;
    let alias: String = alias
        .unwrap_or("Appareil")
        .trim()
        .trim_end_matches(".local")
        .trim_end_matches(".local.")
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    let alias = if alias.is_empty() {
        "Appareil".to_string()
    } else {
        alias
    };
    Some((id.to_string(), alias))
}

fn build_service(spec: &AdvertSpec) -> Result<(String, ServiceInfo)> {
    let short = spec.device_id.chars().take(6).collect::<String>();
    let hostname = format!("droplink-{short}.local.");
    let instance = format!("{}-{short}", spec.alias);

    let mut properties = HashMap::new();
    properties.insert("id".to_string(), spec.device_id.clone());
    properties.insert("alias".to_string(), spec.alias.clone());
    properties.insert("v".to_string(), PROTOCOL_VERSION.to_string());
    properties.insert("ip".to_string(), spec.ip.to_string());
    properties.insert("port".to_string(), spec.port.to_string());

    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance,
        &hostname,
        std::net::IpAddr::V4(spec.ip),
        spec.port,
        properties,
    )
    .map_err(|e| Error::Protocol(e.to_string()))?;

    Ok((service.get_fullname().to_string(), service))
}

/// Resolve the advertisement address, returning `None` when mDNS must stay disabled.
pub fn advert_for(device_id: &str, alias: &str, port: u16) -> Option<AdvertSpec> {
    select_advertisable_ip(&system_interfaces()).map(|ip| AdvertSpec {
        device_id: device_id.to_string(),
        alias: alias.to_string(),
        ip,
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_service_exposes_identity() {
        let spec = AdvertSpec {
            device_id: "11111111-1111-4111-8111-111111111111".into(),
            alias: "Mac".into(),
            ip: Ipv4Addr::new(192, 168, 1, 10),
            port: 7777,
        };
        let (fullname, service) = build_service(&spec).unwrap();
        assert!(fullname.contains("Mac-111111"));
        assert_eq!(
            service.get_property_val_str("id"),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(service.get_property_val_str("alias"), Some("Mac"));
        assert_eq!(service.get_property_val_str("v"), Some("2"));
        assert_eq!(service.get_port(), 7777);
    }

    #[tokio::test]
    async fn start_without_spec_reports_unavailable() {
        let (tx, mut rx) = mpsc::channel(4);
        let handle = DiscoveryHandle::start(None, tx).unwrap();
        assert_eq!(rx.recv().await, Some(DiscoveryEvent::Unavailable));
        handle.shutdown();
    }

    #[test]
    fn advert_for_never_panics() {
        // System-dependent: either an address or None, but never a loopback advertisement.
        if let Some(spec) = advert_for("11111111-1111-4111-8111-111111111111", "Mac", 7777) {
            assert!(!spec.ip.is_loopback());
            assert!(!spec.ip.is_link_local());
        }
    }

    #[test]
    fn txt_parsing_rules() {
        let id = "11111111-1111-4111-8111-111111111111";
        assert!(parse_peer_txt(Some(id), Some("Mac"), Some("2"), "other").is_some());
        assert!(parse_peer_txt(Some(id), Some("Mac"), Some("2"), id).is_none());
        assert!(parse_peer_txt(Some(id), Some("Mac"), Some("1"), "other").is_none());
        assert!(parse_peer_txt(Some("nope"), Some("Mac"), Some("2"), "other").is_none());
        assert!(parse_peer_txt(None, Some("Mac"), Some("2"), "other").is_none());
        let (_, alias) = parse_peer_txt(Some(id), None, Some("2"), "other").unwrap();
        assert_eq!(alias, "Appareil");
        let long = "x".repeat(200);
        let (_, alias) = parse_peer_txt(Some(id), Some(&long), Some("2"), "other").unwrap();
        assert_eq!(alias.chars().count(), 64);
    }

    #[test]
    fn instance_name_is_unique_per_device() {
        let a = build_service(&AdvertSpec {
            device_id: "aaaaaaaa-1111-4111-8111-111111111111".into(),
            alias: "Mac".into(),
            ip: Ipv4Addr::new(192, 168, 1, 10),
            port: 7777,
        })
        .unwrap()
        .0;
        let b = build_service(&AdvertSpec {
            device_id: "bbbbbbbb-1111-4111-8111-111111111111".into(),
            alias: "Mac".into(),
            ip: Ipv4Addr::new(192, 168, 1, 11),
            port: 7777,
        })
        .unwrap()
        .0;
        assert_ne!(a, b);
    }
}

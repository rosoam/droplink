use serde::{Deserialize, Serialize};
use std::net::IpAddr;

use crate::protocol::{DeviceId, PeerId, MAX_ALIAS_LEN};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Mdns,
    Relay,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    Direct {
        ip: IpAddr,
        port: u16,
    },
    Relay {
        peer_id: PeerId,
    },
    Both {
        ip: IpAddr,
        port: u16,
        peer_id: PeerId,
    },
}

impl Reach {
    pub fn direct_addr(&self) -> Option<(IpAddr, u16)> {
        match self {
            Reach::Direct { ip, port } | Reach::Both { ip, port, .. } => Some((*ip, *port)),
            Reach::Relay { .. } => None,
        }
    }

    pub fn relay_id(&self) -> Option<&PeerId> {
        match self {
            Reach::Relay { peer_id } | Reach::Both { peer_id, .. } => Some(peer_id),
            Reach::Direct { .. } => None,
        }
    }
}

/// A peer as known by the registry. `device_id` is only `Some` once proven by a handshake
/// (or advertised by mDNS, which is still unauthenticated until the handshake).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub device_id: Option<DeviceId>,
    pub alias: String,
    pub reach: Reach,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PeerKey {
    Direct(DeviceId),
    Relay(PeerId),
}

impl Peer {
    pub fn validate(&self) -> crate::Result<()> {
        if self.alias.trim().is_empty() || self.alias.chars().count() > MAX_ALIAS_LEN {
            return Err(crate::Error::InvalidPeer("invalid alias".into()));
        }
        if let Some(id) = &self.device_id {
            uuid::Uuid::parse_str(id)
                .map_err(|_| crate::Error::InvalidPeer(format!("invalid device_id: {id}")))?;
        }
        match &self.reach {
            Reach::Direct { ip, port } | Reach::Both { ip, port, .. } => {
                if *port == 0 {
                    return Err(crate::Error::InvalidPeer("port must not be 0".into()));
                }
                if ip.is_unspecified() {
                    return Err(crate::Error::InvalidPeer("unspecified ip".into()));
                }
            }
            Reach::Relay { peer_id } => {
                if peer_id.trim().is_empty() {
                    return Err(crate::Error::InvalidPeer("empty peer_id".into()));
                }
            }
        }
        Ok(())
    }

    pub fn key(&self, source: Source) -> PeerKey {
        match source {
            Source::Mdns => {
                PeerKey::Direct(self.device_id.clone().unwrap_or_else(|| self.alias.clone()))
            }
            Source::Relay => PeerKey::Relay(self.reach.relay_id().cloned().unwrap_or_default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn direct(port: u16) -> Peer {
        Peer {
            device_id: Some("11111111-1111-4111-8111-111111111111".into()),
            alias: "Mac".into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
                port,
            },
        }
    }

    #[test]
    fn direct_peer_valid() {
        assert!(direct(7777).validate().is_ok());
    }

    #[test]
    fn port_zero_rejected() {
        assert!(matches!(
            direct(0).validate(),
            Err(crate::Error::InvalidPeer(_))
        ));
    }

    #[test]
    fn unspecified_ip_rejected() {
        let mut p = direct(7777);
        p.reach = Reach::Direct {
            ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 7777,
        };
        assert!(matches!(p.validate(), Err(crate::Error::InvalidPeer(_))));
    }

    #[test]
    fn relay_peer_without_ip_is_valid() {
        let p = Peer {
            device_id: None,
            alias: "Phone".into(),
            reach: Reach::Relay {
                peer_id: "abc".into(),
            },
        };
        assert!(p.validate().is_ok());
        assert!(p.reach.direct_addr().is_none());
    }

    #[test]
    fn empty_relay_id_rejected() {
        let p = Peer {
            device_id: None,
            alias: "Phone".into(),
            reach: Reach::Relay {
                peer_id: "  ".into(),
            },
        };
        assert!(matches!(p.validate(), Err(crate::Error::InvalidPeer(_))));
    }

    #[test]
    fn invalid_device_id_rejected() {
        let mut p = direct(7777);
        p.device_id = Some("not-a-uuid".into());
        assert!(matches!(p.validate(), Err(crate::Error::InvalidPeer(_))));
    }

    #[test]
    fn long_alias_rejected() {
        let mut p = direct(7777);
        p.alias = "x".repeat(65);
        assert!(matches!(p.validate(), Err(crate::Error::InvalidPeer(_))));
    }

    #[test]
    fn both_reach_exposes_both() {
        let p = Peer {
            device_id: Some("11111111-1111-4111-8111-111111111111".into()),
            alias: "Mac".into(),
            reach: Reach::Both {
                ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
                port: 7800,
                peer_id: "rid".into(),
            },
        };
        assert!(p.validate().is_ok());
        assert!(p.reach.direct_addr().is_some());
        assert_eq!(p.reach.relay_id().map(String::as_str), Some("rid"));
    }
}

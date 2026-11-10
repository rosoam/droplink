use crate::peer::{Peer, PeerKey, Reach, Source};
use indexmap::IndexMap;
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(Peer),
    Updated(Peer),
    Removed(PeerKey),
}

#[derive(Clone, Debug)]
struct Entry {
    peer: Peer,
    sources: HashSet<Source>,
}

/// Multi-source peer registry.
///
/// Direct peers (mDNS) are keyed by `device_id`; relay peers are keyed by the ephemeral `peer_id`
/// until a successful handshake proves their `device_id` (`bind_identity`).
#[derive(Default)]
pub struct DeviceRegistry {
    entries: IndexMap<PeerKey, Entry>,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn upsert(&mut self, peer: Peer, source: Source) -> Option<Change> {
        let key = Self::key_for(&peer, source)?;

        if let Some(id) = peer.device_id.as_deref() {
            let conflict = self
                .entries
                .iter()
                .any(|(k, e)| k != &key && e.peer.device_id.as_deref() == Some(id));
            if conflict {
                return None;
            }
        }

        if let Some(entry) = self.entries.get_mut(&key) {
            let mut changed = entry.peer.reach != peer.reach
                || entry.peer.alias != peer.alias
                || !entry.sources.contains(&source);
            entry.peer.reach = merge_reach(&entry.peer.reach, &peer.reach);
            entry.peer.alias = peer.alias;
            if entry.peer.device_id.is_none() {
                entry.peer.device_id = peer.device_id;
                changed = true;
            }
            entry.sources.insert(source);
            return changed.then(|| Change::Updated(entry.peer.clone()));
        }

        self.entries.insert(
            key,
            Entry {
                peer: peer.clone(),
                sources: HashSet::from([source]),
            },
        );
        Some(Change::Added(peer))
    }

    pub fn remove_source(&mut self, key: &PeerKey, source: Source) -> Option<Change> {
        let entry = self.entries.get_mut(key)?;
        entry.sources.remove(&source);
        if entry.sources.is_empty() {
            self.entries.shift_remove(key);
            return Some(Change::Removed(key.clone()));
        }
        None
    }

    pub fn purge_source(&mut self, source: Source) -> Vec<Change> {
        let keys: Vec<PeerKey> = self
            .entries
            .iter()
            .filter(|(_, e)| e.sources.contains(&source))
            .map(|(k, _)| k.clone())
            .collect();
        let mut changes = Vec::new();
        for key in keys {
            if let Some(change) = self.remove_source(&key, source) {
                changes.push(change);
            }
        }
        changes
    }

    pub fn snapshot(&self) -> Vec<Peer> {
        self.entries.values().map(|e| e.peer.clone()).collect()
    }

    pub fn by_device(&self, id: &str) -> Option<&Peer> {
        self.entries
            .values()
            .find(|e| e.peer.device_id.as_deref() == Some(id))
            .map(|e| &e.peer)
    }

    pub fn get(&self, key: &PeerKey) -> Option<&Peer> {
        self.entries.get(key).map(|e| &e.peer)
    }

    /// Bind a cryptographically proven `device_id` to an entry.
    ///
    /// If a direct entry for that device already exists, both entries are merged into a single
    /// `Reach::Both` peer. Returns `None` when the id is already bound elsewhere (anti-usurpation).
    pub fn bind_identity(&mut self, key: &PeerKey, id: String) -> Option<Change> {
        // A direct entry holding this id is the merge target, not a conflict.
        let merge_target = PeerKey::Direct(id.clone());
        let other_owner = self.entries.iter().any(|(k, e)| {
            k != key && *k != merge_target && e.peer.device_id.as_deref() == Some(id.as_str())
        });
        if other_owner {
            return None;
        }

        if let PeerKey::Relay(_) = key {
            let direct_key = PeerKey::Direct(id.clone());
            if let Some(direct) = self.entries.get(&direct_key) {
                // A direct peer already linked to another relay circuit cannot be re-bound:
                // that would let a relay peer hijack an existing identity.
                if direct.peer.reach.relay_id().is_some() {
                    return None;
                }
                let relay = self.entries.shift_remove(key)?;
                let direct = self.entries.get_mut(&direct_key)?;
                direct.peer.reach = merge_reach(&direct.peer.reach, &relay.peer.reach);
                direct.sources.extend(relay.sources);
                return Some(Change::Updated(direct.peer.clone()));
            }
        }

        let entry = self.entries.get_mut(key)?;
        if entry.peer.device_id.as_deref() == Some(id.as_str()) {
            return None;
        }
        entry.peer.device_id = Some(id);
        Some(Change::Updated(entry.peer.clone()))
    }

    fn key_for(peer: &Peer, source: Source) -> Option<PeerKey> {
        match source {
            Source::Mdns => peer.device_id.clone().map(PeerKey::Direct),
            Source::Manual => Some(PeerKey::Direct(peer.direct_key())),
            Source::Relay => peer.reach.relay_id().cloned().map(PeerKey::Relay),
        }
    }
}

fn merge_reach(old: &Reach, new: &Reach) -> Reach {
    match (old, new) {
        (Reach::Direct { ip, port }, Reach::Relay { peer_id }) => Reach::Both {
            ip: *ip,
            port: *port,
            peer_id: peer_id.clone(),
        },
        (Reach::Relay { peer_id }, Reach::Direct { ip, port }) => Reach::Both {
            ip: *ip,
            port: *port,
            peer_id: peer_id.clone(),
        },
        (Reach::Both { peer_id, .. }, Reach::Direct { ip, port }) => Reach::Both {
            ip: *ip,
            port: *port,
            peer_id: peer_id.clone(),
        },
        (Reach::Both { ip, port, .. }, Reach::Relay { peer_id }) => Reach::Both {
            ip: *ip,
            port: *port,
            peer_id: peer_id.clone(),
        },
        (_, new) => new.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    const ID_A: &str = "11111111-1111-4111-8111-111111111111";
    const ID_B: &str = "22222222-2222-4222-8222-222222222222";

    fn direct(id: &str, alias: &str, ip: [u8; 4], port: u16) -> Peer {
        Peer {
            device_id: Some(id.into()),
            alias: alias.into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::from(ip)),
                port,
            },
        }
    }

    fn relay(peer_id: &str, alias: &str) -> Peer {
        Peer {
            device_id: None,
            alias: alias.into(),
            reach: Reach::Relay {
                peer_id: peer_id.into(),
            },
        }
    }

    fn manual(ip: [u8; 4], port: u16) -> Peer {
        Peer {
            device_id: None,
            alias: "direct".into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::from(ip)),
                port,
            },
        }
    }

    #[test]
    fn manual_peer_without_identity_is_registered_by_address() {
        let mut r = DeviceRegistry::new();
        let change = r.upsert(manual([172, 20, 10, 1], 7777), Source::Manual);
        assert!(matches!(change, Some(Change::Added(_))));
        assert_eq!(r.len(), 1);
        assert!(r.get(&PeerKey::Direct("172.20.10.1:7777".into())).is_some());
    }

    #[test]
    fn manual_peer_updates_in_place() {
        let mut r = DeviceRegistry::new();
        r.upsert(manual([172, 20, 10, 1], 7777), Source::Manual);
        let mut autre = manual([172, 20, 10, 1], 7777);
        autre.alias = "iPhone".into();
        let change = r.upsert(autre, Source::Manual);
        assert!(matches!(change, Some(Change::Updated(_))));
        assert_eq!(r.len(), 1);
        assert_eq!(
            r.get(&PeerKey::Direct("172.20.10.1:7777".into()))
                .map(|p| p.alias.as_str()),
            Some("iPhone")
        );
    }

    #[test]
    fn add_first_source_emits_added() {
        let mut r = DeviceRegistry::new();
        let change = r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        assert!(matches!(change, Some(Change::Added(_))));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn same_peer_same_source_is_noop() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        assert!(r
            .upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns)
            .is_none());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn ip_change_emits_updated() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        let change = r.upsert(direct(ID_A, "A", [2, 2, 2, 2], 7778), Source::Mdns);
        match change {
            Some(Change::Updated(p)) => {
                assert_eq!(p.reach.direct_addr().unwrap().1, 7778);
            }
            other => panic!("expected updated, got {other:?}"),
        }
    }

    #[test]
    fn same_name_two_devices() {
        let mut r = DeviceRegistry::new();
        r.upsert(
            direct(ID_A, "MacBook Pro", [1, 1, 1, 1], 7777),
            Source::Mdns,
        );
        r.upsert(
            direct(ID_B, "MacBook Pro", [2, 2, 2, 2], 7777),
            Source::Mdns,
        );
        assert_eq!(r.len(), 2);
        assert_eq!(r.snapshot().len(), 2);
    }

    #[test]
    fn duplicate_device_id_rejected() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        // Same device_id advertised from a different reach under a different key.
        let mut impostor = relay("peer-x", "A");
        impostor.device_id = Some(ID_A.into());
        assert!(r.upsert(impostor, Source::Relay).is_none());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn remove_last_source_emits_removed() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        let change = r.remove_source(&PeerKey::Direct(ID_A.into()), Source::Mdns);
        assert!(matches!(change, Some(Change::Removed(_))));
        assert!(r.is_empty());
    }

    #[test]
    fn remove_one_of_two_sources_keeps_peer() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        r.upsert(relay("peer-a", "A"), Source::Relay);
        r.bind_identity(&PeerKey::Relay("peer-a".into()), ID_A.into());
        assert!(r
            .remove_source(&PeerKey::Direct(ID_A.into()), Source::Relay)
            .is_none());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn purge_source_keeps_shared() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        r.upsert(relay("peer-a", "A"), Source::Relay);
        r.bind_identity(&PeerKey::Relay("peer-a".into()), ID_A.into());
        r.upsert(relay("peer-b", "B"), Source::Relay);
        let changes = r.purge_source(Source::Relay);
        assert_eq!(changes.len(), 1);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn bind_identity_merges_direct_and_relay() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        r.upsert(relay("peer-a", "A"), Source::Relay);
        let change = r.bind_identity(&PeerKey::Relay("peer-a".into()), ID_A.into());
        assert!(matches!(change, Some(Change::Updated(_))));
        assert_eq!(r.len(), 1);
        let peer = r.get(&PeerKey::Direct(ID_A.into())).unwrap();
        assert!(matches!(peer.reach, Reach::Both { .. }));
    }

    #[test]
    fn bind_identity_rejects_already_bound() {
        let mut r = DeviceRegistry::new();
        r.upsert(direct(ID_A, "A", [1, 1, 1, 1], 7777), Source::Mdns);
        r.upsert(relay("peer-a", "A"), Source::Relay);
        r.upsert(relay("peer-b", "B"), Source::Relay);
        assert!(r
            .bind_identity(&PeerKey::Relay("peer-a".into()), ID_A.into())
            .is_some());
        assert!(r
            .bind_identity(&PeerKey::Relay("peer-b".into()), ID_A.into())
            .is_none());
    }

    #[test]
    fn bind_identity_sets_id_without_direct_entry() {
        let mut r = DeviceRegistry::new();
        r.upsert(relay("peer-a", "A"), Source::Relay);
        let change = r.bind_identity(&PeerKey::Relay("peer-a".into()), ID_A.into());
        assert!(matches!(change, Some(Change::Updated(_))));
        assert_eq!(r.by_device(ID_A).map(|p| p.alias.as_str()), Some("A"));
    }
}

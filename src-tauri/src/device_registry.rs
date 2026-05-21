use crate::protocol::DeviceInfo;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Mdns,
    Relay,
    #[allow(dead_code)] // used in Task 7 (BLE integration)
    Ble,
}

#[derive(Default)]
pub struct DeviceRegistry {
    devices: Vec<DeviceInfo>,
    sources: HashMap<String, HashSet<Source>>,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<DeviceInfo> {
        self.devices.clone()
    }

    /// Add a device from a source. Returns Some(device) if this is the first
    /// source for this device (UI should emit `device-discovered`).
    pub fn add(&mut self, device: DeviceInfo, source: Source) -> Option<DeviceInfo> {
        let id = key(&device);
        let entry = self.sources.entry(id.clone()).or_default();
        let was_empty = entry.is_empty();
        entry.insert(source);
        if was_empty {
            self.devices.push(device.clone());
            Some(device)
        } else {
            None
        }
    }

    /// Remove a device from a source. Returns Some(name) if the device has no
    /// remaining sources (UI should emit `device-lost`).
    pub fn remove(&mut self, id: &str, source: Source) -> Option<String> {
        let entry = self.sources.get_mut(id)?;
        entry.remove(&source);
        if entry.is_empty() {
            self.sources.remove(id);
            let name = self
                .devices
                .iter()
                .find(|d| key(d) == id)
                .map(|d| d.name.clone());
            self.devices.retain(|d| key(d) != id);
            name
        } else {
            None
        }
    }

    /// Remove all devices from a given source (used when a channel disconnects).
    /// Returns names of devices that are now fully gone.
    pub fn purge_source(&mut self, source: Source) -> Vec<String> {
        let ids: Vec<String> = self.sources.keys().cloned().collect();
        let mut lost = Vec::new();
        for id in ids {
            if let Some(name) = self.remove(&id, source) {
                lost.push(name);
            }
        }
        lost
    }

    /// Return ip:port ids known via a specific source.
    pub fn ids_for_source(&self, source: Source) -> HashSet<String> {
        self.sources
            .iter()
            .filter_map(|(id, sources)| sources.contains(&source).then(|| id.clone()))
            .collect()
    }
}

pub fn key(d: &DeviceInfo) -> String {
    format!("{}:{}", d.ip, d.port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, ip: &str, port: u16) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            ip: ip.to_string(),
            port,
        }
    }

    #[test]
    fn add_first_source_emits_discovered() {
        let mut r = DeviceRegistry::new();
        let added = r.add(dev("A", "1.1.1.1", 7777), Source::Mdns);
        assert!(added.is_some());
        assert_eq!(r.snapshot().len(), 1);
    }

    #[test]
    fn add_second_source_does_not_re_emit() {
        let mut r = DeviceRegistry::new();
        r.add(dev("A", "1.1.1.1", 7777), Source::Mdns);
        let added = r.add(dev("A", "1.1.1.1", 7777), Source::Relay);
        assert!(added.is_none());
        assert_eq!(r.snapshot().len(), 1);
    }

    #[test]
    fn remove_last_source_emits_lost() {
        let mut r = DeviceRegistry::new();
        r.add(dev("A", "1.1.1.1", 7777), Source::Mdns);
        let lost = r.remove("1.1.1.1:7777", Source::Mdns);
        assert_eq!(lost.as_deref(), Some("A"));
        assert!(r.snapshot().is_empty());
    }

    #[test]
    fn remove_one_of_two_sources_keeps_device() {
        let mut r = DeviceRegistry::new();
        r.add(dev("A", "1.1.1.1", 7777), Source::Mdns);
        r.add(dev("A", "1.1.1.1", 7777), Source::Relay);
        let lost = r.remove("1.1.1.1:7777", Source::Relay);
        assert!(lost.is_none());
        assert_eq!(r.snapshot().len(), 1);
    }

    #[test]
    fn purge_source_removes_only_exclusive_devices() {
        let mut r = DeviceRegistry::new();
        r.add(dev("A", "1.1.1.1", 7777), Source::Mdns);
        r.add(dev("A", "1.1.1.1", 7777), Source::Relay);
        r.add(dev("B", "2.2.2.2", 7777), Source::Relay);
        let lost = r.purge_source(Source::Relay);
        assert_eq!(lost, vec!["B".to_string()]);
        assert_eq!(r.snapshot().len(), 1);
    }
}

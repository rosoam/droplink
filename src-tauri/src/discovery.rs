use crate::protocol::DeviceInfo;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::IpAddr;
use tokio::sync::mpsc;

const SERVICE_TYPE: &str = "_droplink._tcp.local.";

pub struct Discovery {
    mdns: ServiceDaemon,
    service_name: String,
}

pub enum DiscoveryEvent {
    DeviceFound(DeviceInfo),
    DeviceLost(String),
}

impl Discovery {
    pub fn new(device_name: &str, local_ip: &str, port: u16) -> Result<Self, String> {
        let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
        let hostname = format!("{}.local.", device_name.replace(' ', "-").to_lowercase());
        let service_name = device_name.to_string();

        let mut properties = HashMap::new();
        properties.insert("name".to_string(), device_name.to_string());

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            device_name,
            &hostname,
            local_ip,
            port,
            Some(properties),
        )
        .map_err(|e| e.to_string())?;

        mdns.register(service_info).map_err(|e| e.to_string())?;

        Ok(Discovery { mdns, service_name })
    }

    pub fn start_browsing(&self, tx: mpsc::Sender<DiscoveryEvent>) -> Result<(), String> {
        let receiver = self
            .mdns
            .browse(SERVICE_TYPE)
            .map_err(|e| e.to_string())?;
        let own_name = self.service_name.clone();

        tokio::spawn(async move {
            loop {
                match receiver.recv_async().await {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        let name = info
                            .get_property_val_str("name")
                            .unwrap_or(info.get_fullname())
                            .to_string();
                        if name == own_name {
                            continue;
                        }
                        if let Some(addr) = info.get_addresses().iter().find(|a| matches!(a, IpAddr::V4(_))) {
                            let device = DeviceInfo {
                                name: name.clone(),
                                ip: addr.to_string(),
                                port: info.get_port(),
                            };
                            let _ = tx.send(DiscoveryEvent::DeviceFound(device)).await;
                        }
                    }
                    Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                        let _ = tx
                            .send(DiscoveryEvent::DeviceLost(fullname.clone()))
                            .await;
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        });

        Ok(())
    }
}

pub fn find_free_port(start: u16, end: u16) -> Option<u16> {
    for port in start..=end {
        if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

pub fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_free_port_returns_valid_port() {
        let port = find_free_port(7777, 7800);
        assert!(port.is_some());
        let p = port.unwrap();
        assert!(p >= 7777 && p <= 7800);
    }

    #[test]
    fn test_get_local_ip_returns_ipv4() {
        let ip = get_local_ip();
        assert!(ip.is_some());
        let ip_str = ip.unwrap();
        assert!(ip_str.contains('.'), "Expected IPv4, got: {}", ip_str);
    }
}

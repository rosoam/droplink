use crate::protocol::DeviceId;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_RELAY_URL: &str = "wss://relay.droplink.app/ws";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub device_id: DeviceId,
    pub device_name: String,
    pub receive_dir: PathBuf,
    #[serde(default)]
    pub port_pref: Option<u16>,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    #[serde(default = "default_accept_timeout")]
    pub accept_timeout_secs: u64,
    /// Deprecated: the relay now starts only when a session is requested. Kept for config
    /// migration so an older file does not reset the device identity.
    #[serde(default)]
    pub relay_enabled: bool,
    #[serde(default)]
    pub relay_url: Option<String>,
    /// Deprecated: replaced by server-issued ephemeral session codes.
    #[serde(default)]
    pub room_code: Option<String>,
    #[serde(default = "default_history_len")]
    pub history_len: usize,
}

fn default_max_parallel() -> usize {
    4
}

fn default_accept_timeout() -> u64 {
    60
}

fn default_history_len() -> usize {
    100
}

impl Config {
    pub fn new(device_id: DeviceId, device_name: String, receive_dir: PathBuf) -> Self {
        Self {
            device_id,
            device_name,
            receive_dir,
            port_pref: None,
            max_parallel: 4,
            accept_timeout_secs: 60,
            // Le relay est une option explicite : aucun appel réseau par défaut.
            relay_enabled: false,
            relay_url: Some(DEFAULT_RELAY_URL.to_string()),
            room_code: None,
            history_len: 100,
        }
    }

    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.device_id)
            .map_err(|_| Error::Protocol(format!("invalid device_id: {}", self.device_id)))?;
        if self.device_name.trim().is_empty() || self.device_name.chars().count() > 64 {
            return Err(Error::Protocol("invalid device name".into()));
        }
        if self.max_parallel == 0 || self.max_parallel > 32 {
            return Err(Error::Protocol("max_parallel out of range".into()));
        }
        if self.accept_timeout_secs < 5 || self.accept_timeout_secs > 3600 {
            return Err(Error::Protocol("accept_timeout_secs out of range".into()));
        }
        if self.history_len > 10_000 {
            return Err(Error::Protocol("history_len too large".into()));
        }
        if self.receive_dir.as_os_str().is_empty() {
            return Err(Error::Protocol("receive_dir must not be empty".into()));
        }
        if let Some(port) = self.port_pref {
            if port < 1024 {
                return Err(Error::Protocol("port_pref must be >= 1024".into()));
            }
        }
        if let Some(code) = &self.room_code {
            if code.len() < 4 || code.len() > 64 {
                return Err(Error::Protocol("room_code length out of range".into()));
            }
        }
        Ok(())
    }
}

pub fn generate_device_id() -> DeviceId {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::new(
            generate_device_id(),
            "Mac".into(),
            PathBuf::from("/tmp/drop"),
        )
    }

    #[test]
    fn defaults_are_valid() {
        let c = cfg();
        assert!(c.validate().is_ok());
        assert_eq!(c.max_parallel, 4);
        assert_eq!(c.accept_timeout_secs, 60);
        assert!(!c.relay_enabled);
    }

    #[test]
    fn old_config_without_new_fields_keeps_the_identity() {
        let base = cfg();
        let mut value = serde_json::to_value(&base).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("room_code");
        object.remove("relay_enabled");
        object.remove("relay_url");
        let back: Config = serde_json::from_value(value).unwrap();
        assert_eq!(back.device_id, base.device_id);
        assert_eq!(back.device_name, "Mac");
        assert!(back.room_code.is_none());
        assert!(!back.relay_enabled);
    }

    #[test]
    fn json_roundtrip() {
        let c = cfg();
        let json = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn rejects_bad_parallelism() {
        let mut c = cfg();
        c.max_parallel = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_short_timeout() {
        let mut c = cfg();
        c.accept_timeout_secs = 1;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_invalid_device_id() {
        let mut c = cfg();
        c.device_id = "nope".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_privileged_port() {
        let mut c = cfg();
        c.port_pref = Some(80);
        assert!(c.validate().is_err());
        c.port_pref = Some(7788);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn rejects_empty_receive_dir() {
        let mut c = cfg();
        c.receive_dir = PathBuf::new();
        assert!(c.validate().is_err());
    }

    #[test]
    fn device_ids_are_unique() {
        assert_ne!(generate_device_id(), generate_device_id());
    }
}

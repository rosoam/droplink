use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Message {
    Hello {
        sender_name: String,
        file_name: String,
        file_size: u64,
        checksum: String,
        transfer_id: String,
    },
    Accept {
        transfer_id: String,
    },
    Refuse {
        transfer_id: String,
    },
    Done,
    DoneAck,
    Error {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_info_roundtrip() {
        let device = DeviceInfo {
            name: "MacBook de Léa".to_string(),
            ip: "192.168.1.42".to_string(),
            port: 7777,
        };
        let json = serde_json::to_string(&device).unwrap();
        let parsed: DeviceInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(device, parsed);
    }

    #[test]
    fn test_message_hello_serialization() {
        let msg = Message::Hello {
            sender_name: "Léa".to_string(),
            file_name: "photo.jpg".to_string(),
            file_size: 1024,
            checksum: "abc123".to_string(),
            transfer_id: "uuid-1234".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"HELLO\""));
        assert!(json.contains("photo.jpg"));
    }

    #[test]
    fn test_message_accept_deserialization() {
        let json = r#"{"type":"ACCEPT","transfer_id":"uuid-1234"}"#;
        let msg: Message = serde_json::from_str(json).unwrap();
        match msg {
            Message::Accept { transfer_id } => assert_eq!(transfer_id, "uuid-1234"),
            _ => panic!("Wrong variant"),
        }
    }
}

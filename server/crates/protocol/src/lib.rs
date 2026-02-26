use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 64 * 1024;

pub fn is_supported_protocol_version(version: u16) -> bool {
    (MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&version)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub v: u16,
    pub t: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p: Option<Value>,
}

impl Message {
    pub fn validate(&self, max_payload_bytes: usize) -> Result<(), ProtocolError> {
        if !is_supported_protocol_version(self.v) {
            return Err(ProtocolError::UnsupportedVersion(self.v));
        }

        if self.t.trim().is_empty() {
            return Err(ProtocolError::MissingType);
        }

        if let Some(payload) = &self.p {
            let payload_len = serde_json::to_vec(payload)
                .map(|bytes| bytes.len())
                .unwrap_or(usize::MAX);
            if payload_len > max_payload_bytes {
                return Err(ProtocolError::PayloadTooLarge);
            }
        }

        Ok(())
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ProtocolError {
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u16),
    #[error("message type is required")]
    MissingType,
    #[error("payload exceeds maximum size")]
    PayloadTooLarge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handshake {
    pub v: u16,
    pub codecs: Vec<String>,
    pub project_id: String,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_message() -> Message {
        Message {
            v: PROTOCOL_VERSION,
            t: "room.join".to_owned(),
            rid: Some("rid-1".to_owned()),
            room: Some("room-a".to_owned()),
            p: Some(json!({ "k": "v" })),
        }
    }

    #[test]
    fn invalid_version_rejected() {
        let mut msg = valid_message();
        msg.v = 2;

        let err = msg.validate(DEFAULT_MAX_PAYLOAD_BYTES).unwrap_err();
        assert_eq!(err, ProtocolError::UnsupportedVersion(2));
    }

    #[test]
    fn supported_version_helper_matches_range() {
        assert!(is_supported_protocol_version(1));
        assert!(!is_supported_protocol_version(0));
        assert!(!is_supported_protocol_version(2));
    }

    #[test]
    fn missing_type_rejected() {
        let mut msg = valid_message();
        msg.t = "".to_owned();

        let err = msg.validate(DEFAULT_MAX_PAYLOAD_BYTES).unwrap_err();
        assert_eq!(err, ProtocolError::MissingType);
    }

    #[test]
    fn payload_size_limit_enforced() {
        let mut msg = valid_message();
        msg.p = Some(json!({ "blob": "x".repeat(128) }));

        let err = msg.validate(8).unwrap_err();
        assert_eq!(err, ProtocolError::PayloadTooLarge);
    }

    #[test]
    fn handshake_accepts_missing_session_id() {
        let raw = json!({
            "v": 1,
            "codecs": ["json"],
            "project_id": "p1",
            "token": "t1"
        });

        let handshake: Handshake =
            serde_json::from_value(raw).expect("handshake should decode without session_id");
        assert_eq!(handshake.session_id, None);
    }
}

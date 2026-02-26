use std::collections::HashMap;

use protocol::Message;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum RpcError {
    #[error("request id already exists")]
    DuplicateRid,
    #[error("unknown request id")]
    UnknownRid,
}

#[derive(Debug, Default)]
pub struct RpcTracker {
    pending: HashMap<String, String>,
}

impl RpcTracker {
    pub fn register_request(&mut self, rid: String, request_type: String) -> Result<(), RpcError> {
        if self.pending.contains_key(&rid) {
            return Err(RpcError::DuplicateRid);
        }

        self.pending.insert(rid, request_type);
        Ok(())
    }

    pub fn resolve_response(&mut self, response: &Message) -> Result<String, RpcError> {
        let rid = response.rid.as_ref().ok_or(RpcError::UnknownRid)?;
        self.pending.remove(rid).ok_or(RpcError::UnknownRid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::PROTOCOL_VERSION;

    #[test]
    fn request_with_rid_maps_to_correct_response() {
        let mut tracker = RpcTracker::default();
        tracker
            .register_request("rid-1".to_owned(), "room.join".to_owned())
            .expect("register should pass");

        let response = Message {
            v: PROTOCOL_VERSION,
            t: "room.join.ok".to_owned(),
            rid: Some("rid-1".to_owned()),
            room: Some("room-a".to_owned()),
            p: None,
        };

        let request_type = tracker
            .resolve_response(&response)
            .expect("rid should be found");
        assert_eq!(request_type, "room.join");
    }

    #[test]
    fn unknown_rid_rejected() {
        let mut tracker = RpcTracker::default();

        let response = Message {
            v: PROTOCOL_VERSION,
            t: "room.join.ok".to_owned(),
            rid: Some("missing".to_owned()),
            room: None,
            p: None,
        };

        let err = tracker.resolve_response(&response).unwrap_err();
        assert_eq!(err, RpcError::UnknownRid);
    }

    #[test]
    fn duplicate_rid_rejected() {
        let mut tracker = RpcTracker::default();
        tracker
            .register_request("rid-1".to_owned(), "room.join".to_owned())
            .expect("first register should pass");

        let err = tracker
            .register_request("rid-1".to_owned(), "room.leave".to_owned())
            .unwrap_err();
        assert_eq!(err, RpcError::DuplicateRid);
    }

    #[test]
    fn late_duplicate_response_rejected_after_resolution() {
        let mut tracker = RpcTracker::default();
        tracker
            .register_request("rid-2".to_owned(), "room.message".to_owned())
            .expect("register should pass");

        let response = Message {
            v: PROTOCOL_VERSION,
            t: "rpc.response".to_owned(),
            rid: Some("rid-2".to_owned()),
            room: Some("room-a".to_owned()),
            p: None,
        };

        tracker
            .resolve_response(&response)
            .expect("first response should resolve");
        let err = tracker.resolve_response(&response).unwrap_err();
        assert_eq!(err, RpcError::UnknownRid);
    }
}

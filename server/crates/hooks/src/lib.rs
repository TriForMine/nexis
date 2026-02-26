use serde_json::Value;

pub trait RoomHooks: Send + Sync {
    fn on_create(&self, _room_id: &str, _room_type: &str) {}
    fn on_join(&self, _room_id: &str, _client_id: &str) {}
    fn on_leave(&self, _room_id: &str, _client_id: &str) {}
    fn on_message(
        &self,
        _room_id: &str,
        _client_id: &str,
        _message_type: &str,
        _payload: &Option<Value>,
    ) {
    }
    fn on_tick(&self, _room_id: &str) {}
}

#[derive(Debug, Default)]
pub struct NoopHooks;

impl RoomHooks for NoopHooks {}

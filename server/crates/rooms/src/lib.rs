use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use hooks::RoomHooks;
use serde_json::json;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub id: String,
    pub room_type: String,
    pub clients: HashSet<String>,
    pub state: Value,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub last_tick_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomMetadata {
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub last_tick_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomSummary {
    pub id: String,
    pub room_type: String,
    pub members: usize,
}

#[derive(Debug, Error, PartialEq)]
pub enum RoomError {
    #[error("unsupported room type")]
    UnsupportedType,
    #[error("duplicate room type")]
    DuplicateRoomType,
    #[error("client already joined")]
    AlreadyJoined,
    #[error("room not found")]
    RoomNotFound,
    #[error("client not found")]
    ClientNotFound,
}

pub const ECHO_ROOM: &str = "echo_room";
pub const COUNTER_ROOM: &str = "counter_room";

type RoomStateFactory = Arc<dyn Fn() -> Value + Send + Sync>;

pub trait RoomPlugin: Send + Sync {
    fn room_type(&self) -> &str;
    fn initial_state(&self) -> Value;
}

pub struct RoomTypeRegistry {
    factories: HashMap<String, RoomStateFactory>,
}

impl Default for RoomTypeRegistry {
    fn default() -> Self {
        let mut registry = Self {
            factories: HashMap::new(),
        };
        registry
            .register_room_plugin(ECHO_ROOM, || json!({}))
            .expect("builtin echo room must register");
        registry
            .register_room_plugin(COUNTER_ROOM, || json!({ "counter": 0 }))
            .expect("builtin counter room must register");
        registry
    }
}

impl RoomTypeRegistry {
    pub fn register_plugin<P>(&mut self, plugin: P) -> Result<(), RoomError>
    where
        P: RoomPlugin + 'static,
    {
        let room_type = plugin.room_type().to_owned();
        self.register_room_plugin(&room_type, move || plugin.initial_state())
    }

    pub fn register_room_plugin<F>(&mut self, room_type: &str, factory: F) -> Result<(), RoomError>
    where
        F: Fn() -> Value + Send + Sync + 'static,
    {
        if self.factories.contains_key(room_type) {
            return Err(RoomError::DuplicateRoomType);
        }
        self.factories
            .insert(room_type.to_owned(), Arc::new(factory));
        Ok(())
    }

    fn state_for(&self, room_type: &str) -> Option<Value> {
        self.factories.get(room_type).map(|factory| factory())
    }
}

pub struct RoomManager<H: RoomHooks> {
    hooks: H,
    room_types: RoomTypeRegistry,
    rooms: HashMap<String, Room>,
}

impl<H: RoomHooks> RoomManager<H> {
    pub fn new(hooks: H) -> Self {
        Self::with_registry(hooks, RoomTypeRegistry::default())
    }

    pub fn with_registry(hooks: H, room_types: RoomTypeRegistry) -> Self {
        Self {
            hooks,
            room_types,
            rooms: HashMap::new(),
        }
    }

    pub fn register_plugin<P>(&mut self, plugin: P) -> Result<(), RoomError>
    where
        P: RoomPlugin + 'static,
    {
        self.room_types.register_plugin(plugin)
    }

    pub fn register_room_plugin<F>(&mut self, room_type: &str, factory: F) -> Result<(), RoomError>
    where
        F: Fn() -> Value + Send + Sync + 'static,
    {
        self.room_types.register_room_plugin(room_type, factory)
    }

    pub fn join_or_create(
        &mut self,
        room_id: &str,
        room_type: &str,
        client_id: &str,
    ) -> Result<(), RoomError> {
        self.join_or_create_at(room_id, room_type, client_id, Utc::now())
    }

    pub fn join_or_create_at(
        &mut self,
        room_id: &str,
        room_type: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(), RoomError> {
        if self.room_types.state_for(room_type).is_none() {
            return Err(RoomError::UnsupportedType);
        }

        if let Some(room) = self.rooms.get_mut(room_id) {
            if room.room_type != room_type {
                return Err(RoomError::UnsupportedType);
            }
            if room.clients.contains(client_id) {
                return Err(RoomError::AlreadyJoined);
            }
            room.clients.insert(client_id.to_owned());
            room.last_activity_at = now;
            self.hooks.on_join(room_id, client_id);
            return Ok(());
        }

        let mut clients = HashSet::new();
        clients.insert(client_id.to_owned());

        let room = Room {
            id: room_id.to_owned(),
            room_type: room_type.to_owned(),
            clients,
            state: self
                .room_types
                .state_for(room_type)
                .expect("validated room type"),
            created_at: now,
            last_activity_at: now,
            last_tick_at: now,
        };

        self.hooks.on_create(room_id, room_type);
        self.hooks.on_join(room_id, client_id);
        self.rooms.insert(room_id.to_owned(), room);
        Ok(())
    }

    pub fn leave(&mut self, room_id: &str, client_id: &str) -> Result<(), RoomError> {
        self.leave_at(room_id, client_id, Utc::now())
    }

    pub fn leave_at(
        &mut self,
        room_id: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(), RoomError> {
        let room = self.rooms.get_mut(room_id).ok_or(RoomError::RoomNotFound)?;
        if !room.clients.remove(client_id) {
            return Err(RoomError::ClientNotFound);
        }
        room.last_activity_at = now;
        self.hooks.on_leave(room_id, client_id);

        if room.clients.is_empty() {
            self.rooms.remove(room_id);
        }

        Ok(())
    }

    pub fn room(&self, room_id: &str) -> Option<&Room> {
        self.rooms.get(room_id)
    }

    pub fn room_mut(&mut self, room_id: &str) -> Option<&mut Room> {
        self.rooms.get_mut(room_id)
    }

    pub fn room_metadata(&self, room_id: &str) -> Option<RoomMetadata> {
        self.rooms.get(room_id).map(|room| RoomMetadata {
            created_at: room.created_at,
            last_activity_at: room.last_activity_at,
            last_tick_at: room.last_tick_at,
        })
    }

    pub fn mark_activity(&mut self, room_id: &str, now: DateTime<Utc>) -> Result<(), RoomError> {
        let room = self.rooms.get_mut(room_id).ok_or(RoomError::RoomNotFound)?;
        room.last_activity_at = now;
        Ok(())
    }

    pub fn tick(&mut self, now: DateTime<Utc>, interval: Duration) -> Vec<String> {
        let normalized_interval = if interval < Duration::milliseconds(1) {
            Duration::milliseconds(1)
        } else {
            interval
        };

        let mut ticked = Vec::new();
        for room in self.rooms.values_mut() {
            if now.signed_duration_since(room.last_tick_at) >= normalized_interval {
                room.last_tick_at = now;
                self.hooks.on_tick(&room.id);
                ticked.push(room.id.clone());
            }
        }
        ticked.sort();
        ticked
    }

    pub fn room_members(&self, room_id: &str) -> Option<Vec<String>> {
        let room = self.rooms.get(room_id)?;
        let mut members = room.clients.iter().cloned().collect::<Vec<_>>();
        members.sort();
        Some(members)
    }

    pub fn room_type(&self, room_id: &str) -> Option<String> {
        self.rooms.get(room_id).map(|room| room.room_type.clone())
    }

    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    pub fn list_rooms(&self, room_type: Option<&str>) -> Vec<RoomSummary> {
        let mut rooms = self
            .rooms
            .values()
            .filter(|room| room_type.map(|kind| kind == room.room_type).unwrap_or(true))
            .map(|room| RoomSummary {
                id: room.id.clone(),
                room_type: room.room_type.clone(),
                members: room.clients.len(),
            })
            .collect::<Vec<_>>();
        rooms.sort_by(|left, right| left.id.cmp(&right.id));
        rooms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use hooks::NoopHooks;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordingHooks {
        ticks: Arc<Mutex<Vec<String>>>,
    }

    impl hooks::RoomHooks for RecordingHooks {
        fn on_tick(&self, room_id: &str) {
            self.ticks
                .lock()
                .expect("lock tick list")
                .push(room_id.to_owned());
        }
    }

    #[test]
    fn join_or_create_creates_room() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-1")
            .expect("join should work");

        assert_eq!(rooms.room_count(), 1);
        let room = rooms.room("room-1").expect("room must exist");
        assert!(room.clients.contains("client-1"));
    }

    #[test]
    fn joining_twice_prevented() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-1")
            .expect("first join should work");

        let err = rooms
            .join_or_create("room-1", ECHO_ROOM, "client-1")
            .unwrap_err();

        assert_eq!(err, RoomError::AlreadyJoined);
    }

    #[test]
    fn leaving_removes_client() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-1")
            .expect("join should work");
        rooms
            .leave("room-1", "client-1")
            .expect("leave should work");

        assert!(rooms.room("room-1").is_none());
    }

    #[test]
    fn room_disposed_when_empty() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-1")
            .expect("first join should work");
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-2")
            .expect("second join should work");

        rooms
            .leave("room-1", "client-1")
            .expect("first leave should work");
        assert_eq!(rooms.room_count(), 1);

        rooms
            .leave("room-1", "client-2")
            .expect("second leave should work");
        assert_eq!(rooms.room_count(), 0);
    }

    #[test]
    fn room_members_sorted_for_stable_presence_payloads() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-c")
            .expect("join should work");
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-a")
            .expect("join should work");
        rooms
            .join_or_create("room-1", ECHO_ROOM, "client-b")
            .expect("join should work");

        let members = rooms.room_members("room-1").expect("room should exist");
        assert_eq!(
            members,
            vec![
                "client-a".to_owned(),
                "client-b".to_owned(),
                "client-c".to_owned()
            ]
        );
    }

    #[test]
    fn room_discovery_lists_rooms_with_member_count() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .join_or_create("counter_room:alpha", COUNTER_ROOM, "client-1")
            .expect("join should work");
        rooms
            .join_or_create("echo_room:beta", ECHO_ROOM, "client-2")
            .expect("join should work");
        rooms
            .join_or_create("counter_room:alpha", COUNTER_ROOM, "client-3")
            .expect("join should work");

        let discovered = rooms.list_rooms(None);
        assert_eq!(discovered.len(), 2);
        assert_eq!(
            discovered,
            vec![
                RoomSummary {
                    id: "counter_room:alpha".to_owned(),
                    room_type: COUNTER_ROOM.to_owned(),
                    members: 2,
                },
                RoomSummary {
                    id: "echo_room:beta".to_owned(),
                    room_type: ECHO_ROOM.to_owned(),
                    members: 1,
                },
            ]
        );
    }

    #[test]
    fn room_discovery_filters_by_room_type() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .join_or_create("counter_room:alpha", COUNTER_ROOM, "client-1")
            .expect("join should work");
        rooms
            .join_or_create("echo_room:beta", ECHO_ROOM, "client-2")
            .expect("join should work");

        let discovered = rooms.list_rooms(Some(COUNTER_ROOM));
        assert_eq!(
            discovered,
            vec![RoomSummary {
                id: "counter_room:alpha".to_owned(),
                room_type: COUNTER_ROOM.to_owned(),
                members: 1,
            }]
        );
    }

    #[test]
    fn tick_invokes_hook_on_interval() {
        let hooks = RecordingHooks::default();
        let mut rooms = RoomManager::new(hooks.clone());
        let t0 = Utc.with_ymd_and_hms(2026, 2, 25, 18, 0, 0).unwrap();

        rooms
            .join_or_create_at("counter_room:alpha", COUNTER_ROOM, "client-1", t0)
            .expect("join should work");

        let first = rooms.tick(t0 + Duration::milliseconds(500), Duration::seconds(1));
        assert!(first.is_empty());

        let second = rooms.tick(t0 + Duration::seconds(1), Duration::seconds(1));
        assert_eq!(second, vec!["counter_room:alpha".to_owned()]);

        let third = rooms.tick(
            t0 + Duration::seconds(1) + Duration::milliseconds(500),
            Duration::seconds(1),
        );
        assert!(third.is_empty());

        let fourth = rooms.tick(t0 + Duration::seconds(2), Duration::seconds(1));
        assert_eq!(fourth, vec!["counter_room:alpha".to_owned()]);

        assert_eq!(
            hooks.ticks.lock().expect("lock ticks").clone(),
            vec![
                "counter_room:alpha".to_owned(),
                "counter_room:alpha".to_owned()
            ]
        );
    }

    #[test]
    fn room_activity_timestamp_is_updated_when_touched() {
        let mut rooms = RoomManager::new(NoopHooks);
        let t0 = Utc.with_ymd_and_hms(2026, 2, 25, 18, 0, 0).unwrap();
        let t1 = t0 + Duration::seconds(15);

        rooms
            .join_or_create_at("echo_room:alpha", ECHO_ROOM, "client-1", t0)
            .expect("join should work");
        rooms
            .mark_activity("echo_room:alpha", t1)
            .expect("room should exist");

        let metadata = rooms
            .room_metadata("echo_room:alpha")
            .expect("metadata should exist");
        assert_eq!(metadata.created_at, t0);
        assert_eq!(metadata.last_activity_at, t1);
    }

    #[test]
    fn custom_room_plugin_registration_allows_join() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .register_room_plugin("duel_room", || json!({ "hp": 100 }))
            .expect("plugin registration should succeed");

        rooms
            .join_or_create("duel_room:alpha", "duel_room", "client-1")
            .expect("join should work for plugin room type");

        let room = rooms.room("duel_room:alpha").expect("room should exist");
        assert_eq!(room.state, json!({ "hp": 100 }));
    }

    #[test]
    fn duplicate_room_plugin_registration_rejected() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .register_room_plugin("duel_room", || json!({ "hp": 100 }))
            .expect("first plugin registration should succeed");

        let err = rooms
            .register_room_plugin("duel_room", || json!({ "hp": 200 }))
            .expect_err("duplicate plugin registration should fail");

        assert_eq!(err, RoomError::DuplicateRoomType);
    }

    #[test]
    fn plugin_room_state_factory_returns_fresh_state() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .register_room_plugin("match_room", || json!({ "score": 0 }))
            .expect("plugin registration should succeed");

        rooms
            .join_or_create("match_room:a", "match_room", "client-1")
            .expect("join should work");
        rooms
            .join_or_create("match_room:b", "match_room", "client-2")
            .expect("join should work");

        let room_a = rooms.room("match_room:a").expect("room a should exist");
        let room_b = rooms.room("match_room:b").expect("room b should exist");
        assert_eq!(room_a.state, json!({ "score": 0 }));
        assert_eq!(room_b.state, json!({ "score": 0 }));
    }

    #[derive(Clone, Copy)]
    struct DuelPlugin;

    impl RoomPlugin for DuelPlugin {
        fn room_type(&self) -> &str {
            "duel_plugin_room"
        }

        fn initial_state(&self) -> Value {
            json!({ "hp": 100 })
        }
    }

    #[test]
    fn room_plugin_trait_registration_allows_join() {
        let mut rooms = RoomManager::new(NoopHooks);
        rooms
            .register_plugin(DuelPlugin)
            .expect("plugin registration should succeed");

        rooms
            .join_or_create("duel_plugin_room:alpha", "duel_plugin_room", "client-1")
            .expect("join should work");

        let room = rooms
            .room("duel_plugin_room:alpha")
            .expect("room should exist");
        assert_eq!(room.state, json!({ "hp": 100 }));
    }
}

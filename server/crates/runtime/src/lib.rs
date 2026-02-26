use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use auth::verify_token;
use chrono::{DateTime, Duration, Utc};
use protocol::{Handshake, Message, PROTOCOL_VERSION};
use serde::Deserialize;
use serde_json::{json, Value};
use state_sync::PatchOp;
use thiserror::Error;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

pub mod data_plane;
pub use data_plane::{
    run_data_plane, TransportAcceptor, TransportPacket, TransportReceiver, TransportReliability,
    TransportSender, TransportSocket,
};

#[derive(Debug, Error)]
pub enum WasmPluginError {
    #[error("failed to compile wasm module: {0}")]
    Compile(String),
    #[error("missing required wasm export '{0}'")]
    MissingExport(&'static str),
    #[error("wasm execution failed: {0}")]
    Execution(String),
    #[error("wasm returned invalid pointer/length")]
    InvalidPointerLength,
    #[error("wasm returned invalid utf-8")]
    InvalidUtf8,
    #[error("wasm returned invalid json")]
    InvalidJson,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WasmRoomPluginOutput {
    pub state: Value,
    pub event: Option<Value>,
}

#[derive(Deserialize)]
struct RawWasmRoomPluginOutput {
    #[serde(default)]
    state: Option<Value>,
    #[serde(default)]
    event: Option<Value>,
}

pub struct WasmRoomPluginRuntime {
    engine: Engine,
    module: Module,
}

impl WasmRoomPluginRuntime {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WasmPluginError> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes)
            .map_err(|error| WasmPluginError::Compile(error.to_string()))?;
        Ok(Self { engine, module })
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, WasmPluginError> {
        let bytes = std::fs::read(path.as_ref())
            .map_err(|error| WasmPluginError::Compile(error.to_string()))?;
        Self::from_bytes(&bytes)
    }

    pub fn initial_state(&self) -> Result<Value, WasmPluginError> {
        let mut store = Store::new(&self.engine, ());
        let instance = Instance::new(&mut store, &self.module, &[])
            .map_err(|error| WasmPluginError::Execution(error.to_string()))?;
        let memory = get_memory(&mut store, &instance)?;
        let get_state = get_typed_func::<(), i64>(&mut store, &instance, "nexis_initial_state")?;
        let encoded = get_state
            .call(&mut store, ())
            .map_err(|error| WasmPluginError::Execution(error.to_string()))?;
        let bytes = read_plugin_buffer(&mut store, &memory, encoded)?;
        serde_json::from_slice::<Value>(&bytes).map_err(|_| WasmPluginError::InvalidJson)
    }

    pub fn on_message(
        &self,
        state: &Value,
        input: &Value,
    ) -> Result<WasmRoomPluginOutput, WasmPluginError> {
        self.invoke_optional_hook("nexis_on_message", state, input)
    }

    pub fn on_create(
        &self,
        state: &Value,
        input: &Value,
    ) -> Result<WasmRoomPluginOutput, WasmPluginError> {
        self.invoke_optional_hook("nexis_on_create", state, input)
    }

    pub fn on_join(
        &self,
        state: &Value,
        input: &Value,
    ) -> Result<WasmRoomPluginOutput, WasmPluginError> {
        self.invoke_optional_hook("nexis_on_join", state, input)
    }

    pub fn on_leave(
        &self,
        state: &Value,
        input: &Value,
    ) -> Result<WasmRoomPluginOutput, WasmPluginError> {
        self.invoke_optional_hook("nexis_on_leave", state, input)
    }

    pub fn on_tick(
        &self,
        state: &Value,
        input: &Value,
    ) -> Result<WasmRoomPluginOutput, WasmPluginError> {
        self.invoke_optional_hook("nexis_on_tick", state, input)
    }

    pub fn on_dispose(
        &self,
        state: &Value,
        input: &Value,
    ) -> Result<WasmRoomPluginOutput, WasmPluginError> {
        self.invoke_optional_hook("nexis_on_dispose", state, input)
    }

    fn invoke_optional_hook(
        &self,
        export_name: &'static str,
        state: &Value,
        input: &Value,
    ) -> Result<WasmRoomPluginOutput, WasmPluginError> {
        let mut store = Store::new(&self.engine, ());
        let instance = Instance::new(&mut store, &self.module, &[])
            .map_err(|error| WasmPluginError::Execution(error.to_string()))?;
        let memory = get_memory(&mut store, &instance)?;

        let alloc = get_typed_func::<i32, i32>(&mut store, &instance, "alloc")?;
        let Some(hook) =
            maybe_get_typed_func::<(i32, i32, i32, i32), i64>(&mut store, &instance, export_name)?
        else {
            return Ok(WasmRoomPluginOutput {
                state: state.clone(),
                event: None,
            });
        };

        let encoded_state = serde_json::to_vec(state).map_err(|_| WasmPluginError::InvalidJson)?;
        let encoded_input = serde_json::to_vec(input).map_err(|_| WasmPluginError::InvalidJson)?;

        let state_ptr = alloc
            .call(&mut store, encoded_state.len() as i32)
            .map_err(|error| WasmPluginError::Execution(error.to_string()))?;
        memory
            .write(&mut store, state_ptr as usize, &encoded_state)
            .map_err(|_| WasmPluginError::InvalidPointerLength)?;

        let input_ptr = alloc
            .call(&mut store, encoded_input.len() as i32)
            .map_err(|error| WasmPluginError::Execution(error.to_string()))?;
        memory
            .write(&mut store, input_ptr as usize, &encoded_input)
            .map_err(|_| WasmPluginError::InvalidPointerLength)?;

        let encoded = hook
            .call(
                &mut store,
                (
                    state_ptr,
                    encoded_state.len() as i32,
                    input_ptr,
                    encoded_input.len() as i32,
                ),
            )
            .map_err(|error| WasmPluginError::Execution(error.to_string()))?;

        let bytes = read_plugin_buffer(&mut store, &memory, encoded)?;
        let payload = serde_json::from_slice::<RawWasmRoomPluginOutput>(&bytes)
            .map_err(|_| WasmPluginError::InvalidJson)?;

        Ok(WasmRoomPluginOutput {
            state: payload.state.unwrap_or_else(|| state.clone()),
            event: payload.event,
        })
    }
}

#[derive(Default, Clone)]
pub struct WasmRoomPlugins {
    plugins: HashMap<String, Arc<WasmRoomPluginRuntime>>,
}

impl WasmRoomPlugins {
    pub fn insert(&mut self, room_type: String, runtime: WasmRoomPluginRuntime) {
        self.plugins.insert(room_type, Arc::new(runtime));
    }

    pub fn get(&self, room_type: &str) -> Option<Arc<WasmRoomPluginRuntime>> {
        self.plugins.get(room_type).cloned()
    }

    pub fn entries(&self) -> Vec<(String, Arc<WasmRoomPluginRuntime>)> {
        self.plugins
            .iter()
            .map(|(room_type, runtime)| (room_type.clone(), Arc::clone(runtime)))
            .collect()
    }
}

fn get_memory(store: &mut Store<()>, instance: &Instance) -> Result<Memory, WasmPluginError> {
    instance
        .get_memory(store, "memory")
        .ok_or(WasmPluginError::MissingExport("memory"))
}

fn get_typed_func<P, R>(
    store: &mut Store<()>,
    instance: &Instance,
    name: &'static str,
) -> Result<TypedFunc<P, R>, WasmPluginError>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    instance
        .get_typed_func::<P, R>(&mut *store, name)
        .map_err(|_| WasmPluginError::MissingExport(name))
}

fn maybe_get_typed_func<P, R>(
    store: &mut Store<()>,
    instance: &Instance,
    name: &'static str,
) -> Result<Option<TypedFunc<P, R>>, WasmPluginError>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    if instance.get_func(&mut *store, name).is_none() {
        return Ok(None);
    }
    get_typed_func::<P, R>(store, instance, name).map(Some)
}

fn unpack_ptr_len(encoded: i64) -> Result<(u32, u32), WasmPluginError> {
    if encoded < 0 {
        return Err(WasmPluginError::InvalidPointerLength);
    }
    let ptr = ((encoded >> 32) & 0xffff_ffff) as u32;
    let len = (encoded & 0xffff_ffff) as u32;
    Ok((ptr, len))
}

fn read_plugin_buffer(
    store: &mut Store<()>,
    memory: &Memory,
    encoded: i64,
) -> Result<Vec<u8>, WasmPluginError> {
    let (ptr, len) = unpack_ptr_len(encoded)?;
    let data = memory.data(store);
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or(WasmPluginError::InvalidPointerLength)?;
    if end > data.len() {
        return Err(WasmPluginError::InvalidPointerLength);
    }
    Ok(data[start..end].to_vec())
}

#[derive(Debug, Error, PartialEq)]
pub enum TransportError {
    #[error("unsupported protocol version")]
    UnsupportedVersion,
    #[error("unsupported codec")]
    UnsupportedCodec,
    #[error("auth failed")]
    Auth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Required,
    Optional,
    Disabled,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMode::Required => "required",
            AuthMode::Optional => "optional",
            AuthMode::Disabled => "disabled",
        }
    }
}

impl Default for AuthMode {
    fn default() -> Self {
        Self::Required
    }
}

impl FromStr for AuthMode {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "required" => Ok(Self::Required),
            "optional" => Ok(Self::Optional),
            "disabled" => Ok(Self::Disabled),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryChannel {
    Reliable,
    Unreliable,
}

#[derive(Debug, Clone)]
pub struct ChannelPolicy {
    unreliable_types: HashSet<String>,
}

impl Default for ChannelPolicy {
    fn default() -> Self {
        Self {
            unreliable_types: HashSet::from([
                "position.update".to_owned(),
                "input.delta".to_owned(),
                "aim.update".to_owned(),
            ]),
        }
    }
}

impl ChannelPolicy {
    pub fn classify(&self, message_type: &str) -> DeliveryChannel {
        if self.unreliable_types.contains(message_type) {
            DeliveryChannel::Unreliable
        } else {
            DeliveryChannel::Reliable
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomMembership {
    pub room_id: String,
    pub room_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub project_id: String,
    pub rooms: Vec<RoomMembership>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    ttl: Duration,
    suspended: HashMap<String, SessionSnapshot>,
}

impl SessionStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            suspended: HashMap::new(),
        }
    }

    pub fn park(
        &mut self,
        session_id: String,
        project_id: String,
        rooms: Vec<RoomMembership>,
        now: DateTime<Utc>,
    ) {
        if rooms.is_empty() {
            self.suspended.remove(&session_id);
            return;
        }

        self.suspended.insert(
            session_id,
            SessionSnapshot {
                project_id,
                rooms,
                expires_at: now + self.ttl,
            },
        );
    }

    pub fn resume(
        &mut self,
        session_id: &str,
        project_id: &str,
        now: DateTime<Utc>,
    ) -> Option<SessionSnapshot> {
        let snapshot = self.suspended.remove(session_id)?;
        if snapshot.project_id != project_id {
            return None;
        }
        if now > snapshot.expires_at {
            return None;
        }
        Some(snapshot)
    }

    pub fn prune_expired(&mut self, now: DateTime<Utc>) {
        self.suspended
            .retain(|_, snapshot| now <= snapshot.expires_at);
    }

    pub fn has_session(&self, session_id: &str) -> bool {
        self.suspended.contains_key(session_id)
    }

    pub fn remove(&mut self, session_id: &str) -> Option<SessionSnapshot> {
        self.suspended.remove(session_id)
    }
}

#[derive(Debug, Default, Clone)]
pub struct RoomSequencer {
    seq_by_room: HashMap<String, u64>,
}

impl RoomSequencer {
    pub fn current(&self, room_id: &str) -> u64 {
        *self.seq_by_room.get(room_id).unwrap_or(&0)
    }

    pub fn advance(&mut self, room_id: &str) -> u64 {
        let next = self.current(room_id).saturating_add(1);
        self.seq_by_room.insert(room_id.to_owned(), next);
        next
    }

    pub fn remove(&mut self, room_id: &str) {
        self.seq_by_room.remove(room_id);
    }
}

#[derive(Debug, Default, Clone)]
pub struct AckTracker {
    latest_ack: HashMap<String, HashMap<String, u64>>,
}

impl AckTracker {
    pub fn ack(&mut self, session_id: &str, room_id: &str, seq: u64) {
        let room_map = self.latest_ack.entry(session_id.to_owned()).or_default();
        let current = room_map.get(room_id).copied().unwrap_or(0);
        if seq > current {
            room_map.insert(room_id.to_owned(), seq);
        }
    }

    pub fn last_acked(&self, session_id: &str, room_id: &str) -> Option<u64> {
        self.latest_ack
            .get(session_id)
            .and_then(|room_map| room_map.get(room_id).copied())
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.latest_ack.remove(session_id);
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotCadence {
    every_patches: u64,
    patch_counts: HashMap<String, u64>,
}

impl Default for SnapshotCadence {
    fn default() -> Self {
        Self::new(20)
    }
}

impl SnapshotCadence {
    pub fn new(every_patches: u64) -> Self {
        Self {
            every_patches: every_patches.max(1),
            patch_counts: HashMap::new(),
        }
    }

    pub fn record_patch(&mut self, room_id: &str) -> bool {
        let counter = self.patch_counts.entry(room_id.to_owned()).or_insert(0);
        *counter = counter.saturating_add(1);

        if *counter >= self.every_patches {
            *counter = 0;
            return true;
        }

        false
    }

    pub fn remove_room(&mut self, room_id: &str) {
        self.patch_counts.remove(room_id);
    }
}

#[derive(Debug, Clone)]
pub struct PatchChecksumCadence {
    every_patches: u64,
    patch_counts: HashMap<String, u64>,
}

impl Default for PatchChecksumCadence {
    fn default() -> Self {
        Self::new(10)
    }
}

impl PatchChecksumCadence {
    pub fn new(every_patches: u64) -> Self {
        Self {
            every_patches: every_patches.max(1),
            patch_counts: HashMap::new(),
        }
    }

    pub fn include_checksum(&mut self, room_id: &str) -> bool {
        let counter = self.patch_counts.entry(room_id.to_owned()).or_insert(0);
        *counter = counter.saturating_add(1);

        if *counter >= self.every_patches {
            *counter = 0;
            return true;
        }

        false
    }

    pub fn remove_room(&mut self, room_id: &str) {
        self.patch_counts.remove(room_id);
    }
}

pub fn ack_requires_resync(
    current_seq: u64,
    ack_seq: u64,
    ack_checksum: Option<&str>,
    current_checksum: &str,
) -> bool {
    if ack_seq > current_seq {
        return true;
    }

    if ack_seq < current_seq {
        return false;
    }

    match ack_checksum {
        Some(checksum) => checksum != current_checksum,
        None => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchmakingOutcome {
    Queued {
        room_type: String,
        size: usize,
        position: usize,
    },
    Matched {
        room_type: String,
        size: usize,
        participants: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchmakingEntry {
    session_id: String,
    room_type: String,
    size: usize,
    enqueued_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct MatchmakingQueue {
    waiting: Vec<MatchmakingEntry>,
    timeout: Duration,
}

impl Default for MatchmakingQueue {
    fn default() -> Self {
        Self::with_timeout(Duration::seconds(30))
    }
}

impl MatchmakingQueue {
    pub fn with_timeout(timeout: Duration) -> Self {
        let normalized_timeout = if timeout < Duration::seconds(1) {
            Duration::seconds(1)
        } else {
            timeout
        };
        Self {
            waiting: Vec::new(),
            timeout: normalized_timeout,
        }
    }

    pub fn enqueue(
        &mut self,
        session_id: &str,
        room_type: &str,
        size: usize,
    ) -> MatchmakingOutcome {
        self.enqueue_at(session_id, room_type, size, Utc::now())
    }

    pub fn enqueue_at(
        &mut self,
        session_id: &str,
        room_type: &str,
        size: usize,
        now: DateTime<Utc>,
    ) -> MatchmakingOutcome {
        let normalized_size = size.clamp(2, 16);
        self.prune_expired(now);
        self.dequeue(session_id);
        self.waiting.push(MatchmakingEntry {
            session_id: session_id.to_owned(),
            room_type: room_type.to_owned(),
            size: normalized_size,
            enqueued_at: now,
        });

        let mut bucket_indexes = Vec::new();
        for (index, entry) in self.waiting.iter().enumerate() {
            if entry.room_type == room_type && entry.size == normalized_size {
                bucket_indexes.push(index);
                if bucket_indexes.len() == normalized_size {
                    break;
                }
            }
        }

        if bucket_indexes.len() == normalized_size {
            let participants = bucket_indexes
                .iter()
                .map(|index| self.waiting[*index].session_id.clone())
                .collect::<Vec<_>>();
            for index in bucket_indexes.iter().rev() {
                self.waiting.remove(*index);
            }

            return MatchmakingOutcome::Matched {
                room_type: room_type.to_owned(),
                size: normalized_size,
                participants,
            };
        }

        let position = self.queue_len(room_type, normalized_size);
        MatchmakingOutcome::Queued {
            room_type: room_type.to_owned(),
            size: normalized_size,
            position,
        }
    }

    pub fn dequeue(&mut self, session_id: &str) -> bool {
        let original_len = self.waiting.len();
        self.waiting.retain(|entry| entry.session_id != session_id);
        original_len != self.waiting.len()
    }

    pub fn remove_session(&mut self, session_id: &str) {
        let _ = self.dequeue(session_id);
    }

    pub fn prune_expired(&mut self, now: DateTime<Utc>) {
        let timeout = self.timeout;
        self.waiting
            .retain(|entry| now.signed_duration_since(entry.enqueued_at) <= timeout);
    }

    pub fn queue_len(&self, room_type: &str, size: usize) -> usize {
        self.waiting
            .iter()
            .filter(|entry| entry.room_type == room_type && entry.size == size)
            .count()
    }
}

pub fn negotiate_codec(offered: &[String], default_codec: &str) -> Result<String, TransportError> {
    if offered.iter().any(|c| c == "msgpack") {
        return Ok("msgpack".to_owned());
    }
    if offered.iter().any(|c| c == default_codec) {
        return Ok(default_codec.to_owned());
    }
    if offered.iter().any(|c| c == "json") {
        return Ok("json".to_owned());
    }
    Err(TransportError::UnsupportedCodec)
}

pub fn validate_handshake(
    handshake: &Handshake,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<String, TransportError> {
    if handshake.v != PROTOCOL_VERSION {
        return Err(TransportError::UnsupportedVersion);
    }
    let codec = negotiate_codec(&handshake.codecs, "msgpack")?;
    verify_token(&handshake.token, &handshake.project_id, secret, now)
        .map_err(|_| TransportError::Auth)?;
    Ok(codec)
}

#[derive(Debug, Clone, PartialEq)]
pub enum FastPathRoute {
    Unhandled,
    Error(String),
    ToSelf(Message),
    ToRoom { room_id: String, message: Message },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeAction {
    Noop,
    SendToSelf(Message),
    SendToRoom {
        room_id: String,
        message: Message,
    },
    SendToMany {
        session_ids: Vec<String>,
        message: Message,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinOrCreateRequest {
    pub room_type: String,
    pub room_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateAckRequest {
    pub room_id: String,
    pub seq: u64,
    pub checksum: Option<String>,
}

pub fn event_message(room: Option<String>, event_type: &str, payload: Value) -> Message {
    Message {
        v: PROTOCOL_VERSION,
        t: event_type.to_owned(),
        rid: None,
        room,
        p: Some(payload),
    }
}

pub fn handshake_ok_message(codec_name: &str, session_id: &str, resumed: bool) -> Message {
    event_message(
        None,
        "handshake.ok",
        json!({
            "codec": codec_name,
            "session_id": session_id,
            "resumed": resumed
        }),
    )
}

pub fn room_members_message(room_id: &str, members: Vec<String>) -> Message {
    event_message(
        Some(room_id.to_owned()),
        "room.members",
        json!({
            "members": members,
            "count": members.len()
        }),
    )
}

pub fn room_joined_message(room_id: &str, client_id: &str) -> Message {
    event_message(
        Some(room_id.to_owned()),
        "room.joined",
        json!({ "client_id": client_id }),
    )
}

pub fn room_left_message(room_id: &str, client_id: &str) -> Message {
    event_message(
        Some(room_id.to_owned()),
        "room.left",
        json!({ "client_id": client_id }),
    )
}

pub fn state_snapshot_message(room_id: &str, seq: u64, checksum: String, state: Value) -> Message {
    event_message(
        Some(room_id.to_owned()),
        "state.snapshot",
        json!({
            "seq": seq,
            "checksum": checksum,
            "state": state
        }),
    )
}

pub fn state_patch_message(
    room_id: &str,
    seq: u64,
    checksum: Option<String>,
    patch: Vec<PatchOp>,
) -> Message {
    let mut payload = serde_json::Map::new();
    payload.insert("seq".to_owned(), json!(seq));
    payload.insert("ops".to_owned(), json!(patch));
    if let Some(checksum) = checksum {
        payload.insert("checksum".to_owned(), Value::String(checksum));
    }

    event_message(
        Some(room_id.to_owned()),
        "state.patch",
        Value::Object(payload),
    )
}

pub fn rpc_response(rid: Option<String>, room: Option<String>, payload: Value) -> Message {
    Message {
        v: PROTOCOL_VERSION,
        t: "rpc.response".to_owned(),
        rid,
        room,
        p: Some(payload),
    }
}

pub fn rpc_ok(inbound: &Message, room: Option<String>) -> Message {
    rpc_response(inbound.rid.clone(), room, json!({ "ok": true }))
}

pub fn rpc_error(inbound: &Message, room: Option<String>, error: impl AsRef<str>) -> Message {
    rpc_response(
        inbound.rid.clone(),
        room,
        json!({ "ok": false, "error": error.as_ref() }),
    )
}

pub fn parse_join_or_create_request(inbound: &Message) -> JoinOrCreateRequest {
    let payload = inbound.p.as_ref();
    let room_type = payload
        .and_then(|value| value.get("roomType"))
        .and_then(Value::as_str)
        .unwrap_or("echo_room")
        .to_owned();
    let room_id = inbound
        .room
        .clone()
        .or_else(|| {
            payload
                .and_then(|value| value.get("roomId"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("{room_type}:default"));
    JoinOrCreateRequest { room_type, room_id }
}

pub fn parse_room_id(inbound: &Message) -> Option<String> {
    inbound
        .room
        .clone()
        .or_else(|| payload_string(&inbound.p, "roomId").map(str::to_owned))
}

pub fn parse_state_ack_request(inbound: &Message) -> Option<StateAckRequest> {
    let room_id = parse_room_id(inbound)?;
    let seq = payload_u64(&inbound.p, "seq")?;
    let checksum = payload_string(&inbound.p, "checksum").map(str::to_owned);
    Some(StateAckRequest {
        room_id,
        seq,
        checksum,
    })
}

pub fn join_or_create_success_actions(
    inbound: &Message,
    room_id: &str,
    session_id: &str,
    joined_recipients: Vec<String>,
    latest_members: Vec<String>,
    snapshot: Message,
) -> Vec<RuntimeAction> {
    vec![
        RuntimeAction::SendToSelf(rpc_response(
            inbound.rid.clone(),
            Some(room_id.to_owned()),
            json!({ "ok": true, "room": room_id }),
        )),
        RuntimeAction::SendToMany {
            session_ids: joined_recipients.clone(),
            message: room_joined_message(room_id, session_id),
        },
        RuntimeAction::SendToMany {
            session_ids: joined_recipients,
            message: room_members_message(room_id, latest_members),
        },
        RuntimeAction::SendToSelf(snapshot),
    ]
}

pub fn room_leave_success_actions(
    inbound: &Message,
    room_id: &str,
    session_id: &str,
    recipients: Vec<String>,
    latest_members: Vec<String>,
) -> Vec<RuntimeAction> {
    vec![
        RuntimeAction::SendToSelf(rpc_ok(inbound, Some(room_id.to_owned()))),
        RuntimeAction::SendToMany {
            session_ids: recipients.clone(),
            message: room_left_message(room_id, session_id),
        },
        RuntimeAction::SendToMany {
            session_ids: recipients,
            message: room_members_message(room_id, latest_members),
        },
    ]
}

pub fn plugin_input_from_inbound(inbound: &Message, session_id: &str) -> Value {
    if inbound.t == "room.message" {
        let msg_type = inbound
            .p
            .as_ref()
            .and_then(|payload| payload.get("type"))
            .cloned()
            .unwrap_or(Value::String("message".to_owned()));
        let msg_type = Value::String(match msg_type {
            Value::String(value) => value,
            other => other.to_string(),
        });
        let data = inbound
            .p
            .as_ref()
            .and_then(|payload| payload.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        json!({
            "type": msg_type,
            "data": data,
            "client_id": session_id
        })
    } else if inbound.t == "room.message.bytes" {
        let msg_type = inbound
            .p
            .as_ref()
            .and_then(|payload| payload.get("type"))
            .cloned()
            .unwrap_or(Value::String("message".to_owned()));
        let msg_type = Value::String(match msg_type {
            Value::String(value) => value,
            other => other.to_string(),
        });
        let data_b64 = inbound
            .p
            .as_ref()
            .and_then(|payload| payload.get("data_b64"))
            .cloned()
            .unwrap_or(Value::String(String::new()));
        json!({
            "type": msg_type,
            "data_b64": data_b64,
            "client_id": session_id
        })
    } else {
        inbound.p.clone().unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomListItem {
    pub id: String,
    pub room_type: String,
    pub members: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchmakingRequest {
    pub room_type: String,
    pub size: usize,
}

pub fn room_list_message(inbound: &Message, rooms: Vec<RoomListItem>) -> Message {
    let payload = json!({
        "ok": true,
        "rooms": rooms.into_iter().map(|room| {
            json!({
                "id": room.id,
                "room_type": room.room_type,
                "members": room.members
            })
        }).collect::<Vec<_>>()
    });

    if inbound.rid.is_some() {
        rpc_response(inbound.rid.clone(), inbound.room.clone(), payload)
    } else {
        event_message(inbound.room.clone(), "room.list", payload)
    }
}

pub fn room_list_action(inbound: &Message, rooms: Vec<RoomListItem>) -> RuntimeAction {
    RuntimeAction::SendToSelf(room_list_message(inbound, rooms))
}

pub fn parse_matchmaking_request(payload: &Option<Value>) -> MatchmakingRequest {
    let room_type = payload
        .as_ref()
        .and_then(|value| value.get("roomType"))
        .and_then(Value::as_str)
        .unwrap_or("counter_room")
        .to_owned();
    let size = payload
        .as_ref()
        .and_then(|value| value.get("size"))
        .and_then(Value::as_u64)
        .map(|value| value.clamp(2, 16) as usize)
        .unwrap_or(2);
    MatchmakingRequest { room_type, size }
}

pub fn matchmaking_queued_message(
    inbound: &Message,
    room_type: &str,
    size: usize,
    position: usize,
) -> Message {
    rpc_response(
        inbound.rid.clone(),
        inbound.room.clone(),
        json!({
            "ok": true,
            "queued": true,
            "room_type": room_type,
            "size": size,
            "position": position
        }),
    )
}

pub fn matchmaking_matched_event(
    match_room_id: &str,
    room_type: &str,
    size: usize,
    participants: &[String],
) -> Message {
    event_message(
        Some(match_room_id.to_owned()),
        "match.found",
        json!({
            "room": match_room_id,
            "room_type": room_type,
            "size": size,
            "participants": participants
        }),
    )
}

pub fn matchmaking_matched_ack(inbound: &Message) -> Message {
    rpc_response(
        inbound.rid.clone(),
        None,
        json!({
            "ok": true,
            "matched": true
        }),
    )
}

pub fn matchmaking_dequeue_message(inbound: &Message, removed: bool) -> Message {
    rpc_response(
        inbound.rid.clone(),
        inbound.room.clone(),
        json!({
            "ok": true,
            "removed": removed
        }),
    )
}

pub fn unknown_message_reply(inbound: &Message) -> Message {
    let payload = json!({ "ok": false, "error": "unknown message type" });
    if inbound.rid.is_some() {
        rpc_response(inbound.rid.clone(), inbound.room.clone(), payload)
    } else {
        event_message(inbound.room.clone(), "error", payload)
    }
}

pub fn unknown_message_action(inbound: &Message) -> RuntimeAction {
    RuntimeAction::SendToSelf(unknown_message_reply(inbound))
}

pub fn route_fast_path(inbound: &Message) -> FastPathRoute {
    match inbound.t.as_str() {
        "echo" => {
            let payload = inbound.p.clone().unwrap_or(Value::Null);
            let message = event_message(inbound.room.clone(), "echo", payload);
            match inbound.room.clone() {
                Some(room_id) => FastPathRoute::ToRoom { room_id, message },
                None => FastPathRoute::ToSelf(message),
            }
        }
        "position.update" => {
            let Some(room_id) = inbound.room.clone() else {
                return FastPathRoute::Error("room id is required".to_owned());
            };
            let payload = inbound.p.clone().unwrap_or(Value::Null);
            FastPathRoute::ToRoom {
                room_id: room_id.clone(),
                message: event_message(Some(room_id), "position.update", payload),
            }
        }
        _ => FastPathRoute::Unhandled,
    }
}

pub fn route_fast_path_action(inbound: &Message) -> RuntimeAction {
    match route_fast_path(inbound) {
        FastPathRoute::Unhandled => RuntimeAction::Noop,
        FastPathRoute::Error(reason) => {
            RuntimeAction::SendToSelf(event_message(None, "error", json!({ "reason": reason })))
        }
        FastPathRoute::ToSelf(message) => RuntimeAction::SendToSelf(message),
        FastPathRoute::ToRoom { room_id, message } => {
            RuntimeAction::SendToRoom { room_id, message }
        }
    }
}

fn payload_u64(payload: &Option<Value>, key: &str) -> Option<u64> {
    payload
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(Value::as_u64)
}

fn payload_string<'a>(payload: &'a Option<Value>, key: &str) -> Option<&'a str> {
    payload
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn channel_policy_classifies_known_fast_paths_as_unreliable() {
        let policy = ChannelPolicy::default();
        assert_eq!(
            policy.classify("position.update"),
            DeliveryChannel::Unreliable
        );
        assert_eq!(policy.classify("rpc.response"), DeliveryChannel::Reliable);
    }

    #[test]
    fn session_store_park_resume_and_prune() {
        let mut store = SessionStore::new(Duration::seconds(30));
        let t0 = Utc.with_ymd_and_hms(2026, 2, 25, 18, 0, 0).unwrap();

        store.park(
            "s-1".to_owned(),
            "p-1".to_owned(),
            vec![RoomMembership {
                room_id: "counter_room:default".to_owned(),
                room_type: "counter_room".to_owned(),
            }],
            t0,
        );
        assert!(store.has_session("s-1"));

        let resumed = store
            .resume("s-1", "p-1", t0 + Duration::seconds(10))
            .expect("session should resume");
        assert_eq!(resumed.rooms.len(), 1);
        assert!(!store.has_session("s-1"));

        store.park(
            "s-2".to_owned(),
            "p-1".to_owned(),
            vec![RoomMembership {
                room_id: "room-a".to_owned(),
                room_type: "echo_room".to_owned(),
            }],
            t0,
        );
        store.prune_expired(t0 + Duration::seconds(31));
        assert!(!store.has_session("s-2"));
    }

    #[test]
    fn session_store_remove_returns_snapshot() {
        let mut store = SessionStore::new(Duration::seconds(30));
        let t0 = Utc::now();
        store.park(
            "s-9".to_owned(),
            "p-9".to_owned(),
            vec![RoomMembership {
                room_id: "room-z".to_owned(),
                room_type: "echo_room".to_owned(),
            }],
            t0,
        );

        let removed = store.remove("s-9").expect("session should be removed");
        assert_eq!(removed.project_id, "p-9");
        assert!(!store.has_session("s-9"));
    }

    #[test]
    fn room_sequencer_advances_monotonically() {
        let mut seq = RoomSequencer::default();
        assert_eq!(seq.current("room-a"), 0);
        assert_eq!(seq.advance("room-a"), 1);
        assert_eq!(seq.advance("room-a"), 2);
        assert_eq!(seq.current("room-a"), 2);
        seq.remove("room-a");
        assert_eq!(seq.current("room-a"), 0);
    }

    #[test]
    fn ack_tracker_keeps_highest_ack() {
        let mut tracker = AckTracker::default();
        tracker.ack("s-1", "room-a", 3);
        tracker.ack("s-1", "room-a", 2);
        tracker.ack("s-1", "room-a", 5);
        assert_eq!(tracker.last_acked("s-1", "room-a"), Some(5));
        tracker.remove_session("s-1");
        assert_eq!(tracker.last_acked("s-1", "room-a"), None);
    }

    #[test]
    fn matchmaking_pairs_players_when_queue_reaches_target_size() {
        let mut queue = MatchmakingQueue::default();
        let first = queue.enqueue("s-1", "counter_room", 2);
        assert_eq!(
            first,
            MatchmakingOutcome::Queued {
                room_type: "counter_room".to_owned(),
                size: 2,
                position: 1
            }
        );

        let second = queue.enqueue("s-2", "counter_room", 2);
        assert_eq!(
            second,
            MatchmakingOutcome::Matched {
                room_type: "counter_room".to_owned(),
                size: 2,
                participants: vec!["s-1".to_owned(), "s-2".to_owned()]
            }
        );
    }

    #[test]
    fn matchmaking_dequeue_removes_session() {
        let mut queue = MatchmakingQueue::default();
        let _ = queue.enqueue("s-1", "echo_room", 3);
        let _ = queue.enqueue("s-2", "echo_room", 3);

        assert!(queue.dequeue("s-2"));
        assert!(!queue.dequeue("s-2"));
        assert_eq!(queue.queue_len("echo_room", 3), 1);
    }

    #[test]
    fn matchmaking_ticket_expires_before_match() {
        let mut queue = MatchmakingQueue::with_timeout(Duration::seconds(5));
        let t0 = Utc.with_ymd_and_hms(2026, 2, 25, 18, 0, 0).unwrap();

        let first = queue.enqueue_at("s-1", "counter_room", 2, t0);
        assert_eq!(
            first,
            MatchmakingOutcome::Queued {
                room_type: "counter_room".to_owned(),
                size: 2,
                position: 1,
            }
        );

        let second = queue.enqueue_at("s-2", "counter_room", 2, t0 + Duration::seconds(6));
        assert_eq!(
            second,
            MatchmakingOutcome::Queued {
                room_type: "counter_room".to_owned(),
                size: 2,
                position: 1,
            }
        );
    }

    #[test]
    fn snapshot_cadence_emits_periodic_snapshot_signal() {
        let mut cadence = SnapshotCadence::new(2);

        assert!(!cadence.record_patch("counter_room:default"));
        assert!(cadence.record_patch("counter_room:default"));
        assert!(!cadence.record_patch("counter_room:default"));
    }

    #[test]
    fn snapshot_cadence_resets_when_room_is_removed() {
        let mut cadence = SnapshotCadence::new(3);
        assert!(!cadence.record_patch("room-a"));
        assert!(!cadence.record_patch("room-a"));

        cadence.remove_room("room-a");
        assert!(!cadence.record_patch("room-a"));
    }

    #[test]
    fn patch_checksum_cadence_emits_periodically() {
        let mut cadence = PatchChecksumCadence::new(3);

        assert!(!cadence.include_checksum("room-a"));
        assert!(!cadence.include_checksum("room-a"));
        assert!(cadence.include_checksum("room-a"));
        assert!(!cadence.include_checksum("room-a"));
    }

    #[test]
    fn patch_checksum_cadence_resets_when_room_is_removed() {
        let mut cadence = PatchChecksumCadence::new(2);

        assert!(!cadence.include_checksum("room-a"));
        cadence.remove_room("room-a");
        assert!(!cadence.include_checksum("room-a"));
    }

    #[test]
    fn ack_checksum_mismatch_requires_resync() {
        assert!(ack_requires_resync(10, 10, Some("expected"), "actual"));
        assert!(!ack_requires_resync(10, 10, Some("actual"), "actual"));
        assert!(!ack_requires_resync(10, 10, None, "actual"));
        assert!(ack_requires_resync(10, 11, Some("actual"), "actual"));
    }

    #[test]
    fn auth_mode_parse_supports_all_values() {
        assert_eq!(
            "required".parse::<AuthMode>().ok(),
            Some(AuthMode::Required)
        );
        assert_eq!(
            "optional".parse::<AuthMode>().ok(),
            Some(AuthMode::Optional)
        );
        assert_eq!(
            "disabled".parse::<AuthMode>().ok(),
            Some(AuthMode::Disabled)
        );
        assert_eq!("invalid".parse::<AuthMode>().ok(), None);
    }

    #[test]
    fn wasm_runtime_reads_initial_state() {
        let runtime =
            WasmRoomPluginRuntime::from_bytes(&test_plugin_wasm()).expect("plugin should compile");
        let state = runtime.initial_state().expect("initial state should parse");

        assert_eq!(state, json!({ "counter": 0 }));
    }

    #[test]
    fn wasm_runtime_on_message_updates_state() {
        let runtime =
            WasmRoomPluginRuntime::from_bytes(&test_plugin_wasm()).expect("plugin should compile");

        let output = runtime
            .on_message(&json!({ "counter": 3 }), &json!({ "by": 2 }))
            .expect("on_message should succeed");

        assert_eq!(output.state, json!({ "counter": 5 }));
        assert_eq!(output.event, Some(json!({ "type": "counter.updated" })));
    }

    #[test]
    fn wasm_runtime_missing_optional_hook_returns_unchanged_state() {
        let runtime =
            WasmRoomPluginRuntime::from_bytes(&test_plugin_wasm()).expect("plugin should compile");
        let original = json!({ "counter": 9 });
        let output = runtime
            .on_tick(&original, &json!({ "dt_ms": 100 }))
            .expect("missing optional hooks should noop");

        assert_eq!(output.state, original);
        assert_eq!(output.event, None);
    }

    #[test]
    fn wasm_runtime_rejects_module_missing_exports() {
        let minimal =
            wat::parse_str("(module (memory (export \"memory\") 1))").expect("wat should compile");
        let runtime =
            WasmRoomPluginRuntime::from_bytes(&minimal).expect("module should still compile");
        let err = runtime
            .initial_state()
            .expect_err("missing exports should fail");
        assert_eq!(
            err.to_string(),
            WasmPluginError::MissingExport("nexis_initial_state").to_string()
        );
    }

    #[test]
    fn rust_example_wasm_plugin_compiles_and_runs_when_enabled() {
        if std::env::var("NEXIS_RUN_EXAMPLE_WASM_TEST").ok().as_deref() != Some("1") {
            return;
        }

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let plugin_dir = manifest_dir.join("../../../examples/wasm-plugins/counter_rust_plugin");
        let status = Command::new("cargo")
            .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
            .current_dir(&plugin_dir)
            .status()
            .expect("cargo build should run for example plugin");
        assert!(
            status.success(),
            "example wasm plugin build failed; ensure wasm target exists via `rustup target add wasm32-unknown-unknown`"
        );

        let wasm_path =
            plugin_dir.join("target/wasm32-unknown-unknown/release/nexis_counter_plugin.wasm");
        let runtime = WasmRoomPluginRuntime::from_file(&wasm_path)
            .expect("example plugin should load from wasm file");
        let state = runtime
            .initial_state()
            .expect("initial state should decode");
        assert_eq!(state, json!({ "counter": 0 }));

        let output = runtime
            .on_message(
                &json!({ "counter": 7 }),
                &json!({ "type": "inc", "data": { "by": 5 } }),
            )
            .expect("plugin execution should succeed");
        assert_eq!(output.state, json!({ "counter": 12 }));
        assert_eq!(
            output.event,
            Some(json!({ "type": "counter.updated", "data": { "by": 5, "counter": 12 } }))
        );
    }

    fn test_plugin_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (global $heap (mut i32) (i32.const 4096))

              (data (i32.const 0) "{\"counter\":0}")
              (data (i32.const 64) "{\"state\":{\"counter\":5},\"event\":{\"type\":\"counter.updated\"}}")

              (func (export "alloc") (param $len i32) (result i32)
                (local $ptr i32)
                global.get $heap
                local.set $ptr
                global.get $heap
                local.get $len
                i32.add
                global.set $heap
                local.get $ptr
              )

              (func (export "nexis_initial_state") (result i64)
                (i64.or
                  (i64.shl (i64.extend_i32_u (i32.const 0)) (i64.const 32))
                  (i64.extend_i32_u (i32.const 13))
                )
              )

              (func (export "nexis_on_message") (param i32 i32 i32 i32) (result i64)
                (i64.or
                  (i64.shl (i64.extend_i32_u (i32.const 64)) (i64.const 32))
                  (i64.extend_i32_u (i32.const 58))
                )
              )
            )
            "#,
        )
        .expect("test wat should compile")
    }

    #[test]
    fn fast_path_echo_without_room_routes_to_self() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "echo".to_owned(),
            rid: Some("r-1".to_owned()),
            room: None,
            p: Some(json!({ "ok": true })),
        };

        let routed = route_fast_path(&inbound);
        match routed {
            FastPathRoute::ToSelf(message) => {
                assert_eq!(message.t, "echo");
                assert_eq!(message.room, None);
            }
            _ => panic!("expected ToSelf"),
        }
    }

    #[test]
    fn fast_path_position_update_requires_room() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "position.update".to_owned(),
            rid: None,
            room: None,
            p: Some(json!({ "x": 1 })),
        };

        let routed = route_fast_path(&inbound);
        assert_eq!(
            routed,
            FastPathRoute::Error("room id is required".to_owned())
        );
    }

    #[test]
    fn fast_path_position_update_routes_to_room() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "position.update".to_owned(),
            rid: None,
            room: Some("counter_room:default".to_owned()),
            p: Some(json!({ "x": 1 })),
        };

        let routed = route_fast_path(&inbound);
        match routed {
            FastPathRoute::ToRoom { room_id, message } => {
                assert_eq!(room_id, "counter_room:default");
                assert_eq!(message.t, "position.update");
                assert_eq!(message.room, Some("counter_room:default".to_owned()));
            }
            _ => panic!("expected ToRoom"),
        }
    }

    #[test]
    fn unknown_message_reply_uses_rpc_response_when_rid_exists() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "unknown".to_owned(),
            rid: Some("r-1".to_owned()),
            room: Some("room-a".to_owned()),
            p: None,
        };
        let reply = unknown_message_reply(&inbound);
        assert_eq!(reply.t, "rpc.response");
        assert_eq!(reply.rid, Some("r-1".to_owned()));
    }

    #[test]
    fn parse_matchmaking_request_applies_defaults() {
        let request = parse_matchmaking_request(&None);
        assert_eq!(
            request,
            MatchmakingRequest {
                room_type: "counter_room".to_owned(),
                size: 2
            }
        );
    }

    #[test]
    fn unknown_message_action_routes_to_self() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "unknown".to_owned(),
            rid: None,
            room: None,
            p: None,
        };
        match unknown_message_action(&inbound) {
            RuntimeAction::SendToSelf(message) => assert_eq!(message.t, "error"),
            _ => panic!("expected SendToSelf"),
        }
    }

    #[test]
    fn route_fast_path_action_unhandled_returns_noop() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "room.join_or_create".to_owned(),
            rid: None,
            room: None,
            p: None,
        };
        assert_eq!(route_fast_path_action(&inbound), RuntimeAction::Noop);
    }

    #[test]
    fn parse_join_or_create_request_defaults_and_format() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "room.join_or_create".to_owned(),
            rid: None,
            room: None,
            p: Some(json!({ "roomType": "my_room" })),
        };

        let request = parse_join_or_create_request(&inbound);
        assert_eq!(request.room_type, "my_room");
        assert_eq!(request.room_id, "my_room:default");
    }

    #[test]
    fn parse_state_ack_request_extracts_fields() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "state.ack".to_owned(),
            rid: None,
            room: Some("room-a".to_owned()),
            p: Some(json!({ "seq": 42, "checksum": "abc" })),
        };
        let parsed = parse_state_ack_request(&inbound).expect("ack should parse");
        assert_eq!(
            parsed,
            StateAckRequest {
                room_id: "room-a".to_owned(),
                seq: 42,
                checksum: Some("abc".to_owned())
            }
        );
    }

    #[test]
    fn plugin_input_from_inbound_normalizes_message_type() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "room.message".to_owned(),
            rid: None,
            room: Some("room-a".to_owned()),
            p: Some(json!({ "type": 123, "data": { "x": 1 } })),
        };
        let input = plugin_input_from_inbound(&inbound, "s-1");
        assert_eq!(input["type"], json!("123"));
        assert_eq!(input["client_id"], json!("s-1"));
    }

    #[test]
    fn join_or_create_success_actions_emit_expected_sequence() {
        let inbound = Message {
            v: PROTOCOL_VERSION,
            t: "room.join_or_create".to_owned(),
            rid: Some("join-1".to_owned()),
            room: Some("counter_room:default".to_owned()),
            p: None,
        };
        let snapshot = state_snapshot_message(
            "counter_room:default",
            7,
            "abc".to_owned(),
            json!({ "counter": 7 }),
        );
        let actions = join_or_create_success_actions(
            &inbound,
            "counter_room:default",
            "s-1",
            vec!["s-1".to_owned()],
            vec!["s-1".to_owned()],
            snapshot,
        );
        assert_eq!(actions.len(), 4);
        assert!(matches!(actions[0], RuntimeAction::SendToSelf(_)));
        assert!(matches!(actions[1], RuntimeAction::SendToMany { .. }));
        assert!(matches!(actions[2], RuntimeAction::SendToMany { .. }));
        assert!(matches!(actions[3], RuntimeAction::SendToSelf(_)));
    }
}

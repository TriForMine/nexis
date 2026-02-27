use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use crate::{
    ack_requires_resync, AckTracker, AuthMode, ChannelPolicy, DeliveryChannel, MatchmakingOutcome,
    MatchmakingQueue, PatchChecksumCadence, RoomListItem, RoomMembership, RoomSequencer,
    RuntimeAction, SessionStore, SnapshotCadence, WasmRoomPluginRuntime, WasmRoomPlugins,
};
use async_trait::async_trait;
use auth::{decode_claims_unverified, derive_project_secret, verify_token, TokenClaims};
use chrono::{Duration, Utc};
use codec::Codec;
use codec_json::JsonCodec;
use codec_msgpack::MsgpackCodec;
use hooks::NoopHooks;
use protocol::{Handshake, Message, DEFAULT_MAX_PAYLOAD_BYTES, PROTOCOL_VERSION};
use reqwest::Client as HttpClient;
use rooms::{RoomManager, RoomTypeRegistry};
use serde::Deserialize;
use serde_json::{json, Value};
use state_sync::{diff, state_checksum, PatchOp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::sleep;

type SharedRooms = Arc<Mutex<RoomManager<NoopHooks>>>;
type SharedSecrets = Arc<HashMap<String, String>>;
type SharedMasterSecret = Arc<String>;
type SharedPeers = Arc<Mutex<HashMap<String, PeerHandle>>>;
type SharedSessions = Arc<Mutex<SessionStore>>;
type SharedSequences = Arc<Mutex<RoomSequencer>>;
type SharedAcks = Arc<Mutex<AckTracker>>;
type SharedChannelPolicy = Arc<ChannelPolicy>;
type SharedMatchmaking = Arc<Mutex<MatchmakingQueue>>;
type SharedSnapshotCadence = Arc<Mutex<SnapshotCadence>>;
type SharedPatchChecksumCadence = Arc<Mutex<PatchChecksumCadence>>;
type SharedKeyStatusVerifier = Arc<Option<KeyStatusVerifier>>;
type SharedAuthMode = Arc<AuthMode>;
type SharedRuntimeMetrics = Arc<RuntimeMetrics>;
type SharedWasmRoomPlugins = Arc<WasmRoomPlugins>;

const UNRELIABLE_QUEUE_CAPACITY: usize = 64;
const RELIABLE_STREAM_ID: u16 = 0;
const UNRELIABLE_STREAM_ID: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportReliability {
    Reliable,
    Unreliable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportPacket {
    pub bytes: Vec<u8>,
    pub reliability: TransportReliability,
    pub stream_id: Option<u16>,
}

impl TransportPacket {
    pub fn new(bytes: Vec<u8>, reliability: TransportReliability, stream_id: Option<u16>) -> Self {
        Self {
            bytes,
            reliability,
            stream_id,
        }
    }

    pub fn reliable(bytes: Vec<u8>) -> Self {
        Self::new(
            bytes,
            TransportReliability::Reliable,
            Some(RELIABLE_STREAM_ID),
        )
    }

    pub fn unreliable(bytes: Vec<u8>) -> Self {
        Self::new(
            bytes,
            TransportReliability::Unreliable,
            Some(UNRELIABLE_STREAM_ID),
        )
    }
}

#[async_trait]
pub trait TransportSender: Send {
    async fn send(&mut self, packet: TransportPacket) -> Result<(), String>;
}

#[async_trait]
pub trait TransportReceiver: Send {
    async fn receive(&mut self) -> Result<Option<TransportPacket>, String>;
}

pub struct TransportSocket {
    pub sender: Box<dyn TransportSender>,
    pub receiver: Box<dyn TransportReceiver>,
}

#[async_trait]
pub trait TransportAcceptor: Send + Sync {
    async fn accept(&self) -> Result<(TransportSocket, String), String>;
}

#[derive(Default)]
struct RuntimeMetrics {
    active_connections: AtomicU64,
    transport_messages_in_total: AtomicU64,
    transport_messages_out_total: AtomicU64,
    rpc_requests_total: AtomicU64,
    state_patches_total: AtomicU64,
    state_resync_total: AtomicU64,
}

struct ActiveConnectionGuard {
    metrics: SharedRuntimeMetrics,
}

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.metrics
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}

fn is_transient_accept_error(error: &str) -> bool {
    error.starts_with("transient:")
}

fn is_benign_send_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("sending after closing is not allowed")
        || normalized.contains("already closed")
        || normalized.contains("connection closed")
        || normalized.contains("broken pipe")
}

#[derive(Clone)]
struct PeerHandle {
    reliable_tx: mpsc::UnboundedSender<Message>,
    unreliable_tx: mpsc::Sender<Message>,
}

#[derive(Clone)]
struct KeyStatusVerifier {
    control_api_url: String,
    internal_token: Option<String>,
    require_key_id: bool,
    required_scope: String,
    client: HttpClient,
}

#[derive(Debug, Deserialize)]
struct KeyStatusResponse {
    exists: bool,
    revoked: bool,
    scopes: Vec<String>,
}

pub async fn run_data_plane(acceptor: Arc<dyn TransportAcceptor>) -> Result<(), String> {
    let mut room_types = RoomTypeRegistry::default();
    let wasm_room_plugins = Arc::new(load_wasm_room_plugins());
    for (room_type, runtime) in wasm_room_plugins.entries() {
        let room_type_for_log = room_type.clone();
        let runtime_for_factory = Arc::clone(&runtime);
        let register = room_types.register_room_plugin(&room_type, move || {
            runtime_for_factory.initial_state().unwrap_or_else(|error| {
                eprintln!("wasm room plugin '{room_type_for_log}' init failed: {error}");
                json!({})
            })
        });
        if let Err(error) = register {
            eprintln!("failed to register wasm room plugin '{room_type}': {error}");
        }
    }
    register_configured_room_plugins(&mut room_types);
    let rooms = Arc::new(Mutex::new(RoomManager::with_registry(
        NoopHooks, room_types,
    )));
    let peers = Arc::new(Mutex::new(HashMap::<String, PeerHandle>::new()));
    let sessions = Arc::new(Mutex::new(SessionStore::new(load_session_ttl())));
    let sequences = Arc::new(Mutex::new(RoomSequencer::default()));
    let acks = Arc::new(Mutex::new(AckTracker::default()));
    let channel_policy = Arc::new(ChannelPolicy::default());
    let matchmaking = Arc::new(Mutex::new(MatchmakingQueue::with_timeout(
        load_matchmaking_ticket_ttl(),
    )));
    let snapshot_cadence = Arc::new(Mutex::new(SnapshotCadence::new(
        load_snapshot_every_patches(),
    )));
    let patch_checksum_cadence = Arc::new(Mutex::new(PatchChecksumCadence::new(
        load_patch_checksum_every_patches(),
    )));
    let key_status_verifier = Arc::new(load_key_status_verifier());
    let secrets = Arc::new(load_project_secrets());
    let master_secret = Arc::new(load_master_secret());
    let auth_mode = Arc::new(load_auth_mode());
    let metrics = Arc::new(RuntimeMetrics::default());
    let metrics_bind = load_metrics_bind();
    let tick_interval = load_room_tick_interval();
    let tick_interval_chrono = Duration::milliseconds(tick_interval.as_millis() as i64);

    {
        let rooms = Arc::clone(&rooms);
        let peers = Arc::clone(&peers);
        let channel_policy = Arc::clone(&channel_policy);
        let sequences = Arc::clone(&sequences);
        let snapshot_cadence = Arc::clone(&snapshot_cadence);
        let patch_checksum_cadence = Arc::clone(&patch_checksum_cadence);
        let metrics = Arc::clone(&metrics);
        let wasm_room_plugins = Arc::clone(&wasm_room_plugins);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tick_interval);
            loop {
                ticker.tick().await;
                let now = Utc::now();
                let ticked = rooms.lock().await.tick(now, tick_interval_chrono);
                for room_id in ticked {
                    let (room_type, state) = {
                        let manager = rooms.lock().await;
                        let Some(room) = manager.room(&room_id) else {
                            continue;
                        };
                        (room.room_type.clone(), room.state.clone())
                    };
                    let Some(plugin) = wasm_room_plugins.get(&room_type) else {
                        continue;
                    };
                    let output = match plugin.on_tick(&state, &json!({ "ts": now.to_rfc3339() })) {
                        Ok(output) => output,
                        Err(error) => {
                            eprintln!("wasm plugin on_tick failed for room '{room_id}': {error}");
                            continue;
                        }
                    };
                    let _ = apply_plugin_room_update(
                        &rooms,
                        &peers,
                        channel_policy.as_ref(),
                        &sequences,
                        &snapshot_cadence,
                        &patch_checksum_cadence,
                        &metrics,
                        &room_id,
                        output.state,
                        output.event,
                    )
                    .await;
                }
            }
        });
    }

    {
        let metrics = Arc::clone(&metrics);
        let rooms = Arc::clone(&rooms);
        tokio::spawn(async move {
            run_metrics_server(metrics_bind, metrics, rooms).await;
        });
    }

    println!("nexis data-plane runtime started");

    loop {
        let (transport, peer_addr) = match acceptor.accept().await {
            Ok(pair) => pair,
            Err(error) => {
                if !is_transient_accept_error(&error) {
                    eprintln!("accept error: {error}");
                }
                continue;
            }
        };

        let rooms = Arc::clone(&rooms);
        let peers = Arc::clone(&peers);
        let sessions = Arc::clone(&sessions);
        let sequences = Arc::clone(&sequences);
        let acks = Arc::clone(&acks);
        let channel_policy = Arc::clone(&channel_policy);
        let matchmaking = Arc::clone(&matchmaking);
        let snapshot_cadence = Arc::clone(&snapshot_cadence);
        let patch_checksum_cadence = Arc::clone(&patch_checksum_cadence);
        let key_status_verifier = Arc::clone(&key_status_verifier);
        let auth_mode = Arc::clone(&auth_mode);
        let metrics = Arc::clone(&metrics);
        let secrets = Arc::clone(&secrets);
        let master_secret = Arc::clone(&master_secret);
        let wasm_room_plugins = Arc::clone(&wasm_room_plugins);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(
                transport,
                rooms,
                peers,
                sessions,
                sequences,
                acks,
                channel_policy,
                matchmaking,
                snapshot_cadence,
                patch_checksum_cadence,
                key_status_verifier,
                auth_mode,
                metrics,
                secrets,
                master_secret,
                wasm_room_plugins,
            )
            .await
            {
                eprintln!("connection {peer_addr} failed: {error}");
            }
        });
    }

    #[allow(unreachable_code)]
    Ok(())
}

fn load_auth_mode() -> AuthMode {
    env::var("NEXIS_AUTH_MODE")
        .ok()
        .and_then(|raw| raw.parse::<AuthMode>().ok())
        .unwrap_or_default()
}

fn load_project_secrets() -> HashMap<String, String> {
    if let Ok(raw) = env::var("NEXIS_PROJECT_SECRETS") {
        if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&raw) {
            if !map.is_empty() {
                return map;
            }
        }
    }

    let project_id =
        env::var("NEXIS_DEMO_PROJECT_ID").unwrap_or_else(|_| "demo-project".to_owned());
    let project_secret =
        env::var("NEXIS_DEMO_PROJECT_SECRET").unwrap_or_else(|_| "demo-secret".to_owned());

    HashMap::from([(project_id, project_secret)])
}

fn register_configured_room_plugins(registry: &mut RoomTypeRegistry) {
    let Some(raw) = env::var("NEXIS_ROOM_TYPE_PLUGINS").ok() else {
        return;
    };
    if raw.trim().is_empty() {
        return;
    }

    let configured = match serde_json::from_str::<HashMap<String, Value>>(&raw) {
        Ok(configured) => configured,
        Err(error) => {
            eprintln!("failed to parse NEXIS_ROOM_TYPE_PLUGINS: {error}");
            return;
        }
    };

    for (room_type, template_state) in configured {
        let room_type_name = room_type.clone();
        if let Err(error) =
            registry.register_room_plugin(&room_type, move || template_state.clone())
        {
            eprintln!("failed to register room plugin '{room_type_name}': {error}");
        }
    }
}

fn load_master_secret() -> String {
    match env::var("NEXIS_MASTER_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            eprintln!(
                "[nexis] WARNING: NEXIS_MASTER_SECRET is not set. \
                Using insecure dev default. Set this env var before deploying to production."
            );
            "nexis-dev-master-secret".to_owned()
        }
    }
}

fn load_wasm_room_plugins() -> WasmRoomPlugins {
    let mut plugins = WasmRoomPlugins::default();
    let Some(raw) = env::var("NEXIS_WASM_ROOM_PLUGINS").ok() else {
        return plugins;
    };
    if raw.trim().is_empty() {
        return plugins;
    }

    let configured = match serde_json::from_str::<HashMap<String, String>>(&raw) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("failed to parse NEXIS_WASM_ROOM_PLUGINS: {error}");
            return plugins;
        }
    };

    for (room_type, path) in configured {
        match WasmRoomPluginRuntime::from_file(&path) {
            Ok(runtime) => plugins.insert(room_type, runtime),
            Err(error) => eprintln!("failed to load wasm room plugin '{path}': {error}"),
        }
    }

    plugins
}

fn load_session_ttl() -> Duration {
    let ttl_seconds = env::var("NEXIS_SESSION_TTL_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(30);
    Duration::seconds(ttl_seconds.max(5))
}

fn load_matchmaking_ticket_ttl() -> Duration {
    let ttl_seconds = env::var("NEXIS_MATCHMAKING_TICKET_TTL_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(30);
    Duration::seconds(ttl_seconds.max(5))
}

fn load_room_tick_interval() -> StdDuration {
    let interval_ms = env::var("NEXIS_ROOM_TICK_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(100);
    StdDuration::from_millis(interval_ms.clamp(16, 60_000))
}

fn load_snapshot_every_patches() -> u64 {
    env::var("NEXIS_STATE_SNAPSHOT_EVERY_PATCHES")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(|value| value.clamp(1, 1_000))
        .unwrap_or(20)
}

fn load_patch_checksum_every_patches() -> u64 {
    env::var("NEXIS_STATE_PATCH_CHECKSUM_EVERY_PATCHES")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(|value| value.clamp(1, 1_000))
        .unwrap_or(10)
}

fn load_metrics_bind() -> String {
    env::var("NEXIS_METRICS_BIND").unwrap_or_else(|_| "0.0.0.0:9100".to_owned())
}

fn render_metrics(metrics: &RuntimeMetrics, room_count: usize) -> String {
    format!(
        concat!(
            "# HELP nexis_active_connections Active transport connections.\n",
            "# TYPE nexis_active_connections gauge\n",
            "nexis_active_connections {}\n",
            "# HELP nexis_room_count Active room count.\n",
            "# TYPE nexis_room_count gauge\n",
            "nexis_room_count {}\n",
            "# HELP nexis_transport_messages_in_total Incoming transport packets.\n",
            "# TYPE nexis_transport_messages_in_total counter\n",
            "nexis_transport_messages_in_total {}\n",
            "# HELP nexis_transport_messages_out_total Outgoing transport packets.\n",
            "# TYPE nexis_transport_messages_out_total counter\n",
            "nexis_transport_messages_out_total {}\n",
            "# HELP nexis_rpc_requests_total Incoming rpc request envelopes.\n",
            "# TYPE nexis_rpc_requests_total counter\n",
            "nexis_rpc_requests_total {}\n",
            "# HELP nexis_state_patches_total State patch messages produced.\n",
            "# TYPE nexis_state_patches_total counter\n",
            "nexis_state_patches_total {}\n",
            "# HELP nexis_state_resync_total State resync requests received.\n",
            "# TYPE nexis_state_resync_total counter\n",
            "nexis_state_resync_total {}\n",
        ),
        metrics.active_connections.load(Ordering::Relaxed),
        room_count,
        metrics.transport_messages_in_total.load(Ordering::Relaxed),
        metrics.transport_messages_out_total.load(Ordering::Relaxed),
        metrics.rpc_requests_total.load(Ordering::Relaxed),
        metrics.state_patches_total.load(Ordering::Relaxed),
        metrics.state_resync_total.load(Ordering::Relaxed),
    )
}

async fn handle_metrics_connection(
    mut stream: TcpStream,
    metrics: SharedRuntimeMetrics,
    rooms: SharedRooms,
) {
    let mut buffer = [0_u8; 2048];
    let read = match stream.read(&mut buffer).await {
        Ok(n) => n,
        Err(_) => return,
    };
    if read == 0 {
        return;
    }

    let request = String::from_utf8_lossy(&buffer[..read]);
    let first_line = request.lines().next().unwrap_or_default();
    let (status_line, body, content_type) = if first_line.starts_with("GET /metrics") {
        let room_count = rooms.lock().await.room_count();
        (
            "HTTP/1.1 200 OK\r\n",
            render_metrics(metrics.as_ref(), room_count),
            "text/plain; version=0.0.4; charset=utf-8",
        )
    } else if first_line.starts_with("GET /health") {
        (
            "HTTP/1.1 200 OK\r\n",
            "ok\n".to_owned(),
            "text/plain; charset=utf-8",
        )
    } else {
        (
            "HTTP/1.1 404 Not Found\r\n",
            "not found\n".to_owned(),
            "text/plain; charset=utf-8",
        )
    };

    let response = format!(
        "{status}Content-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.as_bytes().len(),
        status = status_line
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

async fn run_metrics_server(bind: String, metrics: SharedRuntimeMetrics, rooms: SharedRooms) {
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("failed to bind metrics server on {bind}: {error}");
            return;
        }
    };
    println!("nexis metrics listening on http://{bind}/metrics");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(error) => {
                eprintln!("metrics accept error: {error}");
                continue;
            }
        };
        let metrics = Arc::clone(&metrics);
        let rooms = Arc::clone(&rooms);
        tokio::spawn(async move {
            handle_metrics_connection(stream, metrics, rooms).await;
        });
    }
}

fn load_bool_env(key: &str, default_value: bool) -> bool {
    match env::var(key) {
        Ok(raw) => matches!(
            raw.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default_value,
    }
}

fn load_key_status_verifier() -> Option<KeyStatusVerifier> {
    let control_api_url = env::var("NEXIS_CONTROL_API_URL").ok()?;
    let normalized_url = control_api_url.trim_end_matches('/').to_owned();
    if normalized_url.is_empty() {
        return None;
    }

    let internal_token = env::var("NEXIS_INTERNAL_TOKEN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let require_key_id = load_bool_env("NEXIS_REQUIRE_KEY_ID", true);
    let required_scope = env::var("NEXIS_REQUIRED_KEY_SCOPE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "token:mint".to_owned());

    Some(KeyStatusVerifier {
        control_api_url: normalized_url,
        internal_token,
        require_key_id,
        required_scope,
        client: HttpClient::new(),
    })
}

async fn enforce_key_status(
    verifier: &KeyStatusVerifier,
    claims: &TokenClaims,
) -> Result<(), String> {
    let Some(key_id) = claims.key_id.as_deref() else {
        if verifier.require_key_id {
            return Err("token missing key_id claim".to_owned());
        }
        return Ok(());
    };

    let endpoint = format!("{}/internal/key-status", verifier.control_api_url);
    let mut request = verifier.client.get(endpoint).query(&[
        ("project_id", claims.project_id.as_str()),
        ("key_id", key_id),
    ]);
    if let Some(token) = verifier.internal_token.as_deref() {
        request = request.header("x-nexis-internal-token", token);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("key status request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "key status request rejected: {}",
            response.status()
        ));
    }

    let status = response
        .json::<KeyStatusResponse>()
        .await
        .map_err(|error| format!("invalid key status payload: {error}"))?;

    if !status.exists {
        return Err(format!("unknown key id '{key_id}'"));
    }
    if status.revoked {
        return Err(format!("key '{key_id}' revoked"));
    }
    if !status
        .scopes
        .iter()
        .any(|scope| scope == &verifier.required_scope)
    {
        return Err(format!(
            "key '{key_id}' missing required scope '{}'",
            verifier.required_scope
        ));
    }

    Ok(())
}

fn codec_from_name(name: &str) -> Box<dyn Codec> {
    match name {
        "msgpack" => Box::<MsgpackCodec>::default(),
        _ => Box::<JsonCodec>::default(),
    }
}

fn encode_transport_payload(codec_name: &str, codec: &dyn Codec, envelope: &Message) -> Vec<u8> {
    let bytes = codec.encode(envelope);
    if codec_name == "json" {
        match String::from_utf8(bytes) {
            Ok(text) => text.into_bytes(),
            Err(_) => b"{}".to_vec(),
        }
    } else {
        bytes
    }
}

fn event_message(room: Option<String>, event_type: &str, payload: Value) -> Message {
    crate::event_message(room, event_type, payload)
}

fn handshake_ok_message(codec_name: &str, session_id: &str, resumed: bool) -> Message {
    crate::handshake_ok_message(codec_name, session_id, resumed)
}

fn room_members_message(room_id: &str, members: Vec<String>) -> Message {
    crate::room_members_message(room_id, members)
}

fn room_joined_message(room_id: &str, client_id: &str) -> Message {
    crate::room_joined_message(room_id, client_id)
}

fn room_left_message(room_id: &str, client_id: &str) -> Message {
    crate::room_left_message(room_id, client_id)
}

fn state_snapshot_message(room_id: &str, seq: u64, checksum: String, state: Value) -> Message {
    crate::state_snapshot_message(room_id, seq, checksum, state)
}

fn state_patch_message(
    room_id: &str,
    seq: u64,
    checksum: Option<String>,
    patch: Vec<PatchOp>,
) -> Message {
    crate::state_patch_message(room_id, seq, checksum, patch)
}

async fn writer_loop(
    mut sink: Box<dyn TransportSender>,
    codec_name: String,
    codec: Box<dyn Codec>,
    metrics: SharedRuntimeMetrics,
    mut reliable_rx: mpsc::UnboundedReceiver<Message>,
    mut unreliable_rx: mpsc::Receiver<Message>,
) -> Result<(), String> {
    async fn send_message(
        sink: &mut dyn TransportSender,
        codec_name: &str,
        codec: &dyn Codec,
        metrics: &SharedRuntimeMetrics,
        message: Message,
        channel: DeliveryChannel,
    ) -> Result<(), String> {
        let payload = encode_transport_payload(codec_name, codec, &message);
        let packet = match channel {
            DeliveryChannel::Reliable => TransportPacket::reliable(payload),
            DeliveryChannel::Unreliable => TransportPacket::unreliable(payload),
        };
        sink.send(packet).await?;
        metrics
            .transport_messages_out_total
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    loop {
        tokio::select! {
            biased;
            Some(message) = reliable_rx.recv() => {
                if let Err(error) = send_message(
                    sink.as_mut(),
                    &codec_name,
                    codec.as_ref(),
                    &metrics,
                    message,
                    DeliveryChannel::Reliable,
                ).await {
                    if is_benign_send_error(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
            Some(message) = unreliable_rx.recv() => {
                if let Err(error) = send_message(
                    sink.as_mut(),
                    &codec_name,
                    codec.as_ref(),
                    &metrics,
                    message,
                    DeliveryChannel::Unreliable,
                ).await {
                    if is_benign_send_error(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
            else => break,
        }
    }
    Ok(())
}

fn queue_with_policy(peer: &PeerHandle, policy: &ChannelPolicy, message: Message) {
    match policy.classify(&message.t) {
        DeliveryChannel::Reliable => {
            let _ = peer.reliable_tx.send(message);
        }
        DeliveryChannel::Unreliable => {
            let _ = peer.unreliable_tx.try_send(message);
        }
    }
}

async fn send_to_peer(
    peers: &SharedPeers,
    policy: &ChannelPolicy,
    session_id: &str,
    message: Message,
) {
    let peer = peers.lock().await.get(session_id).cloned();
    if let Some(peer) = peer {
        queue_with_policy(&peer, policy, message);
    }
}

async fn send_to_many(
    peers: &SharedPeers,
    policy: &ChannelPolicy,
    recipients: Vec<String>,
    message: Message,
) {
    for recipient in recipients {
        send_to_peer(peers, policy, &recipient, message.clone()).await;
    }
}

async fn execute_runtime_action(
    action: RuntimeAction,
    peers: &SharedPeers,
    policy: &ChannelPolicy,
    rooms: &SharedRooms,
    session_id: &str,
) {
    match action {
        RuntimeAction::Noop => {}
        RuntimeAction::SendToSelf(message) => {
            send_to_peer(peers, policy, session_id, message).await;
        }
        RuntimeAction::SendToRoom { room_id, message } => {
            let recipients = {
                let mut manager = rooms.lock().await;
                let _ = manager.mark_activity(&room_id, Utc::now());
                manager.room_members(&room_id).unwrap_or_default()
            };
            send_to_many(peers, policy, recipients, message).await;
        }
        RuntimeAction::SendToMany {
            session_ids,
            message,
        } => {
            send_to_many(peers, policy, session_ids, message).await;
        }
    }
}

async fn execute_runtime_actions(
    actions: Vec<RuntimeAction>,
    peers: &SharedPeers,
    policy: &ChannelPolicy,
    rooms: &SharedRooms,
    session_id: &str,
) {
    for action in actions {
        execute_runtime_action(action, peers, policy, rooms, session_id).await;
    }
}

async fn apply_plugin_room_update(
    rooms: &SharedRooms,
    peers: &SharedPeers,
    channel_policy: &ChannelPolicy,
    sequences: &SharedSequences,
    snapshot_cadence: &SharedSnapshotCadence,
    patch_checksum_cadence: &SharedPatchChecksumCadence,
    metrics: &SharedRuntimeMetrics,
    room_id: &str,
    next_state: Value,
    event_payload: Option<Value>,
) -> bool {
    let mut manager = rooms.lock().await;
    let Some(room) = manager.room_mut(room_id) else {
        return false;
    };
    let previous_state = room.state.clone();
    room.state = next_state.clone();
    room.last_activity_at = Utc::now();
    let patch = diff(&previous_state, &room.state);
    let members = manager.room_members(room_id).unwrap_or_default();
    drop(manager);

    if !patch.is_empty() {
        let seq = sequences.lock().await.advance(room_id);
        let include_checksum = patch_checksum_cadence
            .lock()
            .await
            .include_checksum(room_id);
        let checksum = state_checksum(&next_state);
        let patch_checksum = if include_checksum {
            Some(checksum.clone())
        } else {
            None
        };
        let should_snapshot = snapshot_cadence.lock().await.record_patch(room_id);
        send_to_many(
            peers,
            channel_policy,
            members.clone(),
            state_patch_message(room_id, seq, patch_checksum, patch),
        )
        .await;
        metrics.state_patches_total.fetch_add(1, Ordering::Relaxed);
        if should_snapshot {
            send_to_many(
                peers,
                channel_policy,
                members.clone(),
                state_snapshot_message(room_id, seq, checksum, next_state),
            )
            .await;
        }
    }

    if let Some(event_payload) = event_payload {
        send_to_many(
            peers,
            channel_policy,
            members,
            event_message(Some(room_id.to_owned()), "room.message", event_payload),
        )
        .await;
    }

    true
}

async fn finalize_disconnected_session(
    session_id: &str,
    memberships: Vec<RoomMembership>,
    rooms: &SharedRooms,
    peers: &SharedPeers,
    channel_policy: &ChannelPolicy,
    sequences: &SharedSequences,
    snapshot_cadence: &SharedSnapshotCadence,
    patch_checksum_cadence: &SharedPatchChecksumCadence,
    metrics: &SharedRuntimeMetrics,
    wasm_room_plugins: &SharedWasmRoomPlugins,
) {
    for membership in memberships {
        let room_id = membership.room_id;
        let room_type = membership.room_type;
        let state_before_leave = {
            let manager = rooms.lock().await;
            manager
                .room(&room_id)
                .map(|room| room.state.clone())
                .unwrap_or(Value::Null)
        };

        if let Some(plugin) = wasm_room_plugins.get(&room_type) {
            if let Ok(output) = plugin.on_leave(
                &state_before_leave,
                &json!({ "client_id": session_id, "code": 1001 }),
            ) {
                let _ = apply_plugin_room_update(
                    rooms,
                    peers,
                    channel_policy,
                    sequences,
                    snapshot_cadence,
                    patch_checksum_cadence,
                    metrics,
                    &room_id,
                    output.state,
                    output.event,
                )
                .await;
            }
        }

        let mut manager = rooms.lock().await;
        let members_before_leave = manager.room_members(&room_id).unwrap_or_default();
        let leave_result = manager.leave(&room_id, session_id);
        let members_after = manager.room_members(&room_id).unwrap_or_default();
        let room_exists = manager.room(&room_id).is_some();
        let state_after_leave = manager
            .room(&room_id)
            .map(|room| room.state.clone())
            .unwrap_or(Value::Null);
        drop(manager);

        if leave_result.is_err() {
            continue;
        }

        if let Some(plugin) = wasm_room_plugins.get(&room_type) {
            if members_before_leave.len() <= 1 {
                if let Ok(output) = plugin.on_dispose(
                    &state_after_leave,
                    &json!({ "reason": "disconnect_timeout" }),
                ) {
                    let _ = apply_plugin_room_update(
                        rooms,
                        peers,
                        channel_policy,
                        sequences,
                        snapshot_cadence,
                        patch_checksum_cadence,
                        metrics,
                        &room_id,
                        output.state,
                        output.event,
                    )
                    .await;
                }
            }
        }

        if !room_exists {
            sequences.lock().await.remove(&room_id);
            snapshot_cadence.lock().await.remove_room(&room_id);
            patch_checksum_cadence.lock().await.remove_room(&room_id);
        }

        send_to_many(
            peers,
            channel_policy,
            members_after.clone(),
            room_left_message(&room_id, session_id),
        )
        .await;
        send_to_many(
            peers,
            channel_policy,
            members_after,
            room_members_message(
                &room_id,
                rooms
                    .lock()
                    .await
                    .room_members(&room_id)
                    .unwrap_or_default(),
            ),
        )
        .await;
    }
}

async fn handle_connection(
    transport: TransportSocket,
    rooms: SharedRooms,
    peers: SharedPeers,
    sessions: SharedSessions,
    sequences: SharedSequences,
    acks: SharedAcks,
    channel_policy: SharedChannelPolicy,
    matchmaking: SharedMatchmaking,
    snapshot_cadence: SharedSnapshotCadence,
    patch_checksum_cadence: SharedPatchChecksumCadence,
    key_status_verifier: SharedKeyStatusVerifier,
    auth_mode: SharedAuthMode,
    metrics: SharedRuntimeMetrics,
    secrets: SharedSecrets,
    master_secret: SharedMasterSecret,
    wasm_room_plugins: SharedWasmRoomPlugins,
) -> Result<(), String> {
    metrics.active_connections.fetch_add(1, Ordering::Relaxed);
    let _active_connection_guard = ActiveConnectionGuard {
        metrics: Arc::clone(&metrics),
    };
    let TransportSocket {
        sender: sink,
        mut receiver,
    } = transport;

    let handshake_packet = receiver
        .receive()
        .await?
        .ok_or_else(|| "missing handshake message".to_owned())?;
    let handshake: Handshake = serde_json::from_slice(&handshake_packet.bytes)
        .map_err(|error| format!("invalid handshake json: {error}"))?;

    if handshake.v != PROTOCOL_VERSION {
        return Err("unsupported handshake protocol version".to_owned());
    }

    let trimmed_token = handshake.token.trim();
    let trimmed_project = handshake.project_id.trim();
    let effective_project = if trimmed_project.is_empty() {
        "anonymous".to_owned()
    } else {
        trimmed_project.to_owned()
    };

    let mut verified_project_id = effective_project.clone();
    let mut verified_claims: Option<TokenClaims> = None;
    match *auth_mode {
        AuthMode::Required => {
            if trimmed_token.is_empty() {
                return Err("token verification failed: missing token".to_owned());
            }
            verified_project_id = decode_claims_unverified(trimmed_token)
                .map(|claims| claims.project_id)
                .unwrap_or_else(|_| effective_project.clone());

            let secret = secrets
                .get(&verified_project_id)
                .cloned()
                .unwrap_or_else(|| {
                    derive_project_secret(master_secret.as_str(), &verified_project_id)
                });
            let claims = verify_token(trimmed_token, &verified_project_id, &secret, Utc::now())
                .map_err(|error| format!("token verification failed: {error}"))?;
            verified_claims = Some(claims);
        }
        AuthMode::Optional => {
            if !trimmed_token.is_empty() {
                verified_project_id = decode_claims_unverified(trimmed_token)
                    .map(|claims| claims.project_id)
                    .unwrap_or_else(|_| effective_project.clone());

                let secret = secrets
                    .get(&verified_project_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        derive_project_secret(master_secret.as_str(), &verified_project_id)
                    });
                let claims = verify_token(trimmed_token, &verified_project_id, &secret, Utc::now())
                    .map_err(|error| format!("token verification failed: {error}"))?;
                verified_claims = Some(claims);
            }
        }
        AuthMode::Disabled => {}
    }

    if let (Some(verifier), Some(claims)) = (key_status_verifier.as_ref(), verified_claims.as_ref())
    {
        enforce_key_status(verifier, claims)
            .await
            .map_err(|error| format!("token key status failed: {error}"))?;
    }

    let negotiated_codec_name = crate::negotiate_codec(&handshake.codecs, "msgpack")
        .map_err(|error| format!("codec negotiation failed: {error}"))?;
    let decode_codec = codec_from_name(&negotiated_codec_name);
    let encode_codec = codec_from_name(&negotiated_codec_name);

    let now = Utc::now();
    let resumed_snapshot = {
        let mut sessions = sessions.lock().await;
        sessions.prune_expired(now);
        handshake
            .session_id
            .as_ref()
            .and_then(|session_id| sessions.resume(session_id, &verified_project_id, now))
    };
    let resumed = resumed_snapshot.is_some();
    let session_id = if resumed {
        handshake
            .session_id
            .clone()
            .unwrap_or_else(|| format!("s-{:016x}", rand::random::<u64>()))
    } else {
        format!("s-{:016x}", rand::random::<u64>())
    };

    let (reliable_tx, reliable_rx) = mpsc::unbounded_channel::<Message>();
    let (unreliable_tx, unreliable_rx) = mpsc::channel::<Message>(UNRELIABLE_QUEUE_CAPACITY);
    let peer = PeerHandle {
        reliable_tx,
        unreliable_tx,
    };
    peers.lock().await.insert(session_id.clone(), peer.clone());

    let writer = tokio::spawn(writer_loop(
        sink,
        negotiated_codec_name.clone(),
        encode_codec,
        Arc::clone(&metrics),
        reliable_rx,
        unreliable_rx,
    ));

    queue_with_policy(&peer, channel_policy.as_ref(), {
        let mut message = handshake_ok_message(&negotiated_codec_name, &session_id, resumed);
        if let Some(payload) = message.p.as_mut().and_then(Value::as_object_mut) {
            payload.insert(
                "auth_mode".to_owned(),
                Value::String(auth_mode.as_str().to_owned()),
            );
            payload.insert(
                "authenticated".to_owned(),
                Value::Bool(verified_claims.is_some()),
            );
            payload.insert(
                "project_id".to_owned(),
                Value::String(verified_project_id.clone()),
            );
        }
        message
    });

    let mut joined_rooms = HashMap::<String, String>::new();

    if let Some(snapshot) = resumed_snapshot {
        for membership in snapshot.rooms {
            let mut manager = rooms.lock().await;
            let was_member = manager
                .room(&membership.room_id)
                .map(|room| room.clients.contains(&session_id))
                .unwrap_or(false);
            let join = if was_member {
                Ok(())
            } else {
                manager.join_or_create(&membership.room_id, &membership.room_type, &session_id)
            };
            let members = manager
                .room_members(&membership.room_id)
                .unwrap_or_default();
            let state = manager
                .room(&membership.room_id)
                .map(|room| room.state.clone());
            drop(manager);

            if join.is_err() {
                continue;
            }
            joined_rooms.insert(membership.room_id.clone(), membership.room_type.clone());

            if was_member {
                send_to_many(
                    &peers,
                    channel_policy.as_ref(),
                    members.clone(),
                    event_message(
                        Some(membership.room_id.clone()),
                        "room.reconnected",
                        json!({ "client_id": session_id.clone() }),
                    ),
                )
                .await;
            } else {
                send_to_many(
                    &peers,
                    channel_policy.as_ref(),
                    members.clone(),
                    room_joined_message(&membership.room_id, &session_id),
                )
                .await;
                send_to_many(
                    &peers,
                    channel_policy.as_ref(),
                    members,
                    room_members_message(&membership.room_id, {
                        let manager = rooms.lock().await;
                        manager
                            .room_members(&membership.room_id)
                            .unwrap_or_default()
                    }),
                )
                .await;
            }

            if let Some(state) = state {
                let seq = sequences.lock().await.current(&membership.room_id);
                let checksum = state_checksum(&state);
                send_to_peer(
                    &peers,
                    channel_policy.as_ref(),
                    &session_id,
                    state_snapshot_message(&membership.room_id, seq, checksum, state),
                )
                .await;
            }
        }
    }

    loop {
        let packet = match receiver.receive().await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(error) => return Err(format!("socket read error: {error}")),
        };
        metrics
            .transport_messages_in_total
            .fetch_add(1, Ordering::Relaxed);

        let inbound = match decode_codec.decode(&packet.bytes) {
            Ok(message) => message,
            Err(error) => {
                send_to_peer(
                    &peers,
                    channel_policy.as_ref(),
                    &session_id,
                    event_message(None, "error", json!({ "reason": error.to_string() })),
                )
                .await;
                continue;
            }
        };

        if let Err(error) = inbound.validate(DEFAULT_MAX_PAYLOAD_BYTES) {
            send_to_peer(
                &peers,
                channel_policy.as_ref(),
                &session_id,
                event_message(None, "error", json!({ "reason": error.to_string() })),
            )
            .await;
            continue;
        }

        if inbound.rid.is_some() {
            metrics.rpc_requests_total.fetch_add(1, Ordering::Relaxed);
        }

        let fast_path_action = crate::route_fast_path_action(&inbound);
        if !matches!(fast_path_action, RuntimeAction::Noop) {
            execute_runtime_action(
                fast_path_action,
                &peers,
                channel_policy.as_ref(),
                &rooms,
                &session_id,
            )
            .await;
            continue;
        }

        match inbound.t.as_str() {
            "room.join_or_create" => {
                let payload = inbound.p.as_ref().unwrap_or(&Value::Null);
                let join_request = crate::parse_join_or_create_request(&inbound);
                let room_type = join_request.room_type;
                let room_id = join_request.room_id;

                let existed_before = {
                    let manager = rooms.lock().await;
                    manager.room(&room_id).is_some()
                };
                let mut manager = rooms.lock().await;
                match manager.join_or_create(&room_id, &room_type, &session_id) {
                    Ok(()) => {
                        joined_rooms.insert(room_id.clone(), room_type.clone());
                        let members = manager.room_members(&room_id).unwrap_or_default();
                        let state = manager
                            .room(&room_id)
                            .map(|room| room.state.clone())
                            .unwrap_or(Value::Null);
                        drop(manager);

                        if let Some(plugin) = wasm_room_plugins.get(&room_type) {
                            if !existed_before {
                                match plugin.on_create(&state, payload) {
                                    Ok(output) => {
                                        let _ = apply_plugin_room_update(
                                            &rooms,
                                            &peers,
                                            channel_policy.as_ref(),
                                            &sequences,
                                            &snapshot_cadence,
                                            &patch_checksum_cadence,
                                            &metrics,
                                            &room_id,
                                            output.state,
                                            output.event,
                                        )
                                        .await;
                                    }
                                    Err(error) => {
                                        eprintln!("wasm plugin on_create failed for room '{room_id}': {error}");
                                    }
                                }
                            }

                            let state_after_create = {
                                let manager = rooms.lock().await;
                                manager
                                    .room(&room_id)
                                    .map(|room| room.state.clone())
                                    .unwrap_or(Value::Null)
                            };
                            match plugin.on_join(
                                &state_after_create,
                                &json!({
                                    "client_id": session_id.clone(),
                                    "options": payload
                                }),
                            ) {
                                Ok(output) => {
                                    let _ = apply_plugin_room_update(
                                        &rooms,
                                        &peers,
                                        channel_policy.as_ref(),
                                        &sequences,
                                        &snapshot_cadence,
                                        &patch_checksum_cadence,
                                        &metrics,
                                        &room_id,
                                        output.state,
                                        output.event,
                                    )
                                    .await;
                                }
                                Err(error) => {
                                    eprintln!(
                                        "wasm plugin on_join failed for room '{room_id}': {error}"
                                    );
                                }
                            }
                        }

                        let latest_members = rooms
                            .lock()
                            .await
                            .room_members(&room_id)
                            .unwrap_or_default();

                        let latest_state = {
                            let manager = rooms.lock().await;
                            manager
                                .room(&room_id)
                                .map(|room| room.state.clone())
                                .unwrap_or(Value::Null)
                        };
                        let seq = sequences.lock().await.current(&room_id);
                        let checksum = state_checksum(&latest_state);
                        let snapshot =
                            state_snapshot_message(&room_id, seq, checksum, latest_state);
                        execute_runtime_actions(
                            crate::join_or_create_success_actions(
                                &inbound,
                                &room_id,
                                &session_id,
                                members,
                                latest_members,
                                snapshot,
                            ),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                    Err(error) => {
                        drop(manager);
                        execute_runtime_action(
                            RuntimeAction::SendToSelf(crate::rpc_error(
                                &inbound,
                                Some(room_id.clone()),
                                error.to_string(),
                            )),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                }
            }
            "room.list" => {
                let requested_type = inbound
                    .p
                    .as_ref()
                    .and_then(|payload| payload.get("roomType"))
                    .and_then(Value::as_str);
                let discovered = rooms
                    .lock()
                    .await
                    .list_rooms(requested_type)
                    .into_iter()
                    .map(|room| RoomListItem {
                        id: room.id,
                        room_type: room.room_type,
                        members: room.members,
                    })
                    .collect::<Vec<_>>();
                execute_runtime_action(
                    crate::room_list_action(&inbound, discovered),
                    &peers,
                    channel_policy.as_ref(),
                    &rooms,
                    &session_id,
                )
                .await;
            }
            "matchmaking.enqueue" => {
                let request = crate::parse_matchmaking_request(&inbound.p);

                let outcome =
                    matchmaking
                        .lock()
                        .await
                        .enqueue(&session_id, &request.room_type, request.size);

                match outcome {
                    MatchmakingOutcome::Queued {
                        room_type,
                        size,
                        position,
                    } => {
                        execute_runtime_action(
                            RuntimeAction::SendToSelf(crate::matchmaking_queued_message(
                                &inbound, &room_type, size, position,
                            )),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                    MatchmakingOutcome::Matched {
                        room_type,
                        size,
                        participants,
                    } => {
                        let match_room_id =
                            format!("{room_type}:match:{:08x}", rand::random::<u32>());

                        execute_runtime_action(
                            RuntimeAction::SendToMany {
                                session_ids: participants.clone(),
                                message: crate::matchmaking_matched_event(
                                    &match_room_id,
                                    &room_type,
                                    size,
                                    &participants,
                                ),
                            },
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;

                        execute_runtime_action(
                            RuntimeAction::SendToSelf(crate::matchmaking_matched_ack(&inbound)),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                }
            }
            "matchmaking.dequeue" => {
                let removed = matchmaking.lock().await.dequeue(&session_id);
                execute_runtime_action(
                    RuntimeAction::SendToSelf(crate::matchmaking_dequeue_message(
                        &inbound, removed,
                    )),
                    &peers,
                    channel_policy.as_ref(),
                    &rooms,
                    &session_id,
                )
                .await;
            }
            "room.leave" => {
                let room_id = crate::parse_room_id(&inbound);
                let Some(room_id) = room_id else {
                    if inbound.rid.is_some() {
                        execute_runtime_action(
                            RuntimeAction::SendToSelf(crate::rpc_error(
                                &inbound,
                                None,
                                "room id is required",
                            )),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                    continue;
                };

                let (room_type_before_leave, state_before_leave, member_count_before_leave) = {
                    let manager = rooms.lock().await;
                    match manager.room(&room_id) {
                        Some(room) => (
                            room.room_type.clone(),
                            room.state.clone(),
                            room.clients.len(),
                        ),
                        None => (String::new(), Value::Null, 0),
                    }
                };
                if let Some(plugin) = wasm_room_plugins.get(&room_type_before_leave) {
                    match plugin.on_leave(
                        &state_before_leave,
                        &json!({ "client_id": session_id.clone(), "code": 1000 }),
                    ) {
                        Ok(output) => {
                            let _ = apply_plugin_room_update(
                                &rooms,
                                &peers,
                                channel_policy.as_ref(),
                                &sequences,
                                &snapshot_cadence,
                                &patch_checksum_cadence,
                                &metrics,
                                &room_id,
                                output.state,
                                output.event,
                            )
                            .await;
                        }
                        Err(error) => {
                            eprintln!("wasm plugin on_leave failed for room '{room_id}': {error}");
                        }
                    }

                    if member_count_before_leave <= 1 {
                        let latest_state = {
                            let manager = rooms.lock().await;
                            manager
                                .room(&room_id)
                                .map(|room| room.state.clone())
                                .unwrap_or(Value::Null)
                        };
                        match plugin.on_dispose(&latest_state, &json!({ "reason": "empty_room" })) {
                            Ok(output) => {
                                let _ = apply_plugin_room_update(
                                    &rooms,
                                    &peers,
                                    channel_policy.as_ref(),
                                    &sequences,
                                    &snapshot_cadence,
                                    &patch_checksum_cadence,
                                    &metrics,
                                    &room_id,
                                    output.state,
                                    output.event,
                                )
                                .await;
                            }
                            Err(error) => {
                                eprintln!(
                                    "wasm plugin on_dispose failed for room '{room_id}': {error}"
                                );
                            }
                        }
                    }
                }

                let mut manager = rooms.lock().await;
                let leave_result = manager.leave(&room_id, &session_id);
                let members_after = manager.room_members(&room_id).unwrap_or_default();
                let room_exists = manager.room(&room_id).is_some();
                drop(manager);

                match leave_result {
                    Ok(()) => {
                        joined_rooms.remove(&room_id);
                        if !room_exists {
                            sequences.lock().await.remove(&room_id);
                            snapshot_cadence.lock().await.remove_room(&room_id);
                            patch_checksum_cadence.lock().await.remove_room(&room_id);
                        }

                        let latest_members = rooms
                            .lock()
                            .await
                            .room_members(&room_id)
                            .unwrap_or_default();
                        execute_runtime_actions(
                            crate::room_leave_success_actions(
                                &inbound,
                                &room_id,
                                &session_id,
                                members_after,
                                latest_members,
                            ),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                    Err(error) => {
                        execute_runtime_action(
                            RuntimeAction::SendToSelf(crate::rpc_error(
                                &inbound,
                                Some(room_id.clone()),
                                error.to_string(),
                            )),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                }
            }
            "room.message" | "room.message.bytes" | "room.plugin.call" => {
                let room_id = crate::parse_room_id(&inbound);
                let Some(room_id) = room_id else {
                    execute_runtime_action(
                        RuntimeAction::SendToSelf(crate::rpc_error(
                            &inbound,
                            None,
                            "room id is required",
                        )),
                        &peers,
                        channel_policy.as_ref(),
                        &rooms,
                        &session_id,
                    )
                    .await;
                    continue;
                };

                let (room_type, current_state) = {
                    let manager = rooms.lock().await;
                    match manager.room(&room_id) {
                        Some(room) => (room.room_type.clone(), room.state.clone()),
                        None => {
                            if inbound.rid.is_some() {
                                execute_runtime_action(
                                    RuntimeAction::SendToSelf(crate::rpc_error(
                                        &inbound,
                                        Some(room_id.clone()),
                                        "room not found",
                                    )),
                                    &peers,
                                    channel_policy.as_ref(),
                                    &rooms,
                                    &session_id,
                                )
                                .await;
                            }
                            continue;
                        }
                    }
                };

                let Some(plugin) = wasm_room_plugins.get(&room_type) else {
                    if inbound.rid.is_some() {
                        execute_runtime_action(
                            RuntimeAction::SendToSelf(crate::rpc_error(
                                &inbound,
                                Some(room_id.clone()),
                                "room type has no wasm plugin",
                            )),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                    continue;
                };

                let plugin_input = crate::plugin_input_from_inbound(&inbound, &session_id);

                let plugin_output = plugin.on_message(&current_state, &plugin_input);
                let plugin_output = match plugin_output {
                    Ok(output) => output,
                    Err(error) => {
                        if inbound.rid.is_some() {
                            execute_runtime_action(
                                RuntimeAction::SendToSelf(crate::rpc_error(
                                    &inbound,
                                    Some(room_id.clone()),
                                    format!("plugin execution failed: {error}"),
                                )),
                                &peers,
                                channel_policy.as_ref(),
                                &rooms,
                                &session_id,
                            )
                            .await;
                        }
                        continue;
                    }
                };

                let updated = apply_plugin_room_update(
                    &rooms,
                    &peers,
                    channel_policy.as_ref(),
                    &sequences,
                    &snapshot_cadence,
                    &patch_checksum_cadence,
                    &metrics,
                    &room_id,
                    plugin_output.state,
                    plugin_output.event,
                )
                .await;
                if !updated {
                    if inbound.rid.is_some() {
                        execute_runtime_action(
                            RuntimeAction::SendToSelf(crate::rpc_error(
                                &inbound,
                                Some(room_id.clone()),
                                "room not found",
                            )),
                            &peers,
                            channel_policy.as_ref(),
                            &rooms,
                            &session_id,
                        )
                        .await;
                    }
                    continue;
                }

                if inbound.rid.is_some() {
                    execute_runtime_action(
                        RuntimeAction::SendToSelf(crate::rpc_ok(&inbound, Some(room_id.clone()))),
                        &peers,
                        channel_policy.as_ref(),
                        &rooms,
                        &session_id,
                    )
                    .await;
                }
            }
            "state.ack" => {
                if let Some(ack) = crate::parse_state_ack_request(&inbound) {
                    acks.lock().await.ack(&session_id, &ack.room_id, ack.seq);

                    let state = {
                        let manager = rooms.lock().await;
                        manager.room(&ack.room_id).map(|room| room.state.clone())
                    };

                    if let Some(state) = state {
                        let current_seq = sequences.lock().await.current(&ack.room_id);
                        let current_checksum = state_checksum(&state);
                        if ack_requires_resync(
                            current_seq,
                            ack.seq,
                            ack.checksum.as_deref(),
                            &current_checksum,
                        ) {
                            send_to_peer(
                                &peers,
                                channel_policy.as_ref(),
                                &session_id,
                                state_snapshot_message(
                                    &ack.room_id,
                                    current_seq,
                                    current_checksum,
                                    state,
                                ),
                            )
                            .await;
                        }
                    }
                }
            }
            "state.resync" => {
                metrics.state_resync_total.fetch_add(1, Ordering::Relaxed);
                let room_id = crate::parse_room_id(&inbound);
                let Some(room_id) = room_id else {
                    continue;
                };

                let state = {
                    let mut manager = rooms.lock().await;
                    let _ = manager.mark_activity(&room_id, Utc::now());
                    manager.room(&room_id).map(|room| room.state.clone())
                };
                if let Some(state) = state {
                    let seq = sequences.lock().await.current(&room_id);
                    let checksum = state_checksum(&state);
                    send_to_peer(
                        &peers,
                        channel_policy.as_ref(),
                        &session_id,
                        state_snapshot_message(&room_id, seq, checksum, state),
                    )
                    .await;
                }
            }
            _ => {
                execute_runtime_action(
                    crate::unknown_message_action(&inbound),
                    &peers,
                    channel_policy.as_ref(),
                    &rooms,
                    &session_id,
                )
                .await;
            }
        }
    }

    let room_memberships = joined_rooms
        .iter()
        .map(|(room_id, room_type)| RoomMembership {
            room_id: room_id.clone(),
            room_type: room_type.clone(),
        })
        .collect::<Vec<_>>();

    sessions.lock().await.park(
        session_id.clone(),
        verified_project_id,
        room_memberships.clone(),
        Utc::now(),
    );

    for membership in &room_memberships {
        let members = {
            let manager = rooms.lock().await;
            manager
                .room_members(&membership.room_id)
                .unwrap_or_default()
        };
        send_to_many(
            &peers,
            channel_policy.as_ref(),
            members,
            event_message(
                Some(membership.room_id.clone()),
                "room.dropped",
                json!({ "client_id": session_id.clone() }),
            ),
        )
        .await;
    }

    let cleanup_ttl = load_session_ttl();
    let cleanup_sessions = Arc::clone(&sessions);
    let cleanup_rooms = Arc::clone(&rooms);
    let cleanup_peers = Arc::clone(&peers);
    let cleanup_sequences = Arc::clone(&sequences);
    let cleanup_snapshot_cadence = Arc::clone(&snapshot_cadence);
    let cleanup_patch_checksum_cadence = Arc::clone(&patch_checksum_cadence);
    let cleanup_metrics = Arc::clone(&metrics);
    let cleanup_wasm_room_plugins = Arc::clone(&wasm_room_plugins);
    let cleanup_channel_policy = Arc::clone(&channel_policy);
    let cleanup_acks = Arc::clone(&acks);
    let cleanup_session_id = session_id.clone();
    tokio::spawn(async move {
        let delay = cleanup_ttl
            .to_std()
            .unwrap_or_else(|_| StdDuration::from_secs(30));
        sleep(delay).await;
        let snapshot = {
            let mut store = cleanup_sessions.lock().await;
            store.remove(&cleanup_session_id)
        };
        let Some(snapshot) = snapshot else {
            return;
        };

        finalize_disconnected_session(
            &cleanup_session_id,
            snapshot.rooms,
            &cleanup_rooms,
            &cleanup_peers,
            cleanup_channel_policy.as_ref(),
            &cleanup_sequences,
            &cleanup_snapshot_cadence,
            &cleanup_patch_checksum_cadence,
            &cleanup_metrics,
            &cleanup_wasm_room_plugins,
        )
        .await;
        cleanup_acks
            .lock()
            .await
            .remove_session(&cleanup_session_id);
    });

    matchmaking.lock().await.remove_session(&session_id);
    peers.lock().await.remove(&session_id);
    drop(peer);

    match writer.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(error) => return Err(format!("writer task failed: {error}")),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec_json::JsonCodec;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    #[derive(Clone)]
    struct MockSender {
        packets: Arc<StdMutex<Vec<TransportPacket>>>,
        fail_once: Arc<StdMutex<Option<String>>>,
    }

    impl MockSender {
        fn new() -> Self {
            Self {
                packets: Arc::new(StdMutex::new(Vec::new())),
                fail_once: Arc::new(StdMutex::new(None)),
            }
        }

        fn with_fail_once(message: &str) -> Self {
            Self {
                packets: Arc::new(StdMutex::new(Vec::new())),
                fail_once: Arc::new(StdMutex::new(Some(message.to_owned()))),
            }
        }
    }

    #[async_trait]
    impl TransportSender for MockSender {
        async fn send(&mut self, packet: TransportPacket) -> Result<(), String> {
            if let Some(error) = self.fail_once.lock().expect("poisoned").take() {
                return Err(error);
            }
            self.packets.lock().expect("poisoned").push(packet);
            Ok(())
        }
    }

    struct MockReceiver {
        packets: Vec<TransportPacket>,
    }

    impl MockReceiver {
        fn new(packets: Vec<TransportPacket>) -> Self {
            Self { packets }
        }
    }

    #[async_trait]
    impl TransportReceiver for MockReceiver {
        async fn receive(&mut self) -> Result<Option<TransportPacket>, String> {
            if self.packets.is_empty() {
                return Ok(None);
            }
            Ok(Some(self.packets.remove(0)))
        }
    }

    async fn run_transport_harness(
        sender: &mut dyn TransportSender,
        receiver: &mut dyn TransportReceiver,
    ) -> Result<(), String> {
        while let Some(packet) = receiver.receive().await? {
            sender.send(packet).await?;
        }
        Ok(())
    }

    fn sample_message(t: &str) -> Message {
        Message {
            v: PROTOCOL_VERSION,
            t: t.to_owned(),
            rid: None,
            room: Some("room-a".to_owned()),
            p: Some(json!({ "ok": true })),
        }
    }

    #[tokio::test]
    async fn transport_harness_preserves_packet_metadata() {
        let mut sender = MockSender::new();
        let mut receiver = MockReceiver::new(vec![
            TransportPacket::new(
                vec![1, 2, 3],
                TransportReliability::Reliable,
                Some(RELIABLE_STREAM_ID),
            ),
            TransportPacket::new(
                vec![4, 5, 6],
                TransportReliability::Unreliable,
                Some(UNRELIABLE_STREAM_ID),
            ),
        ]);

        run_transport_harness(&mut sender, &mut receiver)
            .await
            .expect("harness should pass");

        let packets = sender.packets.lock().expect("poisoned");
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].reliability, TransportReliability::Reliable);
        assert_eq!(packets[0].stream_id, Some(RELIABLE_STREAM_ID));
        assert_eq!(packets[1].reliability, TransportReliability::Unreliable);
        assert_eq!(packets[1].stream_id, Some(UNRELIABLE_STREAM_ID));
    }

    #[tokio::test]
    async fn writer_loop_emits_reliable_and_unreliable_transport_packets() {
        let sender = MockSender::new();
        let captured = Arc::clone(&sender.packets);
        let metrics = Arc::new(RuntimeMetrics::default());
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel::<Message>();
        let (unreliable_tx, unreliable_rx) = mpsc::channel::<Message>(UNRELIABLE_QUEUE_CAPACITY);

        reliable_tx
            .send(sample_message("rpc.response"))
            .expect("reliable queue should accept");
        unreliable_tx
            .send(sample_message("position.update"))
            .await
            .expect("unreliable queue should accept");
        drop(reliable_tx);
        drop(unreliable_tx);

        writer_loop(
            Box::new(sender),
            "json".to_owned(),
            Box::new(JsonCodec),
            Arc::clone(&metrics),
            reliable_rx,
            unreliable_rx,
        )
        .await
        .expect("writer loop should succeed");

        let packets = captured.lock().expect("poisoned");
        assert_eq!(packets.len(), 2);

        let codec = JsonCodec;
        let first = codec
            .decode(&packets[0].bytes)
            .expect("first packet should decode");
        let second = codec
            .decode(&packets[1].bytes)
            .expect("second packet should decode");

        assert_eq!(first.t, "rpc.response");
        assert_eq!(packets[0].reliability, TransportReliability::Reliable);
        assert_eq!(packets[0].stream_id, Some(RELIABLE_STREAM_ID));

        assert_eq!(second.t, "position.update");
        assert_eq!(packets[1].reliability, TransportReliability::Unreliable);
        assert_eq!(packets[1].stream_id, Some(UNRELIABLE_STREAM_ID));
        assert_eq!(
            metrics.transport_messages_out_total.load(Ordering::Relaxed),
            2
        );
    }

    #[tokio::test]
    async fn writer_loop_ignores_benign_close_send_errors() {
        let sender = MockSender::with_fail_once("Sending after closing is not allowed");
        let metrics = Arc::new(RuntimeMetrics::default());
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel::<Message>();
        let (_unreliable_tx, unreliable_rx) = mpsc::channel::<Message>(UNRELIABLE_QUEUE_CAPACITY);

        reliable_tx
            .send(sample_message("rpc.response"))
            .expect("reliable queue should accept");
        drop(reliable_tx);

        let result = writer_loop(
            Box::new(sender),
            "json".to_owned(),
            Box::new(JsonCodec),
            metrics,
            reliable_rx,
            unreliable_rx,
        )
        .await;

        assert!(
            result.is_ok(),
            "benign close-path send errors should be ignored"
        );
    }
}

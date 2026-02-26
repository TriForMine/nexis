# Nexis Protocol (V1)

## Envelope

All runtime messages use the same logical envelope:

```json
{
  "v": 1,
  "t": "message.type",
  "rid": "optional-request-id",
  "room": "optional-room-id",
  "p": {}
}
```

- `v` protocol version (`u16`, current `1`)
- `t` message type (required)
- `rid` request/response correlation id (optional)
- `room` room id (optional)
- `p` payload (optional)

Validation invariants enforced by tests:

- invalid version is rejected
- missing/empty `t` is rejected
- payload size limit is enforced

## Transport

- WebSocket only (V1)
- Handshake request is always JSON

Handshake payload:

```json
{
  "v": 1,
  "codecs": ["msgpack", "json"],
  "project_id": "demo-project",
  "token": "<signed-token-or-empty>",
  "session_id": "optional-resume-session-id"
}
```

Server flow:

1. validate version
2. authenticate according to `NEXIS_AUTH_MODE`:
   - `required` (default): token must be present and valid
   - `optional`: token is verified when present, anonymous accepted when absent
   - `disabled`: token verification skipped
3. negotiate codec (`msgpack` preferred, fallback `json`)
4. attempt session resume when `session_id` is provided and valid
5. send `handshake.ok` with `{ codec, session_id, resumed, auth_mode, authenticated, project_id }`
6. all subsequent messages use negotiated codec

### Logical Channels

Nexis uses logical channels over WebSocket:

- `reliable`: default, ack/resync aware for state/RPC/control signals
- `unreliable`: drop-allowed for high-frequency transient messages (for example `position.update`)

Transport remains WebSocket. `unreliable` means server may drop frames under backpressure instead of retransmitting.

## Codec System

`Codec` trait:

```rust
trait Codec {
    fn encode(&self, message: &Message) -> Vec<u8>;
    fn decode(&self, bytes: &[u8]) -> Result<Message, CodecError>;
}
```

Implemented codecs:

- JSON (`codec_json`)
- MessagePack (`codec_msgpack`)

Roundtrip and safety invariants enforced by tests:

- JSON roundtrip succeeds
- MessagePack roundtrip succeeds
- invalid bytes are rejected
- corrupted payload decoding fails safely

## Auth Token Format

Token shape:

`base64url(claims_json).base64url(hmac_sha256(payload_segment, secret))`

Claims:

```json
{
  "project_id": "demo-project",
  "issued_at": "2026-02-25T18:00:00.000Z",
  "expires_at": "2026-02-25T19:00:00.000Z",
  "key_id": "optional-key-id",
  "aud": "optional-audience"
}
```

Auth invariants enforced by tests:

- valid token accepted
- invalid signature rejected
- wrong project rejected
- expired token rejected

## Rooms, RPC, State Sync

### Built-in rooms

- `echo_room`
- `counter_room` (minimal built-in state template)
- `counter_plugin_room` (WASM plugin room used by demo and load tests)

### Presence Events

- `room.joined` with `{ client_id }`
- `room.left` with `{ client_id }`
- `room.members` with `{ members: string[], count: number }`

### RPC

- request and response are correlated by `rid`
- unknown `rid` is rejected by RPC tracker

Common runtime RPC calls:

- `room.join_or_create`
- `room.leave`
- `room.list` with optional payload `{ "roomType": "counter_plugin_room" }`
- `room.message` with payload `{ "type": string|number, "data": any }` (plugin-dispatched room message)
- `room.message.bytes` with payload `{ "type": string|number, "data_b64": "..." }`
- `matchmaking.enqueue` with payload `{ "roomType": "counter_plugin_room", "size": 2 }`
- `matchmaking.dequeue`

Matchmaking emits `match.found` to matched clients with payload:

```json
{
  "room": "counter_plugin_room:match:abcd1234",
  "room_type": "counter_plugin_room",
  "size": 2,
  "participants": ["s-1", "s-2"]
}
```

Matchmaking queue behavior:

- queued tickets are removed on client disconnect
- tickets expire after configurable TTL (`NEXIS_MATCHMAKING_TICKET_TTL_SECONDS`, default 30s)
- SDK can optionally auto-join the matched room when `match.found` is received

WASM plugin event behavior:

- `room.message` executes room logic in configured WASM plugin
- plugin lifecycle hooks may run on room create/join/tick/leave/dispose
- plugin may emit room-scoped message payload via `room.message`
- plugin state result is diffed and broadcast as normal `state.patch` / `state.snapshot`

### State Snapshot + Patch

Join/resume or explicit resync returns:

```json
{
  "v": 1,
  "t": "state.snapshot",
  "room": "counter_plugin_room:default",
  "p": {
    "seq": 42,
    "checksum": "sha256-hex",
    "state": { "counter": 3 }
  }
}
```

Incremental updates:

```json
{
  "seq": 43,
  "ops": [
    { "op": "set", "path": "/counter", "value": 4 },
    { "op": "del", "path": "/foo" }
  ]
}
```

`checksum` on `state.patch` is optional and emitted periodically (`NEXIS_STATE_PATCH_CHECKSUM_EVERY_PATCHES`).
`state.snapshot` always includes `checksum`.

Client reliability flow:

- apply only `seq = last_seq + 1`
- verify payload checksum against local state after apply
- send `state.ack` with `{ seq }` (or `{ seq, checksum }` when checksum is present)
- if sequence gap or checksum mismatch is detected, send `state.resync` for a fresh snapshot

Server resilience behavior:

- emits periodic full `state.snapshot` after configurable patch cadence (`NEXIS_STATE_SNAPSHOT_EVERY_PATCHES`, default `20`)
- if client sends `state.ack` with mismatched checksum (or future seq), server proactively pushes a fresh snapshot

State sync invariants enforced by tests:

- deterministic patch generation
- patch apply reaches target state
- patch payload decodes identically across JSON and MessagePack

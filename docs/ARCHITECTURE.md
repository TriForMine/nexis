# Nexis Architecture (V1)

## Goal

Nexis is engine-agnostic multiplayer infrastructure split into:

- data plane (Rust runtime)
- control plane (Bun API + dashboard)

Gameplay traffic is direct client -> data plane. Control plane handles project lifecycle and token issuance.

## Monorepo Layout

- `server/` Rust data plane workspace
- `sdks/ts/` TypeScript client SDK
- `dashboard/control-api/` Elysia + Postgres control API
- `dashboard/ui/` Bun-served dashboard UI
- `examples/web-demo/` browser demo for `counter_plugin_room`
- `infra/docker-compose.yml` local orchestration
- `infra/smoke.ts` end-to-end smoke validation script
- `docs/` protocol and operational docs

## Data Plane

Crates:

- `protocol`: envelope + handshake types and validation
- `codec`: codec trait + error model
- `codec_json`, `codec_msgpack`: concrete codecs
- `auth`: HMAC mint/verify
- `rpc`: request id tracking
- `rooms`: room lifecycle and invariants
- `state_sync`: deterministic top-level patch diff/apply
- `runtime`: transport-agnostic message parsing, routing, and action building
- `transport_ws`: WebSocket adapter (socket accept + frame/packet send/receive only)
- `hooks`: room lifecycle hooks
- `modules/mod_rate_limit`: basic rate-limit module

Runtime behavior:

- validates handshake
- verifies token using configured project secrets
- optionally verifies token key status against control API (`/internal/key-status`) for revoke/scope enforcement
- supports auth modes via `NEXIS_AUTH_MODE`:
  - `required` (default): signed token required
  - `optional`: signed token optional, anonymous allowed
  - `disabled`: auth bypassed (dev/local only)
- exposes data-plane metrics on `NEXIS_METRICS_BIND` (default `0.0.0.0:9100`) at `/metrics`
- negotiates codec
- supports resumable sessions (`session_id` with TTL)
- emits room presence (`room.joined`, `room.left`, `room.members`)
- runs periodic room ticks (`RoomHooks::on_tick`) on configurable interval (`NEXIS_ROOM_TICK_MS`)
- supports sequenced/checksummed state sync (`state.snapshot`, `state.patch`, `state.ack`, `state.resync`)
- emits periodic safety snapshots after configurable patch cadence (`NEXIS_STATE_SNAPSHOT_EVERY_PATCHES`)
- supports logical delivery channels (`reliable` by default, drop-allowed `unreliable` types such as `position.update`)
- supports room discovery (`room.list`)
- supports matchmaking queue (`matchmaking.enqueue`, `matchmaking.dequeue`, `match.found`)
- supports matchmaking ticket TTL and cancel-on-disconnect cleanup
- supports `room.join_or_create`, `room.leave`, `room.message`, `echo`, and `position.update`
- supports pluggable room type registration:
  - Rust API: `rooms::RoomPlugin` + `RoomManager::register_plugin` / `register_room_plugin`
  - env bootstrap: `NEXIS_ROOM_TYPE_PLUGINS` JSON map (`room_type -> initial_state_template`)
- supports runtime-loaded WASM room plugins:
  - `NEXIS_WASM_ROOM_PLUGINS` JSON map (`room_type -> wasm_file_path`)
  - plugin initial state comes from exported `nexis_initial_state`
  - runtime calls exported lifecycle hooks (`on_create`, `on_join`, `on_message`, `on_tick`, `on_leave`, `on_dispose`)
  - client room messages route through `room.message` for plugin-backed room behavior

Transport responsibility split:

- `runtime` owns protocol-level decisions and message/action construction.
- `transport_ws` owns WebSocket framing, socket lifecycle, and transport trait wiring.
- adding a new transport should reuse `runtime` and avoid duplicating protocol handlers.

## Control Plane

`dashboard/control-api`:

- `POST /projects`
- `GET /projects`
- `POST /projects/:id/keys`
- `GET /projects/:id/keys`
- `POST /projects/:id/keys/:keyId/revoke`
- `POST /projects/:id/keys/:keyId/rotate`
- `POST /tokens`
- `GET /metrics`
- `GET /internal/key-status` (internal trust path for data-plane key checks)

Persistence and schema management:

- Drizzle ORM (`drizzle-orm/postgres-js`) for queries
- Drizzle SQL migrations in `dashboard/control-api/drizzle/`
- migrations applied at control-api startup

Data model:

- `projects`
- `project_keys` with `scopes`, `revoked_at`, and `rotated_from`

A demo project/key is seeded at startup for fast local bootstrap.

`dashboard/ui` provides a minimal operator surface for creating projects/keys and minting tokens.

## SDK

`sdks/ts` exposes:

- `connect(url, { projectId?, token? })`
- `joinOrCreate(roomType, options) -> NexisRoom`
- `listRooms(roomType?)`
- `enqueueMatchmaking(roomType, size)`
- `dequeueMatchmaking()`
- `onMatchFound(cb)`
- `onStateChange(cb)`
- `sendRPC(type, payload)`
- `onEvent(type, cb)`

Room-scoped API:

- `room.state`
- `room.onStateChange(cb)`, `room.onStateChange.once(cb)`, `room.onStateChange.select(path, cb)`
- `room.send(type, payload)`
- `room.sendBytes(type, bytes)`
- `room.onMessage(type, cb)`

`examples/web-demo` consumes `@nexis/sdk-ts` directly and demonstrates connect (token or anonymous, depending on `NEXIS_AUTH_MODE`), room join, RPC, and state patch handling.

SDK tests (Bun):

- patch apply
- RPC promise resolution
- codec encode/decode

## Tradeoffs (V1)

- state sync is intentionally top-level JSON object diff only (minimal deterministic implementation)
- logical channels are implemented over WebSocket (unreliable is drop-allowed, not UDP)
- project secret distribution is env-driven in data plane for local/dev workflows
- matchmaking and metrics are in-memory in V1 (reset on process restart)
- BetterAuth is integrated as a startup hook and extension point; full production policy wiring is left for next iteration

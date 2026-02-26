# NEXIS

Nexis is an open-source, engine-agnostic multiplayer backend platform with:

- a Rust data plane for realtime gameplay traffic
- a Bun + Elysia control plane for project/key/token management
- Drizzle-managed Postgres schema migrations for control-plane persistence

Gameplay traffic goes directly to the data plane (`ws://...`) and never through the control plane.

## Status

MVP / Beta.

Current stack is production-minded but intentionally minimal.

## Core Capabilities (V1)

- WebSocket transport with codec negotiation (`msgpack` preferred, `json` fallback)
- HMAC auth with project/key token flow and configurable auth mode (`required` / `optional` / `disabled`)
- Room engine (`echo_room`) + plugin rooms (`counter_plugin_room` demo)
- Room plugin system (`RoomPlugin` trait / `register_room_plugin`) for custom room types
- Runtime-loaded WASM room plugins (`NEXIS_WASM_ROOM_PLUGINS`) for no-rebuild room logic
- RPC request/response correlation via `rid`
- Deterministic state diff/patch (`set` / `del`)
- Sequenced + checksummed state sync (`state.snapshot`, `state.patch`, `state.ack`, `state.resync`)
- Session resume with TTL
- Room discovery (`room.list`)
- Basic matchmaking queue (`matchmaking.enqueue`, `matchmaking.dequeue`, `match.found`)
- Logical reliable/unreliable channels over WebSocket
- Control-plane key revoke/rotate enforcement in data-plane handshake
- Basic observability endpoint (`GET /metrics`)

## Monorepo

```text
nexis/
  server/                # Rust data plane workspace
  sdks/ts/               # TypeScript SDK
  dashboard/control-api/ # Bun + Elysia + Postgres API
  dashboard/ui/          # Dashboard UI
  examples/web-demo/     # Counter room demo app
  infra/                 # Docker Compose + smoke checks
  docs/                  # Protocol, architecture, quickstart
```

## Quick Start

From repo root:

```bash
docker compose -f infra/docker-compose.yml up --build
```

Services:

- Postgres: `localhost:5432`
- Control API: `http://localhost:3000`
- Dashboard UI: `http://localhost:5173`
- Data plane (WS): `ws://localhost:4000`
- Data plane metrics: `http://localhost:9100/metrics`
- Web demo: `http://localhost:8080`

Optional room plugin config (no code changes) for template-state custom room types:

```bash
NEXIS_ROOM_TYPE_PLUGINS='{"duel_room":{"hp":100},"lobby_room":{"players":[]}}'
```

Optional WASM room logic plugins (no server rebuild required):

```bash
NEXIS_WASM_ROOM_PLUGINS='{"duel_room":"/app/plugins/duel_room.wasm"}'
```

Then:

1. Create project (or use seeded `demo-project`)
2. Create key
3. Mint token (unless running anonymous mode)
4. Open web demo
5. Connect + join `counter_plugin_room`

Detailed step-by-step: [docs/QUICKSTART.md](docs/QUICKSTART.md)

## Local Development

Rust tests:

```bash
cd server
cargo test
```

TS SDK tests:

```bash
cd sdks/ts
bun test
bunx tsc -p tsconfig.json --noEmit
```

Control API tests:

```bash
cd dashboard/control-api
bun test
bunx tsc --noEmit
```

End-to-end smoke:

```bash
bun infra/smoke.ts
```

Performance baseline:

```bash
k6 run infra/load/k6-ws.js
```

## Protocol + Docs

- Protocol: [docs/PROTOCOL.md](docs/PROTOCOL.md)
- Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Quickstart: [docs/QUICKSTART.md](docs/QUICKSTART.md)
- Load Testing: [docs/LOAD_TESTING.md](docs/LOAD_TESTING.md)
- WASM Plugins: [docs/WASM_PLUGINS.md](docs/WASM_PLUGINS.md)

## License

Apache-2.0.

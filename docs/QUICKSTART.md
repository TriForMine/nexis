# Nexis Quickstart

## 0. Prerequisites

- Docker + Docker Compose
- Bun 1.3.9+
- Rust stable (for local `cargo test`)

## 1. Start the stack

From repo root:

```bash
docker compose -f infra/docker-compose.yml up --build
```

Anonymous/local mode example:

```bash
NEXIS_AUTH_MODE=optional docker compose -f infra/docker-compose.yml up --build
```

Services:

- Postgres: `localhost:5432`
- Control API: `http://localhost:3000`
- Dashboard UI: `http://localhost:5173`
- Rust data plane WS: `ws://localhost:4000`
- Web demo: `http://localhost:8080`

Useful data-plane tuning env vars (already set in `infra/docker-compose.yml`):

- `NEXIS_ROOM_TICK_MS` (default `100`)
- `NEXIS_STATE_SNAPSHOT_EVERY_PATCHES` (default `20`)
- `NEXIS_STATE_PATCH_CHECKSUM_EVERY_PATCHES` (default `10`)
- `NEXIS_AUTH_MODE` (`required` | `optional` | `disabled`, default `required`)
- `NEXIS_ROOM_TYPE_PLUGINS` (JSON map: `room_type -> initial_state_template`)
- `NEXIS_WASM_ROOM_PLUGINS` (JSON map: `room_type -> wasm_file_path`)

## 2. Create a project (optional)

A demo project/key is pre-seeded:

- `project_id`: `demo-project`
- `key_id`: `demo-key`

To create your own:

```bash
curl -X POST http://localhost:3000/projects \
  -H "content-type: application/json" \
  -d '{"name":"my-game"}'
```

## 3. Generate an API key

```bash
curl -X POST http://localhost:3000/projects/demo-project/keys \
  -H "content-type: application/json" \
  -d '{"name":"default"}'
```

Response includes `id` (key id) and `secret`.

You can optionally pass scopes (defaults to `["token:mint"]`):

```bash
curl -X POST http://localhost:3000/projects/demo-project/keys \
  -H "content-type: application/json" \
  -d '{"name":"default","scopes":["token:mint"]}'
```

## 4. Mint a client token

Using seeded key:

```bash
curl -X POST http://localhost:3000/tokens \
  -H "content-type: application/json" \
  -d '{"project_id":"demo-project","key_id":"demo-key","ttl_seconds":3600}'
```

Copy the returned `token`.

## 5. (Optional) Rotate or revoke a key

```bash
curl -X POST http://localhost:3000/projects/demo-project/keys/demo-key/rotate
curl -X POST http://localhost:3000/projects/demo-project/keys/demo-key/revoke
```

After revoke/rotate, data-plane handshake rejects tokens tied to revoked keys.

## 6. Open web demo and join counter plugin room

- Open `http://localhost:8080`
- Set:
  - Project ID: `demo-project` (or paste token first and let it auto-fill; optional in `optional/disabled` auth mode)
  - Token: `<token from /tokens>` (optional in `optional/disabled` auth mode)
- Click `Connect + Join Counter Room`
- Click `Increment` to publish `room.message` (`type: "inc"`) and observe sequenced `state.patch`
- Click `List Counter Rooms` to verify room discovery (`counter_plugin_room`)
- Open a second tab and click `Enqueue Match (1v1)` in both tabs to verify matchmaking events
- Refresh the page: the demo reuses `session_id` and attempts room resume automatically

## 7. Check basic observability metrics

```bash
curl http://localhost:9100/metrics
curl http://localhost:3000/metrics
```

## 8. Run end-to-end smoke check

With the stack running:

```bash
bun infra/smoke.ts
```

To speed up the resume-expiry reliability check locally, you can run with shorter session TTL:

```bash
NEXIS_SESSION_TTL_SECONDS=5 docker compose -f infra/docker-compose.yml up -d --build --wait
NEXIS_SESSION_TTL_SECONDS=5 bun infra/smoke.ts
```

## 9. Run tests locally

Rust core invariants:

```bash
cd server
cargo test
```

TS SDK tests:

```bash
cd sdks/ts
bun test
```

Control API tests:

```bash
cd dashboard/control-api
bun test
```

Optional: run Drizzle migrations manually:

```bash
cd dashboard/control-api
bun run db:migrate
```

# Load Testing Baseline

Nexis includes a k6 WebSocket baseline script:

- script: `infra/load/k6-ws.js`
- flow: connect -> handshake -> `room.join_or_create` -> periodic `room.message` (`type: "inc"`)
- metrics:
  - `nexis_handshake_latency_ms`
  - `nexis_join_latency_ms`
  - `nexis_room_message_rtt_ms`
  - `nexis_state_patch_count`
  - `nexis_ws_errors`

## Prerequisites

- Docker stack running (`docker compose -f infra/docker-compose.yml up -d --build`)
- k6 installed

## Run

Default baseline:

```bash
k6 run infra/load/k6-ws.js
```

Custom baseline:

```bash
VUS=50 \
DURATION=60s \
NEXIS_WS_URL=ws://localhost:4000 \
NEXIS_PROJECT_ID=demo-project \
NEXIS_TOKEN="" \
INC_INTERVAL_MS=200 \
ROOM_SHARDS=10 \
k6 run infra/load/k6-ws.js
```

For `NEXIS_AUTH_MODE=required`, provide a valid `NEXIS_TOKEN`.

## Targets

Current default thresholds in script:

- handshake latency: p95 < 500ms, p99 < 1000ms
- join latency: p95 < 700ms, p99 < 1500ms
- `room.message` RTT: p95 < 700ms, p99 < 1500ms
- websocket errors: 0

## CI Benchmark Job

GitHub Actions workflow: `.github/workflows/perf-k6.yml`

- On PRs:
  - runs k6 on the PR base commit
  - runs k6 on the PR head commit
  - generates markdown comparison (`infra/load/results/comparison.md`)
  - fails PR when gated metrics regress:
    - `room.message(inc) RTT p95` > base * `1.15`
    - `room.message(inc) RTT p99` > base * `1.15`
    - `ws errors` increases above base
  - uploads artifacts and posts/updates a PR comment
- On manual dispatch:
  - runs a single benchmark and uploads artifacts

## Profile Notes

- `ROOM_SHARDS=1` is a worst-case fanout profile (all VUs in one room).
- Increasing `ROOM_SHARDS` spreads VUs across rooms and better models multi-match workloads.

# Security Review

Snapshot of critical risks and improvements identified in the current codebase. File paths use repo-relative locations.

## Findings

1) Control API is unauthenticated (`dashboard/control-api/src/app.ts`, `index.ts`):
- Any caller can create projects/keys and mint tokens; the server trusts CORS allowlists only. Key creation responses return the shared secret, so an unauthenticated actor who knows or guesses a project ID can mint valid gameplay tokens.
- Internal health/metrics endpoints are also exposed.
- Recommendation: require admin authentication (e.g., internal token or better-auth) for all mutating endpoints and token minting; keep `/health` simple if needed. Reject requests when `NEXIS_INTERNAL_TOKEN` is missing or default.

2) Hard-coded default secrets (`dashboard/control-api/src/index.ts`, `server/crates/runtime/src/data_plane.rs`):
- `NEXIS_MASTER_SECRET`, `NEXIS_DEMO_PROJECT_SECRET`, and `NEXIS_INTERNAL_TOKEN` default to known strings (`nexis-dev-*`, `demo-secret`). If unset in production, anyone can forge tokens or call internal endpoints.
- Recommendation: fail fast unless these values are explicitly configured; distinguish dev/test defaults from production by env guard.

3) Revocation enforcement is optional (`server/crates/runtime/src/data_plane.rs`):
- Key status checks only run when `NEXIS_CONTROL_API_URL` is set. Without it, revoked keys remain usable as long as tokens validate locally.
- Recommendation: in `AuthMode::Required`, make control-API verification mandatory (or clearly warn/deny startup) so revocation cannot be skipped silently.

4) No rate limiting on control or data planes:
- `/tokens`, project/key endpoints, and WebSocket handshakes lack request throttling. High-rate calls can exhaust DB connections or runtime resources.
- Recommendation: apply HTTP rate limits per IP/key for control API and use the existing `mod_rate_limit` module (currently unused) to gate handshakes and room actions.

5) WASM plugin execution is unbounded (`server/crates/runtime/src/lib.rs`):
- Plugins run with default wasmtime settings and no fuel/epoch limits; a malicious or buggy plugin can spin forever or allocate large memory and block the runtime thread.
- Recommendation: configure wasmtime with fuel/epoch interrupts and memory limits, and optionally only load plugins from a signed/allowlisted set.

6) Auth mode can disable protection (`server/crates/runtime/src/lib.rs`):
- `NEXIS_AUTH_MODE=optional` or `disabled` admits unauthenticated sessions under project `anonymous`. This is useful for demos but risky if enabled in production.
- Recommendation: treat non-`required` modes as opt-in with a loud startup warning or refuse to run in production profiles.

## Test Baseline
- `cargo test` (server)
- `bun test` and `bunx tsc --noEmit` (sdks/ts)
- `bun test` and `bunx tsc --noEmit` (dashboard/control-api)

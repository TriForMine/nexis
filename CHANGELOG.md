# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [0.1.0] - 2026-02-27

### Added
- Rust data plane MVP with WebSocket transport.
- Protocol envelope with versioning and handshake codec negotiation.
- JSON and MessagePack codecs with roundtrip and corruption-safety coverage.
- HMAC token verification with project/key revocation integration from control-plane.
- Room engine with:
  - built-in `echo_room`
  - plugin-backed room support
  - room discovery (`room.list`)
- RPC request/response routing with unknown/duplicate `rid` protections.
- State sync with:
  - deterministic patches
  - snapshots
  - sequence/checksum integrity checks
  - client ack/resync flow
- Session resume with TTL.
- Matchmaking queue MVP (`matchmaking.enqueue`, `matchmaking.dequeue`, `match.found`).
- TypeScript SDK with room API, RPC API, patch apply, and codec support.
- Bun + Elysia control API with Drizzle-managed Postgres schema/migrations.
- Nextra docs site with getting started, tutorials, guides, API reference, and infra docs.
- Docker Compose stack for local self-hosting.

### Changed
- Standardized project metadata to Apache-2.0 and `0.1.0`.
- Locked JS dependencies to fixed versions.
- Upgraded Rust toolchain baseline in CI to `1.89.0`.

### Security
- Added dedicated `SECURITY.md` policy and reporting channel.

### Notes
- `0.1.0` is the first public MVP/beta release.
- Minor releases in `0.x` may still include breaking changes when required; they will be documented.


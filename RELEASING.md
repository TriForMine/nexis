# Releasing Nexis

This document defines the release process for Nexis.

## Versioning

- Current line: `0.x` (pre-1.0).
- Use `v0.1.0` for first public release.
- Patch (`0.1.x`): bug fixes only.
- Minor (`0.x.0`): may include breaking changes, must include migration notes.

## Release Checklist

1. Confirm branch is up to date and clean.
2. Ensure versions are aligned:
   - `server/Cargo.toml` workspace version
   - `sdks/ts/package.json`
   - `dashboard/control-api/package.json`
3. Update `CHANGELOG.md`.
4. Run quality gates:
   - `cargo test --manifest-path server/Cargo.toml`
   - `bun test` in `sdks/ts`
   - `bunx tsc -p tsconfig.json --noEmit` in `sdks/ts`
   - `bun test` in `dashboard/control-api`
   - `bunx tsc --noEmit` in `dashboard/control-api`
   - `bun run build` in `docs-site`
5. Validate stack (locally or CI compose-smoke):
   - `docker compose -f infra/docker-compose.yml up -d --build --wait`
   - `bun infra/smoke.ts`
6. Commit release metadata changes.
7. Tag release:
   - `git tag v0.1.0`
   - `git push origin v0.1.0`
8. Create GitHub release from tag with changelog highlights.

## Hotfix Process

1. Branch from latest release tag.
2. Apply minimal fix + tests.
3. Bump patch version (`0.1.x`).
4. Update changelog.
5. Tag and release.


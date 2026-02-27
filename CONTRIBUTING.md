# Contributing

Thanks for contributing to Nexis.

## Development Setup

- Rust: `1.89.0`
- Bun: `1.3.9`
- Node: `20+` (for docs site runtime compatibility)

## Commands

- Rust tests:
  - `cargo test --manifest-path server/Cargo.toml`
- SDK tests:
  - `cd sdks/ts && bun install --frozen-lockfile && bun test`
- SDK typecheck:
  - `cd sdks/ts && bunx tsc -p tsconfig.json --noEmit`
- Control API tests:
  - `cd dashboard/control-api && bun install --frozen-lockfile && bun test`
- Control API typecheck:
  - `cd dashboard/control-api && bunx tsc --noEmit`
- Docs build:
  - `cd docs-site && bun install --frozen-lockfile && bun run build`

## Pull Request Guidelines

1. Include tests for behavior changes.
2. Keep scope tight and reviewable.
3. Update docs for user-facing changes.
4. Keep lockfiles in sync when dependencies change.
5. Ensure CI passes before requesting review.

## Commit and Versioning

- Follow conventional, clear commit messages.
- Do not bundle unrelated refactors with feature/fix changes.
- Release process is documented in `RELEASING.md`.


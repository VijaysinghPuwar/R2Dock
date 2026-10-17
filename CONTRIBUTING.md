# Contributing

Thanks for helping. Please read [PLAN.md](PLAN.md) (architecture, security, testing) and [DESIGN.md](DESIGN.md) (UI) before changing a subsystem.

## Ground rules

- Priorities when they conflict: secure, then stable, then easy for beginners, then fast.
- Rust owns all R2 communication, credentials, file system access, and transfers. React only renders UI and calls typed commands in `src/ipc/commands.ts`.
- No command may return a secret. Never store secrets in files, browser storage, frontend state, logs, or error messages.
- Never accept arbitrary local paths from the frontend. Never render remote content as HTML.
- R2 has no folders, rename, atomic move, versioning, or trash. Code and copy must not imply otherwise.
- Do not copy code from GPL projects. Code adapted from MIT projects needs attribution in `THIRD_PARTY_NOTICES.md`.
- New app commands must be added to `src-tauri/build.rs`, `src-tauri/capabilities/main.json`, and `src/ipc/commands.ts`, with a reason in the pull request.

## Workflow

1. Run the full checks before opening a pull request:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   npm run lint && npm run typecheck && npm test && npm run build
   ```

2. Add or update tests with every functional change. Rust edge cases (paths, pagination, part planning, error mapping) get table or property tests.
3. Never use production buckets in tests. Integration tests use the local server in `crates/r2dock-testkit`.
4. Keep documentation factual. Do not describe planned features as done.

## Dependencies

Add a dependency only when it removes real complexity, and say why in the pull request. Run `cargo audit` and `npm audit` after dependency changes.

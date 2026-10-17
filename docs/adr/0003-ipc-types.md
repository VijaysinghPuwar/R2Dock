# ADR 0003: ts-rs for IPC types

Status: Accepted (2026-10-17)

## Decision

Rust DTOs derive `ts_rs::TS`; `cargo test` writes TypeScript definitions to `src/ipc/bindings/`. CI fails if the generated files differ from the committed ones. All frontend calls go through `src/ipc/commands.ts`. 64-bit sizes are exported as `number` (values stay below 2^53).

## Consequences

One source of truth for shapes. Command names and arguments are still listed by hand in `commands.ts`, `build.rs`, and the capability file.

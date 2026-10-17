# ADR 0002: keyring 3 for the OS credential store

Status: Accepted (2026-10-17)

## Context

PLAN.md 6.8 left the keyring version open (3 versus the split 4.x crates).

## Decision

Use `keyring` 3 with explicit per-platform features: `apple-native` (macOS Keychain), `windows-native` (Credential Manager), and `sync-secret-service` with `crypto-rust` (Linux Secret Service). Each entry uses service `app.r2dock.credentials` and a random UUID account per secret. At startup the app writes, reads, and deletes a probe entry; if that fails it switches to session-only memory storage. There is no plaintext fallback.

## Consequences

Linux builds need `libdbus-1`. Secrets are accessed with blocking calls, always from `spawn_blocking`. Moving to keyring 4 later is contained in `r2dock-secrets`.

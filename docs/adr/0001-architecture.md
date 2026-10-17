# ADR 0001: Tauri 2 with a Rust core and a React frontend

Status: Accepted (2026-10-17)

## Context

R2Dock needs a small, secure desktop app on Windows, macOS, and Linux that streams large files and keeps credentials away from UI code.

## Decision

Use Tauri 2. All R2 traffic, credentials, file access, and transfers live in Rust workspace crates that do not depend on Tauri. The React frontend only renders and calls an allowlisted set of typed commands. Logic is split into `r2dock-core` (pure rules), `r2dock-secrets`, `r2dock-r2`, and `r2dock-transfer`, each testable without a window.

## Consequences

Three different webviews must be tested. Most logic is unit and integration tested in Rust without Tauri.

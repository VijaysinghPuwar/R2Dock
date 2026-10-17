# ADR 0005: Integration tests against a local S3 server

Status: Accepted (2026-10-17)

## Context

Tests must never touch production buckets, and real R2 test credentials are not available in CI.

## Decision

`crates/r2dock-testkit` starts an in-process S3-compatible server (`s3s` with `s3s-fs`) on a random local port backed by a temporary folder. R2 client, bulk operation, transfer engine, and connection lifecycle tests run against it, including multipart uploads, ranged downloads, resume after restart, conflicts, cancellation, and wrong credentials.

## Consequences

R2-specific behaviour (conditional writes, error codes, checksum defaults, scoped token responses) is not proven by these tests. Those items stay marked **[Verify]** in PLAN.md until checked against a dedicated R2 test bucket.

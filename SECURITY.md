# Security Policy

## Reporting a vulnerability

Please report security problems privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Do not open a public issue for a vulnerability.

Include the version, your operating system, steps to reproduce, and the impact you expect. Never include real R2 credentials in a report.

## Supported versions

Only the latest release receives fixes while the project is in 0.x.

## Scope

In scope: leaks of secrets (to logs, files, the webview, or error messages), path traversal or overwrite through object keys, the webview gaining access to files or commands it should not have, and unsafe rendering of object names or previews.

Out of scope: a compromised operating system or user account, malware running as the same user, and issues in Cloudflare R2 itself.

## Design summary

- Secrets live in the OS credential store (macOS Keychain, Windows Credential Manager, Linux Secret Service), or in memory for one session when no store is available. There is no plaintext fallback.
- No app command returns a secret. Logs pass through a redaction filter.
- The webview has no file system, shell, or HTTP plugin access; app commands are restricted to an allowlist; the Content Security Policy blocks remote scripts and `unsafe-eval`.
- Local paths come only from native pickers and drag and drop, and are referenced by opaque ids.
- Bulk and folder deletes require a backend confirmation token and, for large deletes, typed confirmation.

The full threat model is in PLAN.md section 9.8.

<p align="center">
  <img src="assets/brand/logo-icon.svg" width="96" height="96" alt="R2Dock logo">
</p>

<h1 align="center">R2Dock</h1>

<p align="center">A desktop file manager for Cloudflare R2. Browse, upload, download, and manage your buckets like a regular drive. Your keys stay on your computer.</p>

R2Dock runs on Windows, macOS, and Linux. It talks directly to the R2 S3 API from your machine. There is no R2Dock server, account, or telemetry.

Status: **0.1.0 preview.** Builds are not code signed yet. Behaviour is tested against a local S3-compatible server; it has not yet been through a full test pass against real R2 (see [Limitations](#limitations)).

## Features

- Connect one or more R2 accounts with R2 API token credentials (Default, EU, and FedRAMP jurisdictions). Test, edit, replace the secret, or remove a connection.
- Bucket list, or manually added buckets for bucket-scoped tokens.
- Folder browsing with pagination, sorting, list and grid views, and a details panel (size, type, modified, ETag, storage class, custom metadata).
- Search within the current folder, or by key prefix across a bucket.
- Uploads of files and folders by picker or drag and drop. Large files upload in parallel parts. Conflict choices: ask, skip, replace, keep both.
- Downloads of files and folders, streamed to disk, resumable, written to a temporary file and moved into place without overwriting.
- Transfer manager with progress, speed, pause and resume (large uploads and downloads), cancel, retry, and history. Unfinished work can continue after a restart.
- New folder, delete with confirmation (typed confirmation for folders and large deletes), rename and move (copy, verify, then delete, with a report of anything that failed).
- Previews for images, text, and audio. Copy object key or public URL.
- Dark and Paper themes, compact density, larger text sizes, keyboard shortcuts.

## Install

Download the installer for your system from [Releases](https://github.com/VijaysinghPuwar/R2Dock/releases).

| System | File | Notes |
|---|---|---|
| Windows 10/11 | `.msi` or `-setup.exe` | Not signed yet: SmartScreen may warn. Choose "More info" then "Run anyway". |
| macOS 11+ (Apple Silicon or Intel) | `.dmg` | Not notarized yet: right-click the app, choose Open, then confirm. |
| Linux | `.AppImage`, `.deb`, or `.rpm` | Needs WebKitGTK 4.1. A Secret Service provider (GNOME Keyring or KWallet) is needed to save keys. |

Each release includes `SHA256SUMS.txt` so you can check the files you downloaded.

## Getting credentials

R2Dock uses R2 S3 credentials, not your Cloudflare password or global API key.

1. In the Cloudflare dashboard, open **R2**.
2. Copy your **Account ID** from the R2 overview page.
3. Open **Manage API tokens** and create a token.
4. Choose **Object Read & Write** and pick the buckets R2Dock should manage. Choose Admin permissions only if you need to see all buckets.
5. Copy the **Access Key ID** and **Secret Access Key**. The secret is shown only once.

If your token is limited to specific buckets, enter a bucket name in the Connect form.

## Security

- Secret keys are stored in the operating system credential store: macOS Keychain, Windows Credential Manager, or Linux Secret Service. If none is available, the key is kept in memory for the current session only. There is no plaintext fallback.
- After you save a connection, the secret never goes back to the user interface, and it is never written to config files or logs.
- The interface can only call a fixed list of app commands. It has no direct file system, shell, or network access, and a strict Content Security Policy blocks remote scripts.
- Local files are chosen with system pickers or drag and drop. Object keys are treated as untrusted: downloads cannot write outside the folder you chose, never follow symbolic links, and never overwrite a file unless you choose Replace.

See [SECURITY.md](SECURITY.md) for reporting a vulnerability and the threat model in [PLAN.md](PLAN.md) section 9.8.

## Limitations

- R2 has no real folders, no rename, no atomic move, no versioning, and no trash. Moving or renaming a folder copies and deletes every object inside it and can partially fail; R2Dock reports what failed. Deletes are permanent.
- Folder sizes are only known after an explicit "Calculate", which lists every object (R2 may bill these operations).
- Video and PDF previews are not available yet. There is no sync and no presigned share links.
- Builds are unsigned and there is no automatic updater yet.
- Integration tests run against a local S3-compatible server. Some R2-specific behaviours listed as **[Verify]** in PLAN.md have not been confirmed against real R2.

## Development

Requirements: Rust (stable), Node.js 22+, and the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```sh
npm ci
npm run tauri dev       # run the app
npm run tauri build     # build installers for this machine
```

Tests and checks:

```sh
cargo test --workspace                                  # Rust unit and integration tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm run lint && npm run typecheck && npm test           # frontend
```

TypeScript types for IPC are generated from Rust by `cargo test` into `src/ipc/bindings/`.

Project layout:

| Path | Contents |
|---|---|
| `crates/r2dock-core` | Pure logic: keys, local path safety, part planning, settings, storage, redaction |
| `crates/r2dock-secrets` | OS credential store adapter and session-only store |
| `crates/r2dock-r2` | R2 client, listing, object operations, bulk delete and move |
| `crates/r2dock-transfer` | Transfer engine: uploads, downloads, scheduling, journal, history |
| `crates/r2dock-testkit` | Test-only local S3 server |
| `src-tauri` | Tauri app: commands, capabilities, media scheme |
| `src` | React frontend |

Design and architecture: [DESIGN.md](DESIGN.md), [PLAN.md](PLAN.md), and [docs/adr](docs/adr).

## License

[MIT](LICENSE). Third-party notices: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

R2Dock is an independent project and is not affiliated with Cloudflare. Cloudflare and R2 are trademarks of Cloudflare, Inc.

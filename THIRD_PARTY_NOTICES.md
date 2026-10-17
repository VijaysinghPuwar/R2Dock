# Third-Party Notices

R2Dock is distributed under the MIT License. It bundles or links the following third-party components. Each remains under its own license.

| Component | License | Use |
|---|---|---|
| IBM Plex Sans and IBM Plex Mono (via @fontsource) | SIL Open Font License 1.1 | Bundled UI fonts |
| Lucide icons (lucide-react) | ISC | Interface icons |
| Tauri and its plugins | MIT or Apache-2.0 | Desktop shell |
| AWS SDK for Rust (aws-sdk-s3) | Apache-2.0 | R2 S3 API client |
| React, TanStack Query, TanStack Virtual, Zustand, Radix UI | MIT | Frontend |
| keyring | MIT or Apache-2.0 | OS credential store access |
| s3s, s3s-fs (tests only, not shipped) | Apache-2.0 | Local S3 server for integration tests |

The full dependency list with versions is in `Cargo.lock` and `package-lock.json`. No source code from the reference projects reviewed during planning was copied into R2Dock.

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
| Symphonia (unmodified crates) | MPL-2.0 | Reading audio file details for compression. Source: https://github.com/pdeljanov/Symphonia |
| process-wrap | MIT or Apache-2.0 | Running FFmpeg in a Job Object or process group |
| zstd | MIT or BSD-3-Clause (zstd library: BSD-3-Clause) | Unpacking the compression tools |

**FFmpeg** is not part of R2Dock and is not included in its installers. When the user turns on compression, R2Dock downloads FFmpeg 9.0.2 (LGPL v3, built with `--enable-version3`, without GPL or nonfree components) from this project's `tools-ffmpeg-*` release and runs it as a separate program. That release lists the exact sources: BtbN FFmpeg-Builds for Windows and Linux (including SVT-AV1 and OpenH264), and `tools/ffmpeg/build-macos.sh` with FFmpeg, LAME 3.100 (LGPL), Opus 1.5.2 (BSD), and SVT-AV1 4.2.0 (BSD, with the Alliance for Open Media patent license) for macOS. Their license texts are published next to the binaries. H.264 encoding uses the operating system's hardware encoders where available.

The full dependency list with versions is in `Cargo.lock` and `package-lock.json`. No source code from the reference projects reviewed during planning was copied into R2Dock.

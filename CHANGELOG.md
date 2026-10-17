# Changelog

All notable changes are listed here. The format follows Keep a Changelog, and the project uses Semantic Versioning.

## [0.4.0] - 2026-10-17

### Added

- Video compression (Settings > Compression): AV1 by default, about a third of the size of a typical H.264 video at the same quality (measured VMAF above 96), or "Most compatible (H.264)" with the computer's hardware encoder (VideoToolbox, NVIDIA, Intel, AMD, Windows, VAAPI) and OpenH264 as fallback. Video size: 720p, 1080p (default), keep, or off.
- Every audio track, text subtitles, chapters, tags, rotation, and variable frame rate are kept. HDR video, image subtitles, and unknown data tracks are left unchanged. MOV and MKV become MP4.
- The upload review counts audio files and videos; transfer rows give video reasons such as "Already efficient (HEVC)" or "HDR video kept as is".
- The macOS compression tools now include SVT-AV1 (tools build 2, about 20 MB).

## [0.3.0] - 2026-10-17

### Added

- Optional audio compression before upload (Settings > Compression, off by default). Speech becomes 32 or 48 kbps MP3, music high-quality MP3, and WAV, AIFF, FLAC, and ALAC become MP3 or FLAC; "Smallest" uses Opus instead. M4A stays M4A. The original file is never changed.
- Files that would not shrink are uploaded as they are, with the reason shown: already compressed, would save too little (a 30 second sample is tested first on long files), compressed by R2Dock before, too small, or not audio.
- Compression tools: FFmpeg 9.0.2 (LGPL) is downloaded once from this project's releases and checked by SHA-256 before it is unpacked and before each use. An FFmpeg already on the computer can be used instead.
- Compare quality: 30 second samples of one of your files at each setting, to listen to before choosing.
- Upload review shows the estimate ("Compress 195 audio files. Estimated 2.4 GB to 1.2 GB."), a switch for each upload, and files whose extension changes.
- Transfer rows show "Compressing 42%" and "Saved 330 MB (50%)"; the details panel shows "Compressed: From 660 MB". Compressed uploads keep working across pause, cancel, retry, and restarts, and compression steps appear in the activity log.

## [0.2.1] - 2026-10-17

### Fixed

- Interrupted transfers are restored after a restart again. A large upload that was paused or interrupted saved a resume record that could not be read back, so every interrupted upload and download was dropped when R2Dock started. Records are now stored in a form that always loads, with tests that restart the engine.

## [0.2.0] - 2026-10-17

### Added

- Settings > Advanced > Logs: activity and technical log locations with Open folder and Copy path, Change and Reset to default for the activity folder, and Export as CSV for Today, the last 7 days, the last 30 days, or everything.
- Help > Open Activity Log Folder on macOS, and the same action in the keyboard shortcut sheet (`?`) on every platform.
- "Open activity log" in error details for failed transfers and connection tests.

### Changed

- The activity log is now CSV (`activity-YYYY-MM-DD.csv`) and opens in Excel, Numbers, or Google Sheets. Cells that a spreadsheet would run as a formula are neutralized. Older `.jsonl` files are still cleaned up and included in exports.
- Logs now hide the account id in R2 endpoint host names.

## [0.1.1] - 2026-10-17

### Fixed

- Release builds: the Rust Tauri crate is now 2.12, matching the JavaScript API package, so installers build on every platform.

## [0.1.0] - 2026-10-17 (not released)

First preview release.

### Added

- Connections: add, test, edit, replace secret, and remove R2 connections; Default, EU, and FedRAMP jurisdictions; bucket-scoped tokens; secrets in the OS credential store with a session-only mode when no store is available.
- Browsing: bucket list, folder navigation with pagination, sorting, list and grid views, details panel, current-folder filter and bucket prefix search.
- Transfers: file and folder uploads (picker and drag and drop), parallel multipart uploads for large files, resumable downloads, conflict policies, pause, resume, cancel, retry, history, and recovery after restart.
- Management: new folder, delete with confirmation tokens and typed confirmation, rename and move with partial failure reports, copy key, public URL per bucket, image, text, and audio previews, favorites, folder size calculation.
- Dark and Paper themes, density and text size settings, keyboard shortcuts.
- Activity log (Settings > Advanced): one line per action and its result, with error codes and R2 request IDs, kept for 14 days by default in a folder you can choose. Keys and secrets are never recorded; file and bucket names are replaced with short codes unless you turn them on.
- Connect form accepts several bucket names separated by commas, for Object Read & Write tokens that cannot list buckets.
- Unsigned installers for Windows, macOS (Apple Silicon and Intel), and Linux.

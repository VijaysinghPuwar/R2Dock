# Changelog

All notable changes are listed here. The format follows Keep a Changelog, and the project uses Semantic Versioning.

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

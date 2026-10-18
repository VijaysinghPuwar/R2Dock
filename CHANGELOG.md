# Changelog

All notable changes are listed here. The format follows Keep a Changelog, and the project uses Semantic Versioning.

## [Unreleased]

### Added

- File details show the object's public link by source, like the Cloudflare dashboard: "Public URL" (set in Bucket settings), "Custom domain", or "r2.dev", with copy and open buttons. Custom and r2.dev domains are found through an optional API token (the *Token value* of an R2 token) entered in the connection form. The token is kept in the system keychain like the secret, and R2Dock then also contacts `api.cloudflare.com` (read-only, only when a file's details or public link are requested). "Copy public URL" uses the same link.

### Fixed

- A new session-only secret is only used once the connection settings are saved, together with dropping the old client, so a request can never mix old and new keys.
- A keychain entry left behind by a failed save, whose deletion also failed, is now recorded in a small side file so it is still deleted after a restart.

## [0.4.1] - 2026-10-18

Tested end to end on Windows 11 against a local S3 server, with stress runs of 3,000 files (1.3 GB) and UI runs of 2,000 transfers.

### Fixed

- Downloads never fall back to the raw object key: a name that cannot exist on the disk (for example only dots or spaces on Windows, or longer than 255 bytes) is given a safe name.
- Downloads of very long file names no longer fail on the temporary file, and "Keep both" names stay within the file name limit.
- Two objects that map to the same file on Windows or macOS (`A.txt` and `a.txt`, or `a:b` and `a_b`) are saved under separate names instead of overwriting each other.
- Windows device names `COM0`, `LPT0`, and the superscript forms (`COM¹`) are renamed like `CON` and `NUL`.
- A resumed download starts over when a server or proxy ignores the byte range, and re-fetches the last 1 MiB after a crash, so a partial file is never joined to the wrong data.
- A transfer that finished just before the app closed is marked complete on the next launch instead of being uploaded or downloaded again.
- The transfer journal and settings are written one at a time, so an older copy can never replace a newer one; a journal write that fails because Windows briefly locks the file is retried.
- Activity rows are no longer lost while today's CSV is open in Excel: they go to a side file that export and clean-up include. Exporting over a daily log with different letter case is refused, and cells with `;` are quoted for Excel in semicolon locales.
- A failed settings save no longer leaves the app using settings the screen does not show.
- Removing or reinstalling the compression tools is refused while files are compressing, and a reinstall that fails keeps the old tools.
- The tools download view survives leaving and reopening Settings; the quality preview can be restarted without mixing samples.
- A file that keeps changing while it is compressed fails after two attempts instead of being re-encoded forever.
- Transfer history trimming no longer drops an entry finished at the same moment, and no longer logs a warning on first launch.
- Selecting a whole drive for upload explains why it was skipped; folders holding only hidden system files upload as empty folders.
- The sidebar shows a connection as online or offline from finished transfers, not only after a connection test.
- The transfer dock no longer shows an old speed and time left while a transfer waits to retry.
- A finished upload's compressed copy and journal entry are removed before the job is shown as complete.

### Changed

- Large queues stay responsive: transfer updates are applied in batches. With 2,000 downloads the longest pause in the window went from 5.1 s to under 20 ms, and the queue finished in half the time.
- The window opens only after the interface has loaded, so Windows no longer shows a blank white window at start.
- F2 renames the selected file or folder.
- Ctrl+Shift+R and F5 refresh the current view and never reload the window.

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

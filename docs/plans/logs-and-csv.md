# Plan: Log folder access and CSV activity log

Status: **Planned. Not implemented.** Target release: 0.2.0 (before media compression, so compression events land in the CSV from day one).

## Goal

Anyone can find, open, and read R2Dock's logs without technical knowledge: the folder location is visible in Settings, opens with one click, and the activity log is a CSV file that opens in Excel, Numbers, or Google Sheets.

## Today (0.1.1)

| Item | Where | Format |
|---|---|---|
| Activity log | Settings > Advanced: location shown, Change, Open | JSON lines, `activity-YYYY-MM-DD.jsonl` |
| Technical (diagnostic) log | Settings > Advanced: "Open log folder" (path not shown) | Text, `r2dock.YYYY-MM-DD.log` |

## Changes

### 1. Activity log becomes CSV

- Daily files `activity-YYYY-MM-DD.csv`, written as now on a background thread, with a header row on file creation.
- Columns (fixed order, documented in the file header and the README):

| Column | Example |
|---|---|
| time (UTC, ISO 8601) | 2026-10-18T09:14:03Z |
| local_time | 2026-10-18 05:14:03 |
| session | 3f2a9c1e |
| action | upload_start |
| outcome | ok / error / info |
| duration_ms | 412 |
| connection | 1a2b3c4d |
| bucket | #9f8e7d6c (or the name if names are allowed) |
| item | #1a2b3c4d (key or file name, same rule) |
| files | 195 |
| bytes | 345000000 |
| error_code | accessDenied |
| error_message | This key cannot do that. |
| request_id | (R2 request id, when present) |
| details | other fields as `key=value; key=value` |

- **CSV safety**: values are quoted per RFC 4180; any cell that starts with `=`, `+`, `-`, `@`, tab, or carriage return is prefixed with `'` so a crafted file name cannot run a formula when the CSV is opened in a spreadsheet (CSV injection). Line breaks inside values are replaced with spaces.
- Privacy rules are unchanged: no secrets, arguments, clipboard text, or link paths; names hashed unless the user turns names on; every line passes the redaction filter.
- Existing `.jsonl` files from 0.1.x stay readable; the export below includes them.

### 2. Settings > Advanced > Logs

| Row | Shows | Buttons |
|---|---|---|
| Activity log | On/Off switch, "Include file and folder names" switch, retention days | |
| Activity log location | Full path, home shown as `~` (for example `~/Library/Logs/app.r2dock.desktop/activity`) | **Open folder**, **Copy path**, **Change...**, **Reset to default** |
| Export activity | Date range (Today, Last 7 days, Last 30 days, All) | **Export as CSV...** (save dialog, one combined file) |
| Technical log location | Full path | **Open folder**, **Copy path** |
| Log level | Standard / Detailed | |

Default locations (shown in Settings and the README):

| OS | Activity log | Technical log |
|---|---|---|
| macOS | `~/Library/Logs/app.r2dock.desktop/activity/` | `~/Library/Logs/app.r2dock.desktop/` |
| Windows | `%LOCALAPPDATA%\app.r2dock.desktop\logs\activity\` | `%LOCALAPPDATA%\app.r2dock.desktop\logs\` |
| Linux | `~/.local/share/app.r2dock.desktop/logs/activity/` | `~/.local/share/app.r2dock.desktop/logs/` |

The exact paths are confirmed per OS during implementation (Tauri's `app_log_dir`) and the table is updated.

### 3. Quick access outside Settings

- Help menu (macOS app menu, Windows and Linux "Help" in the shortcut sheet): "Open activity log folder".
- When an operation fails, the error details disclosure includes "Open activity log" so the user can find the matching row.

## Commands

| Command | Purpose |
|---|---|
| `open_activity_folder` (exists) | Opens the folder in Finder, Explorer, or the file manager |
| `open_log_folder` (exists) | Opens the technical log folder |
| `log_locations` (new) | Returns both display paths (home as `~`) for Settings |
| `activity_export_csv` (new) | Takes a date range, asks for a destination with the Rust save dialog, merges daily CSV and old JSONL files into one CSV. The webview never supplies a path |
| `activity_reset_dir` (new) | Returns to the default folder |
| Copy path | Uses the existing `clipboard_write` |

All new commands are added to `build.rs`, the capability file, and `src/ipc/commands.ts`.

## Testing

- CSV writer: quoting, commas and quotes in values, line breaks, formula-injection prefixes (`=cmd|' /C calc'!A1`, `+1`, `-2`, `@x`), Unicode names, header written once per file.
- Export: merges CSV and JSONL in time order, respects the date range, writes only to the chosen path, handles an empty range.
- Privacy: a fake secret and a fake account ID never appear in any CSV cell; names stay hashed when the switch is off.
- UI: paths shown, Copy path writes the expected text, buttons call the right commands.

## Build order

1. CSV writer and column mapping in `src-tauri/src/activity.rs`, with tests.
2. `log_locations`, `activity_export_csv`, `activity_reset_dir` commands and allowlists.
3. Settings Logs group, Help menu entry, error-details link.
4. README section "Where are the logs?" with the per-OS table; CHANGELOG.

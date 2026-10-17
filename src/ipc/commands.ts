// Typed wrappers for every app command. The frontend calls Rust only through
// this module. Types come from the generated bindings in ./bindings.

import { Channel, invoke } from "@tauri-apps/api/core";

import type { AppError } from "./bindings/AppError";
import type { AppInfo } from "./bindings/AppInfo";
import type { BucketEntry } from "./bindings/BucketEntry";
import type { CompressionStatus } from "./bindings/CompressionStatus";
import type { ConflictPolicy } from "./bindings/ConflictPolicy";
import type { ConnectionInput } from "./bindings/ConnectionInput";
import type { ConnectionSummary } from "./bindings/ConnectionSummary";
import type { DeletePreview } from "./bindings/DeletePreview";
import type { DeleteResult } from "./bindings/DeleteResult";
import type { DownloadStarted } from "./bindings/DownloadStarted";
import type { ExportRange } from "./bindings/ExportRange";
import type { HistoryEntry } from "./bindings/HistoryEntry";
import type { InstallProgress } from "./bindings/InstallProgress";
import type { PreviewSample } from "./bindings/PreviewSample";
import type { JobAction } from "./bindings/JobAction";
import type { ListPage } from "./bindings/ListPage";
import type { LogKind } from "./bindings/LogKind";
import type { MovePreview } from "./bindings/MovePreview";
import type { MoveResult } from "./bindings/MoveResult";
import type { ObjectInfo } from "./bindings/ObjectInfo";
import type { OpProgress } from "./bindings/OpProgress";
import type { Preferences } from "./bindings/Preferences";
import type { PrefixSize } from "./bindings/PrefixSize";
import type { Selection } from "./bindings/Selection";
import type { SettingsView } from "./bindings/SettingsView";
import type { TestResult } from "./bindings/TestResult";
import type { TextPreview } from "./bindings/TextPreview";
import type { TransferEvent } from "./bindings/TransferEvent";
import type { UploadPlan } from "./bindings/UploadPlan";

/** Type guard for errors returned by commands. */
export function isAppError(value: unknown): value is AppError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    typeof (value as { message: unknown }).message === "string"
  );
}

/** A plain sentence for any thrown value. */
export function errorMessage(value: unknown): string {
  if (isAppError(value)) return value.message;
  if (value instanceof Error) return value.message;
  return "Something went wrong. Try again.";
}

// App
export const appInfo = () => invoke<AppInfo>("app_info");

// Connections
export const connectionList = () => invoke<ConnectionSummary[]>("connection_list");
export const connectionTest = (input: ConnectionInput) => invoke<TestResult>("connection_test", { input });
export const connectionSave = (input: ConnectionInput) => invoke<ConnectionSummary>("connection_save", { input });
export const connectionRemove = (id: string) => invoke<void>("connection_remove", { id });
export const connectionSetActive = (id: string) => invoke<void>("connection_set_active", { id });
export const connectionAddBucket = (id: string, bucket: string) =>
  invoke<ConnectionSummary>("connection_add_bucket", { id, bucket });
export const connectionRemoveBucket = (id: string, bucket: string) =>
  invoke<ConnectionSummary>("connection_remove_bucket", { id, bucket });
export const connectionSetPublicUrl = (id: string, bucket: string, url: string | null) =>
  invoke<ConnectionSummary>("connection_set_public_url", { id, bucket, url });
export const connectionToggleFavorite = (id: string, bucket: string) =>
  invoke<ConnectionSummary>("connection_toggle_favorite", { id, bucket });

// Browsing
export const bucketList = (connectionId: string) => invoke<BucketEntry[]>("bucket_list", { connectionId });
export const objectList = (connectionId: string, bucket: string, prefix: string, token: string | null) =>
  invoke<ListPage>("object_list", { connectionId, bucket, prefix, token });
export const objectSearch = (connectionId: string, bucket: string, prefix: string, token: string | null) =>
  invoke<ListPage>("object_search", { connectionId, bucket, prefix, token });
export const objectHead = (connectionId: string, bucket: string, key: string) =>
  invoke<ObjectInfo>("object_head", { connectionId, bucket, key });
export const prefixSize = (connectionId: string, bucket: string, prefix: string) =>
  invoke<PrefixSize>("prefix_size", { connectionId, bucket, prefix });
export const folderCreate = (connectionId: string, bucket: string, prefix: string, name: string) =>
  invoke<string>("folder_create", { connectionId, bucket, prefix, name });

// Previews
export const previewText = (connectionId: string, bucket: string, key: string) =>
  invoke<TextPreview>("preview_text", { connectionId, bucket, key });
/** Opaque token; turn it into a URL with `mediaUrl` in lib/media.ts. */
export const mediaToken = (connectionId: string, bucket: string, key: string) =>
  invoke<string>("media_token", { connectionId, bucket, key });

// Delete, move, rename
export const deletePreview = (connectionId: string, bucket: string, keys: string[], prefixes: string[]) =>
  invoke<DeletePreview>("delete_preview", { connectionId, bucket, keys, prefixes });
export const deleteExecute = (token: string, typed: string | null, onProgress: (p: OpProgress) => void) => {
  const channel = new Channel<OpProgress>();
  channel.onmessage = onProgress;
  return invoke<DeleteResult>("delete_execute", { token, typed, onProgress: channel });
};
export const movePreview = (
  connectionId: string,
  bucket: string,
  keys: string[],
  prefixes: string[],
  destPrefix: string,
  rename: string | null,
) => invoke<MovePreview>("move_preview", { connectionId, bucket, keys, prefixes, destPrefix, rename });
export const moveExecute = (token: string, onProgress: (p: OpProgress) => void) => {
  const channel = new Channel<OpProgress>();
  channel.onmessage = onProgress;
  return invoke<MoveResult>("move_execute", { token, onProgress: channel });
};

// Transfers
export const pickUploadFiles = () => invoke<Selection | null>("pick_upload_files");
export const pickUploadFolder = () => invoke<Selection | null>("pick_upload_folder");
export const uploadPrepare = (selectionId: string, connectionId: string, bucket: string, prefix: string) =>
  invoke<UploadPlan>("upload_prepare", { selectionId, connectionId, bucket, prefix });
/** `compress` is the review dialog's compression switch. */
export const uploadStart = (planId: string, policy: ConflictPolicy, compress = true) =>
  invoke<number>("upload_start", { planId, policy, compress });
/** Returns null when the user cancels the folder picker. */
export const downloadStart = (connectionId: string, bucket: string, keys: string[], prefixes: string[]) =>
  invoke<DownloadStarted | null>("download_start", { connectionId, bucket, keys, prefixes });
export const transferSubscribe = (onEvent: (e: TransferEvent) => void) => {
  const channel = new Channel<TransferEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("transfer_subscribe", { onEvent: channel });
};
export const transferControl = (id: string, action: JobAction) => invoke<void>("transfer_control", { id, action });
export const transferControlMany = (ids: string[], action: JobAction) =>
  invoke<void>("transfer_control_many", { ids, action });
export const transferPauseAll = () => invoke<void>("transfer_pause_all");
export const transferClearFinished = () => invoke<void>("transfer_clear_finished");
export const transferHistory = (limit: number) => invoke<HistoryEntry[]>("transfer_history", { limit });
/** Reveal a finished download in the system file manager. */
export const transferShow = (id: string) => invoke<void>("transfer_show", { id });

// Settings
export const settingsGet = () => invoke<SettingsView>("settings_get");
export const settingsSet = (preferences: Preferences) => invoke<SettingsView>("settings_set", { preferences });
export const settingsChooseDownloadDir = () => invoke<SettingsView>("settings_choose_download_dir");
export const settingsReset = () => invoke<SettingsView>("settings_reset");
export const historyClear = () => invoke<void>("history_clear");
export const openLogFolder = () => invoke<void>("open_log_folder");
export const settingsChooseActivityDir = () => invoke<SettingsView>("settings_choose_activity_dir");
export const openActivityFolder = () => invoke<void>("open_activity_folder");
export const activityResetDir = () => invoke<SettingsView>("activity_reset_dir");
/** Copies the full folder path; Rust writes it to the clipboard. */
export const logCopyPath = (kind: LogKind) => invoke<void>("log_copy_path", { kind });
/** Asks for a destination with the system save dialog. Returns the row count, or null if cancelled. */
export const activityExportCsv = (range: ExportRange) => invoke<number | null>("activity_export_csv", { range });
/** Record a problem seen by the interface in the activity log (best effort, never throws). */
export const activityNote = (kind: string, message: string) =>
  invoke<void>("activity_note", { kind, message }).catch(() => undefined);

// Compression tools
export const compressionStatus = () => invoke<CompressionStatus>("compression_status");
export const compressionInstall = (onProgress: (p: InstallProgress) => void) => {
  const channel = new Channel<InstallProgress>();
  channel.onmessage = onProgress;
  return invoke<CompressionStatus>("compression_install", { onProgress: channel });
};
export const compressionCancelInstall = () => invoke<void>("compression_cancel_install");
export const compressionRemove = () => invoke<CompressionStatus>("compression_remove");
/** Opens the file picker; returns null when cancelled. */
export const compressionUseSystem = () => invoke<CompressionStatus | null>("compression_use_system");
export const compressionUseDownloaded = () => invoke<CompressionStatus>("compression_use_downloaded");
/** Opens the file picker, then encodes 30 second samples. Null when cancelled. */
export const compressionPreview = () => invoke<PreviewSample[] | null>("compression_preview");

// System
export const clipboardWrite = (text: string) => invoke<void>("clipboard_write", { text });
/** Opens an https URL in the default browser. Other schemes are refused by Rust. */
export const openExternal = (url: string) => invoke<void>("open_external", { url });

/** Event name for OS drag and drop over the window (payload: DragEvent). */
export const DRAG_EVENT = "r2dock://drag";

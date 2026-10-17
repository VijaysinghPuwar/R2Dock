//! Build script: restricts app commands to an explicit allowlist (PLAN.md 9.4).
//! Each command also needs an `allow-<name>` entry in capabilities/main.json.

const COMMANDS: &[&str] = &[
    "app_info",
    "clipboard_write",
    "open_external",
    "open_log_folder",
    "connection_list",
    "connection_test",
    "connection_save",
    "connection_remove",
    "connection_set_active",
    "connection_add_bucket",
    "connection_remove_bucket",
    "connection_set_public_url",
    "connection_toggle_favorite",
    "bucket_list",
    "object_list",
    "object_search",
    "object_head",
    "prefix_size",
    "folder_create",
    "preview_text",
    "media_token",
    "delete_preview",
    "delete_execute",
    "move_preview",
    "move_execute",
    "pick_upload_files",
    "pick_upload_folder",
    "upload_prepare",
    "upload_start",
    "download_start",
    "transfer_subscribe",
    "transfer_control",
    "transfer_control_many",
    "transfer_pause_all",
    "transfer_clear_finished",
    "transfer_history",
    "transfer_show",
    "settings_get",
    "settings_set",
    "settings_choose_download_dir",
    "settings_reset",
    "history_clear",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(e) = tauri_build::try_build(attributes) {
        println!("cargo:warning=tauri build failed: {e}");
        std::process::exit(1);
    }
}

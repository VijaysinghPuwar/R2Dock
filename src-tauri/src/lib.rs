//! R2Dock desktop app: wiring between Tauri, the services, and the webview.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod activity;
pub mod commands;
pub mod compression;
pub mod connections;
mod dialogs;
pub mod dragdrop;
pub mod dto;
pub mod logging;
pub mod media;
#[cfg(target_os = "macos")]
mod menu;
pub mod planning;
pub mod state;

use std::sync::Arc;

use parking_lot::RwLock;
use r2dock_core::settings::SettingsFile;
use r2dock_core::storage::read_json_or_default;
use r2dock_secrets::{KeyringStore, SecretStore};
use r2dock_transfer::{Engine, EngineConfig};
use tauri::{Manager, RunEvent};

use crate::connections::ConnectionService;
use crate::state::{AppState, EngineClients, Paths, Registry};

/// Handle for changing the log level at runtime.
pub struct LogHandle(pub Option<logging::ReloadHandle>);

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let resolver = app.path();
    let paths = Paths {
        config: resolver.app_config_dir()?,
        data: resolver.app_data_dir()?,
        cache: resolver.app_cache_dir()?,
        logs: resolver.app_log_dir()?,
        home: resolver.home_dir().ok(),
        downloads: resolver.download_dir().ok(),
    };
    std::fs::create_dir_all(&paths.config)?;
    std::fs::create_dir_all(&paths.data)?;

    let settings: SettingsFile =
        read_json_or_default::<SettingsFile>(&paths.config.join("settings.json"))
            .unwrap_or_default()
            .migrate();
    let log = logging::init(&paths.logs, settings.preferences.log_level);
    tracing::info!("R2Dock {} starting", app.package_info().version);

    let keyring = KeyringStore::new(r2dock_core::CREDENTIAL_SERVICE);
    let keychain: Option<Arc<dyn SecretStore>> = if keyring.probe() {
        Some(Arc::new(keyring))
    } else {
        tracing::warn!(
            "system credential store unavailable; secrets will be kept for this session only"
        );
        None
    };
    let credential_store_label = match &keychain {
        Some(k) => k.backend().label().to_string(),
        None => r2dock_secrets::Backend::SessionMemory.label().to_string(),
    };
    // Debug builds only: point every connection at a local S3 server for
    // manual end-to-end testing. Compiled out of release builds.
    #[cfg(debug_assertions)]
    let endpoint_override = std::env::var("R2DOCK_DEV_ENDPOINT")
        .ok()
        .filter(|e| e.starts_with("http://127.0.0.1:"));
    #[cfg(not(debug_assertions))]
    let endpoint_override = None;
    let connections = Arc::new(ConnectionService::load(
        paths.config.join("connections.json"),
        keychain,
        endpoint_override,
    ));
    // Delete secrets left by connections that were removed or replaced
    // while the credential store could not delete them.
    {
        let connections = connections.clone();
        tauri::async_runtime::spawn(async move {
            connections.cleanup_secrets().await;
        });
    }

    let mut config = EngineConfig::new(paths.data.clone());
    config.concurrency = usize::from(settings.preferences.concurrency);
    config.multipart_threshold =
        u64::from(settings.preferences.multipart_threshold_mib) * r2dock_core::parts::MIB;
    config.home = paths.home.clone();
    let engine = {
        let runtime = tauri::async_runtime::handle();
        let _guard = runtime.inner().enter();
        Engine::start(Arc::new(EngineClients(connections.clone())), config)
    };
    let activity_dir = settings
        .activity_dir
        .clone()
        .unwrap_or_else(|| paths.logs.join("activity"));
    let activity = activity::ActivityLog::start(
        activity_dir,
        settings.preferences.activity_log,
        settings.preferences.activity_include_names,
    );
    {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "version".into(),
            app.package_info().version.to_string().into(),
        );
        fields.insert("os".into(), std::env::consts::OS.into());
        fields.insert("arch".into(), std::env::consts::ARCH.into());
        fields.insert(
            "credential_store".into(),
            credential_store_label.clone().into(),
        );
        activity.event("app_start", fields);
    }
    {
        let log = activity.clone();
        engine.observe_compression(Arc::new(move |e| log.compression(&e)));
    }
    let activity_days = settings.preferences.activity_days;
    let trim_activity = activity.clone();
    tauri::async_runtime::spawn_blocking(move || trim_activity.trim(activity_days));
    let history_days = settings.preferences.history_days;
    let trim = engine.clone();
    tauri::async_runtime::spawn_blocking(move || trim.trim_history(history_days));

    let compression = compression::CompressionService::new(
        paths.data.join("tools"),
        paths.cache.join("compress"),
    );
    // Copies left by a crash, before any encode can start.
    {
        let sweep = engine.clone();
        let work = compression.work_dir.clone();
        tauri::async_runtime::spawn_blocking(move || sweep.sweep_work_dir(&work));
    }
    app.manage(AppState {
        paths,
        settings: RwLock::new(settings),
        connections,
        engine,
        selections: Registry::default(),
        uploads: Registry::default(),
        deletes: Registry::default(),
        moves: Registry::default(),
        media: media::MediaRegistry::default(),
        credential_store_label,
        activity,
        compression,
    });
    app.manage(LogHandle(log));
    // Find and check the compression tools in the background.
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<AppState>();
        state.compression.refresh(&state).await;
    });

    #[cfg(not(debug_assertions))]
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_title(r2dock_core::APP_NAME);
    }
    // Never leave the window hidden if the page-load event does not arrive.
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if let Some(w) = handle.get_webview_window("main")
            && !w.is_visible().unwrap_or(true)
        {
            let _ = w.show();
        }
    });
    Ok(())
}

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("r2dock-media", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(media::serve(&app, request).await);
            });
        })
        .setup(|app| setup(app))
        // The window starts hidden (tauri.conf.json) and is shown once the
        // page has loaded, so Windows never flashes an empty white window.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = webview.window().show();
            }
        })
        .on_window_event(dragdrop::handle);
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(menu::handle);
    let builder = builder.invoke_handler(tauri::generate_handler![
        commands::app::app_info,
        commands::app::clipboard_write,
        commands::app::open_external,
        commands::app::open_log_folder,
        commands::connections::connection_list,
        commands::connections::connection_test,
        commands::connections::connection_save,
        commands::connections::connection_remove,
        commands::connections::connection_set_active,
        commands::connections::connection_add_bucket,
        commands::connections::connection_remove_bucket,
        commands::connections::connection_set_public_url,
        commands::connections::connection_toggle_favorite,
        commands::browse::bucket_list,
        commands::browse::object_list,
        commands::browse::object_search,
        commands::browse::object_head,
        commands::browse::prefix_size,
        commands::browse::folder_create,
        commands::browse::preview_text,
        commands::browse::media_token,
        commands::manage::delete_preview,
        commands::manage::delete_execute,
        commands::manage::move_preview,
        commands::manage::move_execute,
        commands::transfers::pick_upload_files,
        commands::transfers::pick_upload_folder,
        commands::transfers::upload_prepare,
        commands::transfers::upload_start,
        commands::transfers::download_start,
        commands::transfers::transfer_subscribe,
        commands::transfers::transfer_control,
        commands::transfers::transfer_control_many,
        commands::transfers::transfer_pause_all,
        commands::transfers::transfer_clear_finished,
        commands::transfers::transfer_history,
        commands::transfers::transfer_show,
        commands::settings::settings_get,
        commands::settings::settings_set,
        commands::settings::settings_choose_download_dir,
        commands::settings::settings_reset,
        commands::settings::history_clear,
        commands::settings::settings_choose_activity_dir,
        commands::settings::open_activity_folder,
        commands::settings::activity_note,
        commands::settings::activity_reset_dir,
        commands::settings::log_copy_path,
        commands::settings::activity_export_csv,
        commands::compression::compression_status,
        commands::compression::compression_install,
        commands::compression::compression_cancel_install,
        commands::compression::compression_remove,
        commands::compression::compression_use_system,
        commands::compression::compression_use_downloaded,
        commands::compression::compression_preview,
    ]);

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("R2Dock could not start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|handle, event| {
        if let RunEvent::Exit = event
            && let Some(state) = handle.try_state::<AppState>()
        {
            // Keep resumable transfers for the next launch.
            state.engine.flush();
        }
    });
}

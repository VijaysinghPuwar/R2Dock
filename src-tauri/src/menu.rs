//! macOS app menu: Tauri's default menu plus "Open Activity Log Folder" in
//! Help. Windows and Linux have no menu bar; the same action is in the
//! keyboard shortcut sheet.

use tauri::menu::{HELP_SUBMENU_ID, Menu, MenuEvent, MenuItem};
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

const OPEN_ACTIVITY: &str = "r2dock-open-activity-folder";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    if let Some(help) = menu
        .get(HELP_SUBMENU_ID)
        .and_then(|m| m.as_submenu().cloned())
    {
        let item = MenuItem::with_id(
            app,
            OPEN_ACTIVITY,
            "Open Activity Log Folder",
            true,
            None::<&str>,
        )?;
        help.append(&item)?;
    }
    Ok(menu)
}

pub fn handle<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() != OPEN_ACTIVITY {
        return;
    }
    if let Some(state) = app.try_state::<AppState>()
        && let Err(e) = crate::commands::settings::open_dir(app, &state.activity_dir())
    {
        tracing::warn!("could not open the activity folder: {}", e.message);
    }
}

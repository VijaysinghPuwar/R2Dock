//! Bridges OS drag and drop to the webview. Dropped paths stay in Rust and
//! are registered as an opaque selection.

use tauri::{DragDropEvent, Emitter, Manager, Window, WindowEvent};

use crate::commands::transfers::register_selection;
use crate::dto::DragEvent;
use crate::state::AppState;

pub const DRAG_EVENT: &str = "r2dock://drag";

pub fn handle(window: &Window, event: &WindowEvent) {
    let WindowEvent::DragDrop(dd) = event else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0).max(0.1);
    let payload = match dd {
        DragDropEvent::Enter { paths, position } => DragEvent::Enter {
            count: u32::try_from(paths.len()).unwrap_or(u32::MAX),
            x: position.x / scale,
            y: position.y / scale,
        },
        DragDropEvent::Over { position } => DragEvent::Over {
            x: position.x / scale,
            y: position.y / scale,
        },
        DragDropEvent::Drop { paths, position } => {
            if paths.is_empty() {
                DragEvent::Leave
            } else {
                let state = window.state::<AppState>();
                DragEvent::Drop {
                    selection: register_selection(&state, paths.clone()),
                    x: position.x / scale,
                    y: position.y / scale,
                }
            }
        }
        DragDropEvent::Leave => DragEvent::Leave,
        _ => return,
    };
    let _ = window.emit(DRAG_EVENT, payload);
}

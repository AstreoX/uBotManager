//! Events to the frontend (SPEC §7) and the main window's chrome.

use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use ubot_core::model::{Snapshot, Task, TaskLogEvent};
use ubot_core::TaskSink;

pub const SNAPSHOT_EVENT: &str = "ubot://snapshot";
pub const TASK_EVENT: &str = "ubot://task";
pub const TASK_LOG_EVENT: &str = "ubot://task-log";

pub const MAIN_WINDOW: &str = "main";
pub const APP_NAME: &str = "uBot Manager";

/// The title last given to the main window, so repeated snapshots do not re-set it. The window
/// starts with the title from tauri.conf.json.
pub struct WindowTitle(Mutex<String>);

impl Default for WindowTitle {
    fn default() -> Self {
        WindowTitle(Mutex::new(APP_NAME.to_string()))
    }
}

/// Errors nobody waits for. Release builds have no console, so this only helps `tauri dev`.
pub fn log_error(message: &str) {
    eprintln!("uBot Manager: {message}");
}

/// Reports an error the user has to see (no frontend call is waiting for it).
pub fn show_error(app: &AppHandle, message: &str) {
    log_error(message);
    let mut dialog = app
        .dialog()
        .message(message)
        .title(APP_NAME)
        .kind(MessageDialogKind::Error);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        dialog = dialog.parent(&window);
    }
    dialog.show(|_| {});
}

fn emit<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    if let Err(error) = app.emit(event, payload) {
        log_error(&format!("Cannot emit {event}: {error}"));
    }
}

/// "<Project> - uBot Manager" while a project is open (the taskbar shows it; the custom title bar
/// does not).
pub fn window_title(project_name: Option<&str>) -> String {
    match project_name {
        Some(name) => format!("{name} - {APP_NAME}"),
        None => APP_NAME.to_string(),
    }
}

pub fn set_window_title(app: &AppHandle, project_name: Option<&str>) {
    let title = window_title(project_name);
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let state = app.state::<WindowTitle>();
    let mut current = state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *current != title {
        match window.set_title(&title) {
            Ok(()) => *current = title,
            Err(error) => log_error(&format!("Cannot set the window title: {error}")),
        }
    }
}

/// The project name (`.uproject` file stem) of a project path.
pub fn project_name(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

/// Keeps the window title in step with a snapshot a command returns to the frontend.
pub fn snapshot_returned(app: &AppHandle, snapshot: &Snapshot) {
    set_window_title(
        app,
        snapshot
            .project
            .as_ref()
            .map(|project| project.name.as_str()),
    );
}

/// A snapshot from a background refresh: `ubot://snapshot` plus the window title.
pub fn publish_snapshot(app: &AppHandle, snapshot: &Snapshot) {
    snapshot_returned(app, snapshot);
    emit(app, SNAPSHOT_EVENT, snapshot);
}

pub fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Forwards a task's progress to the frontend. The core already batches log lines per step (one
/// call at most every 100 ms), so each call becomes one `ubot://task-log` event.
pub struct EventSink {
    app: AppHandle,
}

impl EventSink {
    pub fn new(app: AppHandle) -> Self {
        EventSink { app }
    }
}

impl TaskSink for EventSink {
    fn task_updated(&self, task: &Task) {
        emit(&self.app, TASK_EVENT, task);
    }

    fn log(&self, task_id: &str, step_id: &str, lines: &[String]) {
        emit(
            &self.app,
            TASK_LOG_EVENT,
            TaskLogEvent {
                task_id: task_id.to_string(),
                step_id: step_id.to_string(),
                lines: lines.to_vec(),
            },
        );
    }

    fn snapshot_updated(&self, snapshot: &Snapshot) {
        publish_snapshot(&self.app, snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(window_title(None), "uBot Manager");
        assert_eq!(window_title(Some("UBotDev")), "UBotDev - uBot Manager");
        assert_eq!(
            project_name(Path::new("E:/Sims/My Sim.uproject")).as_deref(),
            Some("My Sim")
        );
    }
}

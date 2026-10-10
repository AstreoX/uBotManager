//! uBot Manager desktop app (SPEC §12): serves the web frontend in `app/` and forwards its commands
//! to [`ubot_core::Manager`].
//!
//! `ubot-manager.exe [--project <path>]`. A second launch hands its `--project` to the running
//! instance, which opens it and comes to the front.

// Release builds are GUI apps without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod args;
#[cfg(test)]
mod capabilities;
mod commands;
#[cfg(test)]
mod contract;
mod events;
mod registry;
mod startup;

use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager as _, RunEvent};
use ubot_core::{Manager, ManagerOptions};

use commands::AppState;
use startup::Startup;

/// How long closing the app waits for a canceled task to stop its processes and clean up.
const CANCEL_ON_EXIT_TIMEOUT: Duration = Duration::from_secs(10);

fn main() {
    let project = args::project_arg(std::env::args().skip(1));

    let app = tauri::Builder::default()
        // First, so a second instance exits before doing anything else.
        .plugin(tauri_plugin_single_instance::init(on_second_instance))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(events::WindowTitle::default())
        .setup(move |app| {
            let manager = Manager::new(ManagerOptions::default());
            let startup = Arc::new(Startup::default());
            app.manage(AppState {
                manager: manager.clone(),
                startup: startup.clone(),
            });
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("ubot-startup".into())
                .spawn(move || startup::run(&handle, &manager, &startup, project))?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::refresh,
            commands::open_project,
            commands::plan,
            commands::apply,
            commands::cancel_task,
            commands::list_tasks,
            commands::get_task_log,
            commands::close_editor,
            commands::ignore_problem,
            commands::get_settings,
            commands::save_settings,
            commands::test_git,
            commands::test_ros,
            commands::list_engines,
            commands::check_environment,
            commands::recipe_changes,
        ])
        .build(tauri::generate_context!())
        .expect("uBot Manager failed to start");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            cancel_running_task(app);
        }
    });
}

/// A second `ubot-manager.exe` was started: open its `--project` here and come to the front.
fn on_second_instance(app: &AppHandle, argv: Vec<String>, cwd: String) {
    events::focus_main_window(app);
    let Some(project) = args::project_arg(argv.into_iter().skip(1)) else {
        return;
    };
    let path = args::resolve_against(&project, Path::new(&cwd));
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let manager = state.manager.clone();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || match manager.open_project(&path) {
        Ok(snapshot) => events::publish_snapshot(&app, &snapshot),
        Err(error) => events::show_error(&app, &error),
    });
}

/// Closing the window while a task runs cancels it, so no git or build process outlives the app
/// and a half-cloned folder is removed.
fn cancel_running_task(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let manager = state.manager.clone();
    let Some(task) = manager
        .list_tasks()
        .into_iter()
        .find(|task| !task.status.is_finished())
    else {
        return;
    };
    if manager.cancel_task(&task.id).is_err() {
        return;
    }
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        manager.wait_for_task(&task.id);
        let _ = done.send(());
    });
    let _ = finished.recv_timeout(CANCEL_ON_EXIT_TIMEOUT);
}

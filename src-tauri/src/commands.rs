//! The Tauri commands of SPEC §7, one per row, delegating to [`ubot_core::Manager`]. JS calls them
//! with camelCase argument names (`taskId`, `saveRecipe`, …), which Tauri maps to these snake_case
//! parameters. Errors reject the JS promise with an English string.
//!
//! Every command is async so it never runs on the main thread; the manager's blocking calls (file
//! scans, git, network, processes) run on the blocking thread pool.

use std::sync::Arc;

use tauri::{AppHandle, State};
use ubot_core::model::{
    ApplyOptions, Change, EngineInfo, EnvCheck, GitTestResult, Msg, Plan, RosTestResult, Settings,
    Snapshot, Task,
};
use ubot_core::Manager;

use crate::events::{self, EventSink};
use crate::startup::{Phase, Startup};

pub struct AppState {
    pub manager: Manager,
    pub startup: Arc<Startup>,
}

type CommandResult<T> = Result<T, String>;

async fn blocking<T, F>(work: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CommandResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("Internal error: {error}"))?
}

/// Runs `work` with the manager on the blocking pool.
async fn with_manager<T, F>(state: State<'_, AppState>, work: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce(Manager) -> CommandResult<T> + Send + 'static,
{
    let manager = state.manager.clone();
    blocking(move || work(manager)).await
}

/// A snapshot going back to the frontend also renames the window when the project changed.
fn returning(app: &AppHandle, snapshot: CommandResult<Snapshot>) -> CommandResult<Snapshot> {
    if let Ok(snapshot) = &snapshot {
        events::snapshot_returned(app, snapshot);
    }
    snapshot
}

#[tauri::command]
pub async fn get_snapshot(app: AppHandle, state: State<'_, AppState>) -> CommandResult<Snapshot> {
    let startup = state.startup.clone();
    let snapshot = with_manager(state, move |manager| {
        // The startup scan is about to produce the first snapshot; do not start another one.
        startup.wait_for(Phase::Refreshing);
        Ok(manager.get_snapshot())
    })
    .await;
    returning(&app, snapshot)
}

#[tauri::command]
pub async fn refresh(
    app: AppHandle,
    state: State<'_, AppState>,
    sync: bool,
) -> CommandResult<Snapshot> {
    let startup = state.startup.clone();
    let snapshot = with_manager(state, move |manager| {
        if sync && startup.claim_sync() {
            // The frontend's startup sync: the startup refresh is fetching the sources already.
            startup.wait_for(Phase::Done);
            return Ok(manager.get_snapshot());
        }
        manager.refresh(sync)
    })
    .await;
    returning(&app, snapshot)
}

#[tauri::command]
pub async fn open_project(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<Snapshot> {
    let snapshot = with_manager(state, move |manager| manager.open_project(&path)).await;
    returning(&app, snapshot)
}

#[tauri::command]
pub async fn plan(
    state: State<'_, AppState>,
    changes: Vec<Change>,
    options: ApplyOptions,
) -> CommandResult<Plan> {
    with_manager(state, move |manager| manager.plan(&changes, &options)).await
}

#[tauri::command]
pub async fn apply(
    app: AppHandle,
    state: State<'_, AppState>,
    changes: Vec<Change>,
    options: ApplyOptions,
    title: Option<Msg>,
    save_recipe: Option<String>,
) -> CommandResult<String> {
    let sink = Arc::new(EventSink::new(app));
    with_manager(state, move |manager| {
        manager.apply(&changes, &options, title, save_recipe, sink)
    })
    .await
}

#[tauri::command]
pub async fn cancel_task(state: State<'_, AppState>, task_id: String) -> CommandResult<()> {
    state.manager.cancel_task(&task_id)
}

#[tauri::command]
pub async fn list_tasks(state: State<'_, AppState>) -> CommandResult<Vec<Task>> {
    Ok(state.manager.list_tasks())
}

#[tauri::command]
pub async fn get_task_log(
    state: State<'_, AppState>,
    task_id: String,
    step_id: Option<String>,
) -> CommandResult<Vec<String>> {
    state.manager.get_task_log(&task_id, step_id.as_deref())
}

#[tauri::command]
pub async fn close_editor(state: State<'_, AppState>) -> CommandResult<()> {
    with_manager(state, |manager| manager.close_editor()).await
}

#[tauri::command]
pub async fn ignore_problem(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Snapshot> {
    let snapshot = with_manager(state, move |manager| manager.ignore_problem(&id)).await;
    returning(&app, snapshot)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CommandResult<Settings> {
    with_manager(state, |manager| Ok(manager.get_settings())).await
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> CommandResult<Snapshot> {
    let snapshot = with_manager(state, move |manager| manager.save_settings(settings)).await;
    returning(&app, snapshot)
}

#[tauri::command]
pub async fn test_git(state: State<'_, AppState>, path: String) -> CommandResult<GitTestResult> {
    with_manager(state, move |manager| Ok(manager.test_git(&path))).await
}

#[tauri::command]
pub async fn test_ros(
    state: State<'_, AppState>,
    host: String,
    port: u16,
) -> CommandResult<RosTestResult> {
    with_manager(state, move |manager| Ok(manager.test_ros(&host, port))).await
}

#[tauri::command]
pub async fn list_engines(state: State<'_, AppState>) -> CommandResult<Vec<EngineInfo>> {
    with_manager(state, |manager| Ok(manager.list_engines())).await
}

#[tauri::command]
pub async fn check_environment(
    state: State<'_, AppState>,
    checks: Vec<String>,
) -> CommandResult<Vec<EnvCheck>> {
    with_manager(state, move |manager| Ok(manager.check_environment(&checks))).await
}

#[tauri::command]
pub async fn recipe_changes(
    state: State<'_, AppState>,
    recipe_id: Option<String>,
    packages: Vec<String>,
) -> CommandResult<Vec<Change>> {
    with_manager(state, move |manager| {
        Ok(manager.recipe_changes(recipe_id.as_deref(), &packages))
    })
    .await
}

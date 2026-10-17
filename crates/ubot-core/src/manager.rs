//! The facade used by the Tauri shell and the CLI. Its public methods map 1:1 to the commands of
//! SPEC §7. `Manager` is cheap to clone, `Send + Sync`, and synchronizes internally.
//!
//! Errors are English text. Methods that do I/O (`refresh`, `open_project`, `plan`, `apply`,
//! `save_settings`, the `test_*` and `check_environment` calls) block the calling thread and
//! should be called off the UI thread; `apply` returns as soon as the task has started and runs
//! the task on its own thread.
//!
//! ```no_run
//! use std::sync::Arc;
//! use ubot_core::model::{ApplyOptions, Change, ChangeKind, Snapshot, Task};
//! use ubot_core::{Manager, ManagerOptions, TaskSink};
//!
//! /// Forwards task progress to the windows as `ubot://task`, `ubot://task-log` and
//! /// `ubot://snapshot`.
//! struct Events;
//!
//! impl TaskSink for Events {
//!     fn task_updated(&self, task: &Task) { /* emit "ubot://task" */ }
//!     fn log(&self, task_id: &str, step_id: &str, lines: &[String]) { /* emit "ubot://task-log" */ }
//!     fn snapshot_updated(&self, snapshot: &Snapshot) { /* emit "ubot://snapshot" */ }
//! }
//!
//! let manager = Manager::new(ManagerOptions::default()); // opens settings.lastProject
//! let sync = manager.get_settings().index.sync_on_start;
//! let snapshot = manager.refresh(sync)?;
//! let changes = [Change::new(ChangeKind::Install, "UBotROS", None)];
//! let options = ApplyOptions { build: true, reopen_editor: true, run_tests: false };
//! let task_id = manager.apply(&changes, &options, None, None, Arc::new(Events))?;
//! # Ok::<(), String>(())
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use crate::editor;
use crate::engine::{self, EnginePlugins, EngineRegistry, SystemEngineRegistry};
use crate::env;
use crate::exec::{self, notify, ExecContext, RecycleBin, Remover, TaskSink, TaskStore};
use crate::git;
use crate::index::MergedIndex;
use crate::model::{
    ApplyOptions, Change, EngineInfo, EnvCheck, GitTestResult, IndexSourceConfig, Msg, Plan,
    RosTestResult, Settings, Snapshot, StepStatus, Task, UpdateInfo,
};
use crate::msg::{msg, msg0, render_en};
use crate::paths::{self, AppDirs};
use crate::plan;
use crate::problems::{self, ComputedProblem};
use crate::project::Project;
use crate::recipes;
use crate::settings::{self, SettingsStore};
use crate::sources::{self, Fetcher, HttpFetcher, LoadContext};
use crate::status;
use crate::update;
use crate::workspace::{ProjectContext, RosCheck, Workspace};

pub type ApiResult<T> = Result<T, String>;

/// How a [`Manager`] is set up. `Default` is what the desktop app uses.
#[derive(Clone)]
pub struct ManagerOptions {
    pub dirs: AppDirs,
    /// Never fetch remote sources; caches only (CLI `--offline`).
    pub offline: bool,
    /// Replaces the configured index sources (CLI `--index`): files or URLs.
    pub index_override: Option<Vec<String>>,
    /// Write the merged index to `<Project>/Saved/uBot/index.json` after each refresh.
    pub write_merged_index: bool,
    pub remover: Arc<dyn Remover>,
    pub fetcher: Arc<dyn Fetcher>,
    pub engine_registry: Arc<dyn EngineRegistry + Send + Sync>,
    /// Whether a C++ compiler (Visual Studio 2022) is installed; a plan with a build step warns
    /// `plan.noCompiler` without one.
    pub compiler_found: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Default for ManagerOptions {
    fn default() -> Self {
        ManagerOptions {
            dirs: AppDirs::from_env(),
            offline: false,
            index_override: None,
            write_merged_index: true,
            remover: Arc::new(RecycleBin),
            fetcher: Arc::new(HttpFetcher),
            engine_registry: Arc::new(SystemEngineRegistry),
            compiler_found: Arc::new(env::vs2022_found),
        }
    }
}

struct State {
    settings: SettingsStore,
    project_path: Option<PathBuf>,
    workspace: Arc<Workspace>,
    /// Problems before ignored ones are removed.
    problems: Vec<ComputedProblem>,
    snapshot: Option<Snapshot>,
    failed_fetches: HashMap<String, String>,
    engine_plugins: HashMap<String, Arc<EnginePlugins>>,
}

struct RunningTask {
    id: String,
    cancel: Arc<AtomicBool>,
}

struct Inner {
    options: ManagerOptions,
    state: Mutex<State>,
    tasks: Arc<Mutex<TaskStore>>,
    running: Mutex<Option<RunningTask>>,
    threads: Mutex<HashMap<String, JoinHandle<()>>>,
    /// Syncs (rebuilds that re-fetch the remote sources) in progress.
    syncing: AtomicUsize,
    /// Serializes the part of a rebuild that reads and replaces the workspace. Never held while
    /// waiting for the network: the downloads are made first (see `prefetch`).
    refresh_lock: Mutex<()>,
}

/// Counts a sync in progress for as long as it lives.
struct SyncGuard<'a>(&'a AtomicUsize);

impl<'a> SyncGuard<'a> {
    fn new(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        SyncGuard(counter)
    }
}

impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What a rebuild fetched or measured before it took the lock.
#[derive(Default)]
struct Prefetched {
    sources: HashMap<String, Result<String, String>>,
    /// The ROS bridge check of a project.
    ros: Option<(PathBuf, RosCheck)>,
}

/// UBotROS is installed and enabled and the check is switched on.
fn ros_check_wanted(project: &Project, settings: &Settings) -> bool {
    settings.ros.check_on_refresh
        && project
            .find_plugin("UBotROS")
            .filter(|plugin| plugin.is_ubot())
            .is_some_and(|plugin| project.is_plugin_enabled(plugin))
}

/// The TCP check of the bridge at the project's address (`Config/DefaultUBotROS.ini`, else the
/// settings). Takes up to 2 s.
fn run_ros_check(project: &Project, settings: &Settings) -> RosCheck {
    let (host, port) = settings::read_ros_ini(&project.dir);
    let host = host.unwrap_or_else(|| settings.ros.host.clone());
    let port = port.unwrap_or(settings.ros.port);
    let result = env::test_ros(&host, port);
    RosCheck {
        address: env::ros_address(&host, port),
        reachable: result.ok,
    }
}

#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

/// What a rebuild of the workspace should do beyond reading local files.
#[derive(Clone, Copy)]
struct Rebuild {
    sync: bool,
    ros_check: bool,
    /// No fetch at all, not even for sources that were never cached (a refresh follows).
    defer_fetch: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Manager {
    /// Loads settings and task history. The last project (if it still exists) becomes the open
    /// project; nothing is scanned until the first `get_snapshot` / `refresh`.
    pub fn new(options: ManagerOptions) -> Manager {
        let settings = SettingsStore::load(&options.dirs);
        let project_path = settings
            .settings
            .last_project
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.is_file());
        let tasks = TaskStore::load(options.dirs.tasks_file());
        Manager {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    settings,
                    project_path,
                    workspace: Arc::new(Workspace::default()),
                    problems: Vec::new(),
                    snapshot: None,
                    failed_fetches: HashMap::new(),
                    engine_plugins: HashMap::new(),
                }),
                options,
                tasks: Arc::new(Mutex::new(tasks)),
                running: Mutex::new(None),
                threads: Mutex::new(HashMap::new()),
                syncing: AtomicUsize::new(0),
                refresh_lock: Mutex::new(()),
            }),
        }
    }

    /// The open project's `.uproject`, if any.
    pub fn project_path(&self) -> Option<PathBuf> {
        lock(&self.inner.state).project_path.clone()
    }

    /// `get_snapshot`: the cached snapshot with the editor state re-checked. Computes one (without
    /// syncing) when there is none yet.
    pub fn get_snapshot(&self) -> Snapshot {
        let cached = lock(&self.inner.state).snapshot.clone();
        let mut snapshot = match cached {
            Some(snapshot) => snapshot,
            None => match self.refresh(false) {
                Ok(snapshot) => return snapshot,
                Err(_) => Snapshot::default(),
            },
        };
        if let Some(project) = snapshot.project.as_mut() {
            project.editor = editor::find_editor(Path::new(&project.path));
        }
        snapshot.syncing = self.inner.syncing.load(Ordering::SeqCst) > 0;
        snapshot
    }

    /// `refresh`: rescans the project; `sync` re-fetches remote index sources.
    pub fn refresh(&self, sync: bool) -> ApiResult<Snapshot> {
        self.rebuild(Rebuild {
            sync,
            ros_check: true,
            defer_fetch: false,
        })
    }

    /// `open_project`: opens a `.uproject`, adds it to the recent projects and makes it the last project.
    pub fn open_project(&self, path: &str) -> ApiResult<Snapshot> {
        self.open_project_with(path, false)
    }

    /// [`Manager::open_project`] with a choice to re-fetch remote index sources in the same scan.
    pub fn open_project_with(&self, path: &str, sync: bool) -> ApiResult<Snapshot> {
        self.select_project(path)?;
        self.refresh(sync)
    }

    /// [`Manager::open_project`] without the scan: checks the `.uproject`, makes it the open
    /// project, adds it to the recent projects and makes it the last project. The next
    /// `refresh` or [`Manager::rescan`] scans it.
    pub fn select_project(&self, path: &str) -> ApiResult<()> {
        // A task works on the project it was planned for and records its result for the open
        // one; switching meanwhile would mix the two up.
        if lock(&self.inner.running).is_some() {
            return Err(render_en(&msg0("task.busy")));
        }
        let path = paths::absolute(Path::new(path.trim()));
        Project::open(&path).map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
        let mut state = lock(&self.inner.state);
        // All or nothing: a caller that is told "no" keeps working on the project it had, so
        // plan, apply and refresh must not move to the new one meanwhile.
        let previous_path = state.project_path.replace(path.clone());
        let previous_snapshot = state.snapshot.take();
        let previous_recent = state.settings.settings.recent_projects.clone();
        let previous_last = state.settings.settings.last_project.clone();
        state.settings.add_recent(&path);
        if let Err(error) = state.settings.save() {
            state.project_path = previous_path;
            state.snapshot = previous_snapshot;
            state.settings.settings.recent_projects = previous_recent;
            state.settings.settings.last_project = previous_last;
            return Err(format!("Cannot save settings: {error}"));
        }
        Ok(())
    }

    /// A quick rescan from local files and caches, without network access: unlike
    /// `refresh(false)` it neither fetches sources that were never cached (they show as "not
    /// fetched yet") nor checks the ROS bridge again (the last result is kept). The desktop app
    /// shows this first at startup and runs the full `refresh` right after it.
    pub fn rescan(&self) -> ApiResult<Snapshot> {
        self.rebuild(Rebuild {
            sync: false,
            ros_check: false,
            defer_fetch: true,
        })
    }

    /// `plan`: plans `changes` against a fresh scan of the project (no network).
    pub fn plan(&self, changes: &[Change], options: &ApplyOptions) -> ApiResult<Plan> {
        let workspace = self.light_rebuild()?;
        Ok(self.prepare(&workspace, changes, options).plan)
    }

    /// `apply`: re-plans and starts the task; returns its id. Fails when another task runs or the
    /// plan has errors. `title` defaults to "Apply Changes · <project>"; with `save_recipe` the
    /// enabled package set is saved as a user recipe of that name when the task succeeds.
    pub fn apply(
        &self,
        changes: &[Change],
        options: &ApplyOptions,
        title: Option<Msg>,
        save_recipe: Option<String>,
        sink: Arc<dyn TaskSink>,
    ) -> ApiResult<String> {
        if lock(&self.inner.running).is_some() {
            return Err(render_en(&msg0("task.busy")));
        }
        let workspace = self.light_rebuild()?;
        let prepared = self.prepare(&workspace, changes, options);
        if !prepared.plan.errors.is_empty() {
            return Err(prepared
                .plan
                .errors
                .iter()
                .map(render_en)
                .collect::<Vec<_>>()
                .join("\n"));
        }

        let settings = lock(&self.inner.state).settings.settings.clone();
        let title =
            title.unwrap_or_else(|| msg("task.apply", &[("project", &prepared.project_name)]));
        let id = exec::new_task_id();
        let record = exec::new_task(id.clone(), title, &prepared);
        let context = ExecContext {
            git: settings.install.git.clone(),
            wait_for_editor: settings.build.wait_for_editor,
            remover: self.inner.options.remover.clone(),
            editor_running: Arc::new(|project: &Path| editor::find_editor(project).running),
        };

        let mut running = lock(&self.inner.running);
        if running.is_some() {
            return Err(render_en(&msg0("task.busy")));
        }
        let manager = self.clone();
        let finish_sink = sink.clone();
        let project_path = prepared.project_path.clone();
        let on_finish = Box::new(move |task: &Task| {
            manager.task_finished(task, &project_path, save_recipe, finish_sink)
        });
        let handle = exec::start(
            self.inner.tasks.clone(),
            record,
            prepared,
            context,
            sink,
            on_finish,
        );
        *running = Some(RunningTask {
            id: id.clone(),
            cancel: handle.cancel.clone(),
        });
        let mut threads = lock(&self.inner.threads);
        threads.retain(|_, thread| !thread.is_finished());
        threads.insert(id.clone(), handle.thread);
        Ok(id)
    }

    fn prepare(
        &self,
        workspace: &Workspace,
        changes: &[Change],
        options: &ApplyOptions,
    ) -> plan::PreparedPlan {
        let compiler_found = self.inner.options.compiler_found.clone();
        plan::plan_with(workspace, changes, options, &move || compiler_found())
    }

    fn task_finished(
        &self,
        task: &Task,
        project_path: &Path,
        save_recipe: Option<String>,
        sink: Arc<dyn TaskSink>,
    ) {
        let mut snapshot = self.refresh(false).ok();
        let recipe_name = save_recipe.filter(|name| !name.trim().is_empty());
        if let (StepStatus::Done, Some(name), Some(_)) = (task.status, recipe_name, &snapshot) {
            {
                let mut state = lock(&self.inner.state);
                // The recipe is the package set of the project the task changed. The workspace
                // is that project's only while it is still the open one.
                let still_open = state
                    .project_path
                    .as_deref()
                    .is_some_and(|open| paths::same_path(open, project_path));
                if still_open {
                    let recipe = recipes::user_recipe(&state.workspace, &name);
                    state.settings.save_user_recipe(recipe);
                    let _ = state.settings.save();
                }
            }
            snapshot = self.refresh_cached();
        }
        *lock(&self.inner.running) = None;
        if let Some(snapshot) = snapshot {
            notify(|| sink.snapshot_updated(&snapshot));
        }
    }

    /// Recomputes the snapshot from the cached workspace (after settings-only changes).
    fn refresh_cached(&self) -> Option<Snapshot> {
        let mut state = lock(&self.inner.state);
        let workspace = state.workspace.clone();
        let workspace = Arc::new(Workspace {
            user_recipes: state.settings.user_recipes.clone(),
            ..(*workspace).clone()
        });
        state.workspace = workspace.clone();
        let snapshot = self.assemble(&state, &workspace);
        state.snapshot = Some(snapshot.clone());
        Some(snapshot)
    }

    /// Blocks until a task started by this manager has finished (including the refresh after it).
    pub fn wait_for_task(&self, task_id: &str) {
        let handle = lock(&self.inner.threads).remove(task_id);
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// `cancel_task`: kills the running step's process tree; the remaining steps are skipped.
    pub fn cancel_task(&self, task_id: &str) -> ApiResult<()> {
        if let Some(running) = lock(&self.inner.running)
            .as_ref()
            .filter(|running| running.id == task_id)
        {
            running.cancel.store(true, Ordering::SeqCst);
            return Ok(());
        }
        if lock(&self.inner.tasks).get(task_id).is_some() {
            return Ok(());
        }
        Err(format!("There is no task {task_id}."))
    }

    /// `list_tasks`: newest first.
    pub fn list_tasks(&self) -> Vec<Task> {
        lock(&self.inner.tasks).list()
    }

    /// `get_task_log`: a step's log lines, or the task summary lines when `step_id` is `None`.
    pub fn get_task_log(&self, task_id: &str, step_id: Option<&str>) -> ApiResult<Vec<String>> {
        lock(&self.inner.tasks)
            .log(task_id, step_id)
            .ok_or_else(|| format!("There is no task {task_id}."))
    }

    /// `close_editor`: asks the editor running the project to close (it may prompt to save).
    pub fn close_editor(&self) -> ApiResult<()> {
        match self.project_path() {
            Some(path) => editor::close_editor(&path),
            None => Err(render_en(&msg0("plan.noProject"))),
        }
    }

    /// `ignore_problem`: hides a problem for the open project.
    pub fn ignore_problem(&self, id: &str) -> ApiResult<Snapshot> {
        let mut state = lock(&self.inner.state);
        let Some(path) = state.project_path.clone() else {
            return Err(render_en(&msg0("plan.noProject")));
        };
        state.settings.ignore(&path, id);
        state
            .settings
            .save()
            .map_err(|error| format!("Cannot save settings: {error}"))?;
        let workspace = state.workspace.clone();
        let snapshot = self.assemble(&state, &workspace);
        state.snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// `get_settings`. With UBotROS installed in the open project, the ROS host and port are the
    /// project's (`Config/DefaultUBotROS.ini`).
    pub fn get_settings(&self) -> Settings {
        let state = lock(&self.inner.state);
        let mut settings = state.settings.settings.clone();
        if let Some(project_dir) = self.ros_project_dir(&state) {
            let (host, port) = settings::read_ros_ini(&project_dir);
            if let Some(host) = host {
                settings.ros.host = host;
            }
            if let Some(port) = port {
                settings.ros.port = port;
            }
        }
        settings
    }

    /// `save_settings`: stores the settings (ROS host/port go to the project when UBotROS is
    /// installed) and refreshes. `recentProjects` and `lastProject` belong to `open_project`; what
    /// the caller sends for them is ignored, so a copy of the settings loaded earlier cannot
    /// revert them.
    pub fn save_settings(&self, settings: Settings) -> ApiResult<Snapshot> {
        let effective = self.get_settings();
        {
            let mut state = lock(&self.inner.state);
            let mut settings = settings;
            if let Some(project_dir) = self.ros_project_dir(&state) {
                if settings.ros.host != effective.ros.host
                    || settings.ros.port != effective.ros.port
                {
                    settings::write_ros_ini(&project_dir, &settings.ros.host, settings.ros.port)
                        .map_err(|error| {
                            format!("Cannot write Config/DefaultUBotROS.ini: {error}")
                        })?;
                }
                settings.ros.host = state.settings.settings.ros.host.clone();
                settings.ros.port = state.settings.settings.ros.port;
            }
            settings.recent_projects = state.settings.settings.recent_projects.clone();
            settings.last_project = state.settings.settings.last_project.clone();
            state.settings.set(settings);
            state
                .settings
                .save()
                .map_err(|error| format!("Cannot save settings: {error}"))?;
        }
        self.refresh(false)
    }

    /// The project directory when UBotROS is installed there.
    fn ros_project_dir(&self, state: &State) -> Option<PathBuf> {
        let has_ros = |project: &Project| {
            project
                .find_plugin("UBotROS")
                .is_some_and(|plugin| plugin.is_ubot())
        };
        match state.workspace.project() {
            Some(project) => has_ros(project).then(|| project.dir.clone()),
            // Not scanned yet: look at the project on disk.
            None => {
                let project = Project::open(state.project_path.as_ref()?).ok()?;
                has_ros(&project).then_some(project.dir)
            }
        }
    }

    /// Scans the open project once when nothing has been scanned yet.
    fn ensure_scanned(&self) {
        let scanned = lock(&self.inner.state).snapshot.is_some();
        if !scanned {
            let _ = self.refresh(false);
        }
    }

    /// `test_git`: runs `<path> --version`.
    pub fn test_git(&self, path: &str) -> GitTestResult {
        env::test_git(path)
    }

    /// `test_ros`: a TCP connection with a 2 s timeout.
    pub fn test_ros(&self, host: &str, port: u16) -> RosTestResult {
        env::test_ros(host, port)
    }

    /// `check_for_update`: one request to the GitHub Releases API for the latest uBot Manager
    /// (SPEC §7). It never downloads or installs anything.
    pub fn check_for_update(&self) -> ApiResult<UpdateInfo> {
        if self.inner.options.offline {
            return Err("offline: not checking for updates".into());
        }
        update::check(
            self.inner.options.fetcher.as_ref(),
            update::current_version(),
        )
    }

    /// `list_engines`: launcher installs and registered source builds.
    pub fn list_engines(&self) -> Vec<EngineInfo> {
        engine::list_engines(self.inner.options.engine_registry.as_ref())
    }

    /// `check_environment`: `git`, `vs2022`, `engine`; `rosBridge` is reported as not checked.
    pub fn check_environment(&self, checks: &[String]) -> Vec<EnvCheck> {
        self.ensure_scanned();
        let (git_path, engine, project_open) = {
            let state = lock(&self.inner.state);
            let engine = state
                .workspace
                .project
                .as_ref()
                .and_then(|context| context.engine.clone());
            (
                state.settings.settings.install.git.clone(),
                engine,
                state.project_path.is_some(),
            )
        };
        env::check_environment(checks, &git_path, engine.as_ref(), project_open)
    }

    /// `recipe_changes`: install missing and enable disabled packages of a recipe (or of `packages`).
    pub fn recipe_changes(&self, recipe_id: Option<&str>, packages: &[String]) -> Vec<Change> {
        self.ensure_scanned();
        let workspace = lock(&self.inner.state).workspace.clone();
        recipes::recipe_changes(&workspace, recipe_id, packages)
    }

    /// The merged index of the last refresh (CLI `index show`).
    pub fn merged_index(&self) -> MergedIndex {
        self.ensure_scanned();
        lock(&self.inner.state).workspace.index.clone()
    }

    /// A rescan without network access or ROS check, used before planning. It never waits for the
    /// network: sources that are not cached are left for the refresh that fetches them.
    fn light_rebuild(&self) -> ApiResult<Arc<Workspace>> {
        self.rebuild(Rebuild {
            sync: false,
            ros_check: false,
            defer_fetch: true,
        })?;
        Ok(lock(&self.inner.state).workspace.clone())
    }

    /// The index sources in effect: the CLI override, else the configured ones.
    fn source_configs(&self, settings: &Settings) -> Vec<IndexSourceConfig> {
        match &self.inner.options.index_override {
            Some(locations) => locations
                .iter()
                .enumerate()
                .map(|(position, location)| IndexSourceConfig {
                    id: format!("cli-{}", position + 1),
                    name: location.clone(),
                    location: location.clone(),
                    enabled: true,
                    builtin: false,
                })
                .collect(),
            None => settings.index.sources.clone(),
        }
    }

    fn rebuild(&self, mode: Rebuild) -> ApiResult<Snapshot> {
        // A sync in progress is already fetching; scans that run meanwhile use the caches.
        let mode = Rebuild {
            defer_fetch: mode.defer_fetch
                || (!mode.sync && self.inner.syncing.load(Ordering::SeqCst) > 0),
            ..mode
        };
        let _syncing = mode.sync.then(|| SyncGuard::new(&self.inner.syncing));
        // The slow part - downloads, the ROS check - happens before the lock is taken, so a
        // plan, a settings change or a project switch never waits for an unreachable server.
        let prefetched = self.prefetch(mode);
        let _serialized = lock(&self.inner.refresh_lock);
        self.rebuild_locked(mode, prefetched)
    }

    /// Downloads the index sources the rebuild is going to fetch and checks the ROS bridge, with
    /// no lock held. A result that does not fit the state found later (settings or project
    /// changed meanwhile) is simply not used.
    fn prefetch(&self, mode: Rebuild) -> Prefetched {
        let options = &self.inner.options;
        let (settings, project_path, failed_fetches) = {
            let state = lock(&self.inner.state);
            (
                state.settings.settings.clone(),
                state.project_path.clone(),
                state.failed_fetches.clone(),
            )
        };
        let sources = sources::prefetch(
            &self.source_configs(&settings),
            mode.sync,
            options.offline,
            mode.defer_fetch,
            &options.dirs.cache,
            &failed_fetches,
            options.fetcher.as_ref(),
        );
        let ros = if mode.ros_check {
            project_path.and_then(|path| {
                let project = Project::open(&path).ok()?;
                ros_check_wanted(&project, &settings)
                    .then(|| (path, run_ros_check(&project, &settings)))
            })
        } else {
            None
        };
        Prefetched { sources, ros }
    }

    fn rebuild_locked(&self, mode: Rebuild, mut prefetched: Prefetched) -> ApiResult<Snapshot> {
        let options = &self.inner.options;
        let (settings, user_recipes, project_path, mut failed_fetches, engine_cache, previous_ros) = {
            let state = lock(&self.inner.state);
            (
                state.settings.settings.clone(),
                state.settings.user_recipes.clone(),
                state.project_path.clone(),
                state.failed_fetches.clone(),
                state.engine_plugins.clone(),
                state
                    .workspace
                    .project
                    .as_ref()
                    .and_then(|context| context.ros.clone()),
            )
        };

        let mut new_engine_plugins = None;
        let context = match &project_path {
            None => None,
            Some(path) => {
                let project = Project::open(path)
                    .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
                let engine = engine::resolve(
                    options.engine_registry.as_ref(),
                    &project.uproject.engine_association(),
                    &project.dir,
                    settings.build.engine_id.as_deref(),
                );
                let engine_plugins = engine.as_ref().map(|engine| {
                    let key = engine.root.to_lowercase();
                    engine_cache.get(&key).cloned().unwrap_or_else(|| {
                        let scanned = Arc::new(EnginePlugins::scan(Path::new(&engine.root)));
                        new_engine_plugins = Some((key, scanned.clone()));
                        scanned
                    })
                });
                let git_states = project
                    .plugins
                    .iter()
                    .filter(|plugin| plugin.is_ubot())
                    .map(|plugin| {
                        (
                            plugin.name.to_ascii_lowercase(),
                            git::inspect(&settings.install.git, &plugin.dir),
                        )
                    })
                    .collect();
                let session = status::load(&project.status_file());
                let editor = editor::find_editor(&project.path);
                let ros = if !ros_check_wanted(&project, &settings) {
                    None
                } else if mode.ros_check {
                    // Measured before the lock was taken when the project is the same one.
                    let measured = prefetched
                        .ros
                        .take()
                        .filter(|(path, _)| paths::same_path(path, &project.path))
                        .map(|(_, check)| check);
                    Some(measured.unwrap_or_else(|| run_ros_check(&project, &settings)))
                } else {
                    previous_ros
                };
                Some(ProjectContext {
                    project,
                    engine,
                    engine_plugins,
                    git: git_states,
                    session,
                    editor,
                    ros,
                })
            }
        };

        let configs = self.source_configs(&settings);
        let local_index = context.as_ref().and_then(|context| {
            context
                .project
                .find_plugin("UBotCore")
                .filter(|plugin| plugin.is_ubot())
                .map(|plugin| plugin.dir.join("Index").join("index.json"))
        });
        let loaded = {
            let mut load_context = LoadContext {
                sync: mode.sync,
                offline: options.offline,
                defer_fetch: mode.defer_fetch,
                cache_dir: &options.dirs.cache,
                project_dir: context
                    .as_ref()
                    .map(|context| context.project.dir.as_path()),
                local_index,
                fetcher: options.fetcher.as_ref(),
                failed_fetches: &mut failed_fetches,
                prefetched: std::mem::take(&mut prefetched.sources),
            };
            sources::load_sources(&configs, &mut load_context)
        };
        let index = MergedIndex::merge(&loaded.indexes);

        // Saved/uBot/index.json is for the UE panel, which clones with the repository URLs:
        // credentials in them are kept (everything shown or logged redacts them). `binaries`
        // URLs are redacted, because UE ignores them and the file lives in the project folder.
        if options.write_merged_index {
            if let Some(context) = &context {
                let path = context.project.merged_index_file();
                let text = index.to_file().for_panel().to_json();
                let current = paths::read_text(&path).ok().map(|(text, _)| text);
                if current.as_deref() != Some(text.as_str()) {
                    let _ = paths::write_atomic(&path, text.as_bytes());
                }
            }
        }

        let workspace = Arc::new(Workspace {
            project: context,
            index,
            sources: loaded.statuses,
            source_failures: loaded.failures,
            settings,
            user_recipes,
        });

        let mut state = lock(&self.inner.state);
        if state.project_path != project_path {
            // The project changed while scanning; keep the newer selection's data.
            return Err("The project changed during the refresh.".into());
        }
        state.failed_fetches = failed_fetches;
        if let Some((key, plugins)) = new_engine_plugins {
            state.engine_plugins.insert(key, plugins);
        }
        state.problems = problems::compute(&workspace);
        state.workspace = workspace.clone();
        let snapshot = self.assemble(&state, &workspace);
        state.snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn assemble(&self, state: &State, workspace: &Workspace) -> Snapshot {
        let ignored = state
            .project_path
            .as_ref()
            .map(|path| state.settings.ignored_for(path))
            .unwrap_or_default();
        let problems = problems::without_ignored(&state.problems, &ignored);
        let mut packages = workspace.package_infos();
        problems::attach_problem_ids(&mut packages, &problems);
        Snapshot {
            project: workspace.project_info(),
            packages,
            plugins: workspace.plugin_infos(),
            problems: problems
                .into_iter()
                .map(|computed| computed.problem)
                .collect(),
            index: workspace.sources.clone(),
            recipes: workspace.recipes(),
            session: workspace
                .project
                .as_ref()
                .and_then(|context| context.session.as_ref())
                .map(|session| session.session.clone()),
            // A snapshot is assembled when a scan is complete; `get_snapshot` reports a sync in progress.
            syncing: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn manager_is_send_and_sync() {
        assert_send_sync::<Manager>();
    }

    #[test]
    fn works_without_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(ManagerOptions {
            dirs: AppDirs::under(dir.path().to_path_buf()),
            offline: true,
            ..ManagerOptions::default()
        });
        let snapshot = manager.refresh(false).unwrap();
        assert!(snapshot.project.is_none());
        assert!(
            snapshot
                .packages
                .iter()
                .any(|package| package.name == "UBotCore"),
            "the embedded index is used offline"
        );
        assert!(snapshot.index.iter().any(|status| status.id == "embedded"));
        assert_eq!(
            manager
                .plan(
                    &[Change::new(
                        crate::model::ChangeKind::Enable,
                        "UBotCore",
                        None
                    )],
                    &ApplyOptions::default()
                )
                .unwrap()
                .errors[0]
                .code,
            "plan.noProject"
        );
        assert!(manager.list_tasks().is_empty());
        assert!(manager.get_task_log("nope", None).is_err());
        assert!(manager.cancel_task("nope").is_err());
        assert!(manager.ignore_problem("x").is_err());
        assert!(manager
            .open_project(&dir.path().join("missing.uproject").to_string_lossy())
            .is_err());
        let settings = manager.get_settings();
        assert_eq!(settings.install.directory, "Plugins/uBot");
    }

    #[test]
    fn select_project_then_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let uproject = dir.path().join("Sim").join("Sim.uproject");
        std::fs::create_dir_all(uproject.parent().unwrap()).unwrap();
        std::fs::write(
            &uproject,
            "{ \"FileVersion\": 3, \"EngineAssociation\": \"\" }\n",
        )
        .unwrap();
        let options = ManagerOptions {
            dirs: AppDirs::under(home.clone()),
            offline: true,
            ..ManagerOptions::default()
        };
        let manager = Manager::new(options.clone());
        assert!(manager
            .select_project(&dir.path().join("Missing.uproject").to_string_lossy())
            .is_err());
        assert!(manager.project_path().is_none());

        manager.select_project(&uproject.to_string_lossy()).unwrap();
        assert!(paths::same_path(
            &manager.project_path().unwrap(),
            &uproject
        ));
        assert!(
            !uproject.parent().unwrap().join("Saved").exists(),
            "selecting does not scan"
        );
        let snapshot = manager.rescan().unwrap();
        assert_eq!(snapshot.project.unwrap().name, "Sim");
        assert!(uproject
            .parent()
            .unwrap()
            .join("Saved/uBot/index.json")
            .is_file());

        // The selection is stored: a new manager opens the same project.
        let settings = Manager::new(options).get_settings();
        assert_eq!(settings.recent_projects.len(), 1);
        assert!(paths::same_path(
            Path::new(settings.last_project.as_deref().unwrap()),
            &uproject
        ));
    }

    #[test]
    fn apply_rejects_with_one_line_per_plan_error() {
        struct Quiet;
        impl TaskSink for Quiet {
            fn task_updated(&self, _task: &Task) {}
            fn log(&self, _task_id: &str, _step_id: &str, _lines: &[String]) {}
        }
        let dir = tempfile::tempdir().unwrap();
        let uproject = dir.path().join("Sim").join("Sim.uproject");
        std::fs::create_dir_all(uproject.parent().unwrap()).unwrap();
        std::fs::write(
            &uproject,
            "{ \"FileVersion\": 3, \"EngineAssociation\": \"\" }\n",
        )
        .unwrap();
        let manager = Manager::new(ManagerOptions {
            dirs: AppDirs::under(dir.path().join("home")),
            offline: true,
            ..ManagerOptions::default()
        });
        manager.select_project(&uproject.to_string_lossy()).unwrap();

        // Each message is one line: the UI turns the lines into translated messages one by one
        // (joined with a space, the template of the first message would swallow the others).
        let changes = [
            Change::new(crate::model::ChangeKind::Enable, "UBotNope1", None),
            Change::new(crate::model::ChangeKind::Enable, "UBotNope2", None),
        ];
        let error = manager
            .apply(
                &changes,
                &ApplyOptions::default(),
                None,
                None,
                Arc::new(Quiet),
            )
            .unwrap_err();
        assert_eq!(
            error.lines().collect::<Vec<_>>(),
            ["UBotNope1 is not installed.", "UBotNope2 is not installed."],
            "{error:?}"
        );
    }

    #[test]
    fn a_failed_select_project_keeps_the_open_project() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = |name: &str| {
            let uproject = dir.path().join(name).join(format!("{name}.uproject"));
            std::fs::create_dir_all(uproject.parent().unwrap()).unwrap();
            std::fs::write(
                &uproject,
                "{ \"FileVersion\": 3, \"EngineAssociation\": \"\" }
",
            )
            .unwrap();
            uproject
        };
        let (a, b) = (project("A"), project("B"));
        let manager = Manager::new(ManagerOptions {
            dirs: AppDirs::under(home.clone()),
            offline: true,
            ..ManagerOptions::default()
        });
        manager.select_project(&a.to_string_lossy()).unwrap();
        let snapshot = manager.rescan().unwrap();
        assert_eq!(snapshot.project.unwrap().name, "A");
        let before = manager.get_settings();

        // Saving fails: a directory sits where the atomic write puts its temporary file.
        let blocker = home.join(format!(".settings.json.{}.tmp", std::process::id()));
        std::fs::create_dir_all(&blocker).unwrap();
        let error = manager.select_project(&b.to_string_lossy()).unwrap_err();
        assert!(error.starts_with("Cannot save settings:"), "{error}");
        assert!(
            paths::same_path(&manager.project_path().unwrap(), &a),
            "still A"
        );
        let after = manager.get_settings();
        assert_eq!(after.last_project, before.last_project);
        assert_eq!(after.recent_projects, before.recent_projects);
        assert_eq!(
            manager.get_snapshot().project.unwrap().name,
            "A",
            "A's snapshot was not dropped"
        );
        assert_eq!(manager.rescan().unwrap().project.unwrap().name, "A");
        assert!(manager.open_project(&b.to_string_lossy()).is_err());
        assert!(paths::same_path(&manager.project_path().unwrap(), &a));

        // Once saving works again B opens.
        std::fs::remove_dir(&blocker).unwrap();
        manager.select_project(&b.to_string_lossy()).unwrap();
        assert!(paths::same_path(&manager.project_path().unwrap(), &b));
        assert!(paths::same_path(
            Path::new(manager.get_settings().last_project.as_deref().unwrap()),
            &b
        ));
    }

    struct LatestRelease(&'static str);

    impl Fetcher for LatestRelease {
        fn fetch(&self, url: &str) -> Result<String, String> {
            assert_eq!(url, update::LATEST_RELEASE_API);
            Ok(self.0.to_string())
        }
    }

    #[test]
    fn check_for_update_asks_github_and_never_offline() {
        let dir = tempfile::tempdir().unwrap();
        let options = ManagerOptions {
            dirs: AppDirs::under(dir.path().to_path_buf()),
            fetcher: Arc::new(LatestRelease(r#"{ "tag_name": "v999.0.0" }"#)),
            ..ManagerOptions::default()
        };
        let info = Manager::new(options.clone()).check_for_update().unwrap();
        assert!(info.available);
        assert_eq!(info.current, update::current_version());
        assert_eq!(info.latest, "999.0.0");

        let offline = ManagerOptions {
            offline: true,
            ..options
        };
        assert!(Manager::new(offline).check_for_update().is_err());
    }
}

//! Task execution (SPEC §5.3): runs the steps of a prepared plan one by one on a background
//! thread, streams process output to a [`TaskSink`], supports cancelation and keeps a history.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::git;
use crate::model::{Msg, Progress, Snapshot, StepStatus, Task, TaskStep};
use crate::msg::{msg, msg0, render_en};
use crate::paths;
use crate::plan::{PreparedPlan, StepAction, PLATFORM};
use crate::process::{self, RunOutcome, StreamEvent};
use crate::project::Project;
use crate::sources::now_rfc3339;
use crate::uproject;
use crate::url::redact_credentials;

pub const MAX_TASKS: usize = 30;
pub const MAX_STEP_LOG_LINES: usize = 5000;
const LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
const EDITOR_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Runs a callback into the sink. A sink that panics (a closed pipe under `println!`, a window
/// that is gone) must not end the thread that runs the task: the project would be left half
/// changed, the task unfinished and the "one task at a time" slot taken.
pub(crate) fn notify(callback: impl FnOnce()) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback));
}

/// Receives task progress. The Tauri shell forwards these as `ubot://task`, `ubot://task-log` and
/// `ubot://snapshot` events; the CLI prints them.
pub trait TaskSink: Send + Sync {
    /// A task or one of its steps changed status or progress.
    fn task_updated(&self, task: &Task);
    /// New output lines of a step, batched (at most every 100 ms per step).
    fn log(&self, task_id: &str, step_id: &str, lines: &[String]);
    /// The snapshot was refreshed after the task finished.
    fn snapshot_updated(&self, _snapshot: &Snapshot) {}
}

/// Removes an uninstalled package folder.
pub trait Remover: Send + Sync {
    fn remove(&self, dir: &Path) -> Result<(), String>;
}

/// Moves folders to the Recycle Bin.
pub struct RecycleBin;

impl Remover for RecycleBin {
    fn remove(&self, dir: &Path) -> Result<(), String> {
        trash::delete(dir).map_err(|error| error.to_string())
    }
}

/// Deletes folders permanently. Used by tests and CI, where the Recycle Bin is not wanted.
pub struct PermanentDelete;

impl Remover for PermanentDelete {
    fn remove(&self, dir: &Path) -> Result<(), String> {
        remove_dir_robust(dir).map_err(|error| error.to_string())
    }
}

/// Deletes a directory tree, clearing read-only flags (git marks its objects read-only) and
/// retrying briefly while a killed process still holds handles.
pub fn remove_dir_robust(dir: &Path) -> io::Result<()> {
    let mut last_error = None;
    for attempt in 0..5 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(200));
            clear_readonly(dir);
        }
        match fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("could not remove the directory")))
}

fn clear_readonly(path: &Path) {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
        if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    clear_readonly(&entry.path());
                }
            }
        }
    }
}

/// A task with its logs, as kept in memory and in `<data>/tasks.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub task: Task,
    #[serde(default)]
    pub summary: Vec<String>,
    #[serde(default)]
    pub logs: HashMap<String, Vec<String>>,
}

#[derive(Default, Serialize, Deserialize)]
struct TaskFile {
    tasks: Vec<TaskRecord>,
}

/// Task history: the last 30 tasks, oldest first.
#[derive(Debug, Default)]
pub struct TaskStore {
    path: Option<PathBuf>,
    records: Vec<TaskRecord>,
    /// The history file exists but could not be read as a whole; `save` copies it aside first.
    unreadable: bool,
}

impl TaskStore {
    /// Loads the history. Tasks that were still running when the manager exited are marked failed.
    pub fn load(path: PathBuf) -> TaskStore {
        let (mut records, unreadable) = Self::read_records(&path);
        for record in &mut records {
            if !record.task.status.is_finished() {
                record.task.status = StepStatus::Failed;
                record.task.error = Some("The manager exited while the task was running.".into());
                for step in &mut record.task.steps {
                    if !step.status.is_finished() {
                        step.status = StepStatus::Skipped;
                    }
                }
            }
        }
        TaskStore {
            path: Some(path),
            records,
            unreadable,
        }
    }

    /// The records of the history file, one by one (a record the format no longer fits is
    /// dropped, not the whole history), and whether the file exists but is unusable.
    fn read_records(path: &Path) -> (Vec<TaskRecord>, bool) {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => return (Vec::new(), error.kind() != io::ErrorKind::NotFound),
        };
        let parsed = paths::decode_text(&bytes).and_then(|text| {
            serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())
        });
        match parsed {
            Ok(serde_json::Value::Object(mut file)) => {
                let records = match file.remove("tasks") {
                    Some(serde_json::Value::Array(tasks)) => tasks
                        .into_iter()
                        .filter_map(|task| serde_json::from_value::<TaskRecord>(task).ok())
                        .collect(),
                    _ => Vec::new(),
                };
                (records, false)
            }
            _ => (Vec::new(), true),
        }
    }

    pub fn in_memory() -> TaskStore {
        TaskStore::default()
    }

    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        if self.unreadable {
            // The history on disk is about to be replaced by what is in memory.
            match paths::backup_unreadable(&path) {
                Ok(_) => self.unreadable = false,
                Err(error) if error.kind() == io::ErrorKind::NotFound => self.unreadable = false,
                Err(error) => return Err(error),
            }
        }
        let file = TaskFile {
            tasks: self.records.clone(),
        };
        let text = serde_json::to_string(&file).map_err(io::Error::other)?;
        paths::write_atomic(&path, text.as_bytes())
    }

    fn push(&mut self, record: TaskRecord) {
        self.records.push(record);
        if self.records.len() > MAX_TASKS {
            let excess = self.records.len() - MAX_TASKS;
            self.records.drain(..excess);
        }
    }

    pub fn get(&self, id: &str) -> Option<&TaskRecord> {
        self.records.iter().find(|record| record.task.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut TaskRecord> {
        self.records.iter_mut().find(|record| record.task.id == id)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Task> {
        self.records
            .iter()
            .rev()
            .map(|record| record.task.clone())
            .collect()
    }

    /// Log lines of a step, or the task summary lines when `step_id` is `None`.
    pub fn log(&self, task_id: &str, step_id: Option<&str>) -> Option<Vec<String>> {
        let record = self.get(task_id)?;
        Some(match step_id {
            None => record.summary.clone(),
            Some(step_id) => record.logs.get(step_id).cloned().unwrap_or_default(),
        })
    }
}

/// How steps reach the outside world. Injectable for tests.
#[derive(Clone)]
pub struct ExecContext {
    pub git: String,
    pub wait_for_editor: bool,
    pub remover: Arc<dyn Remover>,
    /// True while the editor runs the project.
    pub editor_running: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
}

static TASK_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn new_task_id() -> String {
    let millis = chrono::Utc::now().timestamp_millis();
    format!(
        "task-{millis}-{}",
        TASK_COUNTER.fetch_add(1, Ordering::SeqCst)
    )
}

/// Creates the task record for a prepared plan (all steps pending).
pub fn new_task(id: String, title: Msg, prepared: &PreparedPlan) -> TaskRecord {
    TaskRecord {
        task: Task {
            id,
            title,
            project: prepared.project_name.clone(),
            status: StepStatus::Pending,
            started_at: now_rfc3339(),
            duration_ms: None,
            steps: prepared
                .plan
                .steps
                .iter()
                .map(|step| TaskStep {
                    id: step.id.clone(),
                    title: step.title.clone(),
                    status: StepStatus::Pending,
                    started_at: None,
                    duration_ms: None,
                    progress: None,
                    log_lines: 0,
                })
                .collect(),
            error: None,
            changes: prepared.plan.changes.clone(),
        },
        summary: Vec::new(),
        logs: HashMap::new(),
    }
}

/// Handle of a started task.
pub struct TaskHandle {
    pub id: String,
    pub cancel: Arc<AtomicBool>,
    pub thread: thread::JoinHandle<()>,
}

/// Adds the task to the store and runs it on a background thread. `on_finish` runs on that thread
/// after the final state is saved.
pub fn start(
    store: Arc<Mutex<TaskStore>>,
    record: TaskRecord,
    prepared: PreparedPlan,
    context: ExecContext,
    sink: Arc<dyn TaskSink>,
    on_finish: Box<dyn FnOnce(&Task) + Send>,
) -> TaskHandle {
    let id = record.task.id.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut store = store.lock().expect("task store lock");
        store.push(record);
        let _ = store.save();
    }
    let runner = Runner {
        store,
        id: id.clone(),
        prepared,
        context,
        sink,
        cancel: cancel.clone(),
    };
    let thread = thread::Builder::new()
        .name("ubot-task".into())
        .spawn(move || {
            let task = runner.run();
            on_finish(&task);
        })
        .expect("spawning the task thread");
    TaskHandle { id, cancel, thread }
}

enum StepResult {
    Done,
    Failed(String),
    Canceled,
}

struct Runner {
    store: Arc<Mutex<TaskStore>>,
    id: String,
    prepared: PreparedPlan,
    context: ExecContext,
    sink: Arc<dyn TaskSink>,
    cancel: Arc<AtomicBool>,
}

/// Buffers the output of one step and forwards it in batches.
struct StepLog<'a> {
    runner: &'a Runner,
    step_id: String,
    pending: Vec<String>,
    last_flush: Instant,
    progress: Option<Progress>,
    progress_changed: bool,
    track_progress: bool,
    /// UBT said the target has no rules file (see the build step).
    missing_target_rules: bool,
}

impl<'a> StepLog<'a> {
    fn new(runner: &'a Runner, step_id: &str, track_progress: bool) -> Self {
        StepLog {
            runner,
            step_id: step_id.to_string(),
            pending: Vec::new(),
            last_flush: Instant::now(),
            progress: None,
            progress_changed: false,
            track_progress,
            missing_target_rules: false,
        }
    }

    fn line(&mut self, line: impl Into<String>) {
        let line = redact_credentials(&line.into());
        if self.track_progress {
            if line.contains("find target rules file") {
                self.missing_target_rules = true;
            }
            if let Some((done, total)) = parse_progress(&line) {
                self.progress = Some(Progress { done, total });
                self.progress_changed = true;
            }
        }
        self.pending.push(line);
        if self.last_flush.elapsed() >= LOG_FLUSH_INTERVAL {
            self.flush();
        }
    }

    /// Forwards buffered output once the flush interval has passed, so quiet stretches of a
    /// process do not hold lines back.
    fn flush_if_due(&mut self) {
        if (!self.pending.is_empty() || self.progress_changed)
            && self.last_flush.elapsed() >= LOG_FLUSH_INTERVAL
        {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.last_flush = Instant::now();
        if self.pending.is_empty() && !self.progress_changed {
            return;
        }
        let lines = std::mem::take(&mut self.pending);
        let task = {
            let mut store = self.runner.store.lock().expect("task store lock");
            let Some(record) = store.get_mut(&self.runner.id) else {
                return;
            };
            let log = record.logs.entry(self.step_id.clone()).or_default();
            log.extend(lines.iter().cloned());
            if log.len() > MAX_STEP_LOG_LINES {
                let excess = log.len() - MAX_STEP_LOG_LINES;
                log.drain(..excess);
            }
            let count = log.len() as u32;
            if let Some(step) = record
                .task
                .steps
                .iter_mut()
                .find(|step| step.id == self.step_id)
            {
                step.log_lines = count;
                if self.progress_changed {
                    step.progress = self.progress;
                }
            }
            record.task.clone()
        };
        if !lines.is_empty() {
            notify(|| self.runner.sink.log(&self.runner.id, &self.step_id, &lines));
        }
        if self.progress_changed {
            self.progress_changed = false;
            notify(|| self.runner.sink.task_updated(&task));
        }
    }
}

impl Runner {
    fn update(&self, change: impl FnOnce(&mut TaskRecord)) -> Task {
        let task = {
            let mut store = self.store.lock().expect("task store lock");
            let record = store
                .get_mut(&self.id)
                .expect("the running task is in the store");
            change(record);
            record.task.clone()
        };
        notify(|| self.sink.task_updated(&task));
        task
    }

    fn summary(&self, line: String) {
        let mut store = self.store.lock().expect("task store lock");
        if let Some(record) = store.get_mut(&self.id) {
            record.summary.push(redact_credentials(&line));
        }
    }

    fn canceled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn run(&self) -> Task {
        let started = Instant::now();
        let title = {
            self.store
                .lock()
                .expect("task store lock")
                .get(&self.id)
                .map(|r| render_en(&r.task.title))
                .unwrap_or_default()
        };
        self.summary(format!("{title}: started at {}", now_rfc3339()));
        self.update(|record| record.task.status = StepStatus::Running);

        let total = self.prepared.plan.steps.len();
        let mut failure: Option<String> = None;
        let mut canceled = false;
        for (position, (step, action)) in self
            .prepared
            .plan
            .steps
            .iter()
            .zip(&self.prepared.actions)
            .enumerate()
        {
            let step_title = render_en(&step.title);
            if failure.is_some() || canceled {
                self.set_step(&step.id, |s| s.status = StepStatus::Skipped);
                continue;
            }
            if self.canceled() {
                canceled = true;
                self.set_step(&step.id, |s| s.status = StepStatus::Canceled);
                self.summary(format!("[{}/{total}] {step_title}: canceled", position + 1));
                continue;
            }

            let step_started = Instant::now();
            let result = match self.wait_for_editor(step.needs_editor_closed, &step.id) {
                Err(result) => {
                    // The step never started, but its log should still say why it failed.
                    if let StepResult::Failed(error) = &result {
                        let mut log = StepLog::new(self, &step.id, false);
                        log.line(format!("error: {error}"));
                        log.flush();
                    }
                    result
                }
                Ok(()) => {
                    let started_at = now_rfc3339();
                    self.set_step(&step.id, |s| {
                        s.status = StepStatus::Running;
                        s.started_at = Some(started_at);
                    });
                    let mut log =
                        StepLog::new(self, &step.id, matches!(action, StepAction::Build { .. }));
                    let result = self.execute(action, &mut log);
                    if let StepResult::Failed(error) = &result {
                        log.line(format!("error: {error}"));
                    }
                    log.flush();
                    result
                }
            };
            let duration = step_started.elapsed().as_millis() as u64;
            let (status, outcome) = match &result {
                StepResult::Done => (StepStatus::Done, "done".to_string()),
                StepResult::Failed(error) => (StepStatus::Failed, format!("failed: {error}")),
                StepResult::Canceled => (StepStatus::Canceled, "canceled".to_string()),
            };
            self.set_step(&step.id, |s| {
                s.status = status;
                s.duration_ms = Some(duration);
            });
            self.summary(format!(
                "[{}/{total}] {step_title}: {outcome} ({:.1} s)",
                position + 1,
                duration as f64 / 1000.0
            ));
            match result {
                StepResult::Done => {}
                StepResult::Failed(error) => failure = Some(format!("{step_title}: {error}")),
                StepResult::Canceled => canceled = true,
            }
        }

        let status = match (&failure, canceled) {
            (Some(_), _) => StepStatus::Failed,
            (None, true) => StepStatus::Canceled,
            (None, false) => StepStatus::Done,
        };
        let duration = started.elapsed().as_millis() as u64;
        self.summary(format!(
            "{title}: {} after {:.1} s",
            match status {
                StepStatus::Done => "finished",
                StepStatus::Canceled => "canceled",
                _ => "failed",
            },
            duration as f64 / 1000.0
        ));
        let task = self.update(|record| {
            record.task.status = status;
            record.task.duration_ms = Some(duration);
            record.task.error = failure.map(|error| redact_credentials(&error));
        });
        let _ = self.store.lock().expect("task store lock").save();
        task
    }

    fn set_step(&self, step_id: &str, change: impl FnOnce(&mut TaskStep)) {
        self.update(|record| {
            if let Some(step) = record.task.steps.iter_mut().find(|step| step.id == step_id) {
                change(step);
            }
        });
    }

    /// Before a step that needs the editor closed: wait (polling every second) or fail.
    fn wait_for_editor(&self, needs_closed: bool, step_id: &str) -> Result<(), StepResult> {
        if !needs_closed || !(self.context.editor_running)(&self.prepared.project_path) {
            return Ok(());
        }
        if !self.context.wait_for_editor {
            return Err(StepResult::Failed(render_en(&msg0("task.editorRunning"))));
        }
        self.set_step(step_id, |s| s.status = StepStatus::Waiting);
        loop {
            let poll_started = Instant::now();
            while poll_started.elapsed() < EDITOR_POLL_INTERVAL {
                if self.canceled() {
                    return Err(StepResult::Canceled);
                }
                thread::sleep(Duration::from_millis(50));
            }
            if !(self.context.editor_running)(&self.prepared.project_path) {
                return Ok(());
            }
        }
    }

    fn execute(&self, action: &StepAction, log: &mut StepLog<'_>) -> StepResult {
        match action {
            StepAction::Clone {
                package,
                url,
                git_ref,
                dest,
            } => self.clone_package(package, url, git_ref, dest, log),
            StepAction::Checkout { dir, git_ref, .. } => self.checkout_package(dir, git_ref, log),
            StepAction::SetEnabled { plugin, enabled } => {
                let path = &self.prepared.project_path;
                match uproject::edit(path, |project| project.set_enabled(plugin, *enabled)) {
                    Ok(changed) => {
                        let file = path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        log.line(format!(
                            "{plugin}: \"Enabled\": {enabled} in {file}{}",
                            if changed { "" } else { " (unchanged)" }
                        ));
                        StepResult::Done
                    }
                    Err(error) => StepResult::Failed(error.to_string()),
                }
            }
            StepAction::Remove { package, dir } => {
                let project_dir = self.prepared.project_path.parent().unwrap_or(Path::new(""));
                if !paths::is_strictly_within(dir, project_dir) {
                    return StepResult::Failed(format!("{} is outside the project", dir.display()));
                }
                // A plugin root holds other plugins too (the descriptor of this package may lie
                // directly in it); never trash it, and leave the .uproject untouched.
                let roots = match Project::open(&self.prepared.project_path) {
                    Ok(project) => project.plugin_roots,
                    Err(_) => vec![project_dir.join("Plugins")],
                };
                if roots.iter().any(|root| paths::same_path(root, dir)) {
                    return StepResult::Failed(format!(
                        "{} is a plugin folder that holds more than {package}",
                        dir.display()
                    ));
                }
                match uproject::edit(&self.prepared.project_path, |project| {
                    project.remove_entry(package)
                }) {
                    Ok(changed) => log.line(format!(
                        "{package}: {} the .uproject",
                        if changed {
                            "entry removed from"
                        } else {
                            "no entry in"
                        }
                    )),
                    Err(error) => return StepResult::Failed(error.to_string()),
                }
                if !dir.exists() {
                    log.line(format!("{} does not exist", dir.display()));
                    return StepResult::Done;
                }
                match self.context.remover.remove(dir) {
                    Ok(()) => {
                        log.line(format!("Removed {}", paths::display(dir)));
                        StepResult::Done
                    }
                    Err(error) => {
                        StepResult::Failed(format!("could not remove {}: {error}", dir.display()))
                    }
                }
            }
            StepAction::Build {
                engine_root,
                target,
                configuration,
                uproject,
            } => {
                if !cfg!(windows) {
                    return StepResult::Failed("building is only supported on Windows".into());
                }
                if !is_safe_build_name(target) {
                    return StepResult::Failed(format!(
                        "the build target '{target}' is not a plain target name"
                    ));
                }
                if paths::display(uproject).contains('%')
                    || paths::display(engine_root).contains('%')
                {
                    return StepResult::Failed(
                        "cmd.exe would expand the % in the project or engine path".into(),
                    );
                }
                let result =
                    self.run_build(engine_root, target, configuration.as_str(), uproject, log);
                if matches!(result, StepResult::Failed(_))
                    && log.missing_target_rules
                    && target != STOCK_EDITOR_TARGET
                {
                    // A content-only project has no <Name>Editor target unless UBT makes a
                    // temporary one; the stock editor target builds the project's plugins too.
                    log.line(format!(
                        "{target} has no target rules file; building {STOCK_EDITOR_TARGET} with -Project instead."
                    ));
                    log.missing_target_rules = false;
                    let step_id = log.step_id.clone();
                    self.set_step(&step_id, |step| {
                        step.title = msg("step.build", &[("target", STOCK_EDITOR_TARGET)])
                    });
                    return self.run_build(
                        engine_root,
                        STOCK_EDITOR_TARGET,
                        configuration.as_str(),
                        uproject,
                        log,
                    );
                }
                result
            }
            StepAction::RunTests {
                engine_root,
                uproject,
                packages,
            } => {
                let editor = editor_cmd_path(engine_root);
                let arguments = run_tests_arguments(uproject, packages);
                log.line(format!("\"{}\" {arguments}", editor.display()));
                self.run_process(raw_command(&editor, &arguments), log)
            }
            StepAction::OpenEditor {
                engine_root,
                uproject,
            } => {
                let editor = engine_root
                    .join("Engine")
                    .join("Binaries")
                    .join("Win64")
                    .join("UnrealEditor.exe");
                let mut command = Command::new(&editor);
                command.arg(uproject);
                log.line(format!(
                    "\"{}\" \"{}\"",
                    editor.display(),
                    uproject.display()
                ));
                match process::spawn_detached(command) {
                    Ok(()) => StepResult::Done,
                    Err(error) => {
                        StepResult::Failed(format!("could not start {}: {error}", editor.display()))
                    }
                }
            }
        }
    }

    fn run_build(
        &self,
        engine_root: &Path,
        target: &str,
        configuration: &str,
        uproject: &Path,
        log: &mut StepLog<'_>,
    ) -> StepResult {
        let arguments = build_arguments(engine_root, target, configuration, uproject);
        log.line(format!("cmd.exe {arguments}"));
        self.run_process(raw_command("cmd.exe", &arguments), log)
    }

    /// Fetches, resolves `git_ref` and detaches HEAD there.
    ///
    /// The ref is resolved here (tag, then the branch of `origin`, then whatever git makes of the
    /// name: a commit, a local branch) instead of leaving it to `git checkout`, which refuses a
    /// bare name that exists only as `origin/<name>` and would check out a stale local branch of
    /// that name without moving it.
    fn checkout_package(&self, dir: &Path, git_ref: &str, log: &mut StepLog<'_>) -> StepResult {
        if git_ref.is_empty() || git_ref.starts_with('-') {
            return StepResult::Failed(format!("'{git_ref}' is not a usable git ref"));
        }
        // The folder may have come with the project: its `.git/config` must not name a program
        // (ssh, proxy and credential commands, filters) that fetch or checkout would run.
        if let Some(message) = git::untrusted_config(&self.context.git, dir) {
            return StepResult::Failed(format!("{message} in {}", dir.display()));
        }
        let mut fetch = git::command_in_checkout(&self.context.git);
        // A submodule has a configuration of its own, equally untrusted, and nothing here
        // checks out submodules.
        fetch
            .arg("-C")
            .arg(dir)
            .args(["fetch", "--tags", "--no-recurse-submodules", "origin"]);
        match self.run_process(fetch, log) {
            StepResult::Done => {}
            other => return other,
        }
        let mut commit = None;
        for candidate in [
            format!("refs/tags/{git_ref}"),
            format!("refs/remotes/origin/{git_ref}"),
            git_ref.to_string(),
        ] {
            let mut resolve = git::command_in_checkout(&self.context.git);
            resolve
                .arg("-C")
                .arg(dir)
                .args(["rev-parse", "--verify", "--quiet"])
                .arg(format!("{candidate}^{{commit}}"));
            if let Ok(sha) = process::output_text(resolve) {
                if !sha.is_empty() {
                    commit = Some(sha);
                    break;
                }
            }
        }
        let Some(commit) = commit else {
            return StepResult::Failed(format!(
                "{git_ref} is neither a tag, a branch of origin nor a commit in {}",
                dir.display()
            ));
        };
        log.line(format!(
            "{git_ref} -> {}",
            commit.get(..12).unwrap_or(&commit)
        ));
        let mut checkout = git::command_in_checkout(&self.context.git);
        checkout
            .arg("-C")
            .arg(dir)
            .args(["checkout", "--detach"])
            .arg(&commit);
        self.run_process(checkout, log)
    }

    fn run_process(&self, command: Command, log: &mut StepLog<'_>) -> StepResult {
        let on_event = |event| match event {
            StreamEvent::Line(line) => log.line(line),
            StreamEvent::Idle => log.flush_if_due(),
        };
        match process::run_streaming_events(command, &self.cancel, on_event) {
            Ok(RunOutcome::Exited(status)) if status.success() => StepResult::Done,
            Ok(RunOutcome::Exited(status)) => StepResult::Failed(match status.code() {
                Some(code) => format!("exit code {code}"),
                None => "the process was terminated".into(),
            }),
            Ok(RunOutcome::Canceled) => StepResult::Canceled,
            Err(error) => StepResult::Failed(format!("could not start the process: {error}")),
        }
    }

    fn clone_package(
        &self,
        package: &str,
        url: &str,
        git_ref: &str,
        dest: &Path,
        log: &mut StepLog<'_>,
    ) -> StepResult {
        if dest.exists() {
            return StepResult::Failed(format!("{} already exists", dest.display()));
        }
        if let Some(parent) = dest.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return StepResult::Failed(format!(
                    "could not create {}: {error}",
                    parent.display()
                ));
            }
        }

        let commit = git::is_commit_id(git_ref);
        let mut clone = git::command(&self.context.git);
        clone.args(["clone", "--progress"]);
        if !commit {
            clone.args(["--branch", git_ref]);
        }
        clone.arg("--").arg(url).arg(dest);
        let branch = if commit {
            String::new()
        } else {
            format!("--branch {git_ref} ")
        };
        log.line(format!(
            "git clone --progress {branch}-- {url} {}",
            dest.display()
        ));
        let mut result = self.run_process(clone, log);
        if commit && matches!(result, StepResult::Done) {
            let mut checkout = git::command(&self.context.git);
            checkout
                .arg("-C")
                .arg(dest)
                .args(["checkout", "--detach", git_ref]);
            result = self.run_process(checkout, log);
        }
        if matches!(result, StepResult::Done) && !has_descriptor(dest, package) {
            result = StepResult::Failed(format!(
                "the repository has no {package}.uplugin at its root"
            ));
        }

        if !matches!(result, StepResult::Done) && dest.exists() {
            // The folder did not exist before this step (the step fails when it does), so
            // everything in it came from this clone - also when it lies in a plugin root outside
            // the project. Never a bare root such as C:\.
            if dest.parent().is_some() && dest.file_name().is_some() {
                match remove_dir_robust(dest) {
                    Ok(()) => log.line(format!("Removed the incomplete clone {}", dest.display())),
                    Err(error) => log.line(format!(
                        "Could not remove the incomplete clone {}: {error}",
                        dest.display()
                    )),
                }
            }
        }
        result
    }
}

fn has_descriptor(dir: &Path, package: &str) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                let path = entry.path();
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("uplugin"))
                    && path
                        .file_stem()
                        .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(package))
            })
        })
        .unwrap_or(false)
}

/// A command whose argument string is passed to the process verbatim (Windows), so cmd.exe and
/// Unreal see exactly the quoting written here.
fn raw_command(program: impl AsRef<std::ffi::OsStr>, arguments: &str) -> Command {
    let mut command = process::command(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(arguments);
    }
    #[cfg(not(windows))]
    {
        command.args(arguments.split_whitespace());
    }
    command
}

/// The build target is the one word of the Build.bat command line that is not quoted, and cmd.exe
/// reads `&`, `|`, `^`, `(` and `%` in it as its own syntax. Only UBT target names (letters, digits,
/// `_`) pass.
pub fn is_safe_build_name(target: &str) -> bool {
    !target.is_empty()
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The target of the engine's own editor, built with `-Project=` for projects that have no target
/// of their own.
pub const STOCK_EDITOR_TARGET: &str = "UnrealEditor";

pub fn build_bat_path(engine_root: &Path) -> PathBuf {
    engine_root
        .join("Engine")
        .join("Build")
        .join("BatchFiles")
        .join("Build.bat")
}

pub fn editor_cmd_path(engine_root: &Path) -> PathBuf {
    engine_root
        .join("Engine")
        .join("Binaries")
        .join("Win64")
        .join("UnrealEditor-Cmd.exe")
}

/// Arguments of `cmd.exe` for a UBT build:
/// `/S /C ""<Build.bat>" <Target> Win64 <Configuration> -Project="<uproject>" -WaitMutex -NoHotReloadFromIDE"`.
/// `/S` makes cmd strip exactly the outer pair of quotes.
pub fn build_arguments(
    engine_root: &Path,
    target: &str,
    configuration: &str,
    uproject: &Path,
) -> String {
    format!(
        "/S /C \"\"{}\" {target} {PLATFORM} {configuration} -Project=\"{}\" -WaitMutex -NoHotReloadFromIDE\"",
        paths::display(&build_bat_path(engine_root)),
        paths::display(uproject)
    )
}

/// Arguments of `UnrealEditor-Cmd.exe` that run the automation tests of `packages` headless.
/// `-stdout -FullStdOutLogOutput` puts the log on the pipe the task reads; without it a GUI-less
/// editor prints nothing there and a failing run is only "exit code 255". `-culture=en` keeps the
/// results readable: the console code page turns localized words such as "Result={Success}" into "??".
pub fn run_tests_arguments(uproject: &Path, packages: &[String]) -> String {
    format!(
        "\"{}\" -unattended -nop4 -nosplash -NullRHI -culture=en -stdout -FullStdOutLogOutput -ExecCmds=\"Automation RunTests {}; Quit\" -TestExit=\"Automation Test Queue Empty\"",
        paths::display(uproject),
        packages.join("+")
    )
}

/// The first `[n/m]` in a line, as UBT prints for each action.
pub fn parse_progress(line: &str) -> Option<(u32, u32)> {
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        if let Some(close) = after.find(']') {
            if let Some((done, total)) = after[..close].split_once('/') {
                if let (Ok(done), Ok(total)) =
                    (done.trim().parse::<u32>(), total.trim().parse::<u32>())
                {
                    if total > 0 && done <= total {
                        return Some((done, total));
                    }
                }
            }
        }
        rest = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BuildConfiguration, Plan, PlanStep, StepGroup, StepKind};
    use crate::msg::msg;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    pub struct RecordingSink {
        pub updates: Mutex<Vec<Task>>,
        pub lines: Mutex<Vec<(String, String)>>,
    }

    impl TaskSink for RecordingSink {
        fn task_updated(&self, task: &Task) {
            self.updates.lock().unwrap().push(task.clone());
        }
        fn log(&self, _task_id: &str, step_id: &str, lines: &[String]) {
            let mut all = self.lines.lock().unwrap();
            for line in lines {
                all.push((step_id.to_string(), line.clone()));
            }
        }
    }

    fn prepared(project: &Path, actions: Vec<(StepKind, StepAction)>) -> PreparedPlan {
        let steps = actions
            .iter()
            .enumerate()
            .map(|(position, (kind, _))| PlanStep {
                id: format!("s{}", position + 1),
                n: position as u32 + 1,
                group: StepGroup::Project,
                kind: *kind,
                title: msg("step.enablePackage", &[("package", "X")]),
                detail: msg("step.detail.project", &[("file", "P.uproject")]),
                needs_editor_closed: *kind != StepKind::Clone,
            })
            .collect();
        PreparedPlan {
            plan: Plan {
                steps,
                ..Plan::default()
            },
            actions: actions.into_iter().map(|(_, action)| action).collect(),
            project_path: project.to_path_buf(),
            project_name: "P".into(),
        }
    }

    fn context(editor: Arc<dyn Fn(&Path) -> bool + Send + Sync>, wait: bool) -> ExecContext {
        ExecContext {
            git: "git".into(),
            wait_for_editor: wait,
            remover: Arc::new(PermanentDelete),
            editor_running: editor,
        }
    }

    fn run_to_end(
        prepared: PreparedPlan,
        context: ExecContext,
        sink: Arc<RecordingSink>,
    ) -> (Task, Arc<Mutex<TaskStore>>) {
        let store = Arc::new(Mutex::new(TaskStore::in_memory()));
        let record = new_task(
            new_task_id(),
            msg("task.apply", &[("project", "P")]),
            &prepared,
        );
        let handle = start(
            store.clone(),
            record,
            prepared,
            context,
            sink,
            Box::new(|_| {}),
        );
        let id = handle.id.clone();
        handle.thread.join().unwrap();
        let task = store.lock().unwrap().get(&id).unwrap().task.clone();
        (task, store)
    }

    #[cfg(windows)]
    #[test]
    fn command_lines() {
        let root = Path::new(r"D:\Program Files\Epic Games\UE_5.5");
        let uproject = Path::new(r"E:\Dev Host\UBotDev.uproject");
        assert_eq!(
            build_arguments(
                root,
                "UBotDevEditor",
                BuildConfiguration::Development.as_str(),
                uproject
            ),
            r#"/S /C ""D:\Program Files\Epic Games\UE_5.5\Engine\Build\BatchFiles\Build.bat" UBotDevEditor Win64 Development -Project="E:\Dev Host\UBotDev.uproject" -WaitMutex -NoHotReloadFromIDE""#
        );
        assert_eq!(
            build_arguments(
                root,
                "FooEditor",
                BuildConfiguration::DebugGame.as_str(),
                Path::new("C:/P/Foo.uproject")
            ),
            r#"/S /C ""D:\Program Files\Epic Games\UE_5.5\Engine\Build\BatchFiles\Build.bat" FooEditor Win64 DebugGame -Project="C:\P\Foo.uproject" -WaitMutex -NoHotReloadFromIDE""#
        );
        assert_eq!(
            run_tests_arguments(uproject, &["UBotCore".into(), "UBotROS".into()]),
            r#""E:\Dev Host\UBotDev.uproject" -unattended -nop4 -nosplash -NullRHI -culture=en -stdout -FullStdOutLogOutput -ExecCmds="Automation RunTests UBotCore+UBotROS; Quit" -TestExit="Automation Test Queue Empty""#
        );
        assert!(editor_cmd_path(root).ends_with(r"Engine\Binaries\Win64\UnrealEditor-Cmd.exe"));
    }

    /// cmd.exe hands the arguments of `build_arguments` to Build.bat intact (it passes `%*` on to
    /// UBT), spaces in paths included: a fake engine whose Build.bat echoes `%*` shows them.
    #[cfg(windows)]
    #[test]
    fn build_command_reaches_the_batch_file_intact() {
        let dir = tempfile::Builder::new()
            .prefix("engine root ")
            .tempdir()
            .unwrap();
        let bat = build_bat_path(dir.path());
        fs::create_dir_all(bat.parent().unwrap()).unwrap();
        fs::write(
            &bat,
            "@echo off\r\necho args=%*\r\necho [1/2] Compile A.cpp\r\necho [2/2] Link A.dll\r\nexit /b 0\r\n",
        )
        .unwrap();
        let uproject = dir.path().join("My Project").join("Foo.uproject");
        let command = raw_command(
            "cmd.exe",
            &build_arguments(dir.path(), "FooEditor", "Development", &uproject),
        );
        let cancel = AtomicBool::new(false);
        let mut lines = Vec::new();
        let outcome = process::run_streaming(command, &cancel, |line| lines.push(line)).unwrap();
        assert!(
            matches!(outcome, RunOutcome::Exited(status) if status.success()),
            "{lines:?}"
        );
        assert_eq!(
            lines[0],
            format!(
                "args=FooEditor Win64 Development -Project=\"{}\" -WaitMutex -NoHotReloadFromIDE",
                paths::display(&uproject)
            )
        );
        let progress: Vec<_> = lines
            .iter()
            .filter_map(|line| parse_progress(line))
            .collect();
        assert_eq!(progress, [(1, 2), (2, 2)]);
    }

    #[test]
    fn parses_progress() {
        assert_eq!(
            parse_progress("[12/345] Compile [x64] Module.UBotCore.cpp"),
            Some((12, 345))
        );
        assert_eq!(
            parse_progress("  [1/1] Link UnrealEditor-UBotCore.dll"),
            Some((1, 1))
        );
        assert_eq!(parse_progress("Building [x64] [3/4] step"), Some((3, 4)));
        assert_eq!(
            parse_progress("[Adaptive Build] Excluded from UBotCore unity file"),
            None
        );
        assert_eq!(parse_progress("[5/0] nothing"), None);
        assert_eq!(parse_progress("[a/b] [/] []"), None);
        assert_eq!(
            parse_progress("Total time in Parallel executor: 5.2 seconds"),
            None
        );
    }

    #[test]
    fn runs_steps_and_records_logs() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        fs::write(&project, "{\r\n\t\"FileVersion\": 3,\r\n\t\"Plugins\": [\r\n\t\t{\r\n\t\t\t\"Name\": \"Old\",\r\n\t\t\t\"Enabled\": true\r\n\t\t}\r\n\t]\r\n}\r\n").unwrap();
        let folder = dir.path().join("Plugins").join("Old");
        fs::create_dir_all(folder.join("sub")).unwrap();
        fs::write(folder.join("sub").join("f.txt"), "x").unwrap();

        let plan = prepared(
            &project,
            vec![
                (
                    StepKind::EnablePackage,
                    StepAction::SetEnabled {
                        plugin: "New".into(),
                        enabled: true,
                    },
                ),
                (
                    StepKind::RemovePackage,
                    StepAction::Remove {
                        package: "Old".into(),
                        dir: folder.clone(),
                    },
                ),
            ],
        );
        let sink = Arc::new(RecordingSink::default());
        let (task, store) = run_to_end(plan, context(Arc::new(|_| false), true), sink.clone());
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        assert!(task
            .steps
            .iter()
            .all(|step| step.status == StepStatus::Done && step.duration_ms.is_some()));
        assert!(!folder.exists());
        let text = fs::read_to_string(&project).unwrap();
        assert_eq!(text, "{\r\n\t\"FileVersion\": 3,\r\n\t\"Plugins\": [\r\n\t\t{\r\n\t\t\t\"Name\": \"New\",\r\n\t\t\t\"Enabled\": true\r\n\t\t}\r\n\t]\r\n}\r\n");

        let store = store.lock().unwrap();
        let log = store.log(&task.id, Some("s1")).unwrap();
        assert_eq!(log, ["New: \"Enabled\": true in P.uproject"]);
        assert_eq!(task.steps[0].log_lines, 1);
        let summary = store.log(&task.id, None).unwrap();
        assert!(summary[0].starts_with("Apply Changes · P: started"));
        assert!(summary[1].starts_with("[1/2] Enable X: done"));
        assert!(summary.last().unwrap().contains("finished"));
        assert!(sink
            .lines
            .lock()
            .unwrap()
            .iter()
            .any(|(step, line)| step == "s2" && line.starts_with("Removed")));
        let statuses: Vec<StepStatus> = sink
            .updates
            .lock()
            .unwrap()
            .iter()
            .map(|task| task.status)
            .collect();
        assert_eq!(statuses.first(), Some(&StepStatus::Running));
        assert_eq!(statuses.last(), Some(&StepStatus::Done));
    }

    #[test]
    fn editor_running_fails_or_waits() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        fs::write(&project, "{}").unwrap();
        let steps = || {
            prepared(
                &project,
                vec![
                    (
                        StepKind::EnablePackage,
                        StepAction::SetEnabled {
                            plugin: "A".into(),
                            enabled: true,
                        },
                    ),
                    (
                        StepKind::EnablePackage,
                        StepAction::SetEnabled {
                            plugin: "B".into(),
                            enabled: true,
                        },
                    ),
                ],
            )
        };

        let (task, store) = run_to_end(
            steps(),
            context(Arc::new(|_| true), false),
            Arc::new(RecordingSink::default()),
        );
        assert_eq!(task.status, StepStatus::Failed);
        assert_eq!(task.steps[0].status, StepStatus::Failed);
        assert_eq!(task.steps[1].status, StepStatus::Skipped);
        assert_eq!(
            task.error.as_deref(),
            Some("Enable X: Unreal Editor is running this project.")
        );
        // The failed step explains itself in its log.
        assert_eq!(task.steps[0].log_lines, 1);
        assert_eq!(
            store.lock().unwrap().log(&task.id, Some("s1")).unwrap(),
            ["error: Unreal Editor is running this project."]
        );

        // The editor closes after two polls.
        let polls = Arc::new(AtomicUsize::new(0));
        let counter = polls.clone();
        let sink = Arc::new(RecordingSink::default());
        let (task, _) = run_to_end(
            steps(),
            context(
                Arc::new(move |_| counter.fetch_add(1, Ordering::SeqCst) < 2),
                true,
            ),
            sink.clone(),
        );
        assert_eq!(task.status, StepStatus::Done);
        assert!(sink
            .updates
            .lock()
            .unwrap()
            .iter()
            .any(|task| task.steps[0].status == StepStatus::Waiting));
    }

    #[test]
    fn cancel_stops_a_waiting_step() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        fs::write(&project, "{}").unwrap();
        let plan = prepared(
            &project,
            vec![
                (
                    StepKind::EnablePackage,
                    StepAction::SetEnabled {
                        plugin: "A".into(),
                        enabled: true,
                    },
                ),
                (
                    StepKind::EnablePackage,
                    StepAction::SetEnabled {
                        plugin: "B".into(),
                        enabled: true,
                    },
                ),
            ],
        );
        let store = Arc::new(Mutex::new(TaskStore::in_memory()));
        let record = new_task(new_task_id(), msg("task.apply", &[("project", "P")]), &plan);
        let handle = start(
            store.clone(),
            record,
            plan,
            context(Arc::new(|_| true), true),
            Arc::new(RecordingSink::default()),
            Box::new(|_| {}),
        );
        thread::sleep(Duration::from_millis(300));
        handle.cancel.store(true, Ordering::SeqCst);
        handle.thread.join().unwrap();
        let task = store.lock().unwrap().get(&handle.id).unwrap().task.clone();
        assert_eq!(task.status, StepStatus::Canceled);
        assert_eq!(task.steps[0].status, StepStatus::Canceled);
        assert_eq!(task.steps[1].status, StepStatus::Skipped);
    }

    #[test]
    fn failed_clone_is_removed() {
        if git::version("git").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        fs::write(&project, "{}").unwrap();
        let dest = dir.path().join("Plugins").join("uBot").join("Missing");
        let plan = prepared(
            &project,
            vec![(
                StepKind::Clone,
                StepAction::Clone {
                    package: "Missing".into(),
                    url: dir
                        .path()
                        .join("no-such-repo.git")
                        .to_string_lossy()
                        .into_owned(),
                    git_ref: "v0.1.0".into(),
                    dest: dest.clone(),
                },
            )],
        );
        let sink = Arc::new(RecordingSink::default());
        let (task, _) = run_to_end(plan, context(Arc::new(|_| false), true), sink);
        assert_eq!(task.status, StepStatus::Failed);
        assert!(task.error.as_deref().unwrap().contains("exit code"));
        assert!(!dest.exists());
    }

    #[test]
    fn history_is_capped_and_interrupted_tasks_are_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.json");
        let mut store = TaskStore::load(path.clone());
        let plan = prepared(
            &dir.path().join("P.uproject"),
            vec![(
                StepKind::Build,
                StepAction::SetEnabled {
                    plugin: "A".into(),
                    enabled: true,
                },
            )],
        );
        for _ in 0..(MAX_TASKS + 5) {
            store.push(new_task(
                new_task_id(),
                msg("task.apply", &[("project", "P")]),
                &plan,
            ));
        }
        let newest = store.list()[0].id.clone();
        store
            .get_mut(&newest)
            .unwrap()
            .logs
            .insert("s1".into(), vec!["line".into()]);
        store.save().unwrap();
        let loaded = TaskStore::load(path);
        let tasks = loaded.list();
        assert_eq!(tasks.len(), MAX_TASKS);
        assert_eq!(tasks[0].id, newest);
        assert_eq!(
            tasks[0].status,
            StepStatus::Failed,
            "a pending task in the history was interrupted"
        );
        assert_eq!(tasks[0].steps[0].status, StepStatus::Skipped);
        assert_eq!(loaded.log(&newest, Some("s1")).unwrap(), ["line"]);
        assert!(loaded.log("nope", None).is_none());
    }

    /// `git` in `dir` with an identity, returning its output.
    fn git_in(dir: &Path, args: &[&str]) -> String {
        let mut command = git::command("git");
        command
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
            ])
            .args(args);
        process::output_text(command).unwrap()
    }

    /// Adds a commit that changes `file` and returns its id.
    fn commit(dir: &Path, file: &str, text: &str) -> String {
        fs::write(dir.join(file), text).unwrap();
        git_in(dir, &["add", file]);
        git_in(dir, &["commit", "-q", "-m", text]);
        git_in(dir, &["rev-parse", "HEAD"])
    }

    /// A repository with `file` committed and tagged `tag`.
    fn tagged_repo(dir: &Path, files: &[&str], tag: &str) {
        fs::create_dir_all(dir).unwrap();
        git_in(dir, &["init", "-q", "-b", "main"]);
        for file in files {
            commit(dir, file, "one");
        }
        git_in(dir, &["tag", tag]);
    }

    #[test]
    fn failed_clone_is_removed_outside_the_project_too() {
        if git::version("git").is_err() {
            return;
        }
        // An install folder in a plugin root that lies outside the project
        // (AdditionalPluginDirectories ../Shared): the partial clone is removed there as well.
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("Host");
        fs::create_dir_all(&project_dir).unwrap();
        let project = project_dir.join("Host.uproject");
        fs::write(&project, "{}").unwrap();
        let repo = dir.path().join("repo");
        // The repository has no Missing.uplugin, so the clone succeeds and is then rejected.
        tagged_repo(&repo, &["readme.txt"], "v0.1.0");
        let dest = dir.path().join("Shared").join("uBot").join("Missing");
        let plan = prepared(
            &project,
            vec![(
                StepKind::Clone,
                StepAction::Clone {
                    package: "Missing".into(),
                    url: repo.to_string_lossy().into_owned(),
                    git_ref: "v0.1.0".into(),
                    dest: dest.clone(),
                },
            )],
        );
        let (task, _) = run_to_end(
            plan,
            context(Arc::new(|_| false), true),
            Arc::new(RecordingSink::default()),
        );
        assert_eq!(task.status, StepStatus::Failed);
        assert!(task.error.as_deref().unwrap().contains("Missing.uplugin"));
        assert!(!dest.exists(), "the partial clone was left behind");
    }

    #[test]
    fn checkout_resolves_tags_branches_of_origin_and_commits() {
        if git::version("git").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        fs::write(&project, "{}").unwrap();
        let origin = dir.path().join("origin");
        tagged_repo(&origin, &["a.txt"], "v0.1.0");
        let tagged = git_in(&origin, &["rev-parse", "HEAD"]);
        git_in(&origin, &["checkout", "-q", "-b", "develop"]);
        let develop_1 = commit(&origin, "b.txt", "develop 1");
        git_in(&origin, &["checkout", "-q", "main"]);

        // Installed at the tag, like the manager does it.
        let work = dir.path().join("work");
        git_in(
            dir.path(),
            &[
                "clone",
                "-q",
                "--branch",
                "v0.1.0",
                "--",
                &origin.to_string_lossy(),
                &work.to_string_lossy(),
            ],
        );

        let head = |dir: &Path| git_in(dir, &["rev-parse", "HEAD"]);
        let checkout = |git_ref: &str| {
            let plan = prepared(
                &project,
                vec![(
                    StepKind::Checkout,
                    StepAction::Checkout {
                        package: "X".into(),
                        dir: work.clone(),
                        git_ref: git_ref.into(),
                    },
                )],
            );
            run_to_end(
                plan,
                context(Arc::new(|_| false), true),
                Arc::new(RecordingSink::default()),
            )
        };

        // A branch that exists only as origin/develop (plain `git checkout --detach develop`
        // fails on it).
        let (task, store) = checkout("develop");
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        assert_eq!(head(&work), develop_1);
        let log = store.lock().unwrap().log(&task.id, Some("s1")).unwrap();
        assert!(
            log.iter().any(|line| line.starts_with("develop -> ")),
            "{log:?}"
        );

        // The remote branch moves on; the next checkout reaches the new commit.
        git_in(&origin, &["checkout", "-q", "develop"]);
        let develop_2 = commit(&origin, "c.txt", "develop 2");
        git_in(&origin, &["checkout", "-q", "main"]);
        let (task, _) = checkout("develop");
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        assert_eq!(head(&work), develop_2);

        // A stale local branch of that name is not what is checked out.
        git_in(&work, &["checkout", "-q", "-B", "develop", &develop_2]);
        git_in(&origin, &["checkout", "-q", "develop"]);
        let develop_3 = commit(&origin, "d.txt", "develop 3");
        git_in(&origin, &["checkout", "-q", "main"]);
        let (task, _) = checkout("develop");
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        assert_eq!(head(&work), develop_3);

        // Tags and commits (full or abbreviated) still work.
        let (task, _) = checkout("v0.1.0");
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        assert_eq!(head(&work), tagged);
        let (task, _) = checkout(&develop_1);
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        assert_eq!(head(&work), develop_1);
        let (task, _) = checkout(&develop_2[..8]);
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        assert_eq!(head(&work), develop_2);

        // Unknown refs and refs that look like options fail without touching the folder.
        for bad in ["no-such-ref", "--pathspec-from-file=x"] {
            let (task, _) = checkout(bad);
            assert_eq!(task.status, StepStatus::Failed, "{bad}");
        }
        assert_eq!(head(&work), develop_2);
    }

    #[test]
    fn checkout_refuses_a_configuration_that_names_a_program() {
        if git::version("git").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        fs::write(&project, "{}").unwrap();
        let origin = dir.path().join("origin");
        tagged_repo(&origin, &["a.txt"], "v0.1.0");
        let work = dir.path().join("work");
        git_in(
            dir.path(),
            &[
                "clone",
                "-q",
                "--",
                &origin.to_string_lossy(),
                &work.to_string_lossy(),
            ],
        );

        // An ssh remote with a program for ssh: a fetch would start it.
        let marker = dir.path().join("ssh-ran.txt");
        let script = dir.path().join("evil-ssh.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho pwned > \"{}\"\nexit 1\n",
                marker.to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        git_in(
            &work,
            &["remote", "set-url", "origin", "ssh://127.0.0.1:1/x.git"],
        );
        git_in(
            &work,
            &[
                "config",
                "core.sshCommand",
                &format!("sh {}", script.to_string_lossy().replace('\\', "/")),
            ],
        );

        // Control: a plain fetch starts the program, so the check below means something.
        let mut plain = process::command("git");
        plain.arg("-C").arg(&work).args(["fetch", "origin"]);
        let _ = process::output_text(plain);
        let control_ran = marker.exists();
        let _ = fs::remove_file(&marker);

        let plan = prepared(
            &project,
            vec![(
                StepKind::Checkout,
                StepAction::Checkout {
                    package: "X".into(),
                    dir: work.clone(),
                    git_ref: "v0.1.0".into(),
                },
            )],
        );
        let (task, _) = run_to_end(
            plan,
            context(Arc::new(|_| false), true),
            Arc::new(RecordingSink::default()),
        );
        assert_eq!(task.status, StepStatus::Failed);
        let error = task.error.as_deref().unwrap();
        assert!(
            error.contains("untrusted .git/config: core.sshcommand"),
            "{error}"
        );
        if control_ran {
            assert!(!marker.exists(), "the checkout step ran core.sshCommand");
        } else {
            eprintln!("core.sshCommand control did not run on this machine; weak check only");
        }
    }

    #[derive(Default)]
    struct CountingRemover(AtomicUsize);

    impl Remover for CountingRemover {
        fn remove(&self, dir: &Path) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            remove_dir_robust(dir).map_err(|error| error.to_string())
        }
    }

    #[test]
    fn remove_refuses_a_plugin_root() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        let original = "{\n\t\"FileVersion\": 3,\n\t\"Plugins\": [ { \"Name\": \"UBotLoose\", \"Enabled\": true } ]\n}\n";
        fs::write(&project, original).unwrap();
        let plugins = dir.path().join("Plugins");
        fs::create_dir_all(plugins.join("Other")).unwrap();
        fs::write(plugins.join("Other").join("Other.uplugin"), "{}").unwrap();
        fs::write(plugins.join("UBotLoose.uplugin"), "{}").unwrap();

        let remover = Arc::new(CountingRemover::default());
        let mut context = context(Arc::new(|_| false), true);
        context.remover = remover.clone();
        let plan = prepared(
            &project,
            vec![(
                StepKind::RemovePackage,
                StepAction::Remove {
                    package: "UBotLoose".into(),
                    dir: plugins.clone(),
                },
            )],
        );
        let (task, _) = run_to_end(plan, context, Arc::new(RecordingSink::default()));
        assert_eq!(task.status, StepStatus::Failed);
        assert!(
            task.error.as_deref().unwrap().contains("plugin folder"),
            "{:?}",
            task.error
        );
        assert_eq!(remover.0.load(Ordering::SeqCst), 0, "nothing was removed");
        assert!(plugins.join("Other").join("Other.uplugin").is_file());
        assert_eq!(
            fs::read_to_string(&project).unwrap(),
            original,
            "the .uproject is untouched"
        );
    }

    /// A sink that panics in every callback, as `println!` does on a closed pipe.
    struct PanickingSink;

    impl TaskSink for PanickingSink {
        fn task_updated(&self, _task: &Task) {
            panic!("the output pipe is closed");
        }
        fn log(&self, _task_id: &str, _step_id: &str, _lines: &[String]) {
            panic!("the output pipe is closed");
        }
    }

    #[test]
    fn a_panicking_sink_cannot_strand_a_half_applied_task() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("P.uproject");
        fs::write(&project, "{}").unwrap();
        let plan = prepared(
            &project,
            vec![
                (
                    StepKind::EnablePackage,
                    StepAction::SetEnabled {
                        plugin: "A".into(),
                        enabled: true,
                    },
                ),
                (
                    StepKind::EnablePackage,
                    StepAction::SetEnabled {
                        plugin: "B".into(),
                        enabled: true,
                    },
                ),
            ],
        );
        let store = Arc::new(Mutex::new(TaskStore::in_memory()));
        let record = new_task(new_task_id(), msg("task.apply", &[("project", "P")]), &plan);
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let handle = start(
            store.clone(),
            record,
            plan,
            context(Arc::new(|_| false), true),
            Arc::new(PanickingSink),
            Box::new(move |_| flag.store(true, Ordering::SeqCst)),
        );
        handle.thread.join().expect("the task thread survives");
        assert!(finished.load(Ordering::SeqCst), "on_finish ran");
        let task = store.lock().unwrap().get(&handle.id).unwrap().task.clone();
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        let text = fs::read_to_string(&project).unwrap();
        assert!(text.contains("\"A\"") && text.contains("\"B\""), "{text}");
    }

    /// A fake engine whose Build.bat fails with `failure` for the target `FooEditor` and succeeds
    /// for anything else.
    #[cfg(windows)]
    fn fake_engine(dir: &Path, failure: &str) -> PathBuf {
        let root = dir.join("Engine Root");
        let bat = build_bat_path(&root);
        fs::create_dir_all(bat.parent().unwrap()).unwrap();
        fs::write(
            &bat,
            format!(
                "@echo off\r\necho target=%1\r\nif \"%1\"==\"FooEditor\" (\r\n  echo {failure}\r\n  exit /b 8\r\n)\r\necho [1/1] Link UnrealEditor-Foo.dll\r\nexit /b 0\r\n"
            ),
        )
        .unwrap();
        root
    }

    #[cfg(windows)]
    fn build_plan(project: &Path, root: &Path, target: &str) -> PreparedPlan {
        let mut plan = prepared(
            project,
            vec![(
                StepKind::Build,
                StepAction::Build {
                    engine_root: root.to_path_buf(),
                    target: target.into(),
                    configuration: crate::model::BuildConfiguration::Development,
                    uproject: project.to_path_buf(),
                },
            )],
        );
        plan.plan.steps[0].title = msg("step.build", &[("target", target)]);
        plan
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_editor_target_falls_back_to_the_stock_editor() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Foo.uproject");
        fs::write(&project, "{}").unwrap();

        let root = fake_engine(
            dir.path(),
            "ERROR: Couldn't find target rules file for target 'FooEditor' in rules assembly 'UE5Rules'.",
        );
        let (task, store) = run_to_end(
            build_plan(&project, &root, "FooEditor"),
            context(Arc::new(|_| false), true),
            Arc::new(RecordingSink::default()),
        );
        assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
        let log = store.lock().unwrap().log(&task.id, Some("s1")).unwrap();
        assert!(
            log.iter().any(|line| line.contains("target=FooEditor")),
            "{log:?}"
        );
        assert!(
            log.iter().any(|line| line.contains("target=UnrealEditor")),
            "{log:?}"
        );
        assert!(
            log.iter()
                .any(|line| line.contains("building UnrealEditor with -Project")),
            "{log:?}"
        );
        assert_eq!(
            task.steps[0].title.params["target"], "UnrealEditor",
            "the step shows the target that was built"
        );

        // Any other failure is a failure.
        let root = fake_engine(dir.path(), "error C2065: undeclared identifier");
        let (task, store) = run_to_end(
            build_plan(&project, &root, "FooEditor"),
            context(Arc::new(|_| false), true),
            Arc::new(RecordingSink::default()),
        );
        assert_eq!(task.status, StepStatus::Failed);
        let log = store.lock().unwrap().log(&task.id, Some("s1")).unwrap();
        assert!(!log.iter().any(|line| line.contains("target=UnrealEditor")));
    }

    #[cfg(windows)]
    #[test]
    fn the_build_target_must_be_a_plain_name() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Foo.uproject");
        fs::write(&project, "{}").unwrap();
        let root = fake_engine(dir.path(), "unused");
        let marker = dir.path().join("PWNED");
        let target = format!("x&mkdir \"{}\"&y", marker.display());
        let (task, _) = run_to_end(
            build_plan(&project, &root, &target),
            context(Arc::new(|_| false), true),
            Arc::new(RecordingSink::default()),
        );
        assert_eq!(task.status, StepStatus::Failed);
        assert!(task.error.as_deref().unwrap().contains("plain target name"));
        assert!(!marker.exists(), "nothing was executed");

        // cmd.exe expands %NAME% inside quotes.
        let percent = dir.path().join("a%TEMP%b");
        fs::create_dir_all(&percent).unwrap();
        let uproject = percent.join("Foo.uproject");
        fs::write(&uproject, "{}").unwrap();
        let (task, _) = run_to_end(
            build_plan(&uproject, &root, "FooEditor"),
            context(Arc::new(|_| false), true),
            Arc::new(RecordingSink::default()),
        );
        assert_eq!(task.status, StepStatus::Failed);
        assert!(task.error.as_deref().unwrap().contains('%'));

        assert!(is_safe_build_name("UnrealEditor"));
        assert!(is_safe_build_name("My_Game2Editor"));
        for bad in ["", "a b", "a&b", "a|b", "a^b", "a(b)", "a%b", "a\"b", "a;b"] {
            assert!(!is_safe_build_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_unreadable_history_is_kept_aside_and_good_records_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.json");
        let plan = prepared(
            &dir.path().join("P.uproject"),
            vec![(
                StepKind::EnablePackage,
                StepAction::SetEnabled {
                    plugin: "A".into(),
                    enabled: true,
                },
            )],
        );

        // Garbage: nothing is loaded, and the file is copied aside before it is replaced.
        fs::write(&path, "{ \"tasks\": [ oops").unwrap();
        let mut store = TaskStore::load(path.clone());
        assert!(store.list().is_empty());
        store.push(new_task(
            new_task_id(),
            msg("task.apply", &[("project", "P")]),
            &plan,
        ));
        store.save().unwrap();
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".bad-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(backups[0].path()).unwrap(),
            "{ \"tasks\": [ oops"
        );
        store.save().unwrap();
        assert_eq!(
            fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().contains(".bad-"))
                .count(),
            1,
            "a valid file is not backed up again"
        );

        // One record the format no longer fits does not take the others along.
        let mut file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        file["tasks"]
            .as_array_mut()
            .unwrap()
            .insert(0, serde_json::json!({ "task": "from the future" }));
        fs::write(&path, file.to_string()).unwrap();
        let loaded = TaskStore::load(path);
        assert_eq!(loaded.list().len(), 1);
    }
}

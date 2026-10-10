//! `Manager` under concurrency: a slow index download must not hold anything else up, a project
//! cannot be switched while a task runs, and stale settings cannot revert the recent projects.

// The task test needs a fake Build.bat, so it is Windows only; the helpers it alone uses are not
// dead code there.
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ubot_core::engine::EngineRegistry;
use ubot_core::exec::PermanentDelete;
use ubot_core::model::{ApplyOptions, Change, ChangeKind, Snapshot, StepStatus, Task};
use ubot_core::paths::AppDirs;
use ubot_core::sources::Fetcher;
use ubot_core::{Manager, ManagerOptions, TaskSink};

struct NoEngines;

impl EngineRegistry for NoEngines {
    fn launcher_installs(&self) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    fn registered_builds(&self) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
}

#[derive(Default)]
struct Sink {
    snapshots: Mutex<Vec<Snapshot>>,
}

impl TaskSink for Sink {
    fn task_updated(&self, _task: &Task) {}
    fn log(&self, _task_id: &str, _step_id: &str, _lines: &[String]) {}
    fn snapshot_updated(&self, snapshot: &Snapshot) {
        self.snapshots.lock().unwrap().push(snapshot.clone());
    }
}

/// Serves an index after a delay, as an unreachable server does after its timeout.
struct SlowFetcher {
    delay: Duration,
    calls: AtomicUsize,
}

impl Fetcher for SlowFetcher {
    fn fetch(&self, _url: &str) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.delay);
        Ok(INDEX.to_string())
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

const INDEX: &str = r#"{
    "formatVersion": 2, "name": "test",
    "packages": [
        { "name": "UBotCore", "description": { "en": "Core", "zh-CN": "核心" }, "layer": "Foundation", "repository": "https://example.com/uBotCore.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0" } ] },
        { "name": "UBotSensor", "description": { "en": "Sensor", "zh-CN": "传感器" }, "layer": "Capability", "repository": "https://example.com/uBotSensor.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ] } ] }
    ]
}"#;

fn descriptor(requires: &str) -> String {
    format!(
        r#"{{ "FileVersion": 3, "VersionName": "0.1.0", "Modules": [ {{ "Name": "M", "Type": "Runtime" }} ], "UBot": {{ "Layer": "Foundation", "Requires": [ {requires} ] }} }}"#
    )
}

/// A project with UBotCore enabled and UBotSensor installed but disabled.
fn project(dir: &Path, name: &str, engine_association: &str) -> PathBuf {
    let project_dir = dir.join(name);
    let uproject = project_dir.join(format!("{name}.uproject"));
    write(
        &uproject,
        &format!(
            "{{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"{engine_association}\",\n    \"Plugins\": [ {{ \"Name\": \"UBotSensor\", \"Enabled\": false }} ]\n}}\n"
        ),
    );
    write(
        &project_dir.join("Plugins/uBot/uBotCore/UBotCore.uplugin"),
        &descriptor(""),
    );
    write(
        &project_dir.join("Plugins/uBot/uBotSensor/UBotSensor.uplugin"),
        &descriptor(r#"{ "Name": "UBotCore", "Version": "^0.1.0" }"#),
    );
    uproject
}

fn manager(home: &Path, fetcher: Arc<dyn Fetcher>) -> Manager {
    Manager::new(ManagerOptions {
        dirs: AppDirs::under(home.to_path_buf()),
        offline: false,
        write_merged_index: false,
        remover: Arc::new(PermanentDelete),
        fetcher,
        engine_registry: Arc::new(NoEngines),
        ..ManagerOptions::default()
    })
}

#[test]
fn plan_and_settings_do_not_wait_for_a_slow_sync() {
    let temp = tempfile::tempdir().unwrap();
    let uproject = project(temp.path(), "Sim", "9.9");
    let fetcher = Arc::new(SlowFetcher {
        delay: Duration::from_secs(2),
        calls: AtomicUsize::new(0),
    });
    let manager = manager(&temp.path().join("home"), fetcher.clone());
    manager.select_project(&uproject.to_string_lossy()).unwrap();
    manager.rescan().unwrap();
    assert_eq!(
        fetcher.calls.load(Ordering::SeqCst),
        0,
        "a rescan does not fetch"
    );

    let sync = {
        let manager = manager.clone();
        thread::spawn(move || manager.refresh(true))
    };
    thread::sleep(Duration::from_millis(300));
    assert!(manager.get_snapshot().syncing, "the sync is reported");

    // Planning, saving settings and reading the snapshot are not held up by the download.
    let started = Instant::now();
    let plan = manager
        .plan(
            &[Change::new(ChangeKind::Enable, "UBotSensor", None)],
            &ApplyOptions::default(),
        )
        .unwrap();
    assert!(plan.errors.is_empty(), "{:?}", plan.errors);
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "plan took {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    manager.save_settings(manager.get_settings()).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "save_settings took {:?}",
        started.elapsed()
    );
    let started = Instant::now();
    manager.get_snapshot();
    assert!(started.elapsed() < Duration::from_millis(900));

    // The sync itself still gets its result, from a single download.
    let snapshot = sync.join().unwrap().unwrap();
    let remote = snapshot.index.iter().find(|s| s.id == "ubot").unwrap();
    assert!(remote.ok && !remote.from_cache, "{remote:?}");
    assert!(remote.fetched_at.is_some());
    assert_eq!(remote.packages, 2);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    assert!(!manager.get_snapshot().syncing);
}

#[test]
fn stale_settings_cannot_revert_the_recent_projects() {
    let temp = tempfile::tempdir().unwrap();
    let a = project(temp.path(), "A", "9.9");
    let b = project(temp.path(), "B", "9.9");
    let fetcher = Arc::new(SlowFetcher {
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    });
    let manager = manager(&temp.path().join("home"), fetcher);
    manager.open_project(&a.to_string_lossy()).unwrap();
    let stale = manager.get_settings();
    manager.open_project(&b.to_string_lossy()).unwrap();

    // A dialog that was opened before B was: it saves its copy of the settings.
    let mut edited = stale;
    edited.build.wait_for_editor = false;
    manager.save_settings(edited).unwrap();

    let settings = manager.get_settings();
    assert!(!settings.build.wait_for_editor, "the edit is kept");
    let last = settings.last_project.unwrap();
    assert!(last.ends_with("B.uproject"), "{last}");
    assert!(settings.recent_projects[0].ends_with("B.uproject"));
    assert_eq!(settings.recent_projects.len(), 2);
}

/// A fake engine root above the project (an empty `EngineAssociation` walks up to it) whose
/// Build.bat takes a few seconds.
#[cfg(windows)]
#[test]
fn the_project_cannot_be_switched_while_a_task_runs() {
    let temp = tempfile::tempdir().unwrap();
    let engine = temp.path().join("Engine Root");
    write(
        &engine.join("Engine/Build/Build.version"),
        r#"{ "MajorVersion": 5, "MinorVersion": 5, "PatchVersion": 4 }"#,
    );
    write(
        &engine.join("Engine/Build/BatchFiles/Build.bat"),
        "@echo off\r\necho [1/1] Link\r\nping -n 4 127.0.0.1 >nul\r\nexit /b 0\r\n",
    );
    let a = project(&engine.join("Games"), "A", "");
    let b = project(&engine.join("Games"), "B", "");
    let fetcher = Arc::new(SlowFetcher {
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    });
    let manager = manager(&temp.path().join("home"), fetcher);
    manager.open_project(&a.to_string_lossy()).unwrap();

    let sink = Arc::new(Sink::default());
    let id = manager
        .apply(
            &[Change::new(ChangeKind::Enable, "UBotSensor", None)],
            &ApplyOptions {
                build: true,
                reopen_editor: false,
                run_tests: false,
            },
            None,
            Some("Mine".into()),
            sink.clone(),
        )
        .unwrap();
    thread::sleep(Duration::from_millis(500));

    // B cannot be opened while the task for A runs; A stays the open project.
    for result in [
        manager.select_project(&b.to_string_lossy()),
        manager.open_project(&b.to_string_lossy()).map(|_| ()),
    ] {
        assert_eq!(result.unwrap_err(), "Another task is running.");
    }
    assert!(manager.project_path().unwrap().ends_with("A.uproject"));
    assert!(!manager
        .get_settings()
        .last_project
        .unwrap()
        .ends_with("B.uproject"));

    manager.wait_for_task(&id);
    let task = &manager.list_tasks()[0];
    assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
    // The recipe is A's package set.
    let after = sink.snapshots.lock().unwrap().last().cloned().unwrap();
    let recipe = after.recipes.iter().find(|r| r.user).unwrap();
    assert_eq!(recipe.id, "user-mine");
    assert_eq!(recipe.packages, ["UBotCore", "UBotSensor"]);

    // Once the task is over the switch works.
    let snapshot = manager.open_project(&b.to_string_lossy()).unwrap();
    assert_eq!(snapshot.project.unwrap().name, "B");
}

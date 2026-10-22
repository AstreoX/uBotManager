//! The `Manager` facade as the Tauri shell uses it, on a temporary project without git or engine.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ubot_core::engine::EngineRegistry;
use ubot_core::exec::PermanentDelete;
use ubot_core::model::{ApplyOptions, Change, ChangeKind, FixAction, Snapshot, StepStatus, Task};
use ubot_core::paths::AppDirs;
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
    tasks: Mutex<Vec<Task>>,
    lines: Mutex<Vec<String>>,
    snapshots: Mutex<Vec<Snapshot>>,
}

impl TaskSink for Sink {
    fn task_updated(&self, task: &Task) {
        self.tasks.lock().unwrap().push(task.clone());
    }
    fn log(&self, _task_id: &str, _step_id: &str, lines: &[String]) {
        self.lines.lock().unwrap().extend(lines.iter().cloned());
    }
    fn snapshot_updated(&self, snapshot: &Snapshot) {
        self.snapshots.lock().unwrap().push(snapshot.clone());
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
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ] } ] },
        { "name": "UBotROS", "description": { "en": "ROS", "zh-CN": "ROS" }, "layer": "Adapter", "repository": "https://example.com/uBotROS.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotSensor", "version": "^0.1.0" } ] } ] }
    ],
    "recipes": [ { "id": "sensors", "name": { "en": "Sensors", "zh-CN": "传感器" }, "description": { "en": "S", "zh-CN": "S" }, "packages": [ "UBotCore", "UBotSensor" ] } ],
    "replacements": [ { "legacy": "AgentSensorCore", "replacement": "UBotSensor" } ]
}"#;

fn descriptor(version: &str, layer: &str, requires: &str) -> String {
    format!(
        r#"{{ "FileVersion": 3, "VersionName": "{version}", "Modules": [ {{ "Name": "M", "Type": "Runtime" }} ], "UBot": {{ "Layer": "{layer}", "Requires": [ {requires} ] }} }}"#
    )
}

#[test]
fn apply_rejects_conflicting_explicit_installs_before_starting_a_task() {
    for reverse in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let project_dir = root.join("Project");
        let uproject = project_dir.join("Sim.uproject");
        let original = "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"\"\n}\n";
        write(&uproject, original);
        let index = root.join("index.json");
        write(
            &index,
            include_str!("fixtures/install-version-constraints.json"),
        );
        let home = root.join("home");
        // Offline only blocks index fetching. If the gate regresses, even the failing test must
        // never launch real git or fetch an archive.
        let missing_git = root.join("git-must-not-run");
        assert!(!missing_git.exists());
        write(
            &home.join("settings.json"),
            &serde_json::json!({
                "install": { "git": missing_git, "usePrebuilt": false },
                "ros": { "checkOnRefresh": false }
            })
            .to_string(),
        );
        let manager = Manager::new(ManagerOptions {
            dirs: AppDirs::under(home.clone()),
            offline: true,
            index_override: Some(vec![index.to_string_lossy().into_owned()]),
            engine_registry: Arc::new(NoEngines),
            ..ManagerOptions::default()
        });
        manager.open_project(&uproject.to_string_lossy()).unwrap();
        let sink = Arc::new(Sink::default());
        let mut changes = vec![
            Change::new(ChangeKind::Install, "UBotROS", Some("0.1.3".into())),
            Change::new(ChangeKind::Install, "UBotCore", Some("0.1.0".into())),
        ];
        if reverse {
            changes.reverse();
        }
        // Deliberately call apply directly: it must enforce the gate without a prior plan.
        let result = manager.apply(
            &changes,
            &ApplyOptions {
                build: false,
                reopen_editor: false,
                run_tests: false,
            },
            None,
            None,
            sink.clone(),
        );
        if let Ok(id) = &result {
            manager.wait_for_task(id);
        }
        let error = result.expect_err("a conflicting plan must not start a task");
        assert!(
            error.contains("UBotROS requires UBotCore ^0.1.3."),
            "{error}"
        );
        assert!(manager.list_tasks().is_empty());
        assert!(sink.tasks.lock().unwrap().is_empty());
        assert!(sink.lines.lock().unwrap().is_empty());
        assert_eq!(fs::read_to_string(&uproject).unwrap(), original);
        assert!(!project_dir.join("Plugins").exists());
        assert!(!home.join("tasks.json").exists());
    }
}

#[test]
fn facade_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let project_dir = root.join("Sim");
    let uproject = project_dir.join("Sim.uproject");
    write(
        &uproject,
        "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"9.9\"\n}\n",
    );
    write(
        &project_dir.join("Plugins/uBot/uBotCore/UBotCore.uplugin"),
        &descriptor("0.1.0", "Foundation", ""),
    );
    write(
        &project_dir.join("Plugins/uBot/uBotSensor/UBotSensor.uplugin"),
        &descriptor(
            "0.1.0",
            "Capability",
            r#"{ "Name": "UBotCore", "Version": "^0.1.0" }"#,
        ),
    );
    write(
        &project_dir.join("Plugins/uBot/uBotROS/UBotROS.uplugin"),
        &descriptor(
            "0.1.0",
            "Adapter",
            r#"{ "Name": "UBotSensor", "Version": "^0.1.0" }"#,
        ),
    );
    write(
        &project_dir.join("Plugins/AgentSensorCore/AgentSensorCore.uplugin"),
        r#"{ "VersionName": "0.5.0" }"#,
    );
    write(
        &project_dir.join("Saved/uBot/status.json"),
        r#"{ "formatVersion": 1, "session": { "kind": "Editor", "startedAt": "2026-10-10T07:00:00Z" }, "engineVersion": "9.9.0",
        "problems": [ { "id": "ros.bridge", "package": "UBotROS", "severity": "warning", "code": "rosBridgeUnreachable", "params": { "address": "10.0.0.9:8268" } } ] }"#,
    );
    let index_path = root.join("index.json");
    fs::write(&index_path, INDEX).unwrap();
    let home = root.join("home");
    write(
        &home.join("settings.json"),
        r#"{ "ros": { "checkOnRefresh": false } }"#,
    );

    let manager = Manager::new(ManagerOptions {
        dirs: AppDirs::under(home.clone()),
        offline: true,
        index_override: Some(vec![index_path.to_string_lossy().into_owned()]),
        write_merged_index: true,
        remover: Arc::new(PermanentDelete),
        engine_registry: Arc::new(NoEngines),
        ..ManagerOptions::default()
    });
    assert!(manager.get_snapshot().project.is_none());

    let snapshot = manager.open_project(&uproject.to_string_lossy()).unwrap();
    let project = snapshot.project.as_ref().unwrap();
    assert_eq!(project.name, "Sim");
    assert!(project.engine.is_none());
    assert_eq!(snapshot.session.as_ref().unwrap().engine_version, "9.9.0");
    let ids: Vec<&str> = snapshot.problems.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        ["engine", "legacy:AgentSensorCore", "session:ros.bridge"]
    );
    assert!(
        project_dir.join("Saved/uBot/index.json").is_file(),
        "the merged index is written for the UE panel"
    );
    assert_eq!(manager.get_settings().recent_projects.len(), 1);

    // Ignoring a problem hides it and is remembered per project.
    let snapshot = manager.ignore_problem("engine").unwrap();
    assert!(!snapshot.problems.iter().any(|p| p.id == "engine"));
    let legacy = snapshot
        .problems
        .iter()
        .find(|p| p.id == "legacy:AgentSensorCore")
        .unwrap();
    let FixAction::Changes { changes } = &legacy.fixes[0].action else {
        panic!("a change fix")
    };

    // Planning the primary fix; then disabling UBotSensor (and UBotROS) with a saved recipe.
    let plan = manager.plan(changes, &ApplyOptions::default()).unwrap();
    assert!(plan.errors.is_empty());
    assert_eq!(plan.steps.len(), 1);
    let options = ApplyOptions {
        build: true,
        reopen_editor: false,
        run_tests: false,
    };
    let sink = Arc::new(Sink::default());
    let changes = [Change::new(ChangeKind::Disable, "UBotSensor", None)];
    let id = manager
        .apply(
            &changes,
            &options,
            None,
            Some("Core only".into()),
            sink.clone(),
        )
        .unwrap();
    manager.wait_for_task(&id);

    let tasks = manager.list_tasks();
    assert_eq!(tasks[0].id, id);
    assert_eq!(tasks[0].status, StepStatus::Done, "{:?}", tasks[0].error);
    assert_eq!(tasks[0].steps.len(), 2);
    assert_eq!(
        manager.get_task_log(&id, Some("s1")).unwrap(),
        ["UBotROS: \"Enabled\": false in Sim.uproject"]
    );
    assert!(manager.get_task_log(&id, None).unwrap().len() >= 3);
    assert_eq!(
        sink.tasks.lock().unwrap().last().unwrap().status,
        StepStatus::Done
    );
    assert!(!sink.lines.lock().unwrap().is_empty());
    let snapshots = sink.snapshots.lock().unwrap();
    let after = snapshots
        .last()
        .expect("the snapshot is refreshed after the task");
    let state = |name: &str| {
        after
            .packages
            .iter()
            .find(|p| p.name == name)
            .unwrap()
            .state
    };
    assert_eq!(
        serde_json::to_value(state("UBotSensor")).unwrap(),
        "disabled"
    );
    assert_eq!(serde_json::to_value(state("UBotROS")).unwrap(), "disabled");
    let recipe = after
        .recipes
        .iter()
        .find(|r| r.user)
        .expect("the saved recipe");
    assert_eq!(
        (recipe.id.as_str(), recipe.packages.clone()),
        ("user-core-only", vec!["UBotCore".to_string()])
    );
    drop(snapshots);

    // Recipes become changes; the plan without an engine still works without a build.
    assert_eq!(
        manager.recipe_changes(Some("sensors"), &[]),
        [Change::new(ChangeKind::Enable, "UBotSensor", None)]
    );
    let plan = manager
        .plan(
            &manager.recipe_changes(Some("sensors"), &[]),
            &ApplyOptions {
                build: true,
                ..ApplyOptions::default()
            },
        )
        .unwrap();
    assert_eq!(plan.errors[0].code, "plan.noEngine");
    assert!(
        manager.cancel_task(&id).is_ok(),
        "canceling a finished task is a no-op"
    );

    // ROS settings go to the project's Config/DefaultUBotROS.ini when UBotROS is installed.
    let mut settings = manager.get_settings();
    assert_eq!(settings.ros.port, 8268);
    settings.ros.host = "10.1.2.3".into();
    settings.ros.port = 9000;
    settings.build.wait_for_editor = false;
    manager.save_settings(settings).unwrap();
    let ini = fs::read_to_string(project_dir.join("Config/DefaultUBotROS.ini")).unwrap();
    assert!(
        ini.contains("BridgeHost=10.1.2.3") && ini.contains("BridgePort=9000"),
        "{ini}"
    );
    let settings = manager.get_settings();
    assert_eq!(
        (
            settings.ros.host.as_str(),
            settings.ros.port,
            settings.build.wait_for_editor
        ),
        ("10.1.2.3", 9000, false)
    );
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        saved["ros"]["host"], "127.0.0.1",
        "the app-level address is kept"
    );
    assert_eq!(saved["ignoredProblems"].as_object().unwrap().len(), 1);

    // A second manager reads the history and the last project back.
    let again = Manager::new(ManagerOptions {
        dirs: AppDirs::under(home),
        offline: true,
        index_override: Some(vec![index_path.to_string_lossy().into_owned()]),
        write_merged_index: false,
        engine_registry: Arc::new(NoEngines),
        ..ManagerOptions::default()
    });
    assert_eq!(again.list_tasks().len(), 1);
    assert_eq!(again.project_path().as_deref(), Some(uproject.as_path()));
    let snapshot = again.get_snapshot();
    assert!(
        !snapshot.problems.iter().any(|p| p.id == "engine"),
        "ignored problems stay ignored"
    );
}

/// The index file for the UE panel lives in the project folder: UE clones with the repository
/// URLs (credentials kept), but it ignores `binaries`, whose URLs must not leave secrets there.
#[test]
fn the_panel_index_keeps_repository_credentials_but_not_binaries_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let project_dir = root.join("Sim");
    let uproject = project_dir.join("Sim.uproject");
    write(&uproject, "{\n    \"FileVersion\": 3\n}\n");
    let index = r#"{
        "formatVersion": 2, "name": "test",
        "packages": [
            { "name": "UBotCore", "description": { "en": "Core", "zh-CN": "核心" }, "layer": "Foundation",
              "repository": "https://user:REPOTOKEN@example.com/uBotCore.git",
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0",
                  "binaries": { "5.5": { "Win64": {
                      "url": "https://u:SECRETPW@example.com/dl/UBotCore.zip?sig=SECRETSIG",
                      "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" } } } } ] }
        ]
    }"#;
    let index_path = root.join("index.json");
    fs::write(&index_path, index).unwrap();
    let home = root.join("home");
    write(
        &home.join("settings.json"),
        r#"{ "ros": { "checkOnRefresh": false } }"#,
    );
    let manager = Manager::new(ManagerOptions {
        dirs: AppDirs::under(home),
        offline: true,
        index_override: Some(vec![index_path.to_string_lossy().into_owned()]),
        write_merged_index: true,
        remover: Arc::new(PermanentDelete),
        engine_registry: Arc::new(NoEngines),
        ..ManagerOptions::default()
    });
    manager.open_project(&uproject.to_string_lossy()).unwrap();
    let panel = fs::read_to_string(project_dir.join("Saved/uBot/index.json")).unwrap();
    assert!(
        !panel.contains("SECRETPW") && !panel.contains("SECRETSIG"),
        "{panel}"
    );
    assert!(
        panel.contains("https://***@example.com/dl/UBotCore.zip"),
        "{panel}"
    );
    assert!(
        panel.contains("https://user:REPOTOKEN@example.com/uBotCore.git"),
        "UE clones with the repository URL as written: {panel}"
    );
}

//! Installing prebuilt binaries through the `Manager` facade: a launcher engine (fake), an index
//! whose binaries are local archives, no build step, the install marker, and the engine mismatch
//! problem with its reinstall fix.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use ubot_core::engine::EngineRegistry;
use ubot_core::exec::PermanentDelete;
use ubot_core::model::{
    ApplyOptions, Change, ChangeKind, FixAction, InstallKind, PackageInfo, PrebuiltInfo, Snapshot,
    StepKind, StepStatus, Task,
};
use ubot_core::paths::AppDirs;
use ubot_core::prebuilt::{self, InstallMarker};
use ubot_core::{Manager, ManagerOptions, TaskSink};

struct Launcher(Vec<(String, PathBuf)>);

impl EngineRegistry for Launcher {
    fn launcher_installs(&self) -> Vec<(String, PathBuf)> {
        self.0.clone()
    }
    fn registered_builds(&self) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
}

struct Quiet;

impl TaskSink for Quiet {
    fn task_updated(&self, _task: &Task) {}
    fn log(&self, _task_id: &str, _step_id: &str, _lines: &[String]) {}
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// An engine root whose Build.bat fails: a prebuilt install must never build.
fn fake_engine(root: &Path, minor: u32) {
    write(
        &root.join("Engine/Build/Build.version"),
        &format!(r#"{{ "MajorVersion": 5, "MinorVersion": {minor}, "PatchVersion": 1 }}"#),
    );
    write(
        &root.join("Engine/Build/BatchFiles/Build.bat"),
        "@echo off\r\necho a prebuilt install must not build\r\nexit /b 1\r\n",
    );
}

fn descriptor(name: &str, requires: &str) -> String {
    format!(
        r#"{{ "FileVersion": 3, "VersionName": "0.1.0", "FriendlyName": "{name}", "Modules": [ {{ "Name": "{name}", "Type": "Runtime" }} ], "UBot": {{ "Layer": "Capability", "Requires": [ {requires} ] }} }}"#
    )
}

fn package<'a>(snapshot: &'a Snapshot, name: &str) -> &'a PackageInfo {
    snapshot.packages.iter().find(|p| p.name == name).unwrap()
}

#[test]
fn installs_and_reinstalls_prebuilt_binaries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let engine_55 = root.join("UE_5.5");
    let engine_56 = root.join("UE_5.6");
    fake_engine(&engine_55, 5);
    fake_engine(&engine_56, 6);

    // The project has UBotCore, built (a copied folder with its binaries).
    let project_dir = root.join("Sim");
    let uproject = project_dir.join("Sim.uproject");
    write(
        &uproject,
        "{\n\t\"FileVersion\": 3,\n\t\"EngineAssociation\": \"5.5\"\n}\n",
    );
    let core = project_dir.join("Plugins/uBot/uBotCore");
    write(&core.join("UBotCore.uplugin"), &descriptor("UBotCore", ""));
    write(
        &core.join("Binaries/Win64/UnrealEditor-UBotCore.dll"),
        "dll",
    );

    // The archive of UBotSensor 0.1.0, packed from a built plugin folder.
    let built = root.join("built/uBotSensor");
    write(
        &built.join("UBotSensor.uplugin"),
        &descriptor(
            "UBotSensor",
            r#"{ "Name": "UBotCore", "Version": "^0.1.0" }"#,
        ),
    );
    write(
        &built.join("Binaries/Win64/UnrealEditor-UBotSensor.dll"),
        "dll",
    );
    write(&built.join("Binaries/Win64/UnrealEditor.modules"), "{}");
    write(&built.join("Intermediate/Build/x.obj"), "obj");
    let archive = root.join("mirror/UBotSensor-0.1.0-UE5.5-Win64.zip");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    let packed = prebuilt::pack_package(&built, "UBotSensor", &archive, false).unwrap();
    let binary =
        json!({ "url": archive.to_string_lossy(), "sha256": packed.sha256, "size": packed.size });
    let index = json!({
        "formatVersion": 2, "name": "prebuilt",
        "packages": [
            { "name": "UBotCore", "description": { "en": "Core", "zh-CN": "核心" }, "layer": "Foundation",
              "repository": "https://example.com/uBotCore.git",
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0" } ] },
            { "name": "UBotSensor", "description": { "en": "Sensor", "zh-CN": "传感器" }, "layer": "Capability",
              "repository": "https://example.com/uBotSensor.git",
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0",
                              "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ],
                              "binaries": { "5.5": { "Win64": binary }, "5.6": { "Win64": binary } } } ] }
        ]
    });
    let index_path = root.join("index.json");
    fs::write(&index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();
    let validation = ubot_core::index::validate_text(&fs::read_to_string(&index_path).unwrap());
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);

    let home = root.join("home");
    write(
        &home.join("settings.json"),
        r#"{ "language": "en", "ros": { "checkOnRefresh": false } }"#,
    );
    let manager = Manager::new(ManagerOptions {
        dirs: AppDirs::under(home.clone()),
        offline: true,
        index_override: Some(vec![index_path.to_string_lossy().into_owned()]),
        write_merged_index: false,
        remover: Arc::new(PermanentDelete),
        engine_registry: Arc::new(Launcher(vec![
            ("5.5".into(), engine_55.clone()),
            ("5.6".into(), engine_56.clone()),
        ])),
        compiler_found: Arc::new(|| true),
        ..ManagerOptions::default()
    });
    let snapshot = manager.open_project(&uproject.to_string_lossy()).unwrap();
    assert!(package(&snapshot, "UBotSensor").prebuilt_available);
    assert_eq!(package(&snapshot, "UBotSensor").install_kind, None);
    assert_eq!(
        package(&snapshot, "UBotCore").install_kind,
        Some(InstallKind::Other)
    );

    let options = ApplyOptions {
        build: true,
        reopen_editor: false,
        run_tests: false,
    };
    let install = [Change::new(ChangeKind::Install, "UBotSensor", None)];
    let plan = manager.plan(&install, &options).unwrap();
    assert!(plan.errors.is_empty(), "{:?}", plan.errors);
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    assert_eq!(
        plan.steps.iter().map(|s| s.kind).collect::<Vec<_>>(),
        [StepKind::Download, StepKind::EnablePackage],
        "no build step"
    );
    assert!(!plan.needs_build);

    let id = manager
        .apply(&install, &options, None, None, Arc::new(Quiet))
        .unwrap();
    manager.wait_for_task(&id);
    let task = &manager.list_tasks()[0];
    assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);

    let sensor_dir = project_dir.join("Plugins/uBot/uBotSensor");
    assert!(prebuilt::has_editor_binaries(&sensor_dir));
    assert!(
        !sensor_dir.join("Intermediate").exists(),
        "not in the archive"
    );
    let marker = InstallMarker::read(&sensor_dir, "UBotSensor").unwrap();
    assert_eq!(marker.engine, "5.5");
    assert_eq!(marker.sha256, packed.sha256);
    let snapshot = manager.refresh(false).unwrap();
    let sensor = package(&snapshot, "UBotSensor");
    assert_eq!(sensor.install_kind, Some(InstallKind::Prebuilt));
    assert_eq!(sensor.installed_version.as_deref(), Some("0.1.0"));
    assert_eq!(
        sensor.prebuilt,
        Some(PrebuiltInfo {
            engine: "5.5".into(),
            platform: "Win64".into()
        }),
        "what the marker says"
    );
    assert_eq!(package(&snapshot, "UBotCore").prebuilt, None);
    assert_eq!(
        serde_json::to_value(sensor.state).unwrap(),
        "enabled",
        "{sensor:?}"
    );
    assert!(sensor.problem_ids.is_empty(), "{:?}", sensor.problem_ids);

    // The project moves to UE 5.6: the binaries no longer fit.
    write(
        &uproject,
        "{\n\t\"FileVersion\": 3,\n\t\"EngineAssociation\": \"5.6\",\n\t\"Plugins\": [ { \"Name\": \"UBotSensor\", \"Enabled\": true } ]\n}\n",
    );
    let snapshot = manager.refresh(false).unwrap();
    let mismatch = snapshot
        .problems
        .iter()
        .find(|p| p.id == "prebuilt:UBotSensor")
        .expect("the engine mismatch is reported");
    assert_eq!(mismatch.message.params["built"], "5.5");
    assert_eq!(mismatch.message.params["engine"], "5.6");
    assert_eq!(
        package(&snapshot, "UBotSensor")
            .prebuilt
            .as_ref()
            .map(|built| built.engine.as_str()),
        Some("5.5"),
        "the package says what it was built for, not what the project uses"
    );
    let FixAction::Changes { changes } = &mismatch.fixes[0].action else {
        panic!("the reinstall fix is a change")
    };
    assert_eq!(
        changes,
        &[Change::new(
            ChangeKind::Update,
            "UBotSensor",
            Some("0.1.0".into())
        )]
    );

    // The fix downloads the binaries for UE 5.6 and replaces the folder.
    let plan = manager.plan(changes, &options).unwrap();
    assert!(plan.errors.is_empty(), "{:?}", plan.errors);
    assert_eq!(
        plan.steps.iter().map(|s| s.kind).collect::<Vec<_>>(),
        [StepKind::Download]
    );
    assert!(plan.steps[0].needs_editor_closed);
    let id = manager
        .apply(changes, &options, None, None, Arc::new(Quiet))
        .unwrap();
    manager.wait_for_task(&id);
    let task = &manager.list_tasks()[0];
    assert_eq!(task.status, StepStatus::Done, "{:?}", task.error);
    assert_eq!(
        InstallMarker::read(&sensor_dir, "UBotSensor")
            .unwrap()
            .engine,
        "5.6"
    );
    let snapshot = manager.refresh(false).unwrap();
    assert!(!snapshot
        .problems
        .iter()
        .any(|p| p.id == "prebuilt:UBotSensor"));
    // Nothing of the download is left next to the package.
    let mut names: Vec<String> = fs::read_dir(project_dir.join("Plugins/uBot"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["uBotCore", "uBotSensor"]);

    // A disabled package raises no mismatch problem (only enabled ones do), but what it says it
    // was built for is still the marker's engine (UE 5.6 now), not the project's.
    write(
        &uproject,
        "{\n\t\"FileVersion\": 3,\n\t\"EngineAssociation\": \"5.5\",\n\t\"Plugins\": [ { \"Name\": \"UBotSensor\", \"Enabled\": false } ]\n}\n",
    );
    let snapshot = manager.refresh(false).unwrap();
    assert!(!snapshot
        .problems
        .iter()
        .any(|p| p.id == "prebuilt:UBotSensor"));
    let sensor = package(&snapshot, "UBotSensor");
    assert_eq!(serde_json::to_value(sensor.state).unwrap(), "disabled");
    assert_eq!(
        sensor.prebuilt,
        Some(PrebuiltInfo {
            engine: "5.6".into(),
            platform: "Win64".into()
        })
    );
}

//! End-to-end test of the `ubot` CLI against local bare copies of the real uBotCore, uBotSensor
//! and uBotROS repositories (SPEC §13). The real repositories are only read (`git clone --bare`);
//! tags and commits are made in the temporary copies.
//!
//! The repositories are looked up in `UBOT_PLUGINS_DIR`, else in `../../../../Plugins/uBot`
//! relative to this crate (the uBot workspace layout). Without them, or without git, the test is
//! skipped.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const REPOS: [(&str, &str); 3] = [
    ("UBotCore", "uBotCore"),
    ("UBotSensor", "uBotSensor"),
    ("UBotROS", "uBotROS"),
];

fn plugins_dir() -> PathBuf {
    match std::env::var_os("UBOT_PLUGINS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../Plugins/uBot"),
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=uBot Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct Env {
    project: PathBuf,
    index: PathBuf,
    home: PathBuf,
}

impl Env {
    fn ubot(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_ubot"))
            .arg("--project")
            .arg(&self.project)
            .arg("--index")
            .arg(&self.index)
            .arg("--offline")
            .args(args)
            .env("UBOT_MANAGER_HOME", &self.home)
            .env("UBOT_MANAGER_PERMANENT_DELETE", "1")
            .stdin(Stdio::null())
            .output()
            .expect("ubot runs");
        eprintln!(
            "$ ubot {}\n{}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn ubot_ok(&self, args: &[&str]) -> String {
        let output = self.ubot(args);
        assert!(output.status.success(), "ubot {args:?} failed");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        serde_json::from_str(&self.ubot_ok(&all)).expect("JSON output")
    }

    fn plugin_dir(&self, folder: &str) -> PathBuf {
        self.project
            .parent()
            .unwrap()
            .join("Plugins")
            .join("uBot")
            .join(folder)
    }

    fn uproject(&self) -> String {
        fs::read_to_string(&self.project).unwrap()
    }

    fn version_name(&self, folder: &str, name: &str) -> String {
        let text =
            fs::read_to_string(self.plugin_dir(folder).join(format!("{name}.uplugin"))).unwrap();
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
        value["VersionName"].as_str().unwrap().to_string()
    }
}

fn package<'a>(snapshot: &'a Value, name: &str) -> &'a Value {
    snapshot["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no package {name}"))
}

fn problem_ids(problems: &Value) -> Vec<String> {
    problems
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect()
}

const ORIGINAL_UPROJECT: &str = "{\r\n\t\"FileVersion\": 3,\r\n\t\"EngineAssociation\": \"5.5\",\r\n\t\"Category\": \"\",\r\n\t\"Description\": \"\",\r\n\t\"Plugins\": [\r\n\t\t{\r\n\t\t\t\"Name\": \"ModelingToolsEditorMode\",\r\n\t\t\t\"Enabled\": true,\r\n\t\t\t\"TargetAllowList\": [\r\n\t\t\t\t\"Editor\"\r\n\t\t\t]\r\n\t\t}\r\n\t]\r\n}\r\n";

/// `text` (the contents of a `.uplugin`) with its `VersionName` value replaced by `version`.
fn with_version_name(text: &str, version: &str) -> String {
    const KEY: &str = "\"VersionName\"";
    let after_key = text.find(KEY).expect("the descriptor has a VersionName") + KEY.len();
    let open = after_key
        + text[after_key..]
            .find('"')
            .expect("VersionName has a value")
        + 1;
    let close = open + text[open..].find('"').expect("VersionName value ends");
    format!("{}{version}{}", &text[..open], &text[close..])
}

/// Makes `v{version}` a release in the bare copy `remotes/{folder}.git`: a commit on top of its
/// branch whose `{name}.uplugin` says `version` (no commit when it already does), tagged
/// `v{version}`. The commit is made in a temporary working clone.
fn release(root: &Path, remotes: &Path, name: &str, folder: &str, version: &str) {
    let bare = remotes.join(format!("{folder}.git"));
    let work = root.join(format!("work-{folder}-{version}"));
    let output = Command::new("git")
        .args(["clone", "--quiet", "--"])
        .arg(&bare)
        .arg(&work)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let descriptor = work.join(format!("{name}.uplugin"));
    let text = fs::read_to_string(&descriptor).unwrap();
    let released = with_version_name(&text, version);
    if released != text {
        fs::write(&descriptor, released).unwrap();
        git(
            &work,
            &[
                "commit",
                "--quiet",
                "--no-verify",
                "-am",
                &format!("Release {version}"),
            ],
        );
        let branch = git(&work, &["rev-parse", "--abbrev-ref", "HEAD"]);
        git(&work, &["push", "--quiet", "origin", &branch]);
    }
    // The bare copy's HEAD is its branch, which now carries the release.
    git(&bare, &["tag", "-f", &format!("v{version}"), "HEAD"]);
}

fn setup() -> Option<(tempfile::TempDir, Env)> {
    let plugins = plugins_dir();
    if REPOS
        .iter()
        .any(|(_, folder)| !plugins.join(folder).join(".git").exists())
    {
        eprintln!(
            "skipping: the uBot repositories are not in {}",
            plugins.display()
        );
        return None;
    }
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: git is not available");
        return None;
    }

    // Under target/tmp, so nothing lands on the system drive's temp folder.
    let temp = tempfile::Builder::new()
        .prefix("e2e-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = temp.path().to_path_buf();
    let remotes = root.join("remotes");
    fs::create_dir_all(&remotes).unwrap();

    for (name, folder) in REPOS {
        let bare = remotes.join(format!("{folder}.git"));
        let source = plugins.join(folder);
        let output = Command::new("git")
            .args(["clone", "--bare", "--no-hardlinks", "--quiet", "--"])
            .arg(&source)
            .arg(&bare)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Whatever version the real repository's HEAD carries, the copy's v0.1.0 is a commit whose
        // descriptor says 0.1.0 (a v0.1.0 tag the real repository already has is moved to it).
        release(&root, &remotes, name, folder, "0.1.0");
    }

    // UBotCore 0.2.0: one more commit on top, which bumps VersionName.
    release(&root, &remotes, "UBotCore", "uBotCore", "0.2.0");

    let repo = |folder: &str| {
        remotes
            .join(format!("{folder}.git"))
            .to_string_lossy()
            .into_owned()
    };
    let index = serde_json::json!({
        "formatVersion": 2,
        "name": "e2e",
        "packages": [
            { "name": "UBotCore", "friendlyName": "uBot Core", "description": { "en": "Core", "zh-CN": "核心" }, "layer": "Foundation",
              "repository": repo("uBotCore"),
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0" }, { "version": "0.2.0", "ref": "v0.2.0" } ] },
            { "name": "UBotSensor", "description": { "en": "Sensor", "zh-CN": "传感器" }, "layer": "Capability",
              "repository": repo("uBotSensor"),
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ] } ] },
            { "name": "UBotROS", "description": { "en": "ROS", "zh-CN": "ROS" }, "layer": "Adapter",
              "repository": repo("uBotROS"),
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0",
                              "requires": [ { "name": "UBotCore", "version": "^0.1.0" }, { "name": "UBotSensor", "version": "^0.1.0" } ] } ] }
        ],
        "recipes": [ { "id": "ros", "name": { "en": "ROS", "zh-CN": "ROS" }, "description": { "en": "ROS", "zh-CN": "ROS" },
                       "packages": [ "UBotCore", "UBotSensor", "UBotROS" ], "checks": [ "rosBridge" ] } ],
        "replacements": [ { "legacy": "AgentSensorCore", "replacement": "UBotSensor" } ]
    });
    let index_path = root.join("index.json");
    fs::write(&index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let project_dir = root.join("Project");
    fs::create_dir_all(project_dir.join("Plugins").join("AgentSensorCore")).unwrap();
    fs::write(project_dir.join("Foo.uproject"), ORIGINAL_UPROJECT).unwrap();
    fs::write(
        project_dir.join("Plugins/AgentSensorCore/AgentSensorCore.uplugin"),
        "{ \"VersionName\": \"0.5.0\" }",
    )
    .unwrap();

    // The bridge check would take seconds per run on Windows; it is covered by unit tests.
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("settings.json"),
        r#"{ "language": "en", "ros": { "checkOnRefresh": false }, "custom": 1 }"#,
    )
    .unwrap();

    let env = Env {
        project: project_dir.join("Foo.uproject"),
        index: index_path,
        home,
    };
    Some((temp, env))
}

#[test]
fn install_update_disable_uninstall() {
    let Some((_temp, env)) = setup() else {
        return;
    };

    // The index is valid.
    let output = Command::new(env!("CARGO_BIN_EXE_ubot"))
        .args(["index", "validate"])
        .arg(&env.index)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    // Without --yes and with no answer on stdin nothing happens.
    let output = env.ubot(&["apply", "install:UBotROS", "--no-build"]);
    assert!(!output.status.success());
    assert!(!env.plugin_dir("uBotCore").exists());

    // Install UBotROS: clones and enables all three packages, no build.
    let out = env.ubot_ok(&["apply", "install:UBotROS", "--no-build", "--yes"]);
    assert!(out.contains("Clone UBotCore 0.1.0"), "{out}");
    assert!(!out.contains("Build FooEditor"));
    for (name, folder) in REPOS {
        assert!(
            env.plugin_dir(folder)
                .join(format!("{name}.uplugin"))
                .is_file(),
            "{name} is cloned"
        );
        assert_eq!(
            git(
                &env.plugin_dir(folder),
                &["describe", "--tags", "--exact-match"]
            ),
            "v0.1.0"
        );
    }
    let snapshot = env.json(&["status"]);
    for (name, _) in REPOS {
        let entry = package(&snapshot, name);
        assert_eq!(entry["state"], "enabled", "{name}");
        assert_eq!(entry["installedVersion"], "0.1.0");
        assert_eq!(entry["git"]["ref"], "v0.1.0");
        assert_eq!(entry["git"]["dirty"], false);
        assert_eq!(entry["indexSource"], "cli-1");
    }
    assert_eq!(
        package(&snapshot, "UBotCore")["location"],
        "Plugins/uBot/uBotCore"
    );
    assert_eq!(package(&snapshot, "UBotCore")["updateAvailable"], true);
    let merged: Value = serde_json::from_str(
        &fs::read_to_string(env.project.parent().unwrap().join("Saved/uBot/index.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(merged["formatVersion"], 2);
    assert_eq!(merged["name"], "merged");
    assert_eq!(
        env.uproject(),
        ORIGINAL_UPROJECT.replace(
            "\t\t}\r\n\t]",
            "\t\t},\r\n\t\t{\r\n\t\t\t\"Name\": \"UBotCore\",\r\n\t\t\t\"Enabled\": true\r\n\t\t},\r\n\t\t{\r\n\t\t\t\"Name\": \"UBotSensor\",\r\n\t\t\t\"Enabled\": true\r\n\t\t},\r\n\t\t{\r\n\t\t\t\"Name\": \"UBotROS\",\r\n\t\t\t\"Enabled\": true\r\n\t\t}\r\n\t]"
        )
    );

    // Problems: the legacy plugin conflicts with UBotSensor; requirements are fine.
    let problems = problem_ids(&env.json(&["problems"]));
    assert!(
        problems.contains(&"legacy:AgentSensorCore".to_string()),
        "{problems:?}"
    );
    assert!(
        !problems.iter().any(|id| id.starts_with("requires:")),
        "{problems:?}"
    );
    let text = env.ubot_ok(&["problems"]);
    assert!(
        text.contains("AgentSensorCore and UBotSensor are both enabled."),
        "{text}"
    );

    // UBotCore 0.2.0 does not satisfy the enabled dependents' ^0.1.0.
    let output = env.ubot(&["plan", "update:UBotCore@0.2.0"]);
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("error: UBotROS requires UBotCore ^0.1.0."),
        "{text}"
    );
    assert!(
        text.contains("error: UBotSensor requires UBotCore ^0.1.0."),
        "{text}"
    );

    // Disabling UBotSensor disables its dependent UBotROS too.
    let out = env.ubot_ok(&["apply", "disable:UBotSensor", "--yes"]);
    assert!(
        out.contains("warning: Disabling UBotSensor also disables UBotROS."),
        "{out}"
    );
    let snapshot = env.json(&["status"]);
    assert_eq!(package(&snapshot, "UBotSensor")["state"], "disabled");
    assert_eq!(package(&snapshot, "UBotROS")["state"], "disabled");
    assert!(!problem_ids(&snapshot["problems"]).contains(&"legacy:AgentSensorCore".to_string()));

    // Now the update is a checkout of v0.2.0.
    let out = env.ubot_ok(&["apply", "update:UBotCore@0.2.0", "--no-build", "--yes"]);
    assert!(out.contains("Check out UBotCore 0.2.0"), "{out}");
    assert_eq!(env.version_name("uBotCore", "UBotCore"), "0.2.0");
    assert_eq!(
        git(
            &env.plugin_dir("uBotCore"),
            &["describe", "--tags", "--exact-match"]
        ),
        "v0.2.0"
    );

    // Enabling UBotSensor again resolves its ^0.1.0 requirement by checking UBotCore 0.1.0 out.
    let out = env.ubot_ok(&["apply", "enable:UBotSensor", "--no-build", "--yes"]);
    let checkout = out
        .find("1. Check out UBotCore 0.1.0  (0.2.0 → 0.1.0)")
        .expect("the plan checks UBotCore 0.1.0 out first");
    let enable = out
        .find("2. Enable UBotSensor")
        .expect("then enables UBotSensor");
    assert!(checkout < enable, "{out}");
    assert_eq!(env.version_name("uBotCore", "UBotCore"), "0.1.0");
    let problems = problem_ids(&env.json(&["problems"]));
    assert!(
        !problems.iter().any(|id| id.starts_with("requires:")),
        "{problems:?}"
    );

    // Uninstall UBotROS: the entry goes, the folder is removed (permanently in tests).
    let out = env.ubot_ok(&["apply", "uninstall:UBotROS", "--no-build", "--yes"]);
    assert!(out.contains("Remove UBotROS"), "{out}");
    assert!(!env.plugin_dir("uBotROS").exists());
    let snapshot = env.json(&["status"]);
    assert_eq!(package(&snapshot, "UBotROS")["state"], "notInstalled");

    // The .uproject kept its tabs, CRLF line endings, trailing newline and the original entry.
    assert_eq!(
        env.uproject(),
        ORIGINAL_UPROJECT.replace(
            "\t\t}\r\n\t]",
            "\t\t},\r\n\t\t{\r\n\t\t\t\"Name\": \"UBotCore\",\r\n\t\t\t\"Enabled\": true\r\n\t\t},\r\n\t\t{\r\n\t\t\t\"Name\": \"UBotSensor\",\r\n\t\t\t\"Enabled\": true\r\n\t\t}\r\n\t]"
        )
    );

    // History and settings live under UBOT_MANAGER_HOME; unknown settings keys survive.
    let tasks: Value =
        serde_json::from_str(&fs::read_to_string(env.home.join("tasks.json")).unwrap()).unwrap();
    let statuses: Vec<&str> = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["task"]["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["done"; 5]);
    let settings: Value =
        serde_json::from_str(&fs::read_to_string(env.home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(settings["custom"], 1);
    assert_eq!(settings["ros"]["checkOnRefresh"], false);
    assert!(settings["lastProject"]
        .as_str()
        .unwrap()
        .ends_with("Foo.uproject"));
}

//! CLI behaviour that needs no git repositories: `--index` handling, recipes, JSON output and a
//! closed output pipe. Everything runs against a temporary project, index and
//! `UBOT_MANAGER_HOME`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const INDEX: &str = r#"{
    "formatVersion": 2, "name": "cli-test",
    "packages": [
        { "name": "UBotCore", "description": { "en": "Core", "zh-CN": "核心" }, "layer": "Foundation",
          "repository": "https://user:SECRET123@example.com/team/uBotCore.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0" } ] },
        { "name": "UBotSensor", "description": { "en": "Sensor", "zh-CN": "传感器" }, "layer": "Capability",
          "repository": "https://example.com/uBotSensor.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ] } ] },
        { "name": "UBotAerial", "description": { "en": "Aerial", "zh-CN": "空中" }, "layer": "Content",
          "versions": [ { "version": "0.1.0", "planned": true, "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ] } ] }
    ],
    "recipes": [
        { "id": "sensors", "name": { "en": "Sensors Only", "zh-CN": "仅传感器" }, "description": { "en": "S", "zh-CN": "S" },
          "packages": [ "UBotCore", "UBotSensor" ] },
        { "id": "drone", "name": { "en": "Drone", "zh-CN": "无人机" }, "description": { "en": "D", "zh-CN": "D" },
          "packages": [ "UBotCore", "UBotSensor", "UBotAerial" ] }
    ]
}"#;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

struct Env {
    /// The directory the CLI runs in (`--index` files are relative to it).
    cwd: PathBuf,
    project: PathBuf,
    home: PathBuf,
}

impl Env {
    /// No plugins or user settings; all acquisition paths are inert even if apply's gate fails.
    fn empty(root: &Path) -> Env {
        let cwd = root.join("shell");
        let project = root.join("Project/Sim.uproject");
        let home = root.join("home");
        write(
            &project,
            "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"\"\n}\n",
        );
        write(
            &cwd.join("idx.json"),
            include_str!("../../ubot-core/tests/fixtures/install-version-constraints.json"),
        );
        let missing_git = root.join("git-must-not-run");
        assert!(!missing_git.exists());
        write(
            &home.join("settings.json"),
            &serde_json::json!({
                "language": "en",
                "install": { "git": missing_git, "usePrebuilt": false },
                "ros": { "checkOnRefresh": false }
            })
            .to_string(),
        );
        Env { cwd, project, home }
    }

    /// A project with UBotCore and UBotSensor installed and enabled, an index file `idx.json`
    /// in `cwd` (not in the project).
    fn new(root: &Path) -> Env {
        let cwd = root.join("shell");
        let project_dir = root.join("Project");
        let project = project_dir.join("Sim.uproject");
        write(
            &project,
            "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"\"\n}\n",
        );
        write(
            &project_dir.join("Plugins/uBot/uBotCore/UBotCore.uplugin"),
            r#"{ "VersionName": "0.1.0", "UBot": { "Layer": "Foundation" } }"#,
        );
        write(
            &project_dir.join("Plugins/uBot/uBotSensor/UBotSensor.uplugin"),
            r#"{ "VersionName": "0.1.0", "UBot": { "Layer": "Capability", "Requires": [ { "Name": "UBotCore", "Version": "^0.1.0" } ] } }"#,
        );
        write(&cwd.join("idx.json"), INDEX);
        let home = root.join("home");
        write(
            &home.join("settings.json"),
            r#"{ "language": "en", "ros": { "checkOnRefresh": false } }"#,
        );
        Env { cwd, project, home }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ubot"));
        command
            .current_dir(&self.cwd)
            .arg("--project")
            .arg(&self.project)
            .arg("--offline")
            .args(args)
            .env("UBOT_MANAGER_HOME", &self.home)
            .env("UBOT_MANAGER_PERMANENT_DELETE", "1")
            .stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = self.command(args).output().expect("ubot runs");
        eprintln!(
            "$ ubot {}\n{}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn explicit_install_conflicts_are_reported_in_json() {
    for reverse in [false, true] {
        for apply in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let env = Env::empty(temp.path());
            let original = fs::read(&env.project).unwrap();
            let mut changes = ["install:UBotROS@0.1.3", "install:UBotCore@0.1.0"];
            if reverse {
                changes.reverse();
            }
            let mut args = vec!["--index", "idx.json", "--json"];
            if apply {
                args.extend(["apply", "--no-build", "--yes"]);
            } else {
                args.push("plan");
            }
            args.extend(changes);
            let output = env.run(&args);
            assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
            let plan: Value = serde_json::from_str(&stdout(&output)).unwrap();
            let expected = serde_json::json!({
                "code": "plan.breaksDependent",
                "params": { "package": "UBotCore", "dependent": "UBotROS", "range": "^0.1.3" }
            });
            assert_eq!(
                plan["errors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|error| **error == expected)
                    .count(),
                1,
                "{plan}"
            );
            let core = plan["changes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|change| change["package"] == "UBotCore")
                .unwrap();
            assert_eq!(core["version"], "0.1.0");
            // Plan can also report noEngine (build defaults to true); the exact dependency
            // diagnostic above, rather than the exit code alone, proves this regression.
            assert_eq!(fs::read(&env.project).unwrap(), original);
            assert!(!env.project.parent().unwrap().join("Plugins").exists());
            assert!(!env.home.join("tasks.json").exists());
        }
    }
}

#[test]
fn index_files_are_relative_to_the_current_directory() {
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());

    // `idx.json` is next to the shell, not in the project: it is found, and it is what is used.
    let output = env.run(&["--index", "idx.json", "--json", "index", "show"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(shown["sources"][0]["id"], "cli-1");
    assert_eq!(shown["sources"][0]["ok"], true);
    assert_eq!(
        shown["sources"].as_array().unwrap().len(),
        1,
        "no built-in copy"
    );
    assert_eq!(shown["index"]["name"], "merged");
    assert_eq!(shown["index"]["packages"].as_array().unwrap().len(), 3);

    let output = env.run(&["--index", "idx.json", "--json", "status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let snapshot: Value = serde_json::from_str(&stdout(&output)).unwrap();
    let core = snapshot["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "UBotCore")
        .unwrap();
    assert_eq!(core["indexSource"], "cli-1");
}

#[test]
fn an_index_that_cannot_be_loaded_is_an_error_not_a_silent_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());

    for command in [
        &["--index", "missing.json", "status"][..],
        &["--index", "missing.json", "plan", "enable:UBotCore"],
        &["--index", "missing.json", "recipes"],
    ] {
        let output = env.run(command);
        assert!(!output.status.success(), "{command:?}");
        let error = stderr(&output);
        assert!(error.contains("--index"), "{error}");
        assert!(
            error.contains("file not found"),
            "English, not the OS language: {error}"
        );
        assert!(
            !stdout(&output).contains("Built-in copy"),
            "the built-in index is not used instead"
        );
    }

    // `index show` is how such a source is inspected: it lists it.
    let output = env.run(&["--index", "missing.json", "index", "show"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("not loaded"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_recipe_with_planned_packages_is_not_applied() {
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());

    let output = env.run(&["--index", "idx.json", "recipes"]);
    assert!(output.status.success());
    assert!(
        stdout(&output).contains("not available"),
        "{}",
        stdout(&output)
    );

    for args in [
        &[
            "--index",
            "idx.json",
            "recipe",
            "drone",
            "--no-build",
            "--yes",
        ][..],
        &[
            "--index",
            "idx.json",
            "--json",
            "recipe",
            "drone",
            "--no-build",
            "--yes",
        ],
    ] {
        let output = env.run(args);
        assert!(!output.status.success(), "{args:?}");
        let error = stderr(&output);
        assert!(
            error.contains(
                "recipe 'drone' is not available: UBotAerial is planned or not in the index"
            ),
            "{error}"
        );
        assert!(!stdout(&output).contains("is installed and enabled"));
    }
}

#[test]
fn a_recipe_with_nothing_to_do_says_so_in_both_formats() {
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());

    let output = env.run(&[
        "--index",
        "idx.json",
        "recipe",
        "sensors",
        "--no-build",
        "--yes",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("Every package of Sensors Only is installed and enabled."),
        "{}",
        stdout(&output)
    );

    let output = env.run(&[
        "--index",
        "idx.json",
        "--json",
        "recipe",
        "sensors",
        "--no-build",
        "--yes",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let plan: Value = serde_json::from_str(&stdout(&output)).expect("JSON on stdout");
    assert_eq!(plan["steps"], serde_json::json!([]));
    assert_eq!(plan["changes"], serde_json::json!([]));
    assert_eq!(plan["errors"], serde_json::json!([]));
}

#[test]
fn credentials_in_repository_urls_are_not_printed() {
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());

    let output = env.run(&["--index", "idx.json", "--json", "index", "show"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("SECRET123"), "{text}");
    assert!(
        text.contains("https://***@example.com/team/uBotCore.git"),
        "{text}"
    );

    for args in [
        &["--index", "idx.json", "info", "UBotCore"][..],
        &["--index", "idx.json", "index", "show"],
        &["--index", "idx.json", "--json", "status"],
    ] {
        let output = env.run(args);
        assert!(!stdout(&output).contains("SECRET123"), "{args:?}");
        assert!(!stderr(&output).contains("SECRET123"), "{args:?}");
    }
}

#[test]
fn credentials_in_index_source_urls_and_names_are_not_printed() {
    const SOURCE: &str = "https://user:SECRET456@127.0.0.1:1/i.json";
    const REDACTED: &str = "https://***@127.0.0.1:1/i.json";
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());

    // `--index`: the source is called by its URL.
    let output = env.run(&["--index", SOURCE, "--json", "index", "show"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(shown["sources"][0]["name"], REDACTED);
    assert_eq!(shown["sources"][0]["location"], REDACTED);
    for args in [
        &["--index", SOURCE, "index", "show"][..],
        &["--index", SOURCE, "--json", "index", "show"],
        &["--index", SOURCE, "status"],
        &["--index", SOURCE, "--json", "status"],
        &["--index", SOURCE, "problems"],
        &["--index", SOURCE, "--json", "problems"],
    ] {
        let output = env.run(args);
        assert!(!stdout(&output).contains("SECRET456"), "{args:?}");
        assert!(!stderr(&output).contains("SECRET456"), "{args:?}");
    }
    assert!(stdout(&env.run(&["--index", SOURCE, "index", "show"])).contains(REDACTED));

    // A source saved in the settings without a name is called by its location, too.
    write(
        &env.home.join("settings.json"),
        &format!(
            r#"{{ "language": "en", "ros": {{ "checkOnRefresh": false }},
                 "index": {{ "sources": [ {{ "id": "team", "name": "", "location": "{SOURCE}", "enabled": true }} ] }} }}"#
        ),
    );
    for args in [
        &["index", "show"][..],
        &["--json", "index", "show"],
        &["status"],
        &["problems"],
        &["--json", "problems"],
    ] {
        let output = env.run(args);
        assert!(!stdout(&output).contains("SECRET456"), "{args:?}");
        assert!(!stderr(&output).contains("SECRET456"), "{args:?}");
    }
    let problems = stdout(&env.run(&["problems"]));
    assert!(
        problems.contains(&format!("Could not load {REDACTED}")),
        "{problems}"
    );
}

#[test]
fn an_index_url_with_an_unreadable_cache_is_an_error_not_a_silent_fallback() {
    const URL: &str = "https://127.0.0.1:1/i.json";
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());
    let cache = ubot_core::sources::cache_file(&env.home.join("cache"), "cli-1", URL);
    write(&cache, "{ this is not json");

    // The corrupt cache serves nothing: the command stops instead of listing the built-in copy.
    let output = env.run(&["--index", URL, "status"]);
    assert!(!output.status.success(), "{}", stdout(&output));
    let error = stderr(&output);
    assert!(error.contains(&format!("--index {URL}")), "{error}");
    assert!(error.contains("cached copy: Invalid JSON"), "{error}");

    // `index show` still lists the source, as one that failed.
    let output = env.run(&["--index", URL, "--json", "index", "show"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(shown["sources"][0]["ok"], false);
    assert_eq!(shown["sources"][0]["fromCache"], false);
}

#[test]
fn a_closed_output_pipe_is_not_a_panic() {
    let temp = tempfile::tempdir().unwrap();
    let env = Env::new(temp.path());
    // Like `ubot ... | head -0`: the reader is gone before the output is written.
    let mut command = env.command(&["--index", "idx.json", "status"]);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!error.contains("panicked"), "{error}");
    assert_ne!(output.status.code(), Some(101), "{error}");
}

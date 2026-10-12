//! `ubot package` end to end with a fake launcher engine whose Build.bat "builds" every plugin of
//! the host project (no real UnrealBuildTool): small git repositories made in the test, a local
//! index pointing at them, the archives and their layout, the `.sha256` files, the URL
//! template, `--update-index` on a temporary copy of uBotCore's real index, and an archive
//! installed again by the core.
//!
//! Needs git and Windows (the fake engine is a batch file); skipped otherwise.
#![cfg(windows)]

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::AtomicBool;

use serde_json::Value;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
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
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A plugin repository tagged v0.1.0.
fn plugin_repo(dir: &Path, name: &str, requires: &str) {
    write(
        &dir.join(format!("{name}.uplugin")),
        &format!(
            r#"{{
    "FileVersion": 3,
    "VersionName": "0.1.0",
    "FriendlyName": "{name}",
    "Modules": [ {{ "Name": "{name}", "Type": "Runtime", "LoadingPhase": "Default" }} ],
    "UBot": {{ "Layer": "Capability", "Requires": [ {requires} ] }}
}}
"#
        ),
    );
    write(
        &dir.join(format!("Source/{name}/{name}.Build.cs")),
        "// module rules\n",
    );
    write(&dir.join("Config/FilterPlugin.ini"), "[FilterPlugin]\n");
    write(&dir.join("Resources/Icon128.png"), "png");
    write(&dir.join(".gitignore"), "Binaries/\nIntermediate/\n");
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--no-verify", "-m", "0.1.0"]);
    git(dir, &["tag", "v0.1.0"]);
}

/// The fake engine: an installed build of UE 5.5 whose Build.bat writes editor binaries, a
/// symbol file and intermediate files into every plugin of the project it is given.
fn fake_engine(root: &Path) {
    write(
        &root.join("Engine/Build/Build.version"),
        r#"{ "MajorVersion": 5, "MinorVersion": 5, "PatchVersion": 4 }"#,
    );
    write(&root.join("Engine/Build/InstalledBuild.txt"), "");
    write(
        &root.join("Engine/Build/BatchFiles/Build.bat"),
        concat!(
            "@echo off\r\n",
            "echo Build.bat %*\r\n",
            "set \"PROJ=\"\r\n",
            ":next\r\n",
            "if \"%~1\"==\"\" goto done\r\n",
            "if /i \"%~x1\"==\".uproject\" set \"PROJ=%~dp1\"\r\n",
            "shift\r\n",
            "goto next\r\n",
            ":done\r\n",
            "if \"%PROJ%\"==\"\" (\r\n",
            "  echo no project\r\n",
            "  exit /b 2\r\n",
            ")\r\n",
            "echo [1/2] Compile\r\n",
            "for /d %%P in (\"%PROJ%Plugins\\uBot\\*\") do (\r\n",
            "  if not exist \"%%P\\Binaries\\Win64\" mkdir \"%%P\\Binaries\\Win64\"\r\n",
            "  if not exist \"%%P\\Intermediate\\Build\" mkdir \"%%P\\Intermediate\\Build\"\r\n",
            "  echo dll> \"%%P\\Binaries\\Win64\\UnrealEditor-%%~nxP.dll\"\r\n",
            "  echo pdb> \"%%P\\Binaries\\Win64\\UnrealEditor-%%~nxP.pdb\"\r\n",
            "  echo {\"BuildId\": \"fake\"}> \"%%P\\Binaries\\Win64\\UnrealEditor.modules\"\r\n",
            "  echo obj> \"%%P\\Intermediate\\Build\\x.obj\"\r\n",
            ")\r\n",
            "echo [2/2] Link\r\n",
            "exit /b 0\r\n",
        ),
    );
}

fn ubotcore_index() -> Option<PathBuf> {
    let dir = match std::env::var_os("UBOTCORE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../Plugins/uBot/uBotCore"),
    };
    let index = dir.join("Index").join("index.json");
    index.is_file().then_some(index)
}

struct Env {
    root: PathBuf,
    index: PathBuf,
    engine: PathBuf,
    home: PathBuf,
}

impl Env {
    fn ubot(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_ubot"))
            .current_dir(&self.root)
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
}

fn setup() -> Option<(tempfile::TempDir, Env)> {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: git is not available");
        return None;
    }
    // Under target/tmp, so nothing lands on the system drive's temp folder.
    let temp = tempfile::Builder::new()
        .prefix("package-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = temp.path().to_path_buf();
    let repos = root.join("repos");
    plugin_repo(&repos.join("uBotCore"), "UBotCore", "");
    plugin_repo(
        &repos.join("uBotSensor"),
        "UBotSensor",
        r#"{ "Name": "UBotCore", "Version": "^0.1.0" }"#,
    );
    let repo = |folder: &str| repos.join(folder).to_string_lossy().into_owned();
    let index = serde_json::json!({
        "formatVersion": 2, "name": "package-test",
        "packages": [
            { "name": "UBotCore", "description": { "en": "Core", "zh-CN": "核心" }, "layer": "Foundation",
              "repository": repo("uBotCore"),
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0" } ] },
            { "name": "UBotSensor", "description": { "en": "Sensor", "zh-CN": "传感器" }, "layer": "Capability",
              "repository": repo("uBotSensor"),
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ] } ] },
            { "name": "UBotLater", "description": { "en": "Later", "zh-CN": "以后" }, "layer": "Content",
              "versions": [ { "version": "0.1.0", "planned": true } ] }
        ]
    });
    let index_path = root.join("index.json");
    fs::write(&index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();
    // A space in the engine path: the build command line quotes it.
    let engine = root.join("Epic Games").join("UE_5.5");
    fake_engine(&engine);
    let home = root.join("home");
    write(
        &home.join("settings.json"),
        r#"{ "language": "en", "ros": { "checkOnRefresh": false } }"#,
    );
    Some((
        temp,
        Env {
            root,
            index: index_path,
            engine,
            home,
        },
    ))
}

fn entries(archive: &Path) -> BTreeSet<String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
        .collect()
}

fn read_entry(archive: &Path, name: &str) -> String {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut text = String::new();
    zip.by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn packs_built_packages_and_updates_an_index() {
    let Some((_temp, env)) = setup() else {
        return;
    };
    let out = env.root.join("out");
    let engine = env.engine.to_string_lossy().into_owned();
    let out_arg = out.to_string_lossy().into_owned();

    // A temporary copy of uBotCore's real index (or a stand-in without the workspace).
    let index_copy = env.root.join("index-copy.json");
    let original = match ubotcore_index() {
        Some(real) => fs::read_to_string(real).unwrap(),
        None => fs::read_to_string(&env.index).unwrap(),
    };
    fs::write(&index_copy, &original).unwrap();
    let copy_arg = index_copy.to_string_lossy().into_owned();

    let output = env.ubot(&[
        "package",
        "UBotSensor",
        "--out",
        &out_arg,
        "--engine",
        &engine,
        "--update-index",
        &copy_arg,
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.contains("Clone UBotCore 0.1.0"), "{stdout}");
    assert!(stdout.contains("Build UnrealEditor"), "{stdout}");
    assert!(
        stdout.contains("Build.bat UnrealEditor Win64 Development -Project="),
        "{stdout}"
    );

    // Only the requested package is packed; the host project is gone.
    let archive = out.join("UBotSensor-0.1.0-UE5.5-Win64.zip");
    assert!(archive.is_file());
    assert!(!out.join("UBotCore-0.1.0-UE5.5-Win64.zip").exists());
    assert!(!out.join("_work").exists(), "the host project is removed");
    let names = entries(&archive);
    let expected: BTreeSet<String> = [
        "UBotSensor/.gitignore",
        "UBotSensor/Binaries/Win64/UnrealEditor-uBotSensor.dll",
        "UBotSensor/Binaries/Win64/UnrealEditor.modules",
        "UBotSensor/Config/FilterPlugin.ini",
        "UBotSensor/Resources/Icon128.png",
        "UBotSensor/Source/UBotSensor/UBotSensor.Build.cs",
        "UBotSensor/UBotSensor.uplugin",
    ]
    .iter()
    .map(|name| name.to_string())
    .collect();
    assert_eq!(names, expected, "no .git, Intermediate or symbols");
    assert!(read_entry(&archive, "UBotSensor/UBotSensor.uplugin")
        .contains("\"VersionName\": \"0.1.0\""));

    // The .sha256 file and the printed entry.
    let sha = ubot_core::prebuilt::sha256_file(&archive).unwrap();
    let size = fs::metadata(&archive).unwrap().len();
    assert_eq!(
        fs::read_to_string(out.join("UBotSensor-0.1.0-UE5.5-Win64.zip.sha256")).unwrap(),
        format!("{sha}  UBotSensor-0.1.0-UE5.5-Win64.zip\n")
    );
    let url = "https://github.com/AstreoX/uBotSensor/releases/download/v0.1.0/UBotSensor-0.1.0-UE5.5-Win64.zip";
    assert!(stdout.contains(url), "{stdout}");
    assert!(stdout.contains(&sha), "{stdout}");

    // The index copy has the entry, kept its formatting and validates.
    let updated = fs::read_to_string(&index_copy).unwrap();
    let value: Value = serde_json::from_str(updated.trim_start_matches('\u{feff}')).unwrap();
    let sensor = value["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "UBotSensor")
        .unwrap();
    let version = sensor["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["version"] == "0.1.0")
        .unwrap();
    assert_eq!(
        version["binaries"],
        serde_json::json!({ "5.5": { "Win64": { "url": url, "sha256": sha, "size": size } } })
    );
    for line in original.lines() {
        assert!(updated.contains(line), "the line {line:?} was kept");
    }
    // Only the packed version changed: the real index already lists binaries of other releases.
    let mut without: Value = value.clone();
    for package in without["packages"].as_array_mut().unwrap() {
        if package["name"] != "UBotSensor" {
            continue;
        }
        for version in package["versions"].as_array_mut().unwrap() {
            if version["version"] == "0.1.0" {
                version.as_object_mut().unwrap().remove("binaries");
            }
        }
    }
    let original_value: Value =
        serde_json::from_str(original.trim_start_matches('\u{feff}')).unwrap();
    assert_eq!(without, original_value, "nothing else changed");
    let output = Command::new(env!("CARGO_BIN_EXE_ubot"))
        .args(["index", "validate"])
        .arg(&index_copy)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{report}");
    assert!(report.contains(": 0 error(s)"), "{report}");

    // The archive installs: the core extracts it as the package it names.
    let into = env.root.join("check");
    let top = ubot_core::prebuilt::extract_package(
        &archive,
        "UBotSensor",
        &into,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(ubot_core::prebuilt::has_editor_binaries(&top));

    // Symbols, both packages, another URL template, JSON output, and the host project kept.
    let output = env.ubot(&[
        "--json",
        "package",
        "UBotSensor@0.1.0",
        "UBotCore",
        "--out",
        &out_arg,
        "--engine",
        &engine,
        "--with-symbols",
        "--keep-work",
        "--url-template",
        "https://mirror.example/{name}/{version}/UE{engine}/{file}",
    ]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON on stdout");
    let packages = report["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[1]["package"], "UBotCore");
    assert_eq!(
        packages[1]["binaries"]["5.5"]["Win64"]["url"],
        "https://mirror.example/UBotCore/0.1.0/UE5.5/UBotCore-0.1.0-UE5.5-Win64.zip"
    );
    assert!(entries(&archive).contains("UBotSensor/Binaries/Win64/UnrealEditor-uBotSensor.pdb"));
    assert!(out.join("UBotCore-0.1.0-UE5.5-Win64.zip.sha256").is_file());
    assert!(out
        .join("_work/UBotPackaging/Plugins/uBot/uBotCore/Intermediate")
        .is_dir());
}

#[test]
fn refuses_what_cannot_be_packed() {
    let Some((_temp, env)) = setup() else {
        return;
    };
    let out = env.root.join("out");
    let out_arg = out.to_string_lossy().into_owned();
    let engine = env.engine.to_string_lossy().into_owned();
    let source = env.root.join("Source Build");
    write(
        &source.join("Engine/Build/Build.version"),
        r#"{ "MajorVersion": 5, "MinorVersion": 5, "PatchVersion": 4 }"#,
    );
    let source_arg = source.to_string_lossy().into_owned();
    for (args, fragment) in [
        (
            vec![
                "package",
                "UBotSensor",
                "--out",
                &out_arg,
                "--engine",
                &source_arg,
            ],
            "is a source build",
        ),
        (
            vec![
                "package", "UBotNope", "--out", &out_arg, "--engine", &engine,
            ],
            "UBotNope is not in any package index",
        ),
        (
            vec![
                "package",
                "UBotLater",
                "--out",
                &out_arg,
                "--engine",
                &engine,
            ],
            "UBotLater is planned",
        ),
        (
            vec![
                "package",
                "UBotSensor@9.9.9",
                "--out",
                &out_arg,
                "--engine",
                &engine,
            ],
            "has no version 9.9.9",
        ),
    ] {
        let output = env.ubot(&args);
        assert!(!output.status.success(), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(fragment), "{args:?}: {error}");
        assert!(
            !out.join("_work").exists(),
            "{args:?}: the host project is removed"
        );
    }

    // A template that makes a URL an index cannot use, or names a placeholder that does not
    // exist, fails before anything is built or written: no archive, no host project, no folder.
    let early = env.root.join("out-template");
    let early_arg = early.to_string_lossy().into_owned();
    for (template, fragment) in [
        ("http://insecure.example/{file}", "plain http://"),
        (
            "https://example.com/{reponame}/{file}",
            "known placeholders",
        ),
        ("https://example.com/{file", "unclosed"),
    ] {
        let output = env.ubot(&[
            "package",
            "UBotCore",
            "UBotSensor",
            "--out",
            &early_arg,
            "--engine",
            &engine,
            "--url-template",
            template,
        ]);
        assert!(!output.status.success(), "{template}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(fragment), "{template}: {error}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("Build.bat") && !stdout.contains("Clone "),
            "{template}: nothing ran: {stdout}"
        );
        assert!(
            !early.exists(),
            "{template}: the output folder was not made"
        );
    }
}

/// `_work` is a common folder name. `ubot package` replaces only a folder it made itself.
#[test]
fn never_deletes_a_work_folder_that_is_not_its_own() {
    let Some((_temp, env)) = setup() else {
        return;
    };
    let engine = env.engine.to_string_lossy().into_owned();
    // Awkward characters in the path, as a CI runner's folders have.
    let out = env.root.join("outer").join("o&u t (1)^!x;,=a");
    let out_arg = out.to_string_lossy().into_owned();
    let keep = out.join("_work").join("Documents").join("thesis.txt");
    write(&keep, "mine");

    let output = env.ubot(&[
        "package", "UBotCore", "--out", &out_arg, "--engine", &engine,
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("was not created by ubot package"), "{error}");
    assert_eq!(fs::read_to_string(&keep).unwrap(), "mine");
    assert!(
        !out.join("UBotCore-0.1.0-UE5.5-Win64.zip").exists(),
        "nothing was built"
    );

    // A file named _work is not a folder of ours either.
    let other = env.root.join("other");
    write(&other.join("_work"), "a file");
    let other_arg = other.to_string_lossy().into_owned();
    let output = env.ubot(&[
        "package", "UBotCore", "--out", &other_arg, "--engine", &engine,
    ]);
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(other.join("_work")).unwrap(), "a file");

    // An empty _work, and one an earlier run kept (it carries the marker), are replaced.
    let reuse = env.root.join("reuse");
    fs::create_dir_all(reuse.join("_work")).unwrap();
    let reuse_arg = reuse.to_string_lossy().into_owned();
    let output = env.ubot(&[
        "package",
        "UBotCore",
        "--out",
        &reuse_arg,
        "--engine",
        &engine,
        "--keep-work",
    ]);
    assert!(output.status.success());
    assert!(reuse.join("_work").join(".ubot-package-work").is_file());
    write(&reuse.join("_work").join("stray.txt"), "left over");
    let output = env.ubot(&[
        "package",
        "UBotCore",
        "--out",
        &reuse_arg,
        "--engine",
        &engine,
        "--keep-work",
    ]);
    assert!(output.status.success());
    assert!(
        !reuse.join("_work").join("stray.txt").exists(),
        "the marked folder was replaced"
    );
    assert!(reuse.join("_work").join(".ubot-package-work").is_file());
}

/// SemVer ignores everything after `+`, so a version can carry path separators and still parse.
/// It must not reach a file name or a URL.
#[test]
fn a_version_cannot_leave_the_output_folder() {
    let Some((_temp, env)) = setup() else {
        return;
    };
    let engine = env.engine.to_string_lossy().into_owned();
    let mut index: Value = serde_json::from_str(&fs::read_to_string(&env.index).unwrap()).unwrap();
    for package in index["packages"].as_array_mut().unwrap() {
        if package["name"] == "UBotSensor" {
            package["versions"][0]["version"] = Value::from("0.1.0+x/../../pwn");
        }
    }
    let bad_index = env.root.join("index-bad-version.json");
    fs::write(&bad_index, serde_json::to_string_pretty(&index).unwrap()).unwrap();
    let bad = Env {
        root: env.root.clone(),
        index: bad_index,
        engine: env.engine.clone(),
        home: env.home.clone(),
    };

    let outer = env.root.join("outer");
    let out = outer.join("out");
    let out_arg = out.to_string_lossy().into_owned();
    let output = bad.ubot(&[
        "package",
        "UBotSensor",
        "--out",
        &out_arg,
        "--engine",
        &engine,
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("cannot be used in an archive name or URL"),
        "{error}"
    );
    for dir in [&outer, &out] {
        let names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names
                .iter()
                .all(|name| !name.ends_with(".zip") && !name.ends_with(".sha256")),
            "{dir:?}: {names:?}"
        );
    }
    assert!(!env.root.join("pwn-UE5.5-Win64.zip").exists());

    // The same index fails validation, so a published index cannot carry such a version.
    let output = Command::new(env!("CARGO_BIN_EXE_ubot"))
        .args(["index", "validate"])
        .arg(&bad.index)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("may only contain letters, digits"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

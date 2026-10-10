//! `ubot package`: builds prebuilt binaries of packages (SPEC §10). The packages and their
//! required dependencies are installed from source, at their refs, into a content-only host
//! project under `<out>/_work` through the normal plan and task path, built with a launcher
//! engine, and every requested package is packed into `<Name>-<version>-UE<Major.Minor>-Win64.zip`
//! with a `.sha256` file. The `binaries` entries for the index are printed and, with
//! `--update-index`, written into an index file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use ubot_core::engine::{self, EngineRegistry, SystemEngineRegistry};
use ubot_core::exec::{remove_dir_robust, PermanentDelete};
use ubot_core::index::{self, PrebuiltBinary};
use ubot_core::model::{
    ApplyOptions, BuildConfiguration, Change, ChangeKind, EngineKind, StepStatus,
};
use ubot_core::msg::{msg, render_en};
use ubot_core::paths::{self, AppDirs};
use ubot_core::prebuilt::{self, BinaryUpdate, PLATFORM};
use ubot_core::settings::{SettingsStore, DEFAULT_INSTALL_DIRECTORY};
use ubot_core::url::{check_binary_url, redact_credentials, repository_folder_name};
use ubot_core::version::SemVer;
use ubot_core::{Manager, ManagerOptions};

use crate::{errln, out, outln, print_json, render, run_task, unloaded_index_sources};

/// The name of the host project in `<out>/_work`.
const HOST: &str = "UBotPackaging";

/// The file that marks `<out>/_work` as this command's own folder. `_work` is a common folder name
/// (CI runners, build systems), so a folder without the marker is never touched.
const WORK_MARKER: &str = ".ubot-package-work";

/// What `ubot package` was asked to do.
pub struct PackageArgs {
    pub packages: Vec<String>,
    pub out: PathBuf,
    pub engine: Option<String>,
    pub url_template: String,
    pub update_index: Option<PathBuf>,
    pub with_symbols: bool,
    pub keep_work: bool,
}

/// Global options that apply.
pub struct Globals {
    pub index: Vec<String>,
    pub index_override: Option<Vec<String>>,
    pub offline: bool,
    pub json: bool,
}

/// The one launcher engine the host project is built with.
struct OneLauncher {
    id: String,
    root: PathBuf,
}

impl EngineRegistry for OneLauncher {
    fn launcher_installs(&self) -> Vec<(String, PathBuf)> {
        vec![(self.id.clone(), self.root.clone())]
    }
    fn registered_builds(&self) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
}

/// A launcher engine: its association id ("5.5"), root and version.
#[derive(Debug, PartialEq, Eq)]
pub struct LauncherEngine {
    pub id: String,
    pub root: PathBuf,
    pub version: String,
}

/// `--engine`: a launcher id ("5.5"), or an engine root that is a launcher install (or another
/// installed build: `Engine/Build/InstalledBuild.txt`). Without it, the newest launcher engine.
/// Binaries built with a source build do not load in launcher engines, so those are refused.
pub fn resolve_engine(
    requested: Option<&str>,
    registry: &dyn EngineRegistry,
) -> Result<LauncherEngine> {
    let installs = registry.launcher_installs();
    match requested.map(str::trim).filter(|text| !text.is_empty()) {
        Some(id) if engine::is_launcher_association(id) => {
            let root = installs
                .iter()
                .find(|(association, _)| association == id)
                .map(|(_, root)| root.clone())
                .ok_or_else(|| {
                    anyhow!("Unreal Engine {id} is not installed by the Epic launcher")
                })?;
            let version = engine::engine_version(&root)
                .ok_or_else(|| anyhow!("{} has no Engine/Build/Build.version", root.display()))?;
            Ok(LauncherEngine {
                id: id.to_string(),
                root,
                version,
            })
        }
        Some(path) => {
            let root = paths::absolute(Path::new(path));
            let version = engine::engine_version(&root).ok_or_else(|| {
                anyhow!(
                    "{} is not an engine root (no Engine/Build/Build.version)",
                    root.display()
                )
            })?;
            let registered = installs
                .iter()
                .find(|(_, install)| paths::same_path(install, &root))
                .map(|(association, _)| association.clone());
            let installed_build = root
                .join("Engine")
                .join("Build")
                .join("InstalledBuild.txt")
                .is_file();
            let id = match (registered, installed_build) {
                (Some(id), _) => id,
                (None, true) => prebuilt::engine_major_minor(&version)
                    .ok_or_else(|| anyhow!("{} has an unreadable version", root.display()))?,
                (None, false) => bail!(
                    "{} is a source build; prebuilt binaries must be built with a launcher (installed) engine",
                    root.display()
                ),
            };
            Ok(LauncherEngine { id, root, version })
        }
        None => engine::list_engines(registry)
            .into_iter()
            .filter(|engine| engine.kind == EngineKind::Launcher)
            .filter_map(|engine| SemVer::parse(&engine.version).map(|version| (version, engine)))
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, engine)| LauncherEngine {
                id: engine.id,
                root: PathBuf::from(engine.root),
                version: engine.version,
            })
            .ok_or_else(|| {
                anyhow!("no launcher engine was found; pass --engine <5.5 | engine root>")
            }),
    }
}

/// `Name[@version]`.
fn parse_spec(text: &str) -> Result<(String, Option<String>)> {
    let (name, version) = match text.split_once('@') {
        Some((name, version)) => (name.trim(), Some(version.trim().to_string())),
        None => (text.trim(), None),
    };
    if !index::is_plugin_name(name) {
        bail!("'{text}': expected Name[@version], e.g. UBotSensor@0.1.0");
    }
    if version.as_deref().is_some_and(str::is_empty) {
        bail!("'{text}': the version after '@' is missing");
    }
    Ok((name.to_string(), version))
}

/// One package to pack, as the index describes it.
struct Requested {
    name: String,
    version: String,
    git_ref: String,
    repository: String,
    folder: String,
}

/// The `binaries` value of one archive.
fn binaries_value(engine: &str, binary: &PrebuiltBinary) -> Value {
    let mut platforms = serde_json::Map::new();
    platforms.insert(
        PLATFORM.to_string(),
        serde_json::to_value(binary).expect("a binary serializes"),
    );
    let mut engines = serde_json::Map::new();
    engines.insert(engine.to_string(), Value::Object(platforms));
    Value::Object(engines)
}

pub fn run(args: PackageArgs, globals: Globals) -> Result<ExitCode> {
    let specs = args
        .packages
        .iter()
        .map(|text| parse_spec(text))
        .collect::<Result<Vec<_>>>()?;
    for (position, (name, _)) in specs.iter().enumerate() {
        if specs[..position]
            .iter()
            .any(|(other, _)| other.eq_ignore_ascii_case(name))
        {
            bail!("{name} is listed twice");
        }
    }
    // A template that cannot make a usable URL is found before anything is built.
    binary_url(
        &args.url_template,
        &[
            ("repo", "Repository"),
            ("ref", "v0.0.0"),
            ("file", "Package-0.0.0-UE5.5-Win64.zip"),
            ("name", "Package"),
            ("version", "0.0.0"),
            ("engine", "5.5"),
        ],
    )?;
    let engine = resolve_engine(args.engine.as_deref(), &SystemEngineRegistry)?;
    let engine_key = prebuilt::engine_major_minor(&engine.version)
        .ok_or_else(|| anyhow!("the engine version {} cannot be read", engine.version))?;
    if !globals.json {
        outln(&format!(
            "Engine: UE {} ({}) {}",
            engine.version,
            engine.id,
            paths::display(&engine.root)
        ));
    }

    let out_dir = paths::absolute(&args.out);
    fs::create_dir_all(&out_dir).with_context(|| format!("cannot create {}", out_dir.display()))?;
    let work = out_dir.join("_work");
    prepare_work_dir(&work)?;
    let result = package_in(&args, &globals, &engine, &engine_key, &out_dir, &work);
    if args.keep_work {
        if !globals.json {
            outln(&format!("Kept {}", paths::display(&work)));
        }
    } else if work.exists() {
        if let Err(error) = remove_dir_robust(&work) {
            errln(&format!(
                "warning: could not remove {}: {error}",
                work.display()
            ));
        }
    }
    result
}

/// Makes `<out>/_work` an empty folder that carries the marker. A folder that is already there
/// is replaced only when an earlier run of this command left it (it has the marker) or when it is
/// empty; anything else, a file or a link included, is somebody else's and stays untouched.
fn prepare_work_dir(work: &Path) -> Result<()> {
    match fs::symlink_metadata(work) {
        Ok(metadata) => {
            let owned = work.join(WORK_MARKER).is_file();
            let empty = fs::read_dir(work)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false);
            if !metadata.is_dir() || !(owned || empty) {
                bail!(
                    "{} already exists and was not created by ubot package; remove it or choose another --out",
                    work.display()
                );
            }
            remove_dir_robust(work).with_context(|| format!("cannot remove {}", work.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => bail!("cannot inspect {}: {error}", work.display()),
    }
    fs::create_dir_all(work).with_context(|| format!("cannot create {}", work.display()))?;
    fs::write(
        work.join(WORK_MARKER),
        "ubot package work folder; safe to delete\n",
    )
    .with_context(|| format!("cannot write the marker in {}", work.display()))?;
    Ok(())
}

/// Expands the URL template and checks the result against the URL policy of `binaries`.
fn binary_url(template: &str, values: &[(&str, &str)]) -> Result<String> {
    let url = prebuilt::expand_url_template(template, values).map_err(|error| anyhow!(error))?;
    if let Err(reason) = check_binary_url(&url) {
        bail!(
            "the URL template makes '{}', which an index cannot use: {reason}",
            redact_credentials(&url)
        );
    }
    Ok(url)
}

/// The download URL of one archive.
fn package_url(
    template: &str,
    package: &Requested,
    file_name: &str,
    engine_key: &str,
) -> Result<String> {
    let repo = repository_folder_name(&package.repository).unwrap_or_default();
    binary_url(
        template,
        &[
            ("repo", &repo),
            ("ref", &package.git_ref),
            ("file", file_name),
            ("name", &package.name),
            ("version", &package.version),
            ("engine", engine_key),
        ],
    )
}

fn package_in(
    args: &PackageArgs,
    globals: &Globals,
    engine: &LauncherEngine,
    engine_key: &str,
    out_dir: &Path,
    work: &Path,
) -> Result<ExitCode> {
    // A content-only host project for the engine, with settings of its own: built from
    // source, and nothing of the user's history or projects touched. The index sources and
    // their caches are the user's.
    let host_dir = work.join(HOST);
    let uproject = host_dir.join(format!("{HOST}.uproject"));
    fs::create_dir_all(&host_dir)
        .with_context(|| format!("cannot create {}", host_dir.display()))?;
    fs::write(
        &uproject,
        format!(
            "{{\n\t\"FileVersion\": 3,\n\t\"EngineAssociation\": {},\n\t\"Category\": \"\",\n\t\"Description\": \"uBot Manager packaging host\",\n\t\"Plugins\": []\n}}\n",
            serde_json::to_string(&engine.id)?
        ),
    )
    .with_context(|| format!("cannot write {}", uproject.display()))?;

    let user_dirs = AppDirs::from_env();
    let home = work.join("home");
    let dirs = AppDirs {
        config: home.clone(),
        data: home.clone(),
        cache: user_dirs.cache.clone(),
    };
    let mut settings = SettingsStore::load(&user_dirs).settings;
    settings.install.use_prebuilt = false;
    settings.install.directory = DEFAULT_INSTALL_DIRECTORY.into();
    settings.build.engine_id = None;
    settings.build.configuration = BuildConfiguration::Development;
    settings.build.wait_for_editor = false;
    settings.ros.check_on_refresh = false;
    settings.recent_projects.clear();
    settings.last_project = None;
    let sync = settings.index.sync_on_start && !globals.offline;
    let mut store = SettingsStore::load(&dirs);
    store.set(settings);
    store
        .save()
        .with_context(|| format!("cannot write the settings in {}", home.display()))?;

    let manager = Manager::new(ManagerOptions {
        dirs,
        offline: globals.offline,
        index_override: globals.index_override.clone(),
        write_merged_index: false,
        remover: Arc::new(PermanentDelete),
        engine_registry: Arc::new(OneLauncher {
            id: engine.id.clone(),
            root: engine.root.clone(),
        }),
        ..ManagerOptions::default()
    });
    let snapshot = manager
        .open_project_with(&uproject.to_string_lossy(), sync)
        .map_err(|error| anyhow!(error))?;
    if !globals.index.is_empty() {
        let failed = unloaded_index_sources(&snapshot);
        if !failed.is_empty() {
            bail!("{}", failed.join("\n"));
        }
    }

    // The versions to pack, from the index.
    let merged = manager.merged_index();
    let mut requested = Vec::new();
    for (name, version) in specs_of(args)? {
        let package = merged
            .package(&name)
            .ok_or_else(|| anyhow!("{name} is not in any package index"))?;
        if package.is_planned() {
            bail!("{} is planned and cannot be packed yet", package.name);
        }
        let entry = match &version {
            Some(version) => package
                .find_version(version)
                .ok_or_else(|| anyhow!("{} has no version {version}", package.name))?,
            None => package.latest().expect("not planned"),
        };
        // The version becomes part of a file name and a URL; the index may not be validated.
        if !index::is_safe_version_text(&entry.version) {
            bail!(
                "{} version '{}' cannot be used in an archive name or URL; fix the index (letters, digits, '.', '-' and '+' only)",
                package.name,
                entry.version
            );
        }
        let repository = package.repository.clone().unwrap_or_default();
        requested.push(Requested {
            name: package.name.clone(),
            version: entry.version.clone(),
            git_ref: entry.git_ref.clone().unwrap_or_default(),
            folder: package
                .folder()
                .ok_or_else(|| anyhow!("{} has no install folder", package.name))?,
            repository,
        });
    }
    // Every URL is known before the long part starts: a template that fails for one package must
    // not cost a build.
    for package in &requested {
        let file_name = prebuilt::archive_name(&package.name, &package.version, engine_key);
        package_url(&args.url_template, package, &file_name, engine_key)?;
    }

    // Install from source and build, through the normal plan and task.
    let changes: Vec<Change> = requested
        .iter()
        .map(|package| {
            Change::new(
                ChangeKind::Install,
                package.name.clone(),
                Some(package.version.clone()),
            )
        })
        .collect();
    let options = ApplyOptions {
        build: true,
        reopen_editor: false,
        run_tests: false,
    };
    let plan = manager
        .plan(&changes, &options)
        .map_err(|error| anyhow!(error))?;
    if !globals.json {
        out(&render::plan(&plan));
    }
    if !plan.errors.is_empty() {
        bail!(
            "the packages cannot be installed in the host project:\n{}",
            plan.errors
                .iter()
                .map(render_en)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    let title = msg("task.apply", &[("project", HOST)]);
    let task = run_task(&manager, &changes, &options, Some(title), globals.json)?;
    match task.status {
        StepStatus::Done => {}
        StepStatus::Canceled => {
            errln("Canceled.");
            return Ok(ExitCode::from(130));
        }
        _ => bail!(
            "installing and building in the host project failed: {}",
            task.error.unwrap_or_default()
        ),
    }

    // Pack every requested package.
    let plugins = host_dir.join(DEFAULT_INSTALL_DIRECTORY);
    let mut results = Vec::new();
    let mut updates = Vec::new();
    for package in &requested {
        let dir = plugins.join(&package.folder);
        let descriptor_path = prebuilt::find_descriptor(&dir, &package.name)
            .ok_or_else(|| anyhow!("{} has no {}.uplugin", dir.display(), package.name))?;
        let (text, _) = paths::read_text(&descriptor_path)
            .with_context(|| format!("cannot read {}", descriptor_path.display()))?;
        let descriptor = ubot_core::descriptor::parse(&text)
            .map_err(|error| anyhow!("{}: {error}", descriptor_path.display()))?;
        let found = descriptor.version_name.clone().unwrap_or_default();
        if SemVer::parse(&found) != SemVer::parse(&package.version) {
            bail!(
                "{} at {} has VersionName '{found}', but the index lists {}; prebuilt installs check it",
                package.name,
                package.git_ref,
                package.version
            );
        }
        if descriptor.has_modules {
            if !prebuilt::has_editor_binaries(&dir) {
                bail!(
                    "the build left no Binaries/Win64/UnrealEditor-*.dll in {}",
                    dir.display()
                );
            }
            if !dir
                .join("Binaries")
                .join(PLATFORM)
                .join("UnrealEditor.modules")
                .is_file()
            {
                bail!(
                    "the build left no Binaries/Win64/UnrealEditor.modules in {}",
                    dir.display()
                );
            }
        }

        let file_name = prebuilt::archive_name(&package.name, &package.version, engine_key);
        let archive = out_dir.join(&file_name);
        let packed = prebuilt::pack_package(&dir, &package.name, &archive, args.with_symbols)
            .map_err(|error| anyhow!(error))?;
        let sha_file = out_dir.join(format!("{file_name}.sha256"));
        fs::write(&sha_file, format!("{}  {file_name}\n", packed.sha256))
            .with_context(|| format!("cannot write {}", sha_file.display()))?;

        let url = package_url(&args.url_template, package, &file_name, engine_key)?;
        let binary = PrebuiltBinary {
            url,
            sha256: packed.sha256.clone(),
            size: Some(packed.size),
        };
        let binaries = binaries_value(engine_key, &binary);
        if !globals.json {
            outln(&format!(
                "\nPacked {} {}: {} ({:.1} MB, {} files)",
                package.name,
                package.version,
                paths::display(&archive),
                packed.size as f64 / 1_000_000.0,
                packed.entries.len()
            ));
            outln(&format!("  SHA-256 {}", packed.sha256));
            outln(&format!(
                "  \"binaries\": {}",
                serde_json::to_string_pretty(&binaries)?.replace('\n', "\n  ")
            ));
        }
        results.push(json!({
            "package": package.name,
            "version": package.version,
            "engine": engine_key,
            "platform": PLATFORM,
            "file": paths::display(&archive),
            "sha256File": paths::display(&sha_file),
            "files": packed.entries.len(),
            "binaries": binaries,
        }));
        updates.push(BinaryUpdate {
            package: package.name.clone(),
            version: package.version.clone(),
            engine: engine_key.to_string(),
            platform: PLATFORM.to_string(),
            binary,
        });
    }

    let mut index_report = Value::Null;
    if let Some(index_file) = &args.update_index {
        let (text, warnings) = update_index_file(index_file, &updates)?;
        if !globals.json {
            outln(&format!(
                "\nUpdated {}: {} entr{}, 0 errors, {} warning(s).",
                paths::display(index_file),
                updates.len(),
                if updates.len() == 1 { "y" } else { "ies" },
                warnings.len()
            ));
            for warning in &warnings {
                outln(&format!("warning: {warning}"));
            }
        }
        index_report = json!({ "file": paths::display(index_file), "bytes": text.len(), "warnings": warnings });
    }
    if globals.json {
        print_json(&json!({ "packages": results, "index": index_report }))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn specs_of(args: &PackageArgs) -> Result<Vec<(String, Option<String>)>> {
    args.packages.iter().map(|text| parse_spec(text)).collect()
}

/// Writes the entries into an index file (key order and formatting kept, new text indented
/// with 4 spaces) when the result validates; returns the new text and its warnings.
fn update_index_file(path: &Path, updates: &[BinaryUpdate]) -> Result<(String, Vec<String>)> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow!("{} is not UTF-8 text", path.display()))?;
    let updated = prebuilt::update_index_binaries(&text, updates)
        .map_err(|error| anyhow!("{}: {error}", path.display()))?;
    let validation = index::validate_text(&updated);
    if !validation.is_ok() {
        bail!(
            "{} would not validate; it was not changed:\n{}",
            path.display(),
            validation.errors.join("\n")
        );
    }
    paths::write_atomic(path, updated.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok((updated, validation.warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Registry(Vec<(String, PathBuf)>);

    impl EngineRegistry for Registry {
        fn launcher_installs(&self) -> Vec<(String, PathBuf)> {
            self.0.clone()
        }
        fn registered_builds(&self) -> Vec<(String, PathBuf)> {
            Vec::new()
        }
    }

    fn engine_root(dir: &Path, minor: u32, installed: bool) -> PathBuf {
        let build = dir.join("Engine").join("Build");
        fs::create_dir_all(&build).unwrap();
        fs::write(
            build.join("Build.version"),
            format!(r#"{{ "MajorVersion": 5, "MinorVersion": {minor}, "PatchVersion": 2 }}"#),
        )
        .unwrap();
        if installed {
            fs::write(build.join("InstalledBuild.txt"), "").unwrap();
        }
        dir.to_path_buf()
    }

    #[test]
    fn engines_must_be_launcher_installs() {
        let temp = tempfile::tempdir().unwrap();
        let launcher = engine_root(&temp.path().join("UE_5.5"), 5, true);
        let newer = engine_root(&temp.path().join("UE_5.6"), 6, true);
        let installed = engine_root(&temp.path().join("Installed"), 4, true);
        let source = engine_root(&temp.path().join("Source"), 5, false);
        let registry = Registry(vec![
            ("5.5".into(), launcher.clone()),
            ("5.6".into(), newer.clone()),
        ]);

        let found = resolve_engine(Some("5.5"), &registry).unwrap();
        assert_eq!(
            (found.id.as_str(), found.version.as_str()),
            ("5.5", "5.5.2")
        );
        assert_eq!(
            resolve_engine(None, &registry).unwrap().id,
            "5.6",
            "the newest"
        );
        let by_root = resolve_engine(Some(&launcher.to_string_lossy()), &registry).unwrap();
        assert_eq!(by_root.id, "5.5");
        let other = resolve_engine(Some(&installed.to_string_lossy()), &registry).unwrap();
        assert_eq!(
            (other.id.as_str(), other.version.as_str()),
            ("5.4", "5.4.2")
        );

        let error = resolve_engine(Some(&source.to_string_lossy()), &registry)
            .unwrap_err()
            .to_string();
        assert!(error.contains("is a source build"), "{error}");
        let error = resolve_engine(Some("5.3"), &registry)
            .unwrap_err()
            .to_string();
        assert!(error.contains("5.3 is not installed"), "{error}");
        let error = resolve_engine(Some(&temp.path().join("nope").to_string_lossy()), &registry)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not an engine root"), "{error}");
        let error = resolve_engine(None, &Registry(Vec::new()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no launcher engine"), "{error}");
    }

    #[test]
    fn package_specs() {
        assert_eq!(
            parse_spec("UBotSensor").unwrap(),
            ("UBotSensor".into(), None)
        );
        assert_eq!(
            parse_spec("UBotSensor@0.1.0").unwrap(),
            ("UBotSensor".into(), Some("0.1.0".into()))
        );
        assert!(parse_spec("UBotSensor@").is_err());
        assert!(parse_spec("Bad Name").is_err());
        assert!(parse_spec("").is_err());
        assert_eq!(
            binaries_value(
                "5.5",
                &PrebuiltBinary {
                    url: "https://h/x.zip".into(),
                    sha256: "a".repeat(64),
                    size: Some(3)
                }
            ),
            json!({ "5.5": { "Win64": { "url": "https://h/x.zip", "sha256": "a".repeat(64), "size": 3 } } })
        );
    }
}

//! Planning (SPEC §5.2): turns requested changes into ordered steps over the current installed set
//! and the merged index, resolving requirements, dependents and engine plugins.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::descriptor::Dependency;
use crate::graph;
use crate::index::{is_plugin_name, is_safe_ref, IndexPackage, IndexVersion, PrebuiltBinary};
use crate::model::{
    ApplyOptions, BuildConfiguration, Change, ChangeKind, EngineKind, InstallKind, Msg, Plan,
    PlanStep, StepGroup, StepKind,
};
use crate::msg::{msg, msg0};
use crate::paths;
use crate::prebuilt;
use crate::url::{
    check_repository_url, display_download_source, display_repository, redact_credentials,
};
use crate::version::{Constraint, SemVer};
use crate::workspace::{index_requirements, Workspace};

pub const PLATFORM: &str = "Win64";

/// What a step does when it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepAction {
    Clone {
        package: String,
        url: String,
        git_ref: String,
        dest: PathBuf,
    },
    Checkout {
        package: String,
        dir: PathBuf,
        git_ref: String,
    },
    /// Installs prebuilt binaries: fetch, verify, extract, swap into `dest`, write the marker.
    Download {
        package: String,
        version: String,
        url: String,
        sha256: String,
        size: Option<u64>,
        /// `Major.Minor` of the engine the binaries are for.
        engine: String,
        dest: PathBuf,
        /// `dest` exists and is replaced (an update or reinstall).
        replace: bool,
    },
    SetEnabled {
        plugin: String,
        enabled: bool,
    },
    Remove {
        package: String,
        dir: PathBuf,
    },
    Build {
        engine_root: PathBuf,
        target: String,
        configuration: BuildConfiguration,
        uproject: PathBuf,
    },
    RunTests {
        engine_root: PathBuf,
        uproject: PathBuf,
        packages: Vec<String>,
    },
    OpenEditor {
        engine_root: PathBuf,
        uproject: PathBuf,
    },
}

/// A plan with what its steps need to run.
#[derive(Clone, Debug, Default)]
pub struct PreparedPlan {
    pub plan: Plan,
    /// One per step, in step order.
    pub actions: Vec<StepAction>,
    pub project_path: PathBuf,
    pub project_name: String,
}

#[derive(Clone, Debug)]
struct VPackage {
    name: String,
    installed: bool,
    is_ubot: bool,
    version: Option<String>,
    enabled: bool,
    requires: Vec<Dependency>,
    engine_plugins: Vec<String>,
    dir: Option<PathBuf>,
}

impl VPackage {
    fn semver(&self) -> Option<SemVer> {
        self.version.as_deref().and_then(SemVer::parse)
    }
}

struct CloneRecord {
    package: String,
    version: String,
    url: String,
    git_ref: String,
    dest: PathBuf,
}

struct CheckoutRecord {
    package: String,
    from: String,
    to: String,
    git_ref: String,
    dir: PathBuf,
}

struct DownloadRecord {
    package: String,
    version: String,
    binary: PrebuiltBinary,
    engine: String,
    dest: PathBuf,
    /// The folder exists: an update or reinstall replaces it.
    replace: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestedVersionOrigin {
    Install,
    AppliedUpdate,
}

struct RequestedVersion {
    name: String,
    version: SemVer,
    origin: RequestedVersionOrigin,
}

struct Planner<'a> {
    workspace: &'a Workspace,
    packages: HashMap<String, VPackage>,
    clones: Vec<CloneRecord>,
    checkouts: Vec<CheckoutRecord>,
    downloads: Vec<DownloadRecord>,
    /// (package, required by)
    enables: Vec<(String, Option<String>)>,
    engine_enables: Vec<(String, Option<String>)>,
    disables: Vec<String>,
    removes: Vec<String>,
    /// Packages the user asked to disable or uninstall (and their dependents).
    protected: HashSet<String>,
    resolving: HashSet<String>,
    /// The requirements of the version a package is going to be updated to, by package key, for
    /// packages whose update is known but not applied yet (explicit updates, dependencies about
    /// to be updated). Judging such a package by the requirements of the version it is leaving
    /// would make the result depend on the order of the changes and of the `requires` lists.
    /// An entry is dropped when the version is applied.
    planned_requires: HashMap<String, Vec<Dependency>>,
    /// Explicit install targets, known before resolution, and explicit updates that were applied
    /// to the virtual state. Only install targets pin dependency selection; both are checked
    /// against the final versions in `verify`.
    requested: HashMap<String, RequestedVersion>,
    errors: Vec<Msg>,
    warnings: Vec<Msg>,
}

fn dependencies_of(version: &IndexVersion) -> Vec<Dependency> {
    version
        .requires
        .iter()
        .map(|r| Dependency {
            name: r.name.clone(),
            version: r.version.clone(),
            optional: r.optional,
        })
        .collect()
}

fn key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn range_text(constraint: &str) -> String {
    if constraint.trim().is_empty() {
        "*".into()
    } else {
        constraint.trim().to_string()
    }
}

impl<'a> Planner<'a> {
    fn new(workspace: &'a Workspace) -> Self {
        Planner {
            workspace,
            packages: HashMap::new(),
            clones: Vec::new(),
            checkouts: Vec::new(),
            downloads: Vec::new(),
            enables: Vec::new(),
            engine_enables: Vec::new(),
            disables: Vec::new(),
            removes: Vec::new(),
            protected: HashSet::new(),
            resolving: HashSet::new(),
            planned_requires: HashMap::new(),
            requested: HashMap::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn error(&mut self, message: Msg) {
        if !self.errors.contains(&message) {
            self.errors.push(message);
        }
    }

    fn warning(&mut self, message: Msg) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    /// The virtual state of a package or plugin, created from the workspace on first use.
    fn package(&mut self, name: &str) -> &mut VPackage {
        let workspace = self.workspace;
        self.packages
            .entry(key(name))
            .or_insert_with(|| match workspace.plugin(name) {
                Some(plugin) => {
                    let block = plugin
                        .descriptor
                        .as_ref()
                        .ok()
                        .and_then(|descriptor| descriptor.ubot.as_ref());
                    VPackage {
                        name: workspace.display_name(&plugin.name),
                        installed: true,
                        is_ubot: plugin.is_ubot(),
                        version: plugin.version().map(str::to_string),
                        enabled: workspace.is_enabled(plugin),
                        requires: block
                            .map(|block| block.requires.clone())
                            .unwrap_or_default(),
                        engine_plugins: block
                            .map(|block| block.engine_plugins.clone())
                            .unwrap_or_default(),
                        dir: Some(plugin.dir.clone()),
                    }
                }
                None => {
                    let requirements = workspace
                        .index
                        .package(name)
                        .map(index_requirements)
                        .unwrap_or_default();
                    VPackage {
                        name: workspace.display_name(name),
                        installed: false,
                        is_ubot: workspace.index.package(name).is_some(),
                        version: None,
                        enabled: false,
                        requires: requirements.packages,
                        engine_plugins: requirements.engine_plugins,
                        dir: None,
                    }
                }
            })
    }

    fn index_package(&self, name: &str) -> Option<&'a IndexPackage> {
        self.workspace.index.package(name)
    }

    fn engine_version(&self) -> Option<SemVer> {
        self.workspace
            .project
            .as_ref()?
            .engine
            .as_ref()
            .and_then(|engine| SemVer::parse(&engine.version))
    }

    fn check_engine_constraint(&mut self, package: &str, version: &IndexVersion) {
        let (Some(engine), Some(range)) = (self.engine_version(), version.engine.as_deref()) else {
            return;
        };
        if let Ok(constraint) = Constraint::parse(range) {
            if !constraint.is_satisfied_by(&engine) {
                let engine_text = engine.to_string();
                self.warning(msg(
                    "plan.engineVersion",
                    &[
                        ("package", package),
                        ("version", &version.version),
                        ("range", range),
                        ("engine", &engine_text),
                    ],
                ));
            }
        }
    }

    fn apply_version(&mut self, name: &str, version: &IndexVersion) {
        self.planned_requires.remove(&key(name));
        let package = self.package(name);
        package.version = Some(version.version.clone());
        package.requires = dependencies_of(version);
        package.engine_plugins = version.engine_plugins.clone();
    }

    /// Notes that an installed package is going to be updated to `version`, so that the packages
    /// resolved before the update is applied already see its new requirements.
    fn expect_update(&mut self, name: &str, version: &IndexVersion) {
        let current = self.package(name);
        if !current.installed || current.semver() == version.semver() {
            return;
        }
        self.planned_requires
            .insert(key(name), dependencies_of(version));
    }

    /// Records a valid explicit install target without installing anything. The actual request
    /// path still diagnoses installed plugins, missing versions and unsafe acquisition inputs.
    fn expect_install(&mut self, name: &str, version: &str) {
        if self.workspace.plugin(name).is_some() {
            return;
        }
        let Some(package) = self.index_package(name) else {
            return;
        };
        let Some(version) = package.find_version(version).and_then(IndexVersion::semver) else {
            return;
        };
        self.requested.insert(
            key(name),
            RequestedVersion {
                name: package.name.clone(),
                version,
                origin: RequestedVersionOrigin::Install,
            },
        );
    }

    fn pinned_install_version(&self, name: &str) -> Option<&SemVer> {
        self.requested
            .get(&key(name))
            .filter(|requested| requested.origin == RequestedVersionOrigin::Install)
            .map(|requested| &requested.version)
    }

    /// Installs a package that is not installed: clone, requirements, enable.
    fn install(
        &mut self,
        name: &str,
        requested: Option<&str>,
        constraint: Option<&Constraint>,
        required_by: Option<&str>,
    ) {
        if requested.is_none() && self.package(name).installed {
            return;
        }
        let Some(index_package) = self.index_package(name) else {
            self.error(msg("plan.notInIndex", &[("package", name.trim())]));
            return;
        };
        let display = index_package.name.clone();
        if !is_plugin_name(&display) {
            self.error(msg("plan.unsafeName", &[("package", &display)]));
            return;
        }
        if index_package.is_planned() {
            self.error(msg("plan.planned", &[("package", &display)]));
            return;
        }
        let chosen = match (requested, self.pinned_install_version(name), constraint) {
            (Some(requested), _, _) => index_package.find_version(requested),
            (None, Some(pinned), _) => index_package.find_version(&pinned.to_string()),
            (None, None, Some(constraint)) => index_package
                .released_versions()
                .into_iter()
                .find(|(_, version)| constraint.is_satisfied_by(version))
                .map(|(v, _)| v),
            (None, None, None) => index_package.latest(),
        };
        let Some(version) = chosen else {
            let range = requested
                .map(str::to_string)
                .or_else(|| constraint.map(|c| range_text(c.as_str())))
                .unwrap_or_else(|| "*".into());
            self.error(msg(
                "plan.noVersion",
                &[("package", &display), ("range", &range)],
            ));
            return;
        };
        // An earlier dependency may already have installed this package, but an explicit
        // version must still be validated (including a nonexistent or planned version).
        if self.package(name).installed {
            return;
        }

        let Some(context) = self.workspace.project.as_ref() else {
            return;
        };
        let project = &context.project;
        let repository = index_package.repository.clone().unwrap_or_default();
        if !check_repository_url(&repository).is_allowed() {
            self.error(msg(
                "plan.unsafeUrl",
                &[("url", &redact_credentials(&repository))],
            ));
            return;
        }
        let Some(folder) = index_package
            .folder()
            .filter(|folder| crate::url::is_safe_folder_name(folder))
        else {
            self.error(msg(
                "plan.unsafeUrl",
                &[("url", &redact_credentials(&repository))],
            ));
            return;
        };
        // UE and the manager find a package only inside the Plugins folder or an
        // AdditionalPluginDirectories entry; a clone anywhere else would never show up.
        let install_dir = project.install_dir(&self.workspace.settings.install.directory);
        if !project.is_discoverable(&install_dir) {
            let code = if paths::is_within(&install_dir, &project.dir) {
                "plan.notPluginDir"
            } else {
                "plan.outsideProject"
            };
            self.error(msg(code, &[("path", &project.location(&install_dir))]));
            return;
        }
        let dest = install_dir.join(&folder);
        let dest_display = project.location(&dest);
        if dest.exists()
            || self
                .clones
                .iter()
                .any(|clone| paths::same_path(&clone.dest, &dest))
            || self
                .downloads
                .iter()
                .any(|download| paths::same_path(&download.dest, &dest))
        {
            self.error(msg("plan.folderExists", &[("dest", &dest_display)]));
            return;
        }
        if !self.acquire(&display, version, &repository, &dest) {
            return;
        }
        self.check_engine_constraint(&display, version);
        self.apply_version(&display, version);
        let package = self.package(&display);
        package.installed = true;
        package.is_ubot = true;
        package.dir = Some(dest);
        self.ensure_enabled(&display, required_by);
    }

    /// Plans how a package that is not on disk yet gets to `dest` at `version`: prebuilt binaries
    /// when they apply (`install.usePrebuilt`, a launcher engine and binaries for it in the index),
    /// else a clone of `repository`. Replaces whatever this plan had planned for the package.
    /// Returns false (with an error) when neither is possible.
    fn acquire(
        &mut self,
        display: &str,
        version: &IndexVersion,
        repository: &str,
        dest: &Path,
    ) -> bool {
        self.clones
            .retain(|clone| !clone.package.eq_ignore_ascii_case(display));
        self.downloads
            .retain(|download| !download.package.eq_ignore_ascii_case(display));
        self.warnings.retain(|warning| {
            warning.code != "plan.prebuiltUnavailable"
                || !warning
                    .params
                    .get("package")
                    .is_some_and(|package| package.eq_ignore_ascii_case(display))
        });
        if let Some((binary, engine)) = self.workspace.prebuilt_binary(version) {
            self.downloads.push(DownloadRecord {
                package: display.to_string(),
                version: version.version.clone(),
                binary,
                engine,
                dest: dest.to_path_buf(),
                replace: false,
            });
            return true;
        }
        let Some(git_ref) = version.git_ref.clone() else {
            self.error(msg(
                "plan.noVersion",
                &[("package", display), ("range", &version.version)],
            ));
            return false;
        };
        if !is_safe_ref(&git_ref) {
            self.error(msg(
                "plan.unsafeRef",
                &[("package", display), ("ref", &git_ref)],
            ));
            return false;
        }
        if let Some(engine) = self.workspace.prebuilt_engine() {
            self.warning(msg(
                "plan.prebuiltUnavailable",
                &[
                    ("package", display),
                    ("version", &version.version),
                    ("engine", &engine),
                ],
            ));
        }
        self.clones.push(CloneRecord {
            package: display.to_string(),
            version: version.version.clone(),
            url: repository.to_string(),
            git_ref,
            dest: dest.to_path_buf(),
        });
        true
    }

    /// Replaces an installed package that is not a git checkout with the prebuilt binaries of
    /// `target`: an update, or the reinstall of the version it has. Without binaries for this
    /// project (whatever the reason: the setting is off, the engine is a source build, the index
    /// has none for it) the package has to be uninstalled and installed again, from source;
    /// `plan.noPrebuilt` says so whenever the engine is known. Without an engine the only
    /// thing to say is that the package is not a git checkout.
    fn replace_with_prebuilt(&mut self, display: &str, target: &IndexVersion) {
        let Some((binary, engine)) = self.workspace.prebuilt_binary(target) else {
            match self.workspace.engine_major_minor() {
                Some(engine) => self.error(msg(
                    "plan.noPrebuilt",
                    &[
                        ("package", display),
                        ("version", &target.version),
                        ("engine", &engine),
                    ],
                )),
                None => self.error(msg("plan.notGit", &[("package", display)])),
            }
            return;
        };
        let workspace = self.workspace;
        let Some(project) = workspace.project() else {
            return;
        };
        let dir = self.package(display).dir.clone().unwrap_or_default();
        // The folder goes to the Recycle Bin when the new one is in place: never a plugin root,
        // which holds other plugins too.
        if !project.is_discoverable(&dir) || project.is_plugin_root(&dir) {
            self.error(msg("plan.pluginRoot", &[("path", &project.location(&dir))]));
            return;
        }
        self.check_engine_constraint(display, target);
        self.checkouts
            .retain(|checkout| !checkout.package.eq_ignore_ascii_case(display));
        self.downloads
            .retain(|download| !download.package.eq_ignore_ascii_case(display));
        self.downloads.push(DownloadRecord {
            package: display.to_string(),
            version: target.version.clone(),
            binary,
            engine,
            dest: dir,
            replace: true,
        });
        self.apply_version(display, target);
        if self.package(display).enabled {
            self.resolve_requirements(display);
        }
    }

    /// Enables a package (if needed) and resolves its requirements.
    fn ensure_enabled(&mut self, name: &str, required_by: Option<&str>) {
        let package = self.package(name);
        let display = package.name.clone();
        if !package.enabled {
            package.enabled = true;
            if !self
                .enables
                .iter()
                .any(|(existing, _)| existing.eq_ignore_ascii_case(&display))
            {
                self.enables
                    .push((display.clone(), required_by.map(str::to_string)));
            }
        }
        self.resolve_requirements(&display);
    }

    /// Remembers the requirements of the version that `dependency` is about to be updated to (see
    /// `planned_requires`). Returns the key of the entry it made, if any.
    fn plan_ahead(&mut self, dependency: &Dependency) -> Option<String> {
        let constraint = Constraint::parse(&dependency.version).unwrap_or_default();
        let name = self.package(&dependency.name).name.clone();
        let dep_key = key(&name);
        if self.protected.contains(&dep_key)
            || self.planned_requires.contains_key(&dep_key)
            || self.pinned_install_version(&name).is_some()
        {
            return None;
        }
        let (installed, version) = {
            let package = self.package(&name);
            (package.installed, package.semver())
        };
        let satisfied = constraint.is_any()
            || version
                .as_ref()
                .is_some_and(|version| constraint.is_satisfied_by(version));
        if !installed || satisfied {
            return None;
        }
        let target = self.index_package(&name).and_then(|package| {
            package
                .released_versions()
                .into_iter()
                .find(|(_, version)| constraint.is_satisfied_by(version))
                .map(|(version, _)| version)
        })?;
        self.planned_requires
            .insert(dep_key.clone(), dependencies_of(target));
        Some(dep_key)
    }

    fn resolve_requirements(&mut self, owner: &str) {
        let owner_key = key(owner);
        if !self.resolving.insert(owner_key.clone()) {
            return;
        }
        let (requires, engine_plugins, owner_name) = {
            let package = self.package(owner);
            (
                package.requires.clone(),
                package.engine_plugins.clone(),
                package.name.clone(),
            )
        };

        // Dependencies that this resolution is going to update have new requirements of their
        // own. Note them first, so that no dependency is chosen against the requirements a
        // sibling is about to leave (`UBotROS` needs Core ^0.2.0 and Sensor ^0.2.0: Core must not
        // be refused because the installed Sensor still wants ^0.1.0).
        let ahead: Vec<String> = requires
            .iter()
            .filter(|dependency| !dependency.optional)
            .filter_map(|dependency| self.plan_ahead(dependency))
            .collect();

        for dependency in requires.iter().filter(|dependency| !dependency.optional) {
            let constraint = Constraint::parse(&dependency.version).unwrap_or_default();
            let dep_name = self.package(&dependency.name).name.clone();
            if self.protected.contains(&key(&dep_name)) {
                self.error(msg(
                    "plan.breaksDependent",
                    &[
                        ("package", &dep_name),
                        ("dependent", &owner_name),
                        ("range", &range_text(&dependency.version)),
                    ],
                ));
                continue;
            }
            let (installed, enabled, version) = {
                let package = self.package(&dep_name);
                (package.installed, package.enabled, package.semver())
            };
            if installed {
                let satisfied = constraint.is_any()
                    || version
                        .as_ref()
                        .is_some_and(|version| constraint.is_satisfied_by(version));
                if satisfied {
                    if !enabled {
                        self.ensure_enabled(&dep_name, Some(&owner_name));
                    }
                    continue;
                }
                // Keep the explicit install target even if this owner's current requirements
                // reject it. The owner may still change; only its final requirements diagnose
                // the conflict. Do not select a temporary version or emit a premature noVersion.
                if self.pinned_install_version(&dep_name).is_some() {
                    if !enabled {
                        self.ensure_enabled(&dep_name, Some(&owner_name));
                    }
                    continue;
                }
                // The highest index version that satisfies this requirement and, when there is
                // one, every other enabled dependent. When the others cannot all be satisfied the
                // requirement of this package still decides; the final check names the dependents
                // that end up unsatisfied ("X requires Y ^0.1.0.").
                let others = self.dependent_constraints(&dep_name, &[&owner_name]);
                let candidate = self.index_package(&dep_name).and_then(|package| {
                    let released = package.released_versions();
                    let wanted = |version: &SemVer| constraint.is_satisfied_by(version);
                    released
                        .iter()
                        .find(|(_, version)| {
                            wanted(version)
                                && others
                                    .iter()
                                    .all(|(_, other)| other.is_satisfied_by(version))
                        })
                        .or_else(|| released.iter().find(|(_, version)| wanted(version)))
                        .map(|(version, _)| version.version.clone())
                });
                match candidate {
                    Some(version) => {
                        self.update(&dep_name, &version);
                        if !self.package(&dep_name).enabled {
                            self.ensure_enabled(&dep_name, Some(&owner_name));
                        }
                    }
                    None => self.error(msg(
                        "plan.noVersion",
                        &[
                            ("package", &dep_name),
                            ("range", &range_text(&dependency.version)),
                        ],
                    )),
                }
            } else if self.index_package(&dep_name).is_some() {
                self.install(&dep_name, None, Some(&constraint), Some(&owner_name));
            } else {
                self.error(msg("plan.notInIndex", &[("package", &dep_name)]));
            }
        }

        for plugin in &engine_plugins {
            self.ensure_engine_plugin(plugin, Some(&owner_name));
        }
        // Notes that were not followed by an update (the update failed or went another way) must
        // not shape later decisions.
        for dep_key in ahead {
            self.planned_requires.remove(&dep_key);
        }
        self.resolving.remove(&owner_key);
    }

    /// Enabled packages (other than `except`) that require `name`, with their constraints. A
    /// package that is going to be updated counts with the requirements of the new version.
    fn dependent_constraints(&mut self, name: &str, except: &[&str]) -> Vec<(String, Constraint)> {
        self.load_all_installed();
        let mut result = Vec::new();
        for package in self.packages.values() {
            if !package.enabled
                || package.name.eq_ignore_ascii_case(name)
                || except.iter().any(|e| e.eq_ignore_ascii_case(&package.name))
            {
                continue;
            }
            if self.protected.contains(&key(&package.name)) {
                continue;
            }
            let requires = self
                .planned_requires
                .get(&key(&package.name))
                .unwrap_or(&package.requires);
            for dependency in requires
                .iter()
                .filter(|d| !d.optional && d.name.trim().eq_ignore_ascii_case(name))
            {
                result.push((
                    package.name.clone(),
                    Constraint::parse(&dependency.version).unwrap_or_default(),
                ));
            }
        }
        result.sort_by_key(|entry| entry.0.to_lowercase());
        result
    }

    /// The consistency check on the final state, after every change is applied: an enabled package
    /// whose own version or whose dependency's version this plan changed must have its
    /// requirements met by the final versions, whatever order the changes and the `requires`
    /// lists came in. A conflict that already exists and is not touched by this plan is left to
    /// the problems list.
    fn verify(&mut self) {
        let touched: HashSet<String> = self
            .clones
            .iter()
            .map(|clone| key(&clone.package))
            .chain(self.checkouts.iter().map(|checkout| key(&checkout.package)))
            .chain(self.downloads.iter().map(|download| key(&download.package)))
            .collect();
        let mut conflicts = self.requested_version_conflicts();
        if touched.is_empty() && conflicts.is_empty() {
            return;
        }
        self.load_all_installed();
        let mut packages: Vec<&VPackage> = self
            .packages
            .values()
            .filter(|package| package.enabled)
            .collect();
        packages.sort_by_key(|package| package.name.to_lowercase());

        for package in packages {
            for dependency in package.requires.iter().filter(|d| !d.optional) {
                let dep_key = key(&dependency.name);
                if !touched.contains(&key(&package.name)) && !touched.contains(&dep_key) {
                    continue;
                }
                let Some(dep) = self.packages.get(&dep_key) else {
                    continue;
                };
                let Some(version) = dep.semver().filter(|_| dep.installed && dep.enabled) else {
                    continue;
                };
                let constraint = Constraint::parse(&dependency.version).unwrap_or_default();
                if !constraint.is_any() && !constraint.is_satisfied_by(&version) {
                    let range = range_text(&dependency.version);
                    // Already said when no version of the dependency exists at all.
                    let no_version = msg(
                        "plan.noVersion",
                        &[("package", &dep.name), ("range", &range)],
                    );
                    if self.errors.contains(&no_version) {
                        continue;
                    }
                    conflicts.push(msg(
                        "plan.breaksDependent",
                        &[
                            ("package", &dep.name),
                            ("dependent", &package.name),
                            ("range", &range),
                        ],
                    ));
                }
            }
        }
        for conflict in conflicts {
            self.error(conflict);
        }
    }

    /// Explains requests that ended at another version using final dependent requirements.
    /// Install targets also get a fallback when no dependent explains the mismatch.
    fn requested_version_conflicts(&mut self) -> Vec<Msg> {
        if self.requested.is_empty() {
            return Vec::new();
        }
        self.load_all_installed();
        let mut requested: Vec<(&String, &RequestedVersion)> = self.requested.iter().collect();
        requested.sort_by(|a, b| a.0.cmp(b.0));
        let mut dependents: Vec<&VPackage> = self
            .packages
            .values()
            .filter(|package| package.enabled)
            .collect();
        dependents.sort_by_key(|package| package.name.to_lowercase());

        let mut conflicts = Vec::new();
        for (package_key, requested) in requested {
            let display = &requested.name;
            let wanted = &requested.version;
            let install = requested.origin == RequestedVersionOrigin::Install;
            let final_version = self
                .packages
                .get(package_key)
                .filter(|package| package.installed)
                .and_then(VPackage::semver);
            if final_version.as_ref() == Some(wanted) {
                continue;
            }
            // Failed installs already have a specific diagnostic and no resolved version.
            if install && final_version.is_none() {
                continue;
            }
            let before = conflicts.len();
            for dependent in &dependents {
                for dependency in dependent
                    .requires
                    .iter()
                    .filter(|d| !d.optional && key(&d.name) == *package_key)
                {
                    let constraint = Constraint::parse(&dependency.version).unwrap_or_default();
                    if !constraint.is_any()
                        && (!constraint.is_satisfied_by(wanted)
                            || (install
                                && final_version
                                    .as_ref()
                                    .is_some_and(|version| !constraint.is_satisfied_by(version))))
                    {
                        conflicts.push(msg(
                            "plan.breaksDependent",
                            &[
                                ("package", display),
                                ("dependent", &dependent.name),
                                ("range", &range_text(&dependency.version)),
                            ],
                        ));
                    }
                }
            }
            if install && conflicts.len() == before {
                if let Some(resolved) = final_version {
                    conflicts.push(msg(
                        "plan.requestedVersionChanged",
                        &[
                            ("package", display),
                            ("requested", &wanted.to_string()),
                            ("resolved", &resolved.to_string()),
                        ],
                    ));
                }
            }
        }
        conflicts
    }

    fn load_all_installed(&mut self) {
        let names: Vec<String> = self
            .workspace
            .project()
            .map(|project| project.plugins.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        for name in names {
            self.package(&name);
        }
    }

    /// Enabled packages that require `name` (as a package or an engine plugin), transitively,
    /// dependents first.
    fn enabled_dependents(&mut self, name: &str) -> Vec<String> {
        self.load_all_installed();
        let mut found: Vec<String> = Vec::new();
        let mut frontier = vec![key(name)];
        while let Some(current) = frontier.pop() {
            let mut names: Vec<&VPackage> = self.packages.values().collect();
            names.sort_by_key(|package| package.name.to_lowercase());
            for package in names {
                let package_key = key(&package.name);
                if !package.enabled
                    || package_key == key(name)
                    || found.iter().any(|f| key(f) == package_key)
                {
                    continue;
                }
                let requires = package
                    .requires
                    .iter()
                    .any(|d| !d.optional && key(&d.name) == current)
                    || package
                        .engine_plugins
                        .iter()
                        .any(|plugin| key(plugin) == current);
                if requires {
                    found.push(package.name.clone());
                    frontier.push(package_key);
                }
            }
        }
        self.dependents_first(found)
    }

    /// Orders names so that every package comes before the packages it requires.
    fn dependents_first(&mut self, names: Vec<String>) -> Vec<String> {
        let mut order = self.dependencies_first(names);
        order.reverse();
        order
    }

    /// Orders names so that requirements come first.
    fn dependencies_first(&mut self, names: Vec<String>) -> Vec<String> {
        let mut sorted = names.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        let edges: Vec<Vec<usize>> = sorted
            .iter()
            .map(|name| {
                let requires: Vec<String> = self
                    .package(name)
                    .requires
                    .iter()
                    .filter(|d| !d.optional)
                    .map(|d| key(&d.name))
                    .collect();
                sorted
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| requires.contains(&key(other)))
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect();
        let roots: Vec<usize> = (0..sorted.len()).collect();
        graph::dependency_order(&edges, &roots)
            .into_iter()
            .map(|index| sorted[index].clone())
            .collect()
    }

    /// [`Planner::update`] for an update the user asked for: when it is applied it is remembered
    /// (see `requested`). One that changes nothing is not.
    fn update_requested(&mut self, name: &str, version: &str) {
        let before = self.package(name).semver();
        self.update_to(name, version, true);
        let wanted = self
            .index_package(name)
            .and_then(|package| package.find_version(version))
            .and_then(IndexVersion::semver);
        let package = self.package(name);
        if let Some(wanted) = wanted {
            if before.as_ref() != Some(&wanted) && package.semver().as_ref() == Some(&wanted) {
                let display = package.name.clone();
                self.requested.insert(
                    key(name),
                    RequestedVersion {
                        name: display,
                        version: wanted,
                        origin: RequestedVersionOrigin::AppliedUpdate,
                    },
                );
            }
        }
    }

    /// Updates (or downgrades) an installed package. Whether the dependents still get what they
    /// require is decided once for the whole plan, in `verify`.
    fn update(&mut self, name: &str, version: &str) {
        self.update_to(name, version, false);
    }

    /// [`Planner::update`]; with `reinstall` (an update the user asked for) a prebuilt install
    /// at the version it has is downloaded again.
    fn update_to(&mut self, name: &str, version: &str, reinstall: bool) {
        let display = self.package(name).name.clone();
        if !self.package(name).installed {
            self.error(msg("plan.notInstalled", &[("package", &display)]));
            return;
        }
        let Some(index_package) = self.index_package(name) else {
            self.error(msg("plan.notInIndex", &[("package", &display)]));
            return;
        };
        if !is_plugin_name(&index_package.name) {
            self.error(msg("plan.unsafeName", &[("package", &index_package.name)]));
            return;
        }
        let Some(target) = index_package.find_version(version) else {
            self.error(msg(
                "plan.noVersion",
                &[("package", &display), ("range", version)],
            ));
            return;
        };
        let target_semver = target.semver();
        if self
            .pinned_install_version(name)
            .is_some_and(|pinned| target_semver.as_ref() != Some(pinned))
        {
            return;
        }

        // A package this plan installs is installed at the new version instead (cloned, or
        // downloaded when that version has binaries).
        let fresh = self
            .clones
            .iter()
            .find(|clone| clone.package.eq_ignore_ascii_case(&display))
            .map(|clone| (clone.dest.clone(), clone.url.clone()))
            .or_else(|| {
                self.downloads
                    .iter()
                    .find(|download| {
                        !download.replace && download.package.eq_ignore_ascii_case(&display)
                    })
                    .map(|download| {
                        let repository = index_package.repository.clone().unwrap_or_default();
                        (download.dest.clone(), repository)
                    })
            });
        if let Some((dest, repository)) = fresh {
            if !self.acquire(&display, target, &repository, &dest) {
                return;
            }
            self.check_engine_constraint(&display, target);
            self.apply_version(&display, target);
            self.resolve_requirements(&display);
            return;
        }
        let kind = self.workspace.install_kind(name);
        if self
            .package(name)
            .semver()
            .is_some_and(|current| Some(&current) == target_semver.as_ref())
        {
            // The version it has; a prebuilt install the user asks to update is downloaded again
            // (for another engine, or to repair it).
            if reinstall && kind == Some(InstallKind::Prebuilt) {
                self.replace_with_prebuilt(&display, target);
            }
            return;
        }

        // Back at the version on disk, after this plan had moved the package away: there is
        // nothing to check out (and so nothing to check about the folder).
        let on_disk = self
            .workspace
            .installed(name)
            .and_then(|plugin| plugin.version())
            .and_then(SemVer::parse);
        if on_disk.is_some() && on_disk == target_semver {
            self.checkouts
                .retain(|checkout| !checkout.package.eq_ignore_ascii_case(&display));
            self.downloads
                .retain(|download| !download.package.eq_ignore_ascii_case(&display));
            self.apply_version(&display, target);
            if self.package(name).enabled {
                self.resolve_requirements(&display);
            }
            return;
        }

        // Only a git checkout is updated with git; anything else is replaced by prebuilt
        // binaries, when there are any for this project.
        if kind != Some(InstallKind::Git) {
            self.replace_with_prebuilt(&display, target);
            return;
        }
        let git = self.workspace.git(name).cloned().unwrap_or_default();
        if !git.is_checkout {
            self.error(msg("plan.notGit", &[("package", &display)]));
            return;
        }
        if let Some(error) = &git.error {
            // Git could not read the folder: "clean" would be a guess.
            self.error(msg(
                "plan.gitUnreadable",
                &[("package", &display), ("error", &redact_credentials(error))],
            ));
            return;
        }
        if git.dirty {
            self.error(msg("plan.dirty", &[("package", &display)]));
        }
        if let Some(branch) = &git.branch {
            self.warning(msg(
                "plan.onBranch",
                &[("package", &display), ("branch", branch)],
            ));
        }
        let Some(git_ref) = target.git_ref.clone() else {
            self.error(msg(
                "plan.noVersion",
                &[("package", &display), ("range", version)],
            ));
            return;
        };
        if !is_safe_ref(&git_ref) {
            self.error(msg(
                "plan.unsafeRef",
                &[("package", &display), ("ref", &git_ref)],
            ));
            return;
        }
        self.check_engine_constraint(&display, target);

        let from = self
            .package(name)
            .version
            .clone()
            .unwrap_or_else(|| git.to_state().git_ref.unwrap_or_default());
        let dir = self.package(name).dir.clone().unwrap_or_default();
        self.checkouts
            .retain(|checkout| !checkout.package.eq_ignore_ascii_case(&display));
        self.checkouts.push(CheckoutRecord {
            package: display.clone(),
            from,
            to: target.version.clone(),
            git_ref,
            dir,
        });
        self.apply_version(&display, target);
        if self.package(name).enabled {
            self.resolve_requirements(&display);
        }
    }

    fn ensure_engine_plugin(&mut self, plugin: &str, required_by: Option<&str>) {
        let workspace = self.workspace;
        let Some(project) = workspace.project() else {
            return;
        };
        if self.protected.contains(&key(plugin)) {
            let dependent = required_by.unwrap_or_default().to_string();
            self.error(msg(
                "plan.breaksDependent",
                &[
                    ("package", plugin),
                    ("dependent", &dependent),
                    ("range", "*"),
                ],
            ));
            return;
        }
        if project.find_plugin(plugin).is_some() {
            let package = self.package(plugin);
            if !package.enabled {
                package.enabled = true;
                let name = package.name.clone();
                if !self
                    .engine_enables
                    .iter()
                    .any(|(existing, _)| existing.eq_ignore_ascii_case(&name))
                {
                    self.engine_enables
                        .push((name, required_by.map(str::to_string)));
                }
            }
            return;
        }
        let name = match workspace.engine_plugins() {
            Some(engine_plugins) => match engine_plugins.get(plugin) {
                Some(found) => {
                    if project.is_engine_plugin_enabled(plugin, engine_plugins) {
                        return;
                    }
                    found.name.clone()
                }
                None => {
                    self.error(msg("plan.enginePluginMissing", &[("plugin", plugin)]));
                    return;
                }
            },
            // Without an engine the plugin cannot be checked; the entry is still written.
            None if project.uproject.entry_enabled(plugin) == Some(true) => return,
            None => plugin.to_string(),
        };
        if !self
            .engine_enables
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case(&name))
        {
            self.engine_enables
                .push((name, required_by.map(str::to_string)));
        }
    }

    fn disable(&mut self, name: &str, uninstall: bool) {
        let workspace = self.workspace;
        let Some(project) = workspace.project() else {
            return;
        };
        let plugin = project.find_plugin(name);
        let engine_plugin = workspace
            .engine_plugins()
            .and_then(|engine_plugins| engine_plugins.get(name));
        let display = match (plugin, engine_plugin) {
            (Some(plugin), _) => workspace.display_name(&plugin.name),
            (None, Some(engine_plugin)) if !uninstall => engine_plugin.name.clone(),
            (None, None)
                if !uninstall
                    && workspace.engine_plugins().is_none()
                    && workspace.index.package(name).is_none() =>
            {
                name.trim().to_string()
            }
            _ => {
                self.error(msg(
                    "plan.notInstalled",
                    &[("package", &workspace.display_name(name))],
                ));
                return;
            }
        };
        if uninstall {
            let dir = plugin.map(|plugin| plugin.dir.clone()).unwrap_or_default();
            if !paths::is_strictly_within(&dir, &project.dir) {
                self.error(msg(
                    "plan.outsideProject",
                    &[("path", &project.location(&dir))],
                ));
                return;
            }
            // A descriptor that lies directly in a plugin root (Plugins/UBotX.uplugin) makes the
            // root itself the "package folder"; moving that to the Recycle Bin would take every
            // other plugin in it along.
            if project.is_plugin_root(&dir) {
                self.error(msg("plan.pluginRoot", &[("path", &project.location(&dir))]));
                return;
            }
        }

        let dependents = self.enabled_dependents(&display);
        if !dependents.is_empty() {
            self.warning(msg(
                "plan.alsoDisable",
                &[
                    ("package", &display),
                    ("dependents", &dependents.join(", ")),
                ],
            ));
        }
        for dependent in &dependents {
            self.protected.insert(key(dependent));
            let package = self.package(dependent);
            package.enabled = false;
            let dependent_name = package.name.clone();
            if !self
                .disables
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&dependent_name))
            {
                self.disables.push(dependent_name);
            }
        }
        self.protected.insert(key(&display));

        let currently_enabled = match plugin {
            Some(_) => self.package(&display).enabled,
            None => match workspace.engine_plugins() {
                Some(engine_plugins) => project.is_engine_plugin_enabled(&display, engine_plugins),
                None => project.uproject.entry_enabled(&display) != Some(false),
            },
        };
        if plugin.is_some() {
            self.package(&display).enabled = false;
        }
        if uninstall {
            self.removes.push(display);
        } else if currently_enabled
            && !self
                .disables
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&display))
        {
            self.disables.push(display);
        }
    }

    fn enable(&mut self, name: &str) {
        let workspace = self.workspace;
        let Some(project) = workspace.project() else {
            return;
        };
        if project.find_plugin(name).is_some() {
            self.ensure_enabled(name, None);
            return;
        }
        if workspace
            .engine_plugins()
            .is_some_and(|engine_plugins| engine_plugins.get(name).is_some())
        {
            self.ensure_engine_plugin(name, None);
            return;
        }
        self.error(msg(
            "plan.notInstalled",
            &[("package", &workspace.display_name(name))],
        ));
    }

    fn sorted_download_names(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self.clones.iter().map(|c| c.package.clone()).collect();
        names.extend(self.checkouts.iter().map(|c| c.package.clone()));
        names.extend(self.downloads.iter().map(|d| d.package.clone()));
        self.dependencies_first(names)
    }

    /// Whether the plan leaves code to compile: a package that is enabled after it whose code
    /// comes from source - cloned or checked out by this plan, or installed from source without
    /// editor binaries. Packages that come prebuilt (downloaded now, or with a valid marker)
    /// need no build, and neither do plugins without modules. Engine plugins enabled for an
    /// engine that is not a launcher install may need building, too.
    fn source_build_needed(&mut self) -> bool {
        let launcher = self
            .workspace
            .project
            .as_ref()
            .and_then(|context| context.engine.as_ref())
            .is_some_and(|engine| engine.kind == EngineKind::Launcher);
        if !self.engine_enables.is_empty() && !launcher {
            return true;
        }
        self.load_all_installed();
        let mut enabled: Vec<String> = self
            .packages
            .values()
            .filter(|package| package.installed && package.enabled)
            .map(|package| package.name.clone())
            .collect();
        enabled.sort_by_key(|name| name.to_lowercase());
        for name in enabled {
            let same = |package: &str| package.eq_ignore_ascii_case(&name);
            if self.clones.iter().any(|clone| same(&clone.package))
                || self
                    .checkouts
                    .iter()
                    .any(|checkout| same(&checkout.package))
            {
                return true;
            }
            if self
                .downloads
                .iter()
                .any(|download| same(&download.package))
            {
                continue;
            }
            let Some(plugin) = self.workspace.plugin(&name) else {
                continue;
            };
            if self.workspace.install_kind(&name) == Some(InstallKind::Prebuilt) {
                continue;
            }
            if plugin.has_modules() && !prebuilt::has_editor_binaries(&plugin.dir) {
                return true;
            }
        }
        false
    }
}

/// Keeps one change per package (the last one wins) and trims names.
pub fn normalize_changes(changes: &[Change]) -> Vec<Change> {
    let mut result: Vec<Change> = Vec::new();
    for change in changes {
        let package = change.package.trim().to_string();
        if package.is_empty() {
            continue;
        }
        result.retain(|existing| !existing.package.eq_ignore_ascii_case(&package));
        let version = change
            .version
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        result.push(Change {
            kind: change.kind,
            package,
            version,
        });
    }
    result
}

/// Plans `changes` against the workspace, assuming a C++ compiler is installed.
pub fn plan(workspace: &Workspace, changes: &[Change], options: &ApplyOptions) -> PreparedPlan {
    plan_with(workspace, changes, options, &|| true)
}

/// Plans `changes` against the workspace. `compiler_found` (Visual Studio 2022, SPEC §7
/// `check_environment`) is asked only when the plan has a build step; without one the plan warns
/// `plan.noCompiler`.
pub fn plan_with(
    workspace: &Workspace,
    changes: &[Change],
    options: &ApplyOptions,
    compiler_found: &dyn Fn() -> bool,
) -> PreparedPlan {
    let changes = normalize_changes(changes);
    let Some(context) = workspace.project.as_ref() else {
        let errors = if changes.is_empty() {
            Vec::new()
        } else {
            vec![msg0("plan.noProject")]
        };
        return PreparedPlan {
            plan: Plan {
                changes,
                errors,
                ..Plan::default()
            },
            ..PreparedPlan::default()
        };
    };
    let project = &context.project;
    let mut planner = Planner::new(workspace);
    // Install targets must be visible even to dependencies pulled in during the Update phase.
    for change in &changes {
        if change.kind == ChangeKind::Install {
            if let Some(version) = &change.version {
                planner.expect_install(&change.package, version);
            }
        }
    }

    // Disables and uninstalls first, so that other changes see what is going away.
    let order = |kind: ChangeKind| match kind {
        ChangeKind::Disable | ChangeKind::Uninstall => 0,
        ChangeKind::Update => 1,
        ChangeKind::Install => 2,
        ChangeKind::Enable => 3,
    };
    let mut ordered: Vec<&Change> = changes.iter().collect();
    ordered.sort_by_key(|change| order(change.kind));

    // An explicit update is known up front. Noting it first keeps the other changes from judging
    // the package by the requirements of the version it is leaving.
    for change in &changes {
        if change.kind != ChangeKind::Update {
            continue;
        }
        let Some(index_package) = workspace.index.package(&change.package) else {
            continue;
        };
        let target = match &change.version {
            Some(version) => index_package.find_version(version),
            None => index_package.latest(),
        };
        if let (Some(target), Some(_)) = (target, workspace.installed(&change.package)) {
            planner.expect_update(&change.package, target);
        }
    }

    for change in ordered {
        let name = change.package.as_str();
        match change.kind {
            ChangeKind::Install => {
                if workspace.plugin(name).is_some() {
                    planner.error(msg(
                        "plan.alreadyInstalled",
                        &[("package", &workspace.display_name(name))],
                    ));
                    continue;
                }
                planner.install(name, change.version.as_deref(), None, None);
            }
            ChangeKind::Update => {
                if workspace.installed(name).is_none() {
                    planner.error(msg(
                        "plan.notInstalled",
                        &[("package", &workspace.display_name(name))],
                    ));
                    continue;
                }
                let version = match &change.version {
                    Some(version) => version.clone(),
                    None => match workspace.index.package(name).and_then(IndexPackage::latest) {
                        Some(latest) => latest.version.clone(),
                        None => {
                            planner.error(msg(
                                "plan.notInIndex",
                                &[("package", &workspace.display_name(name))],
                            ));
                            continue;
                        }
                    },
                };
                planner.update_requested(name, &version);
            }
            ChangeKind::Enable => planner.enable(name),
            ChangeKind::Disable => planner.disable(name, false),
            ChangeKind::Uninstall => planner.disable(name, true),
        }
    }

    planner.verify();

    // Echo only final choices. An explicit version may be normalized to equivalent index text,
    // but must never be replaced by a different version, even in an erroneous plan.
    let mut resolved_changes = changes;
    for change in &mut resolved_changes {
        if change.kind != ChangeKind::Install {
            continue;
        }
        change.package = workspace.display_name(&change.package);
        if let Some(requested) = &change.version {
            if let Some(version) = workspace
                .index
                .package(&change.package)
                .and_then(|package| package.find_version(requested))
            {
                change.version = Some(version.version.clone());
            }
        } else {
            change.version = planner
                .clones
                .iter()
                .map(|clone| (&clone.package, &clone.version))
                .chain(
                    planner
                        .downloads
                        .iter()
                        .map(|download| (&download.package, &download.version)),
                )
                .find(|(package, _)| package.eq_ignore_ascii_case(&change.package))
                .map(|(_, version)| version.clone());
        }
    }

    let project_file = project.file_name();
    let engine = context.engine.clone();
    let mut steps: Vec<(StepGroup, StepKind, Msg, Msg, StepAction)> = Vec::new();

    for name in planner.sorted_download_names() {
        if let Some(clone) = planner
            .clones
            .iter()
            .find(|clone| clone.package.eq_ignore_ascii_case(&name))
        {
            steps.push((
                StepGroup::Download,
                StepKind::Clone,
                msg(
                    "step.clone",
                    &[("package", &clone.package), ("version", &clone.version)],
                ),
                msg(
                    "step.clone.detail",
                    &[
                        ("repo", &display_repository(&clone.url)),
                        ("dest", &project.location(&clone.dest)),
                    ],
                ),
                StepAction::Clone {
                    package: clone.package.clone(),
                    url: clone.url.clone(),
                    git_ref: clone.git_ref.clone(),
                    dest: clone.dest.clone(),
                },
            ));
        }
        if let Some(checkout) = planner
            .checkouts
            .iter()
            .find(|checkout| checkout.package.eq_ignore_ascii_case(&name))
        {
            steps.push((
                StepGroup::Download,
                StepKind::Checkout,
                msg(
                    "step.checkout",
                    &[("package", &checkout.package), ("version", &checkout.to)],
                ),
                msg(
                    "step.checkout.detail",
                    &[("from", &checkout.from), ("to", &checkout.to)],
                ),
                StepAction::Checkout {
                    package: checkout.package.clone(),
                    dir: checkout.dir.clone(),
                    git_ref: checkout.git_ref.clone(),
                },
            ));
        }
        if let Some(download) = planner
            .downloads
            .iter()
            .find(|download| download.package.eq_ignore_ascii_case(&name))
        {
            steps.push((
                StepGroup::Download,
                StepKind::Download,
                msg(
                    "step.download",
                    &[
                        ("package", &download.package),
                        ("version", &download.version),
                    ],
                ),
                msg(
                    "step.download.detail",
                    &[
                        ("source", &display_download_source(&download.binary.url)),
                        ("dest", &project.location(&download.dest)),
                    ],
                ),
                StepAction::Download {
                    package: download.package.clone(),
                    version: download.version.clone(),
                    url: download.binary.url.clone(),
                    sha256: download.binary.sha256.clone(),
                    size: download.binary.size,
                    engine: download.engine.clone(),
                    dest: download.dest.clone(),
                    replace: download.replace,
                },
            ));
        }
    }

    let detail_for = |required_by: &Option<String>| match required_by {
        Some(owner) => msg("step.detail.requiredBy", &[("package", owner)]),
        None => msg("step.detail.project", &[("file", &project_file)]),
    };
    for (plugin, required_by) in planner.engine_enables.clone() {
        steps.push((
            StepGroup::Project,
            StepKind::EnableEnginePlugin,
            msg("step.enableEnginePlugin", &[("plugin", &plugin)]),
            detail_for(&required_by),
            StepAction::SetEnabled {
                plugin,
                enabled: true,
            },
        ));
    }
    let enable_names: Vec<String> = planner
        .enables
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    for name in planner.dependencies_first(enable_names) {
        let required_by = planner
            .enables
            .iter()
            .find(|(existing, _)| existing == &name)
            .and_then(|(_, by)| by.clone());
        steps.push((
            StepGroup::Project,
            StepKind::EnablePackage,
            msg("step.enablePackage", &[("package", &name)]),
            detail_for(&required_by),
            StepAction::SetEnabled {
                plugin: name,
                enabled: true,
            },
        ));
    }
    let disable_names = planner.disables.clone();
    for name in planner.dependents_first(disable_names) {
        steps.push((
            StepGroup::Project,
            StepKind::DisablePackage,
            msg("step.disablePackage", &[("package", &name)]),
            msg("step.detail.project", &[("file", &project_file)]),
            StepAction::SetEnabled {
                plugin: name,
                enabled: false,
            },
        ));
    }
    let remove_names = planner.removes.clone();
    for name in planner.dependents_first(remove_names) {
        let dir = planner.package(&name).dir.clone().unwrap_or_default();
        steps.push((
            StepGroup::Project,
            StepKind::RemovePackage,
            msg("step.removePackage", &[("package", &name)]),
            msg(
                "step.removePackage.detail",
                &[("dest", &project.location(&dir))],
            ),
            StepAction::Remove { package: name, dir },
        ));
    }

    // Something new is going to be loaded by the editor; whether that needs a C++ build depends
    // on where its code comes from.
    let touches_code = !planner.clones.is_empty()
        || !planner.checkouts.is_empty()
        || !planner.downloads.is_empty()
        || !planner.enables.is_empty()
        || !planner.engine_enables.is_empty();
    let needs_build = touches_code && planner.source_build_needed();
    let editor_running = context.editor.running;
    let engine_root = engine.as_ref().map(|engine| PathBuf::from(&engine.root));
    let wants_engine = needs_build && (options.build || options.run_tests);
    if wants_engine && engine_root.is_none() {
        planner.error(msg0("plan.noEngine"));
    }

    let mut has_build = false;
    if let Some(engine_root) = &engine_root {
        if needs_build && options.build {
            has_build = true;
            // A clone into a plugin root outside the project also decides the target.
            let new_plugin_dirs: Vec<PathBuf> = planner
                .clones
                .iter()
                .map(|clone| clone.dest.clone())
                .chain(
                    planner
                        .downloads
                        .iter()
                        .filter(|download| !download.replace)
                        .map(|download| download.dest.clone()),
                )
                .collect();
            let target = project.build_target_with(&new_plugin_dirs);
            // The build goes through cmd.exe and Build.bat: refuse what those would misread
            // (SPEC 5.3) instead of building something else, or running something else. The
            // target is the one unquoted word; cmd expands %NAME% even inside quotes.
            let uproject_text = paths::display(&project.path);
            let engine_text = paths::display(engine_root);
            if !crate::exec::is_safe_build_name(&target) {
                planner.error(msg("plan.unsafeProjectName", &[("name", &project.name)]));
            } else if uproject_text.contains('%') {
                planner.error(msg("plan.unsafeBuildPath", &[("path", &uproject_text)]));
            } else if engine_text.contains('%') {
                planner.error(msg("plan.unsafeBuildPath", &[("path", &engine_text)]));
            }
            let configuration = workspace.settings.build.configuration;
            steps.push((
                StepGroup::Build,
                StepKind::Build,
                msg("step.build", &[("target", &target)]),
                msg(
                    "step.build.detail",
                    &[
                        ("platform", PLATFORM),
                        ("configuration", configuration.as_str()),
                    ],
                ),
                StepAction::Build {
                    engine_root: engine_root.clone(),
                    target,
                    configuration,
                    uproject: project.path.clone(),
                },
            ));
            if !compiler_found() {
                planner.warning(msg0("plan.noCompiler"));
            }
        }
        let mut tested: Vec<String> = planner.clones.iter().map(|c| c.package.clone()).collect();
        for name in planner
            .checkouts
            .iter()
            .map(|c| c.package.clone())
            .chain(planner.downloads.iter().map(|d| d.package.clone()))
            .chain(planner.enables.iter().map(|(name, _)| name.clone()))
        {
            if !tested
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&name))
            {
                tested.push(name);
            }
        }
        // The names end up inside the -ExecCmds string of the editor.
        tested.retain(|name| {
            is_plugin_name(name)
                && planner
                    .packages
                    .get(&key(name))
                    .is_some_and(|package| package.is_ubot)
        });
        if options.run_tests && !tested.is_empty() {
            let filter = tested.join("+");
            steps.push((
                StepGroup::After,
                StepKind::RunTests,
                msg0("step.runTests"),
                msg("step.runTests.detail", &[("filter", &filter)]),
                StepAction::RunTests {
                    engine_root: engine_root.clone(),
                    uproject: project.path.clone(),
                    packages: tested,
                },
            ));
        }
        if options.reopen_editor && (editor_running || has_build) && !steps.is_empty() {
            steps.push((
                StepGroup::After,
                StepKind::OpenEditor,
                msg0("step.openEditor"),
                msg("step.openEditor.detail", &[("project", &project_file)]),
                StepAction::OpenEditor {
                    engine_root: engine_root.clone(),
                    uproject: project.path.clone(),
                },
            ));
        }
    }

    // Enable, disable and remove steps rewrite the .uproject; a read-only one (a Perforce
    // workspace) would fail them after the clones were made. Say so before anything runs.
    let edits_project = steps.iter().any(|(_, kind, ..)| {
        matches!(
            kind,
            StepKind::EnablePackage
                | StepKind::DisablePackage
                | StepKind::EnableEnginePlugin
                | StepKind::RemovePackage
        )
    });
    if edits_project
        && std::fs::metadata(&project.path).is_ok_and(|meta| meta.permissions().readonly())
    {
        planner.error(msg("plan.projectReadOnly", &[("file", &project_file)]));
    }

    let mut plan_steps = Vec::with_capacity(steps.len());
    let mut actions = Vec::with_capacity(steps.len());
    for (position, (group, kind, title, detail, action)) in steps.into_iter().enumerate() {
        let n = position as u32 + 1;
        // A new folder does not disturb a running editor; replacing one it may have loaded does.
        let needs_editor_closed = match &action {
            StepAction::Clone { .. } => false,
            StepAction::Download { replace, .. } => *replace,
            _ => true,
        };
        plan_steps.push(PlanStep {
            id: format!("s{n}"),
            n,
            group,
            kind,
            title,
            detail,
            needs_editor_closed,
        });
        actions.push(action);
    }

    PreparedPlan {
        plan: Plan {
            changes: resolved_changes,
            steps: plan_steps,
            needs_build,
            editor_running,
            errors: planner.errors,
            warnings: planner.warnings,
        },
        actions,
        project_path: project.path.clone(),
        project_name: project.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitInfo;
    use crate::msg::render_en;
    use crate::workspace::tests::{descriptor, sample_workspace, write};
    use std::path::Path;

    fn options(build: bool) -> ApplyOptions {
        ApplyOptions {
            build,
            reopen_editor: false,
            run_tests: false,
        }
    }

    fn change(kind: ChangeKind, package: &str, version: Option<&str>) -> Change {
        Change::new(kind, package, version.map(str::to_string))
    }

    fn titles(plan: &Plan) -> Vec<String> {
        plan.steps
            .iter()
            .map(|step| render_en(&step.title))
            .collect()
    }

    fn texts(messages: &[Msg]) -> Vec<String> {
        messages.iter().map(render_en).collect()
    }

    const INSTALL_VERSIONS: &str =
        include_str!("../tests/fixtures/install-version-constraints.json");

    fn install_versions_workspace(dir: &Path, prebuilt: bool) -> Workspace {
        let mut workspace = sample_workspace(dir, false);
        let context = workspace.project.as_mut().unwrap();
        std::fs::remove_dir_all(context.project.dir.join("Plugins")).unwrap();
        write(
            &context.project.path,
            r#"{ "FileVersion": 3, "EngineAssociation": "5.5" }"#,
        );
        context.git.clear();
        context.project.reload().unwrap();
        set_install_index(
            &mut workspace,
            &serde_json::from_str(INSTALL_VERSIONS).unwrap(),
        );
        if prebuilt {
            for (name, version) in [
                ("UBotCore", "0.1.0"),
                ("UBotCore", "0.1.3"),
                ("UBotCore", "0.1.4"),
                ("UBotROS", "0.1.3"),
            ] {
                crate::workspace::tests::add_binaries(&mut workspace, name, version);
            }
        }
        workspace
    }

    fn set_install_index(workspace: &mut Workspace, index: &serde_json::Value) {
        workspace.index = crate::index::MergedIndex::merge(&[crate::index::SourceIndex {
            id: "install-test".into(),
            index: crate::index::parse(&index.to_string()).unwrap(),
        }]);
    }

    fn install_pair(core: Option<&str>, reverse: bool) -> Vec<Change> {
        let mut changes = vec![
            change(ChangeKind::Install, "UBotROS", Some("0.1.3")),
            change(ChangeKind::Install, "UBotCore", core),
        ];
        if reverse {
            changes.reverse();
        }
        changes
    }

    fn assert_core_acquisition(prepared: &PreparedPlan, version: &str, prebuilt: bool) {
        let acquisitions: Vec<&StepAction> = prepared
            .actions
            .iter()
            .filter(|action| match action {
                StepAction::Clone { package, .. } | StepAction::Download { package, .. } => {
                    package == "UBotCore"
                }
                _ => false,
            })
            .collect();
        assert_eq!(acquisitions.len(), 1, "{:?}", prepared.actions);
        match acquisitions[0] {
            StepAction::Clone { git_ref, .. } => {
                assert!(!prebuilt);
                assert_eq!(git_ref, &format!("v{version}"));
            }
            StepAction::Download {
                version: actual, ..
            } => {
                assert!(prebuilt);
                assert_eq!(actual, version);
            }
            _ => unreachable!(),
        }
    }

    fn assert_core_echo(prepared: &PreparedPlan, version: &str) {
        let core = prepared
            .plan
            .changes
            .iter()
            .find(|c| c.package == "UBotCore")
            .unwrap();
        assert_eq!(core.version.as_deref(), Some(version));
    }

    fn core_ros_conflict() -> Msg {
        msg(
            "plan.breaksDependent",
            &[
                ("package", "UBotCore"),
                ("dependent", "UBotROS"),
                ("range", "^0.1.3"),
            ],
        )
    }

    #[test]
    fn explicit_install_conflicts_are_order_independent() {
        for prebuilt in [false, true] {
            for reverse in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let workspace = install_versions_workspace(dir.path(), prebuilt);
                let prepared = plan(
                    &workspace,
                    &install_pair(Some("0.1.0"), reverse),
                    &options(false),
                );
                assert_eq!(prepared.plan.errors, vec![core_ros_conflict()]);
                assert_core_echo(&prepared, "0.1.0");
                assert_core_acquisition(&prepared, "0.1.0", prebuilt);
                let replay = plan(&workspace, &prepared.plan.changes, &options(false));
                assert_eq!(replay.plan.errors, vec![core_ros_conflict()]);
            }
        }
    }

    #[test]
    fn explicit_install_keeps_a_compatible_non_latest_version() {
        for prebuilt in [false, true] {
            for reverse in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let workspace = install_versions_workspace(dir.path(), prebuilt);
                let prepared = plan(
                    &workspace,
                    &install_pair(Some("0.1.3"), reverse),
                    &options(false),
                );
                assert!(
                    prepared.plan.errors.is_empty(),
                    "{:?}",
                    texts(&prepared.plan.errors)
                );
                assert_core_echo(&prepared, "0.1.3");
                assert_core_acquisition(&prepared, "0.1.3", prebuilt);
                let replay = plan(&workspace, &prepared.plan.changes, &options(false));
                assert!(
                    replay.plan.errors.is_empty(),
                    "{:?}",
                    texts(&replay.plan.errors)
                );
                assert_core_acquisition(&replay, "0.1.3", prebuilt);
            }
        }
    }

    #[test]
    fn explicit_install_never_resolves_an_unselected_latest_version() {
        for reverse in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut workspace = install_versions_workspace(dir.path(), false);
            let mut index: serde_json::Value = serde_json::from_str(INSTALL_VERSIONS).unwrap();
            index["packages"][0]["versions"][2]["requires"] =
                serde_json::json!([{ "name": "UBotExtra", "version": "^1.0.0" }]);
            index["packages"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "name": "UBotExtra", "repository": "https://example.invalid/uBotExtra.git",
                    "versions": [{ "version": "0.1.0", "ref": "v0.1.0" }]
                }));
            set_install_index(&mut workspace, &index);
            let prepared = plan(
                &workspace,
                &install_pair(Some("0.1.3"), reverse),
                &options(false),
            );
            assert!(
                prepared.plan.errors.is_empty(),
                "{:?}",
                texts(&prepared.plan.errors)
            );
            assert_core_acquisition(&prepared, "0.1.3", false);
            assert!(!prepared.actions.iter().any(|action| matches!(
                action,
                StepAction::Clone { package, .. } | StepAction::Download { package, .. }
                    if package == "UBotExtra"
            )));
        }
    }

    #[test]
    fn implicit_install_echoes_the_final_version() {
        for reverse in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut workspace = install_versions_workspace(dir.path(), false);
            let automatic = plan(
                &workspace,
                &[change(ChangeKind::Install, "UBotROS", None)],
                &options(false),
            );
            assert!(automatic.plan.errors.is_empty());
            assert_core_acquisition(&automatic, "0.1.4", false);
            let mut index: serde_json::Value = serde_json::from_str(INSTALL_VERSIONS).unwrap();
            index["packages"][1]["versions"][0]["requires"][0]["version"] = "=0.1.3".into();
            set_install_index(&mut workspace, &index);
            let prepared = plan(&workspace, &install_pair(None, reverse), &options(false));
            assert!(
                prepared.plan.errors.is_empty(),
                "{:?}",
                texts(&prepared.plan.errors)
            );
            assert_core_acquisition(&prepared, "0.1.3", false);
            assert_core_echo(&prepared, "0.1.3");
            let replay = plan(&workspace, &prepared.plan.changes, &options(false));
            assert!(
                replay.plan.errors.is_empty(),
                "{:?}",
                texts(&replay.plan.errors)
            );
            assert_core_acquisition(&replay, "0.1.3", false);
        }
    }

    #[test]
    fn explicit_install_validates_a_version_after_implicit_install() {
        for reverse in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let workspace = install_versions_workspace(dir.path(), false);
            let prepared = plan(
                &workspace,
                &install_pair(Some("9.9.9"), reverse),
                &options(false),
            );
            assert_eq!(
                prepared.plan.errors,
                vec![msg(
                    "plan.noVersion",
                    &[("package", "UBotCore"), ("range", "9.9.9")]
                )]
            );
            assert_core_echo(&prepared, "9.9.9");
        }
    }

    #[test]
    fn explicit_install_normalizes_names_and_versions() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = install_versions_workspace(dir.path(), false);
        for version in [" v0.1.3 ", "0.1.3+build.42"] {
            let prepared = plan(
                &workspace,
                &[
                    change(ChangeKind::Install, "UBotROS", Some("0.1.3")),
                    change(ChangeKind::Install, " ubotcore ", Some(version)),
                ],
                &options(false),
            );
            assert!(
                prepared.plan.errors.is_empty(),
                "{:?}",
                texts(&prepared.plan.errors)
            );
            assert_core_echo(&prepared, "0.1.3");
            assert_core_acquisition(&prepared, "0.1.3", false);
        }
        for (first, last) in [("0.1.0", "0.1.3"), ("0.1.3", "0.1.0")] {
            let prepared = plan(
                &workspace,
                &[
                    change(ChangeKind::Install, "UBotCore", Some(first)),
                    change(ChangeKind::Install, "UBotROS", Some("0.1.3")),
                    change(ChangeKind::Install, " ubotcore ", Some(last)),
                ],
                &options(false),
            );
            assert_eq!(prepared.plan.changes.len(), 2);
            assert_core_echo(&prepared, last);
            assert_core_acquisition(&prepared, last, false);
            if last == "0.1.0" {
                assert_eq!(prepared.plan.errors, vec![core_ros_conflict()]);
            } else {
                assert!(
                    prepared.plan.errors.is_empty(),
                    "{:?}",
                    texts(&prepared.plan.errors)
                );
            }
        }
        for (name, version, expected) in [
            (
                "UBotCore",
                "0.2.0",
                msg(
                    "plan.noVersion",
                    &[("package", "UBotCore"), ("range", "0.2.0")],
                ),
            ),
            (
                "UBotFuture",
                "1.0.0",
                msg("plan.planned", &[("package", "UBotFuture")]),
            ),
        ] {
            let prepared = plan(
                &workspace,
                &[change(ChangeKind::Install, name, Some(version))],
                &options(false),
            );
            assert_eq!(prepared.plan.errors, vec![expected]);
        }
        let short = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotCore", Some("0.1"))],
            &options(false),
        );
        assert!(short.plan.errors.is_empty());
        assert_core_echo(&short, "0.1.0");
        assert_core_acquisition(&short, "0.1.0", false);
        let unpinned = plan(
            &workspace,
            &[
                change(ChangeKind::Install, "UBotCore", Some("0.1.0")),
                change(ChangeKind::Install, "UBotROS", Some("0.1.3")),
                change(ChangeKind::Install, "UBotCore", None),
            ],
            &options(false),
        );
        assert!(unpinned.plan.errors.is_empty());
        assert_core_echo(&unpinned, "0.1.4");
        assert_core_acquisition(&unpinned, "0.1.4", false);
        let installed_dir = tempfile::tempdir().unwrap();
        let installed = sample_workspace(installed_dir.path(), false);
        let prepared = plan(
            &installed,
            &[change(ChangeKind::Install, "UBotCore", Some("9.9.9"))],
            &options(false),
        );
        assert_eq!(
            prepared.plan.errors,
            vec![msg("plan.alreadyInstalled", &[("package", "UBotCore")])]
        );
    }

    fn add_installed_for_install_test(
        workspace: &mut Workspace,
        name: &str,
        requires: &[(&str, &str)],
    ) {
        let context = workspace.project.as_mut().unwrap();
        write(
            &context
                .project
                .dir
                .join(format!("Plugins/{name}/{name}.uplugin")),
            &descriptor("0.1.0", "Adapter", requires, &[]),
        );
        context.project.reload().unwrap();
        context.git.insert(
            key(name),
            GitInfo {
                is_checkout: true,
                tag: Some("v0.1.0".into()),
                ..Default::default()
            },
        );
    }

    #[test]
    fn explicit_install_pins_are_used_before_update_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = install_versions_workspace(dir.path(), false);
        add_installed_for_install_test(&mut workspace, "UBotROS", &[]);
        let prepared = plan(
            &workspace,
            &[
                change(ChangeKind::Install, "UBotCore", Some("0.1.3")),
                change(ChangeKind::Update, "UBotROS", Some("0.1.3")),
            ],
            &options(false),
        );
        assert!(
            prepared.plan.errors.is_empty(),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        assert_core_acquisition(&prepared, "0.1.3", false);
        assert_core_echo(&prepared, "0.1.3");
    }

    #[test]
    fn explicit_install_conflicts_use_final_requirements() {
        for disable in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut workspace = install_versions_workspace(dir.path(), false);
            add_installed_for_install_test(&mut workspace, "UBotROS", &[]);
            add_installed_for_install_test(&mut workspace, "UBotA", &[("UBotCore", ">=0.1.4")]);
            let mut index: serde_json::Value = serde_json::from_str(INSTALL_VERSIONS).unwrap();
            index["packages"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "name": "UBotA", "repository": "https://example.invalid/uBotA.git",
                    "versions": [{ "version": "0.1.3", "ref": "v0.1.3",
                        "requires": [{ "name": "UBotCore", "version": "^0.1.3" }] }]
                }));
            set_install_index(&mut workspace, &index);
            let prepared = plan(
                &workspace,
                &[
                    change(ChangeKind::Update, "UBotROS", Some("0.1.3")),
                    change(
                        if disable {
                            ChangeKind::Disable
                        } else {
                            ChangeKind::Update
                        },
                        "UBotA",
                        Some("0.1.3"),
                    ),
                    change(ChangeKind::Install, "UBotCore", Some("0.1.3")),
                ],
                &options(false),
            );
            assert!(
                prepared.plan.errors.is_empty(),
                "{:?}",
                texts(&prepared.plan.errors)
            );
            assert_core_acquisition(&prepared, "0.1.3", false);
        }
    }

    #[test]
    fn explicit_install_defers_requirements_of_an_owner_updated_later() {
        for temporary_range in [">=0.1.4", ">=9.0.0"] {
            let dir = tempfile::tempdir().unwrap();
            let mut workspace = install_versions_workspace(dir.path(), false);
            add_installed_for_install_test(&mut workspace, "UBotROS", &[]);
            add_installed_for_install_test(
                &mut workspace,
                "UBotA",
                &[("UBotCore", temporary_range)],
            );
            let context = workspace.project.as_mut().unwrap();
            write(
                &context.project.path,
                r#"{ "FileVersion": 3, "EngineAssociation": "5.5",
                    "Plugins": [{ "Name": "UBotA", "Enabled": false }] }"#,
            );
            context.project.reload().unwrap();
            let mut index: serde_json::Value = serde_json::from_str(INSTALL_VERSIONS).unwrap();
            index["packages"][1]["versions"][0]["requires"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "name": "UBotA", "version": "^0.1.0"
                }));
            index["packages"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "name": "UBotA", "repository": "https://example.invalid/uBotA.git",
                    "versions": [{ "version": "0.1.3", "ref": "v0.1.3",
                        "requires": [{ "name": "UBotCore", "version": "^0.1.3" }] }]
                }));
            set_install_index(&mut workspace, &index);
            // Updating ROS enables A at its old version, before the explicit update of A.
            // Neither an available alternative Core nor a nonexistent one may poison the plan.
            let prepared = plan(
                &workspace,
                &[
                    change(ChangeKind::Update, "UBotROS", Some("0.1.3")),
                    change(ChangeKind::Update, "UBotA", Some("0.1.3")),
                    change(ChangeKind::Install, "UBotCore", Some("0.1.3")),
                ],
                &options(false),
            );
            assert!(
                prepared.plan.errors.is_empty(),
                "{:?}",
                texts(&prepared.plan.errors)
            );
            assert_core_acquisition(&prepared, "0.1.3", false);
        }
    }

    #[test]
    fn explicit_install_mismatch_has_a_fallback_diagnostic() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = install_versions_workspace(dir.path(), false);
        let mut planner = Planner::new(&workspace);
        planner.expect_install("UBotCore", "v0.1.3");
        // An install that failed before acquiring a version has no resolved value to report.
        assert!(planner.requested_version_conflicts().is_empty());
        let core = planner.package("UBotCore");
        core.installed = true;
        core.enabled = true;
        core.version = Some("0.1.4".into());
        let ros = planner.package("UBotROS");
        ros.installed = true;
        ros.enabled = true;
        ros.version = Some("0.1.3".into());
        assert_eq!(
            planner.requested_version_conflicts(),
            vec![msg(
                "plan.requestedVersionChanged",
                &[
                    ("package", "UBotCore"),
                    ("requested", "0.1.3"),
                    ("resolved", "0.1.4"),
                ],
            )]
        );
        // A dependent rejecting either the requested or resolved version explains the mismatch.
        for range in [">=0.1.4", "=0.1.3"] {
            planner.package("UBotROS").requires[0].version = range.into();
            assert_eq!(
                planner.requested_version_conflicts(),
                vec![msg(
                    "plan.breaksDependent",
                    &[
                        ("package", "UBotCore"),
                        ("dependent", "UBotROS"),
                        ("range", range),
                    ],
                )]
            );
        }
    }

    #[test]
    fn installs_with_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = sample_workspace(dir.path(), false);
        // UBotSensor (disabled) satisfies ^0.1.0 and is enabled; UBotCore 0.1.0 is enabled.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "ubotros", None)],
            &ApplyOptions {
                build: true,
                reopen_editor: true,
                run_tests: true,
            },
        );
        let result = &prepared.plan;
        assert!(result.errors.is_empty(), "{:?}", texts(&result.errors));
        assert_eq!(
            titles(result),
            [
                "Clone UBotROS 0.1.0",
                "Enable UBotSensor",
                "Enable UBotROS",
                "Build UnrealEditor",
                "Run package tests",
                "Reopen Unreal Editor"
            ]
        );
        assert_eq!(
            result.changes,
            [change(ChangeKind::Install, "UBotROS", Some("0.1.0"))]
        );
        assert!(result.needs_build);
        let steps = &result.steps;
        assert_eq!(steps[0].group, StepGroup::Download);
        assert_eq!(steps[3].group, StepGroup::Build);
        assert_eq!(steps[5].group, StepGroup::After);
        assert!(!steps[0].needs_editor_closed);
        assert!(steps[1..].iter().all(|step| step.needs_editor_closed));
        assert_eq!(
            steps.iter().map(|s| s.n).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        assert_eq!(steps[0].id, "s1");
        assert_eq!(
            render_en(&steps[0].detail),
            "github.com/AstreoX/uBotROS → Plugins/uBot/uBotROS"
        );
        assert_eq!(render_en(&steps[1].detail), "required by UBotROS");
        assert_eq!(render_en(&steps[2].detail), "MyRobotSim.uproject");
        assert_eq!(render_en(&steps[3].detail), "Win64 · Development");
        assert_eq!(render_en(&steps[4].detail), "UBotROS+UBotSensor");
        match &prepared.actions[0] {
            StepAction::Clone {
                url, git_ref, dest, ..
            } => {
                assert_eq!(url, "https://github.com/AstreoX/uBotROS.git");
                assert_eq!(git_ref, "v0.1.0");
                assert!(dest.ends_with("Plugins/uBot/uBotROS"));
            }
            other => panic!("unexpected {other:?}"),
        }

        // The editor is running: an openEditor step follows.
        workspace.project.as_mut().unwrap().editor.running = true;
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &ApplyOptions {
                build: false,
                reopen_editor: true,
                run_tests: false,
            },
        );
        assert_eq!(
            titles(&prepared.plan),
            [
                "Clone UBotROS 0.1.0",
                "Enable UBotSensor",
                "Enable UBotROS",
                "Reopen Unreal Editor"
            ]
        );
        assert!(prepared.plan.editor_running);
    }

    #[test]
    fn install_errors() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = sample_workspace(dir.path(), true);
        let errors =
            |changes: &[Change]| texts(&plan(&workspace, changes, &options(false)).plan.errors);
        assert_eq!(
            errors(&[change(ChangeKind::Install, "Nope", None)]),
            ["Nope is not in any package index."]
        );
        assert_eq!(
            errors(&[change(ChangeKind::Install, "UBotVehicles", None)]),
            ["UBotVehicles is planned and cannot be installed yet."]
        );
        assert_eq!(
            errors(&[change(ChangeKind::Install, "UBotCore", None)]),
            ["UBotCore is already installed."]
        );
        assert_eq!(
            errors(&[change(ChangeKind::Install, "UBotROS", Some("9.0.0"))]),
            ["No version of UBotROS matches 9.0.0."]
        );

        std::fs::create_dir_all(dir.path().join("MyRobotSim/Plugins/uBot/uBotROS")).unwrap();
        assert_eq!(
            errors(&[change(ChangeKind::Install, "UBotROS", None)]),
            ["Plugins/uBot/uBotROS already exists."]
        );

        let mut workspace = sample_workspace(&dir.path().join("x"), false);
        workspace.project.as_mut().unwrap().engine = None;
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Enable, "UBotCore", None)],
            &options(true),
        );
        assert!(prepared.plan.errors.is_empty(), "nothing to build");
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Disable, "UBotEnvironment", None)],
            &ApplyOptions {
                build: true,
                reopen_editor: false,
                run_tests: true,
            },
        );
        assert!(prepared.plan.errors.is_empty(), "disabling needs no build");
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Enable, "UBotSensor", None)],
            &options(true),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["No Unreal Engine was found for this project."]
        );
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Enable, "UBotSensor", None)],
            &options(false),
        );
        assert!(prepared.plan.errors.is_empty());
        let none = Workspace::default();
        assert_eq!(
            texts(
                &plan(
                    &none,
                    &[change(ChangeKind::Enable, "A", None)],
                    &options(false)
                )
                .plan
                .errors
            ),
            ["No project is open."]
        );
    }

    #[test]
    fn update_checks_git_and_dependents() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = sample_workspace(dir.path(), true);
        // UBotSensor (enabled) requires UBotCore ^0.1.0, so 0.2.0 breaks it.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotCore", Some("0.2.0"))],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["UBotSensor requires UBotCore ^0.1.0."]
        );

        // With UBotSensor disabled in the same plan the update goes through.
        let prepared = plan(
            &workspace,
            &[
                change(ChangeKind::Update, "UBotCore", Some("0.2.0")),
                change(ChangeKind::Disable, "UBotSensor", None),
            ],
            &options(false),
        );
        assert!(
            prepared.plan.errors.is_empty(),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        assert_eq!(
            titles(&prepared.plan),
            ["Check out UBotCore 0.2.0", "Disable UBotSensor"]
        );
        assert_eq!(render_en(&prepared.plan.steps[0].detail), "0.1.0 → 0.2.0");
        assert!(prepared.plan.steps[0].needs_editor_closed);
        assert!(
            matches!(&prepared.actions[0], StepAction::Checkout { git_ref, .. } if git_ref == "v0.2.0")
        );

        let context = workspace.project.as_mut().unwrap();
        context.git.insert(
            "ubotcore".into(),
            GitInfo {
                is_checkout: true,
                branch: Some("main".into()),
                dirty: true,
                ..Default::default()
            },
        );
        let prepared = plan(
            &workspace,
            &[
                change(ChangeKind::Update, "UBotCore", Some("0.2.0")),
                change(ChangeKind::Disable, "UBotSensor", None),
            ],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["UBotCore has local changes."]
        );
        assert_eq!(
            texts(&prepared.plan.warnings)[0],
            "UBotCore is on branch main; it will be detached at the new version."
        );
        workspace.project.as_mut().unwrap().git.remove("ubotcore");
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotCore", Some("0.2.0"))],
            &options(false),
        );
        assert!(
            texts(&prepared.plan.errors).contains(
                &"No prebuilt UBotCore 0.2.0 for UE 5.5; uninstall it and install it again to build from source."
                    .to_string()
            ),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        // With the setting off the answer is the same: uninstall and install again.
        workspace.settings.install.use_prebuilt = false;
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotCore", Some("0.2.0"))],
            &options(false),
        );
        assert!(
            texts(&prepared.plan.errors).contains(
                &"No prebuilt UBotCore 0.2.0 for UE 5.5; uninstall it and install it again to build from source."
                    .to_string()
            ),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotROS", Some("0.1.0"))],
            &options(false),
        );
        assert_eq!(texts(&prepared.plan.errors), ["UBotROS is not installed."]);
    }

    #[test]
    fn an_update_that_a_later_change_turns_back_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // UBotCore 0.1.0 and UBotEnvironment (needs ^0.2.0) are enabled, UBotSensor (needs
        // ^0.1.0) is installed but disabled; the index has UBotCore 0.2.0.
        let workspace = sample_workspace(dir.path(), false);
        let errors = |changes: &[Change]| {
            let prepared = plan(&workspace, changes, &options(false));
            (texts(&prepared.plan.errors), titles(&prepared.plan))
        };
        let update = change(ChangeKind::Update, "UBotCore", Some("0.2.0"));
        let enable = change(ChangeKind::Enable, "UBotSensor", None);

        // Each change alone is fine.
        let (alone, steps) = errors(std::slice::from_ref(&update));
        assert!(alone.is_empty(), "{alone:?}");
        assert_eq!(steps, ["Check out UBotCore 0.2.0"]);
        let (alone, _) = errors(std::slice::from_ref(&enable));
        assert!(alone.is_empty(), "{alone:?}");

        // Together they contradict each other: enabling UBotSensor needs UBotCore 0.1.0, the
        // version it already has, so no check out is left and the request would be dropped
        // silently without the error. Either order of the changes says the same.
        for changes in [
            [update.clone(), enable.clone()],
            [enable.clone(), update.clone()],
        ] {
            let (together, steps) = errors(&changes);
            assert_eq!(together, ["UBotSensor requires UBotCore ^0.1.0."]);
            assert_eq!(steps, ["Enable UBotSensor"], "no check out back to 0.1.0");
        }

        // A dependent that is switched off in the same plan does not turn anything back.
        let (disabled, steps) = errors(&[
            update.clone(),
            change(ChangeKind::Disable, "UBotSensor", None),
        ]);
        assert!(disabled.is_empty(), "{disabled:?}");
        assert_eq!(steps, ["Check out UBotCore 0.2.0"]);
    }

    #[test]
    fn enable_resolves_versions_and_engine_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = sample_workspace(dir.path(), false);
        // UBotEnvironment needs UBotCore ^0.2.0 (0.1.0 installed, no other enabled dependent) and Niagara.
        let root = dir.path().join("MyRobotSim");
        crate::uproject::edit(&root.join("MyRobotSim.uproject"), |project| {
            project.set_enabled("UBotEnvironment", false)
        })
        .unwrap();
        let mut workspace = workspace;
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Enable, "UBotEnvironment", None)],
            &options(true),
        );
        assert!(
            prepared.plan.errors.is_empty(),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        assert_eq!(
            titles(&prepared.plan),
            [
                "Check out UBotCore 0.2.0",
                "Enable Niagara",
                "Enable UBotEnvironment",
                "Build UnrealEditor"
            ]
        );
        assert_eq!(
            render_en(&prepared.plan.steps[1].detail),
            "required by UBotEnvironment"
        );
        assert_eq!(
            prepared.actions[1],
            StepAction::SetEnabled {
                plugin: "Niagara".into(),
                enabled: true
            }
        );

        // An engine plugin the engine does not have.
        write(
            &root.join("Plugins/uBot/uBotPaper/UBotPaper.uplugin"),
            &descriptor("1.0.0", "Content", &[], &["Paper2D"]),
        );
        crate::uproject::edit(&root.join("MyRobotSim.uproject"), |project| {
            project.set_enabled("UBotPaper", false)
        })
        .unwrap();
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Enable, "UBotPaper", None)],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["The engine has no plugin named Paper2D."]
        );

        // Enabling an engine plugin directly, and something unknown.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Enable, "chaosvehiclesplugin", None)],
            &options(false),
        );
        assert_eq!(titles(&prepared.plan), ["Enable ChaosVehiclesPlugin"]);
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Enable, "UBotROS", None)],
            &options(false),
        );
        assert_eq!(texts(&prepared.plan.errors), ["UBotROS is not installed."]);
    }

    #[test]
    fn disable_and_uninstall_take_dependents_along() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("MyRobotSim");
        let mut workspace = sample_workspace(dir.path(), true);
        write(
            &root.join("Plugins/uBot/uBotROS/UBotROS.uplugin"),
            &descriptor(
                "0.1.0",
                "Adapter",
                &[("UBotCore", "^0.1.0"), ("UBotSensor", "^0.1.0")],
                &[],
            ),
        );
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();

        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Disable, "UBotSensor", None)],
            &options(true),
        );
        assert!(prepared.plan.errors.is_empty());
        assert_eq!(
            titles(&prepared.plan),
            ["Disable UBotROS", "Disable UBotSensor"]
        );
        assert_eq!(
            texts(&prepared.plan.warnings),
            ["Disabling UBotSensor also disables UBotROS."]
        );
        assert!(!prepared.plan.needs_build, "disabling needs no build");

        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Disable, "UBotCore", None)],
            &options(false),
        );
        assert_eq!(
            titles(&prepared.plan),
            [
                "Disable UBotROS",
                "Disable UBotSensor",
                "Disable UBotEnvironment",
                "Disable UBotCore"
            ]
        );
        assert_eq!(
            texts(&prepared.plan.warnings),
            ["Disabling UBotCore also disables UBotROS, UBotSensor, UBotEnvironment."]
        );

        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Uninstall, "UBotSensor", None)],
            &options(true),
        );
        assert_eq!(
            titles(&prepared.plan),
            ["Disable UBotROS", "Remove UBotSensor"]
        );
        assert_eq!(
            render_en(&prepared.plan.steps[1].detail),
            "Plugins/uBot/uBotSensor → Recycle Bin"
        );

        // Installing something that needs a package being disabled is refused.
        let prepared = plan(
            &workspace,
            &[
                change(ChangeKind::Disable, "UBotCore", None),
                change(ChangeKind::Enable, "UBotSensor", None),
            ],
            &options(false),
        );
        assert!(texts(&prepared.plan.errors)
            .contains(&"UBotSensor requires UBotCore ^0.1.0.".to_string()));

        // Legacy plugins are only disabled in the project file.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Disable, "AgentSensorCore", None)],
            &options(true),
        );
        assert_eq!(titles(&prepared.plan), ["Disable AgentSensorCore"]);
        assert!(
            matches!(&prepared.actions[0], StepAction::SetEnabled { plugin, enabled: false } if plugin == "AgentSensorCore")
        );
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Uninstall, "UBotROS2", None)],
            &options(true),
        );
        assert_eq!(texts(&prepared.plan.errors), ["UBotROS2 is not installed."]);
    }

    #[test]
    fn uninstall_refuses_folders_outside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("Host");
        write(
            &project_dir.join("Host.uproject"),
            "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"5.5\",\n    \"AdditionalPluginDirectories\": [ \"../Shared\" ]\n}\n",
        );
        write(
            &dir.path().join("Shared/uBotCore/UBotCore.uplugin"),
            &descriptor("0.1.0", "Foundation", &[], &[]),
        );
        let mut workspace = sample_workspace(&dir.path().join("unused"), true);
        workspace.project.as_mut().unwrap().project =
            crate::project::Project::open(&project_dir.join("Host.uproject")).unwrap();
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Uninstall, "UBotCore", None)],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["../Shared/uBotCore is outside the project."]
        );
    }

    #[test]
    fn normalizes_changes() {
        let changes = normalize_changes(&[
            change(ChangeKind::Install, " A ", Some(" ")),
            change(ChangeKind::Enable, "B", None),
            change(ChangeKind::Disable, "a", None),
            change(ChangeKind::Enable, "", None),
        ]);
        assert_eq!(
            changes,
            [
                change(ChangeKind::Enable, "B", None),
                change(ChangeKind::Disable, "a", None)
            ]
        );
    }

    const COORDINATED_INDEX: &str = r#"{ "formatVersion": 2, "name": "t", "packages": [
        { "name": "UBotCore", "layer": "Foundation", "description": { "en": "c" },
          "repository": "https://github.com/AstreoX/uBotCore.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0" }, { "version": "0.2.0", "ref": "v0.2.0" } ] },
        { "name": "UBotSensor", "layer": "Capability", "description": { "en": "s" },
          "repository": "https://github.com/AstreoX/uBotSensor.git",
          "versions": [
            { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ] },
            { "version": "0.2.0", "ref": "v0.2.0", "requires": [ { "name": "UBotCore", "version": "^0.2.0" } ] } ] },
        { "name": "UBotROS", "layer": "Adapter", "description": { "en": "r" },
          "repository": "https://github.com/AstreoX/uBotROS.git",
          "versions": [
            { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" }, { "name": "UBotSensor", "version": "^0.1.0" } ] },
            { "version": "0.2.0", "ref": "v0.2.0", "requires": @ROS@ } ] },
        { "name": "UBotBaz", "layer": "Content", "description": { "en": "b" },
          "repository": "https://github.com/AstreoX/uBotBaz.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": @BAZ@ } ] }
    ] }"#;

    const CORE_THEN_SENSOR: &str = r#"[ { "name": "UBotCore", "version": "^0.2.0" }, { "name": "UBotSensor", "version": "^0.2.0" } ]"#;
    const SENSOR_THEN_CORE: &str = r#"[ { "name": "UBotSensor", "version": "^0.2.0" }, { "name": "UBotCore", "version": "^0.2.0" } ]"#;

    /// UBotCore and UBotSensor (and, when asked, UBotROS) at 0.1.0, clean tag checkouts, all
    /// enabled; the index has 0.2.0 of each, and UBotBaz.
    fn coordinated(dir: &Path, ros_installed: bool, ros_requires: &str, baz: &str) -> Workspace {
        let mut workspace = sample_workspace(dir, true);
        if ros_installed {
            write(
                &dir.join("MyRobotSim/Plugins/uBot/uBotROS/UBotROS.uplugin"),
                &descriptor(
                    "0.1.0",
                    "Adapter",
                    &[("UBotCore", "^0.1.0"), ("UBotSensor", "^0.1.0")],
                    &[],
                ),
            );
        }
        let context = workspace.project.as_mut().unwrap();
        context.project.reload().unwrap();
        for name in ["ubotcore", "ubotsensor", "ubotros"] {
            context.git.insert(
                name.into(),
                GitInfo {
                    is_checkout: true,
                    tag: Some("v0.1.0".into()),
                    ..Default::default()
                },
            );
        }
        let text = COORDINATED_INDEX
            .replace("@ROS@", ros_requires)
            .replace("@BAZ@", baz);
        workspace.index = crate::index::MergedIndex::merge(&[crate::index::SourceIndex {
            id: "t".into(),
            index: crate::index::parse(&text).unwrap(),
        }]);
        workspace
    }

    const PERMUTATIONS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];

    #[test]
    fn coordinated_updates_do_not_depend_on_the_order() {
        let all = ["UBotCore", "UBotSensor", "UBotROS"];
        for ros_requires in [CORE_THEN_SENSOR, SENSOR_THEN_CORE] {
            let dir = tempfile::tempdir().unwrap();
            let workspace = coordinated(dir.path(), true, ros_requires, "[]");

            // All three at once, in every order of the changes.
            for order in PERMUTATIONS {
                let changes: Vec<Change> = order
                    .iter()
                    .map(|&index| change(ChangeKind::Update, all[index], Some("0.2.0")))
                    .collect();
                let prepared = plan(&workspace, &changes, &options(true));
                assert!(
                    prepared.plan.errors.is_empty(),
                    "{order:?} / {ros_requires}: {:?}",
                    texts(&prepared.plan.errors)
                );
                assert_eq!(
                    titles(&prepared.plan),
                    [
                        "Check out UBotCore 0.2.0",
                        "Check out UBotSensor 0.2.0",
                        "Check out UBotROS 0.2.0",
                        "Build UnrealEditor"
                    ],
                    "{order:?}"
                );
            }

            // UBotROS alone pulls its dependencies along, whatever the order of its `requires`.
            let prepared = plan(
                &workspace,
                &[change(ChangeKind::Update, "UBotROS", Some("0.2.0"))],
                &options(false),
            );
            assert!(
                prepared.plan.errors.is_empty(),
                "{ros_requires}: {:?}",
                texts(&prepared.plan.errors)
            );
            assert_eq!(
                titles(&prepared.plan),
                [
                    "Check out UBotCore 0.2.0",
                    "Check out UBotSensor 0.2.0",
                    "Check out UBotROS 0.2.0"
                ]
            );
        }
    }

    #[test]
    fn installs_pull_coordinated_dependencies_in_any_order() {
        let mut results = Vec::new();
        for baz in [CORE_THEN_SENSOR, SENSOR_THEN_CORE] {
            let dir = tempfile::tempdir().unwrap();
            let workspace = coordinated(dir.path(), false, "[]", baz);
            let prepared = plan(
                &workspace,
                &[change(ChangeKind::Install, "UBotBaz", None)],
                &options(false),
            );
            assert!(
                prepared.plan.errors.is_empty(),
                "{baz}: {:?}",
                texts(&prepared.plan.errors)
            );
            results.push(titles(&prepared.plan));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(
            results[0],
            [
                "Check out UBotCore 0.2.0",
                "Check out UBotSensor 0.2.0",
                "Clone UBotBaz 0.1.0",
                "Enable UBotBaz"
            ]
        );
    }

    #[test]
    fn a_dependency_held_back_by_a_dependent_is_a_conflict_not_a_missing_version() {
        // UBotBaz needs Core ^0.2.0; the enabled UBotSensor, which is not part of the plan, wants
        // ^0.1.0. Version 0.2.0 exists, so the message names the dependent.
        let dir = tempfile::tempdir().unwrap();
        let baz = r#"[ { "name": "UBotCore", "version": "^0.2.0" } ]"#;
        let workspace = coordinated(dir.path(), false, "[]", baz);
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotBaz", None)],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["UBotSensor requires UBotCore ^0.1.0."]
        );

        // Updating Sensor in the same plan resolves it, in either order of the changes.
        for changes in [
            [
                change(ChangeKind::Install, "UBotBaz", None),
                change(ChangeKind::Update, "UBotSensor", Some("0.2.0")),
            ],
            [
                change(ChangeKind::Update, "UBotSensor", Some("0.2.0")),
                change(ChangeKind::Install, "UBotBaz", None),
            ],
        ] {
            let prepared = plan(&workspace, &changes, &options(false));
            assert!(
                prepared.plan.errors.is_empty(),
                "{:?}",
                texts(&prepared.plan.errors)
            );
        }

        // A version that does not exist at all is still "no version".
        let missing = r#"[ { "name": "UBotCore", "version": "^9.0.0" } ]"#;
        let workspace = coordinated(dir.path(), false, "[]", missing);
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotBaz", None)],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["No version of UBotCore matches ^9.0.0."]
        );
    }

    /// A project `Host` with `../Shared` as an additional plugin directory and, when asked, a
    /// descriptor lying directly in `Plugins`.
    fn host_workspace(dir: &Path, loose_descriptor: bool) -> Workspace {
        let mut workspace = sample_workspace(&dir.join("unused"), true);
        let project_dir = dir.join("Host");
        write(
            &project_dir.join("Host.uproject"),
            "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"5.5\",\n    \"AdditionalPluginDirectories\": [ \"../Shared\" ]\n}\n",
        );
        if loose_descriptor {
            write(
                &project_dir.join("Plugins/UBotLoose.uplugin"),
                &descriptor("0.1.0", "Content", &[], &[]),
            );
        }
        workspace.project.as_mut().unwrap().project =
            crate::project::Project::open(&project_dir.join("Host.uproject")).unwrap();
        workspace
    }

    #[test]
    fn uninstall_never_removes_a_plugin_root() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = host_workspace(dir.path(), true);
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Uninstall, "UBotLoose", None)],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["Plugins is a plugin folder that holds more than this package."]
        );
        assert!(prepared.actions.is_empty(), "{:?}", prepared.actions);
        // Disabling it is fine: only the .uproject changes.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Disable, "UBotLoose", None)],
            &options(false),
        );
        assert!(prepared.plan.errors.is_empty());
    }

    #[test]
    fn the_install_folder_must_be_where_plugins_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = host_workspace(dir.path(), false);
        let outcome = |workspace: &Workspace| {
            let prepared = plan(
                workspace,
                &[change(ChangeKind::Install, "UBotSensor", None)],
                &options(false),
            );
            let cloned = prepared
                .actions
                .iter()
                .any(|action| matches!(action, StepAction::Clone { .. }));
            (texts(&prepared.plan.errors), cloned)
        };
        let not_found =
            "is not inside the project's Plugins folder or one of its AdditionalPluginDirectories.";
        for (directory, error) in [
            ("Content/Stuff", Some(format!("Content/Stuff {not_found}"))),
            (".", Some(format!(". {not_found}"))),
            (
                "../Elsewhere",
                Some("../Elsewhere is outside the project.".to_string()),
            ),
            ("Plugins/uBot", None),
            ("Plugins", None),
            ("../Shared", None),
            ("../Shared/uBot", None),
        ] {
            workspace.settings.install.directory = directory.into();
            let (errors, cloned) = outcome(&workspace);
            match error {
                Some(error) => {
                    assert_eq!(errors, [error], "{directory}");
                    assert!(!cloned, "{directory}");
                }
                None => {
                    assert!(errors.is_empty(), "{directory}: {errors:?}");
                    assert!(cloned, "{directory}");
                }
            }
        }
    }

    const HOSTILE_INDEX: &str = r#"{ "formatVersion": 2, "packages": [
        { "name": "UBotCore", "repository": "https://github.com/AstreoX/uBotCore.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0" },
                        { "version": "0.2.0", "ref": "--pathspec-from-file=C:/nonexistent.txt" } ] },
        { "name": "Bad;Name", "repository": "https://github.com/AstreoX/uBotBad.git",
          "versions": [ { "version": "0.1.0", "ref": "v0.1.0" } ] },
        { "name": "UBotBadRef", "repository": "https://github.com/AstreoX/uBotBadRef.git",
          "versions": [ { "version": "0.1.0", "ref": "-b" } ] }
    ] }"#;

    #[test]
    fn refs_and_names_from_an_index_are_checked_before_they_reach_git_or_the_editor() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = sample_workspace(dir.path(), false);
        workspace.index = crate::index::MergedIndex::merge(&[crate::index::SourceIndex {
            id: "t".into(),
            index: crate::index::parse(HOSTILE_INDEX).unwrap(),
        }]);

        let errors_of = |change: Change| {
            let prepared = plan(&workspace, &[change], &options(false));
            assert!(
                prepared.actions.is_empty(),
                "nothing is planned: {:?}",
                prepared.actions
            );
            texts(&prepared.plan.errors)
        };
        assert_eq!(
            errors_of(change(ChangeKind::Update, "UBotCore", Some("0.2.0"))),
            ["Refusing git ref --pathspec-from-file=C:/nonexistent.txt of UBotCore."]
        );
        assert_eq!(
            errors_of(change(ChangeKind::Install, "UBotBadRef", None)),
            ["Refusing git ref -b of UBotBadRef."]
        );
        assert_eq!(
            errors_of(change(ChangeKind::Install, "Bad;Name", None)),
            ["Refusing package name Bad;Name."]
        );
    }

    #[test]
    fn an_unreadable_checkout_blocks_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = sample_workspace(dir.path(), false);
        workspace.project.as_mut().unwrap().git.insert(
            "ubotcore".into(),
            GitInfo {
                is_checkout: true,
                error: Some("detected dubious ownership".into()),
                ..Default::default()
            },
        );
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotCore", Some("0.2.0"))],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["Cannot read the git state of UBotCore: detected dubious ownership"]
        );
        assert!(prepared.actions.is_empty());
    }

    /// A project named `name` in `folder` with UBotCore installed, so that installing UBotSensor
    /// clones, enables and builds.
    fn named_project(dir: &Path, folder: &str, name: &str, code: bool) -> Workspace {
        let mut workspace = sample_workspace(&dir.join("unused"), true);
        let project_dir = dir.join(folder);
        write(
            &project_dir.join(format!("{name}.uproject")),
            "{\n    \"FileVersion\": 3,\n    \"EngineAssociation\": \"5.5\"\n}\n",
        );
        if code {
            write(
                &project_dir.join(format!("Source/{name}.Target.cs")),
                "// target",
            );
        }
        write(
            &project_dir.join("Plugins/uBot/uBotCore/UBotCore.uplugin"),
            &descriptor("0.1.0", "Foundation", &[], &[]),
        );
        workspace.project.as_mut().unwrap().project =
            crate::project::Project::open(&project_dir.join(format!("{name}.uproject"))).unwrap();
        workspace
    }

    #[test]
    fn the_build_command_is_only_planned_for_names_cmd_reads_literally() {
        let dir = tempfile::tempdir().unwrap();
        let install = [change(ChangeKind::Install, "UBotSensor", None)];
        let outcome = |workspace: &Workspace| {
            let prepared = plan(workspace, &install, &options(true));
            let builds: Vec<String> = titles(&prepared.plan)
                .into_iter()
                .filter(|title| title.starts_with("Build"))
                .collect();
            (texts(&prepared.plan.errors), builds)
        };

        // A content-only project is built through the stock editor target: its name, which may
        // hold anything a file name can, never reaches the command line unquoted.
        let (errors, builds) = outcome(&named_project(
            dir.path(),
            "p1",
            "Evil&mkdir PWNED&x",
            false,
        ));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(builds, ["Build UnrealEditor"]);

        // A code project builds <Name>Editor, and a name that cmd.exe would parse is refused.
        let (errors, builds) = outcome(&named_project(dir.path(), "p2", "Fine", true));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(builds, ["Build FineEditor"]);
        for name in ["Evil&mkdir PWNED&x", "My Game", "x^y", "a(b)"] {
            let (errors, _) = outcome(&named_project(dir.path(), "p3", name, true));
            assert_eq!(
                errors,
                [format!("The project name {name} cannot be built; use only letters, digits and underscores in the .uproject file name.")],
                "{name}"
            );
        }

        // cmd.exe expands %NAME% even inside quotes.
        let (errors, _) = outcome(&named_project(dir.path(), "a%TEMP%b", "Fine", false));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("contains a % character"), "{errors:?}");

        // Without a build the name does not matter.
        let workspace = named_project(dir.path(), "p4", "Evil&mkdir PWNED&x", true);
        let prepared = plan(&workspace, &install, &options(false));
        assert!(prepared.plan.errors.is_empty());
    }

    use crate::workspace::tests::{mark_prebuilt, prebuilt_workspace, BINARY_SHA};

    fn kinds(plan: &Plan) -> Vec<StepKind> {
        plan.steps.iter().map(|step| step.kind).collect()
    }

    #[test]
    fn installs_prebuilt_binaries_without_a_build() {
        let dir = tempfile::tempdir().unwrap();
        // UBotROS 0.1.0 has binaries for UE 5.5; its dependencies are installed (UBotSensor
        // disabled) and have no binaries on disk, but they need no build unless they are
        // enabled by the plan... which UBotSensor is.
        let workspace = prebuilt_workspace(dir.path(), true);
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &ApplyOptions {
                build: true,
                reopen_editor: true,
                run_tests: true,
            },
        );
        let result = &prepared.plan;
        assert!(result.errors.is_empty(), "{:?}", texts(&result.errors));
        // UBotCore and UBotSensor are source installs without editor binaries: a build.
        assert_eq!(
            titles(result),
            [
                "Download UBotROS 0.1.0",
                "Enable UBotROS",
                "Build UnrealEditor",
                "Run package tests",
                "Reopen Unreal Editor"
            ]
        );
        assert_eq!(result.steps[0].group, StepGroup::Download);
        assert_eq!(result.steps[0].kind, StepKind::Download);
        assert!(
            !result.steps[0].needs_editor_closed,
            "a new folder does not disturb the editor"
        );
        assert_eq!(
            render_en(&result.steps[0].detail),
            "example.com/UBotROS-0.1.0-UE5.5-Win64.zip → Plugins/uBot/uBotROS"
        );
        assert_eq!(
            result.changes,
            [change(ChangeKind::Install, "UBotROS", Some("0.1.0"))]
        );
        assert!(result.warnings.is_empty(), "{:?}", texts(&result.warnings));
        match &prepared.actions[0] {
            StepAction::Download {
                package,
                version,
                url,
                sha256,
                size,
                engine,
                dest,
                replace,
            } => {
                assert_eq!(package, "UBotROS");
                assert_eq!(version, "0.1.0");
                assert_eq!(url, "https://example.com/UBotROS-0.1.0-UE5.5-Win64.zip");
                assert_eq!(sha256, BINARY_SHA);
                assert_eq!(*size, Some(1234));
                assert_eq!(engine, "5.5");
                assert!(dest.ends_with("Plugins/uBot/uBotROS"));
                assert!(!replace);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(render_en(&result.steps[3].detail), "UBotROS");

        // Once the source packages have their binaries, nothing needs a build.
        for folder in ["uBotCore", "uBotSensor", "uBotEnvironment"] {
            let win64 = dir
                .path()
                .join("MyRobotSim/Plugins/uBot")
                .join(folder)
                .join("Binaries/Win64");
            std::fs::create_dir_all(&win64).unwrap();
            std::fs::write(win64.join(format!("UnrealEditor-{folder}.dll")), "dll").unwrap();
        }
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &ApplyOptions {
                build: true,
                reopen_editor: true,
                run_tests: false,
            },
        );
        assert_eq!(
            titles(&prepared.plan),
            ["Download UBotROS 0.1.0", "Enable UBotROS"],
            "no build step, so no reopen either"
        );
        assert!(!prepared.plan.needs_build);
        // The editor is running: it is reopened, and the download still needs no build.
        let mut running = workspace.clone();
        running.project.as_mut().unwrap().editor.running = true;
        let prepared = plan(
            &running,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &ApplyOptions {
                build: true,
                reopen_editor: true,
                run_tests: true,
            },
        );
        assert_eq!(
            kinds(&prepared.plan),
            [
                StepKind::Download,
                StepKind::EnablePackage,
                StepKind::RunTests,
                StepKind::OpenEditor
            ]
        );
    }

    #[test]
    fn a_mixed_plan_builds_the_packages_that_come_from_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = prebuilt_workspace(dir.path(), true);
        for folder in ["uBotCore", "uBotSensor", "uBotEnvironment"] {
            let win64 = dir
                .path()
                .join("MyRobotSim/Plugins/uBot")
                .join(folder)
                .join("Binaries/Win64");
            std::fs::create_dir_all(&win64).unwrap();
            std::fs::write(win64.join("UnrealEditor-X.dll"), "dll").unwrap();
        }
        // UBotROS has binaries, UBotSensor 0.1.0 (here: a fresh install) has none.
        std::fs::remove_dir_all(dir.path().join("MyRobotSim/Plugins/uBot/uBotSensor")).unwrap();
        workspace.project.as_mut().unwrap().git.remove("ubotsensor");
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &options(true),
        );
        let result = &prepared.plan;
        assert!(result.errors.is_empty(), "{:?}", texts(&result.errors));
        assert_eq!(
            titles(result),
            [
                "Clone UBotSensor 0.1.0",
                "Download UBotROS 0.1.0",
                "Enable UBotSensor",
                "Enable UBotROS",
                "Build UnrealEditor"
            ]
        );
        assert!(result.needs_build);
        assert_eq!(
            texts(&result.warnings),
            ["No prebuilt UBotSensor 0.1.0 for UE 5.5; it will be built from source."]
        );

        // Without a compiler the build is still planned, with a warning.
        let prepared = plan_with(
            &workspace,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &options(true),
            &|| false,
        );
        assert!(texts(&prepared.plan.warnings)
            .contains(&"A C++ build is needed, but Visual Studio 2022 was not found.".to_string()));
        // Not asked when nothing is built.
        let asked = std::cell::Cell::new(false);
        let prepared = plan_with(
            &workspace,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &options(false),
            &|| {
                asked.set(true);
                false
            },
        );
        assert!(!asked.get());
        assert!(!texts(&prepared.plan.warnings)
            .iter()
            .any(|warning| warning.contains("Visual Studio")));
    }

    #[test]
    fn prebuilt_binaries_need_use_prebuilt_and_a_launcher_engine() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = prebuilt_workspace(dir.path(), true);
        let first_kind = |workspace: &Workspace| {
            let prepared = plan(
                workspace,
                &[change(ChangeKind::Install, "UBotROS", None)],
                &options(false),
            );
            (kinds(&prepared.plan)[0], texts(&prepared.plan.warnings))
        };
        assert_eq!(first_kind(&workspace).0, StepKind::Download);

        let mut off = workspace.clone();
        off.settings.install.use_prebuilt = false;
        assert_eq!(first_kind(&off), (StepKind::Clone, vec![]));

        for kind in [
            crate::model::EngineKind::Source,
            crate::model::EngineKind::Custom,
        ] {
            let mut source = workspace.clone();
            source
                .project
                .as_mut()
                .unwrap()
                .engine
                .as_mut()
                .unwrap()
                .kind = kind;
            assert_eq!(first_kind(&source), (StepKind::Clone, vec![]), "{kind:?}");
        }

        // A launcher engine without binaries for its version: cloned, with a warning.
        let mut other = workspace.clone();
        other
            .project
            .as_mut()
            .unwrap()
            .engine
            .as_mut()
            .unwrap()
            .version = "5.6.1".into();
        assert_eq!(
            first_kind(&other),
            (
                StepKind::Clone,
                vec!["No prebuilt UBotROS 0.1.0 for UE 5.6; it will be built from source.".into()]
            )
        );
    }

    #[test]
    fn updates_keep_git_checkouts_and_replace_prebuilt_installs() {
        let dir = tempfile::tempdir().unwrap();
        // UBotSensor disabled, so that UBotCore may move to 0.2.0.
        let mut workspace = prebuilt_workspace(dir.path(), false);
        let update_core = [change(ChangeKind::Update, "UBotCore", Some("0.2.0"))];

        // A git checkout is checked out, although 0.2.0 has binaries.
        let prepared = plan(&workspace, &update_core, &options(true));
        assert!(
            prepared.plan.errors.is_empty(),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        assert_eq!(
            titles(&prepared.plan),
            ["Check out UBotCore 0.2.0", "Build UnrealEditor"]
        );

        // A prebuilt install is replaced by the new binaries; nothing to build.
        for folder in ["uBotSensor", "uBotEnvironment"] {
            let win64 = dir
                .path()
                .join("MyRobotSim/Plugins/uBot")
                .join(folder)
                .join("Binaries/Win64");
            std::fs::create_dir_all(&win64).unwrap();
            std::fs::write(win64.join("UnrealEditor-X.dll"), "dll").unwrap();
        }
        mark_prebuilt(&mut workspace, "UBotCore", "5.5");
        let prepared = plan(&workspace, &update_core, &options(true));
        assert!(
            prepared.plan.errors.is_empty(),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        assert_eq!(titles(&prepared.plan), ["Download UBotCore 0.2.0"]);
        assert!(!prepared.plan.needs_build);
        assert!(
            prepared.plan.steps[0].needs_editor_closed,
            "the editor may have the old binaries loaded"
        );
        assert_eq!(
            render_en(&prepared.plan.steps[0].detail),
            "example.com/UBotCore-0.2.0-UE5.5-Win64.zip → Plugins/uBot/uBotCore"
        );
        assert!(matches!(
            &prepared.actions[0],
            StepAction::Download { replace: true, dest, .. } if dest.ends_with("Plugins/uBot/uBotCore")
        ));

        // A folder that is neither (copied in) is replaced the same way.
        let marker = dir
            .path()
            .join("MyRobotSim/Plugins/uBot/uBotCore")
            .join(crate::prebuilt::MARKER_FILE);
        std::fs::remove_file(&marker).unwrap();
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();
        assert_eq!(
            workspace.install_kind("UBotCore"),
            Some(crate::model::InstallKind::Other)
        );
        let prepared = plan(&workspace, &update_core, &options(true));
        assert_eq!(titles(&prepared.plan), ["Download UBotCore 0.2.0"]);

        // Without binaries for the target version: uninstall and install again. (An update to
        // the version it has is a reinstall, and 0.1.0 has no binaries.)
        mark_prebuilt(&mut workspace, "UBotCore", "5.5");
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotCore", Some("0.1.0"))],
            &options(true),
        );
        assert!(prepared.plan.steps.is_empty());
        assert_eq!(
            texts(&prepared.plan.errors),
            ["No prebuilt UBotCore 0.1.0 for UE 5.5; uninstall it and install it again to build from source."]
        );
        let mut moved = workspace.clone();
        moved
            .project
            .as_mut()
            .unwrap()
            .engine
            .as_mut()
            .unwrap()
            .version = "5.6.0".into();
        let prepared = plan(&moved, &update_core, &options(true));
        assert_eq!(
            texts(&prepared.plan.errors),
            ["No prebuilt UBotCore 0.2.0 for UE 5.6; uninstall it and install it again to build from source."]
        );
        assert!(prepared.actions.is_empty());
    }

    #[test]
    fn a_prebuilt_install_can_be_reinstalled_at_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = prebuilt_workspace(dir.path(), true);
        // UBotROS 0.1.0 installed prebuilt for UE 5.4; the project uses 5.5.
        crate::workspace::tests::write(
            &dir.path()
                .join("MyRobotSim/Plugins/uBot/uBotROS/UBotROS.uplugin"),
            &crate::workspace::tests::descriptor(
                "0.1.0",
                "Adapter",
                &[("UBotCore", "^0.1.0"), ("UBotSensor", "^0.1.0")],
                &[],
            ),
        );
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();
        mark_prebuilt(&mut workspace, "UBotROS", "5.4");
        for folder in ["uBotCore", "uBotSensor", "uBotEnvironment"] {
            let win64 = dir
                .path()
                .join("MyRobotSim/Plugins/uBot")
                .join(folder)
                .join("Binaries/Win64");
            std::fs::create_dir_all(&win64).unwrap();
            std::fs::write(win64.join("UnrealEditor-X.dll"), "dll").unwrap();
        }

        // fix.reinstall: an update to the installed version downloads it again.
        let reinstall = [change(ChangeKind::Update, "UBotROS", Some("0.1.0"))];
        let prepared = plan(&workspace, &reinstall, &options(true));
        assert!(
            prepared.plan.errors.is_empty(),
            "{:?}",
            texts(&prepared.plan.errors)
        );
        assert_eq!(titles(&prepared.plan), ["Download UBotROS 0.1.0"]);
        assert!(matches!(
            &prepared.actions[0],
            StepAction::Download { replace: true, engine, .. } if engine == "5.5"
        ));

        // Without binaries for this engine it cannot be reinstalled prebuilt.
        let mut moved = workspace.clone();
        moved
            .project
            .as_mut()
            .unwrap()
            .engine
            .as_mut()
            .unwrap()
            .version = "5.6.0".into();
        let prepared = plan(&moved, &reinstall, &options(true));
        assert_eq!(
            texts(&prepared.plan.errors),
            ["No prebuilt UBotROS 0.1.0 for UE 5.6; uninstall it and install it again to build from source."]
        );

        // A git checkout at its version: nothing to do, as before.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotCore", Some("0.1.0"))],
            &options(true),
        );
        assert!(prepared.plan.steps.is_empty() && prepared.plan.errors.is_empty());
    }

    #[test]
    fn a_dependency_installed_by_the_plan_follows_the_version_it_ends_at() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = prebuilt_workspace(dir.path(), true);
        // UBotCore is not installed: installing UBotSensor installs it, and a later change to
        // 0.2.0 (which has binaries) turns the clone into a download.
        std::fs::remove_dir_all(dir.path().join("MyRobotSim/Plugins/uBot/uBotCore")).unwrap();
        std::fs::remove_dir_all(dir.path().join("MyRobotSim/Plugins/uBot/uBotSensor")).unwrap();
        std::fs::remove_dir_all(dir.path().join("MyRobotSim/Plugins/uBot/uBotEnvironment"))
            .unwrap();
        let context = workspace.project.as_mut().unwrap();
        context.git.clear();
        context.project.reload().unwrap();
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotCore", None)],
            &options(false),
        );
        assert_eq!(
            titles(&prepared.plan),
            ["Download UBotCore 0.2.0", "Enable UBotCore"]
        );
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotCore", Some("0.1.0"))],
            &options(false),
        );
        assert_eq!(
            titles(&prepared.plan),
            ["Clone UBotCore 0.1.0", "Enable UBotCore"]
        );
        assert_eq!(
            texts(&prepared.plan.warnings),
            ["No prebuilt UBotCore 0.1.0 for UE 5.5; it will be built from source."]
        );
        // UBotSensor needs ^0.1.0: the core it installs is cloned.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotSensor", None)],
            &options(false),
        );
        assert_eq!(
            titles(&prepared.plan),
            [
                "Clone UBotCore 0.1.0",
                "Clone UBotSensor 0.1.0",
                "Enable UBotCore",
                "Enable UBotSensor"
            ]
        );
    }

    #[test]
    fn a_read_only_project_file_is_reported_before_anything_runs() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = sample_workspace(dir.path(), false);
        let uproject = workspace.project().unwrap().path.clone();
        let set_readonly = |readonly: bool| {
            let mut permissions = std::fs::metadata(&uproject).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(readonly);
            std::fs::set_permissions(&uproject, permissions).unwrap();
        };
        set_readonly(true);

        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Install, "UBotROS", None)],
            &options(false),
        );
        assert_eq!(
            texts(&prepared.plan.errors),
            ["MyRobotSim.uproject is read-only; check it out or clear the read-only flag first."]
        );
        // A plan that does not touch the .uproject is not affected.
        let prepared = plan(
            &workspace,
            &[change(ChangeKind::Update, "UBotCore", Some("0.2.0"))],
            &options(false),
        );
        assert!(prepared.plan.errors.is_empty());

        set_readonly(false);
    }
}

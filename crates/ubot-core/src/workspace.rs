//! Everything known after a refresh: the project with its plugins, engine, git states and last
//! session, plus the merged index. Problems, plans and the snapshot are computed from it.

use std::collections::HashMap;
use std::sync::Arc;

use crate::descriptor::Dependency;
use crate::engine::EnginePlugins;
use crate::git::GitInfo;
use crate::index::{IndexPackage, IndexVersion, MergedIndex, PrebuiltBinary};
use crate::model::{
    EditorState, EngineInfo, EngineKind, IndexSourceStatus, InstallKind, Layer, LocalizedText,
    PackageInfo, PackageState, PluginInfo, PrebuiltInfo, ProjectInfo, Recipe, Requirement,
    RequirementKind, RequirementStatus, Settings,
};
use crate::paths;
use crate::prebuilt::{engine_major_minor, PLATFORM};
use crate::project::{DiscoveredPlugin, Project};
use crate::sources::SourceFailure;
use crate::status::SessionFile;
use crate::url::{redact_credentials, web_url};
use crate::version::{Constraint, SemVer};

/// Result of the TCP check of the ROS bridge made during a refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosCheck {
    pub address: String,
    pub reachable: bool,
}

#[derive(Clone, Debug)]
pub struct ProjectContext {
    pub project: Project,
    pub engine: Option<EngineInfo>,
    pub engine_plugins: Option<Arc<EnginePlugins>>,
    /// Git state per installed uBot package (lower-case name).
    pub git: HashMap<String, GitInfo>,
    pub session: Option<SessionFile>,
    pub editor: EditorState,
    pub ros: Option<RosCheck>,
}

#[derive(Clone, Debug, Default)]
pub struct Workspace {
    pub project: Option<ProjectContext>,
    pub index: MergedIndex,
    pub sources: Vec<IndexSourceStatus>,
    pub source_failures: Vec<SourceFailure>,
    pub settings: Settings,
    pub user_recipes: Vec<Recipe>,
}

/// Where an engine plugin requirement stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginPresence {
    Enabled,
    Disabled,
    /// The engine (and the project) has no such plugin.
    Missing,
    /// No engine was found, so this cannot be checked.
    Unknown,
}

/// What a package requires: from its descriptor when installed, else from the index.
#[derive(Clone, Debug, Default)]
pub struct Requirements {
    pub packages: Vec<Dependency>,
    pub engine_plugins: Vec<String>,
    pub provides: Vec<String>,
}

pub fn index_requirements(package: &IndexPackage) -> Requirements {
    package
        .effective_version()
        .map(|version| Requirements {
            packages: version
                .requires
                .iter()
                .map(|r| Dependency {
                    name: r.name.clone(),
                    version: r.version.clone(),
                    optional: r.optional,
                })
                .collect(),
            engine_plugins: version.engine_plugins.clone(),
            provides: version.provides.clone(),
        })
        .unwrap_or_default()
}

impl Workspace {
    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref().map(|context| &context.project)
    }

    /// Any discovered project plugin.
    pub fn plugin(&self, name: &str) -> Option<&DiscoveredPlugin> {
        self.project()?.find_plugin(name)
    }

    /// An installed uBot package.
    pub fn installed(&self, name: &str) -> Option<&DiscoveredPlugin> {
        self.plugin(name).filter(|plugin| plugin.is_ubot())
    }

    pub fn ubot_plugins(&self) -> Vec<&DiscoveredPlugin> {
        self.project()
            .map(|project| {
                project
                    .plugins
                    .iter()
                    .filter(|plugin| plugin.is_ubot())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn is_enabled(&self, plugin: &DiscoveredPlugin) -> bool {
        self.project()
            .is_some_and(|project| project.is_plugin_enabled(plugin))
    }

    /// Display name: the index casing, else the descriptor casing.
    pub fn display_name(&self, name: &str) -> String {
        if let Some(package) = self.index.package(name) {
            return package.name.clone();
        }
        if let Some(plugin) = self.plugin(name) {
            return plugin.name.clone();
        }
        name.trim().to_string()
    }

    pub fn requirements(&self, name: &str) -> Requirements {
        if let Some(plugin) = self.installed(name) {
            if let Ok(descriptor) = &plugin.descriptor {
                if let Some(block) = &descriptor.ubot {
                    return Requirements {
                        packages: block.requires.clone(),
                        engine_plugins: block.engine_plugins.clone(),
                        provides: block.provides.clone(),
                    };
                }
            }
            return Requirements::default();
        }
        self.index
            .package(name)
            .map(index_requirements)
            .unwrap_or_default()
    }

    pub fn engine_plugins(&self) -> Option<&EnginePlugins> {
        self.project.as_ref()?.engine_plugins.as_deref()
    }

    /// Project plugins (non-uBot) take part too: a requirement may be satisfied by a plugin copied
    /// into the project.
    pub fn engine_plugin_presence(&self, name: &str) -> PluginPresence {
        let Some(project) = self.project() else {
            return PluginPresence::Unknown;
        };
        if let Some(plugin) = project.find_plugin(name) {
            return if project.is_plugin_enabled(plugin) {
                PluginPresence::Enabled
            } else {
                PluginPresence::Disabled
            };
        }
        match self.engine_plugins() {
            Some(engine_plugins) if engine_plugins.get(name).is_some() => {
                if project.is_engine_plugin_enabled(name, engine_plugins) {
                    PluginPresence::Enabled
                } else {
                    PluginPresence::Disabled
                }
            }
            Some(_) => PluginPresence::Missing,
            None => PluginPresence::Unknown,
        }
    }

    pub fn git(&self, name: &str) -> Option<&GitInfo> {
        self.project
            .as_ref()?
            .git
            .get(&name.trim().to_ascii_lowercase())
    }

    /// How an installed plugin got into the project: a git checkout, prebuilt binaries the
    /// manager installed (a valid marker), or neither. `None` when it is not installed.
    pub fn install_kind(&self, name: &str) -> Option<InstallKind> {
        let plugin = self.plugin(name)?;
        Some(if self.git(name).is_some_and(|git| git.is_checkout) {
            InstallKind::Git
        } else if plugin.install_marker.is_some() {
            InstallKind::Prebuilt
        } else {
            InstallKind::Other
        })
    }

    /// `Major.Minor` of the project's engine.
    pub fn engine_major_minor(&self) -> Option<String> {
        let engine = self.project.as_ref()?.engine.as_ref()?;
        engine_major_minor(&engine.version)
    }

    /// The engine (`Major.Minor`) prebuilt binaries are chosen for: set when they are used
    /// (`install.usePrebuilt`) and the project's engine is a launcher install, whose binaries
    /// are the same on every machine.
    pub fn prebuilt_engine(&self) -> Option<String> {
        if !self.settings.install.use_prebuilt {
            return None;
        }
        let engine = self.project.as_ref()?.engine.as_ref()?;
        if engine.kind != EngineKind::Launcher {
            return None;
        }
        engine_major_minor(&engine.version)
    }

    /// The prebuilt binaries an install of `version` would use now, with the engine
    /// (`Major.Minor`) they are for.
    pub fn prebuilt_binary(&self, version: &IndexVersion) -> Option<(PrebuiltBinary, String)> {
        let engine = self.prebuilt_engine()?;
        version
            .prebuilt(&engine, PLATFORM)
            .map(|binary| (binary, engine))
    }

    pub fn project_info(&self) -> Option<ProjectInfo> {
        let context = self.project.as_ref()?;
        let project = &context.project;
        Some(ProjectInfo {
            path: paths::display(&project.path),
            name: project.name.clone(),
            dir: paths::display(&project.dir),
            engine_association: project.uproject.engine_association(),
            engine: context.engine.clone(),
            has_code: project.has_code(),
            build_target: project.build_target(),
            editor: context.editor.clone(),
        })
    }

    /// Requirement rows of a package for the UI.
    fn requirement_rows(&self, requirements: &Requirements) -> Vec<Requirement> {
        let mut rows: Vec<Requirement> = requirements
            .packages
            .iter()
            .map(|dependency| {
                let (status, installed_version) = match self.installed(&dependency.name) {
                    Some(plugin) => {
                        let version = plugin.version().map(str::to_string);
                        let satisfied = match Constraint::parse(&dependency.version) {
                            Ok(constraint) if constraint.is_any() => true,
                            Ok(constraint) => version
                                .as_deref()
                                .and_then(SemVer::parse)
                                .is_some_and(|v| constraint.is_satisfied_by(&v)),
                            Err(_) => true,
                        };
                        let status = if !satisfied {
                            RequirementStatus::VersionMismatch
                        } else if !self.is_enabled(plugin) {
                            RequirementStatus::Disabled
                        } else {
                            RequirementStatus::Satisfied
                        };
                        (status, version)
                    }
                    None if self.index.package(&dependency.name).is_some() => {
                        (RequirementStatus::NotInstalled, None)
                    }
                    None => (RequirementStatus::Missing, None),
                };
                Requirement {
                    name: self.display_name(&dependency.name),
                    constraint: dependency.version.clone(),
                    optional: dependency.optional,
                    kind: RequirementKind::Package,
                    status,
                    installed_version,
                }
            })
            .collect();
        for plugin in &requirements.engine_plugins {
            let status = match self.engine_plugin_presence(plugin) {
                PluginPresence::Enabled => RequirementStatus::Satisfied,
                PluginPresence::Disabled => RequirementStatus::Disabled,
                PluginPresence::Missing | PluginPresence::Unknown => RequirementStatus::Missing,
            };
            let name = self
                .plugin(plugin)
                .map(|p| p.name.clone())
                .or_else(|| {
                    self.engine_plugins()
                        .and_then(|e| e.get(plugin))
                        .map(|p| p.name.clone())
                })
                .unwrap_or_else(|| plugin.clone());
            rows.push(Requirement {
                name,
                constraint: String::new(),
                optional: false,
                kind: RequirementKind::EnginePlugin,
                status,
                installed_version: None,
            });
        }
        rows
    }

    /// Index packages and installed uBot packages, sorted by layer order, then name.
    pub fn package_infos(&self) -> Vec<PackageInfo> {
        let mut names: Vec<String> = self
            .index
            .packages
            .iter()
            .map(|entry| entry.package.name.clone())
            .collect();
        for plugin in self.ubot_plugins() {
            if !names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&plugin.name))
            {
                names.push(plugin.name.clone());
            }
        }

        let mut packages: Vec<PackageInfo> =
            names.iter().map(|name| self.package_info(name)).collect();
        packages.sort_by(|a, b| {
            layer_rank(a.layer)
                .cmp(&layer_rank(b.layer))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        packages
    }

    pub fn package_info(&self, name: &str) -> PackageInfo {
        let entry = self.index.find(name);
        let index_package = entry.map(|entry| &entry.package);
        let plugin = self.installed(name);
        let descriptor = plugin.and_then(|plugin| plugin.descriptor.as_ref().ok());
        let block = descriptor.and_then(|descriptor| descriptor.ubot.as_ref());
        let display = self.display_name(name);

        let installed_version = plugin
            .and_then(|plugin| plugin.version())
            .map(str::to_string);
        let latest = index_package.and_then(IndexPackage::latest);
        let latest_version = latest.map(|version| version.version.clone());
        let state = match plugin {
            Some(plugin) if self.is_enabled(plugin) => PackageState::Enabled,
            Some(_) => PackageState::Disabled,
            None if index_package.is_some_and(IndexPackage::is_planned) => PackageState::Planned,
            None => PackageState::NotInstalled,
        };
        let update_available = match (
            installed_version.as_deref().and_then(SemVer::parse),
            latest.and_then(|v| v.semver()),
        ) {
            (Some(installed), Some(latest)) => latest > installed,
            _ => false,
        };

        let description = match index_package.and_then(|package| package.description.as_ref()) {
            Some(text) => text.to_localized(),
            None => LocalizedText::en(
                descriptor
                    .and_then(|d| d.description.clone())
                    .unwrap_or_default(),
            ),
        };
        let friendly_name = index_package
            .and_then(|package| package.friendly_name.clone())
            .or_else(|| descriptor.and_then(|d| d.friendly_name.clone()))
            .filter(|friendly| !friendly.trim().is_empty())
            .unwrap_or_else(|| display.clone());
        let layer = index_package
            .and_then(|package| package.layer.as_deref())
            .and_then(Layer::parse)
            .or(block.map(|block| block.layer))
            .unwrap_or(Layer::Unknown);
        let repository = index_package
            .and_then(|package| package.repository.clone())
            .or_else(|| block.and_then(|b| b.repository.clone()));
        let docs_url = index_package
            .and_then(|package| package.docs_url.clone())
            .or_else(|| descriptor.and_then(|d| d.docs_url.clone()));

        let requirements = self.requirements(name);
        let install_kind = plugin.and_then(|_| self.install_kind(name));
        // What the marker says the binaries are for: shown as is, because it can differ from the
        // project's engine (a disabled package raises no problem about it).
        let prebuilt = plugin
            .filter(|_| install_kind == Some(InstallKind::Prebuilt))
            .and_then(|plugin| plugin.install_marker.as_ref())
            .map(|marker| PrebuiltInfo {
                engine: marker.engine.clone(),
                platform: marker.platform.clone(),
            });
        // An install, or the update of anything but a git checkout, would download it.
        let prebuilt_available = install_kind != Some(InstallKind::Git)
            && latest.is_some_and(|version| self.prebuilt_binary(version).is_some());
        let tags = match (block, index_package) {
            (Some(block), _) => block.tags.clone(),
            (None, Some(package)) => package.tags.clone(),
            (None, None) => Vec::new(),
        };

        PackageInfo {
            name: display.clone(),
            friendly_name,
            description,
            layer,
            state,
            installed_version,
            latest_version,
            versions: index_package
                .map(|package| {
                    package
                        .released_versions()
                        .iter()
                        .map(|(version, _)| version.version.clone())
                        .collect()
                })
                .unwrap_or_default(),
            planned_version: index_package
                .and_then(IndexPackage::planned_version)
                .map(|version| version.version.clone()),
            update_available,
            location: plugin
                .and_then(|plugin| self.project().map(|project| project.location(&plugin.dir))),
            git: plugin.map(|_| self.git(name).cloned().unwrap_or_default().to_state()),
            install_kind,
            prebuilt_available,
            prebuilt,
            web_url: repository.as_deref().and_then(web_url),
            repository: repository.map(|repository| redact_credentials(&repository)),
            docs_url,
            index_source: entry.map(|entry| entry.source.clone()),
            requires: self.requirement_rows(&requirements),
            provides: requirements.provides,
            tags,
            problem_ids: Vec::new(),
        }
    }

    pub fn plugin_infos(&self) -> Vec<PluginInfo> {
        let Some(project) = self.project() else {
            return Vec::new();
        };
        project
            .plugins
            .iter()
            .map(|plugin| PluginInfo {
                name: plugin.name.clone(),
                enabled: project.is_plugin_enabled(plugin),
                location: project.location(&plugin.dir),
                version: plugin.version().map(str::to_string),
                is_ubot: plugin.is_ubot(),
            })
            .collect()
    }

    /// Index recipes, then user recipes; available when every package is available.
    pub fn recipes(&self) -> Vec<Recipe> {
        let available = |packages: &[String]| {
            !packages.is_empty() && packages.iter().all(|name| self.index.is_available(name))
        };
        let mut recipes: Vec<Recipe> = self
            .index
            .recipes
            .iter()
            .map(|entry| Recipe {
                id: entry.recipe.id.clone(),
                name: entry.recipe.name.to_localized(),
                description: entry.recipe.description.to_localized(),
                packages: entry.recipe.packages.clone(),
                checks: entry
                    .recipe
                    .checks
                    .iter()
                    .filter(|check| crate::index::KNOWN_RECIPE_CHECKS.contains(&check.as_str()))
                    .cloned()
                    .collect(),
                available: available(&entry.recipe.packages),
                user: false,
            })
            .collect();
        for recipe in &self.user_recipes {
            recipes.push(Recipe {
                available: available(&recipe.packages),
                user: true,
                ..recipe.clone()
            });
        }
        recipes
    }
}

pub fn layer_rank(layer: Layer) -> u8 {
    match layer {
        Layer::Foundation => 0,
        Layer::Capability => 1,
        Layer::Adapter => 2,
        Layer::Content => 3,
        Layer::Unknown => 4,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::index::{self, SourceIndex};
    use std::fs;
    use std::path::Path;

    pub const TEST_INDEX: &str = r#"{
        "formatVersion": 2, "name": "test",
        "packages": [
            { "name": "UBotCore", "friendlyName": "uBot Core", "description": { "en": "Core", "zh-CN": "核心" }, "layer": "Foundation",
              "repository": "https://github.com/AstreoX/uBotCore.git", "tags": [ "core" ],
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0" }, { "version": "0.2.0", "ref": "v0.2.0" } ] },
            { "name": "UBotSensor", "layer": "Capability", "description": { "en": "Sensor" }, "repository": "https://github.com/AstreoX/uBotSensor.git",
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ], "provides": [ "Sensor.IMU" ] } ] },
            { "name": "UBotROS", "layer": "Adapter", "description": { "en": "ROS" }, "repository": "https://github.com/AstreoX/uBotROS.git",
              "versions": [ { "version": "0.1.0", "ref": "v0.1.0", "requires": [ { "name": "UBotCore", "version": "^0.1.0" }, { "name": "UBotSensor", "version": "^0.1.0" } ] } ] },
            { "name": "UBotVehicles", "layer": "Capability", "description": { "en": "Vehicles" }, "repository": "https://github.com/AstreoX/uBotVehicles.git",
              "versions": [ { "version": "0.1.0", "planned": true, "requires": [ { "name": "UBotCore", "version": "^0.1.0" } ], "enginePlugins": [ "ChaosVehiclesPlugin" ] } ] },
            { "name": "UBotEnvironment", "layer": "Capability", "description": { "en": "Env" }, "repository": "https://github.com/AstreoX/uBotEnvironment.git",
              "versions": [ { "version": "0.2.0", "ref": "v0.2.0", "requires": [ { "name": "UBotCore", "version": "^0.2.0" } ], "enginePlugins": [ "Niagara" ] } ] }
        ],
        "recipes": [ { "id": "ros", "name": { "en": "ROS" }, "description": { "en": "ROS" }, "packages": [ "UBotCore", "UBotSensor", "UBotROS" ], "checks": [ "rosBridge", "warp" ] },
                     { "id": "veh", "name": { "en": "Veh" }, "description": { "en": "Veh" }, "packages": [ "UBotCore", "UBotVehicles" ] } ],
        "replacements": [ { "legacy": "AgentSensorCore", "replacement": "UBotSensor", "redirects": "Config/DefaultUBotSensor.ini" } ]
    }"#;

    pub fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub fn descriptor(
        version: &str,
        layer: &str,
        requires: &[(&str, &str)],
        engine_plugins: &[&str],
    ) -> String {
        let requires: Vec<String> = requires
            .iter()
            .map(|(name, version)| format!(r#"{{ "Name": "{name}", "Version": "{version}" }}"#))
            .collect();
        let plugins: Vec<String> = engine_plugins
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect();
        format!(
            r#"{{ "FileVersion": 3, "VersionName": "{version}", "Modules": [ {{ "Name": "M", "Type": "Runtime" }} ], "UBot": {{ "Layer": "{layer}", "Requires": [ {} ], "EnginePlugins": [ {} ] }} }}"#,
            requires.join(", "),
            plugins.join(", ")
        )
    }

    /// A project with UBotCore 0.1.0 (enabled), UBotEnvironment 0.2.0 (enabled, needs Core ^0.2.0
    /// and Niagara), UBotSensor 0.1.0 (disabled), AgentSensorCore 0.5.0 (legacy, enabled), and an
    /// engine with Niagara (disabled by default) and ChaosVehiclesPlugin.
    pub fn sample_workspace(dir: &Path, sensor_enabled: bool) -> Workspace {
        let project_dir = dir.join("MyRobotSim");
        let sensor_entry = if sensor_enabled {
            ""
        } else {
            r#", { "Name": "UBotSensor", "Enabled": false }"#
        };
        write(
            &project_dir.join("MyRobotSim.uproject"),
            &format!("{{\n\t\"FileVersion\": 3,\n\t\"EngineAssociation\": \"5.5\",\n\t\"Plugins\": [ {{ \"Name\": \"UBotCore\", \"Enabled\": true }}{sensor_entry} ]\n}}"),
        );
        let plugins = project_dir.join("Plugins");
        write(
            &plugins.join("uBot/uBotCore/UBotCore.uplugin"),
            &descriptor("0.1.0", "Foundation", &[], &[]),
        );
        write(
            &plugins.join("uBot/uBotSensor/UBotSensor.uplugin"),
            &descriptor("0.1.0", "Capability", &[("UBotCore", "^0.1.0")], &[]),
        );
        write(
            &plugins.join("uBot/uBotEnvironment/UBotEnvironment.uplugin"),
            &descriptor(
                "0.2.0",
                "Capability",
                &[("UBotCore", "^0.2.0")],
                &["Niagara"],
            ),
        );
        write(
            &plugins.join("AgentSensorCore/AgentSensorCore.uplugin"),
            r#"{ "VersionName": "0.5.0" }"#,
        );

        let mut engine_plugins = EnginePlugins::default();
        for (name, enabled_by_default) in [("Niagara", Some(false)), ("ChaosVehiclesPlugin", None)]
        {
            engine_plugins.plugins.insert(
                name.to_ascii_lowercase(),
                crate::engine::EnginePlugin {
                    name: name.into(),
                    enabled_by_default,
                },
            );
        }

        let project = Project::open(&project_dir.join("MyRobotSim.uproject")).unwrap();
        let mut git = HashMap::new();
        git.insert(
            "ubotcore".to_string(),
            GitInfo {
                is_checkout: true,
                tag: Some("v0.1.0".into()),
                ..Default::default()
            },
        );
        git.insert(
            "ubotsensor".to_string(),
            GitInfo {
                is_checkout: true,
                branch: Some("main".into()),
                short_sha: Some("abc1234".into()),
                ..Default::default()
            },
        );
        Workspace {
            project: Some(ProjectContext {
                project,
                engine: Some(EngineInfo {
                    id: "5.5".into(),
                    version: "5.5.4".into(),
                    root: "D:\\UE_5.5".into(),
                    kind: crate::model::EngineKind::Launcher,
                }),
                engine_plugins: Some(Arc::new(engine_plugins)),
                git,
                session: None,
                editor: EditorState::default(),
                ros: None,
            }),
            index: crate::index::MergedIndex::merge(&[SourceIndex {
                id: "ubot".into(),
                index: index::parse(TEST_INDEX).unwrap(),
            }]),
            sources: Vec::new(),
            source_failures: Vec::new(),
            settings: Settings::default(),
            user_recipes: vec![Recipe {
                id: "mine".into(),
                name: LocalizedText::en("Mine"),
                description: LocalizedText::en(""),
                packages: vec!["UBotCore".into(), "Unknown".into()],
                checks: vec![],
                available: true,
                user: true,
            }],
        }
    }

    pub const BINARY_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// [`sample_workspace`] with binaries for UE 5.5 in the index: UBotROS 0.1.0 and UBotCore
    /// 0.2.0 have them, nothing else does.
    pub fn prebuilt_workspace(dir: &Path, sensor_enabled: bool) -> Workspace {
        let mut workspace = sample_workspace(dir, sensor_enabled);
        let mut index = index::parse(TEST_INDEX).unwrap();
        let binaries = |name: &str, version: &str| {
            serde_json::json!({ "5.5": { "Win64": {
                "url": format!("https://example.com/{name}-{version}-UE5.5-Win64.zip"),
                "sha256": BINARY_SHA, "size": 1234 } } })
        };
        for package in &mut index.packages {
            for version in &mut package.versions {
                let wanted = matches!(
                    (package.name.as_str(), version.version.as_str()),
                    ("UBotROS", "0.1.0") | ("UBotCore", "0.2.0")
                );
                if wanted {
                    version.binaries = Some(binaries(&package.name, &version.version));
                }
            }
        }
        workspace.index = crate::index::MergedIndex::merge(&[SourceIndex {
            id: "ubot".into(),
            index,
        }]);
        workspace
    }

    /// Gives `version` of `name` binaries for UE 5.5 in the workspace's index.
    pub fn add_binaries(workspace: &mut Workspace, name: &str, version: &str) {
        let mut file = workspace.index.to_file();
        for package in file
            .packages
            .iter_mut()
            .filter(|package| package.name.eq_ignore_ascii_case(name))
        {
            for entry in package
                .versions
                .iter_mut()
                .filter(|entry| entry.version == version)
            {
                entry.binaries = Some(serde_json::json!({ "5.5": { "Win64": {
                    "url": format!("https://example.com/{name}-{version}-UE5.5-Win64.zip"),
                    "sha256": BINARY_SHA, "size": 1234 } } }));
            }
        }
        workspace.index = crate::index::MergedIndex::merge(&[SourceIndex {
            id: "ubot".into(),
            index: file,
        }]);
    }

    /// Writes a valid install marker into an installed plugin's folder and rescans.
    pub fn mark_prebuilt(workspace: &mut Workspace, name: &str, engine: &str) {
        let context = workspace.project.as_mut().unwrap();
        let dir = context.project.find_plugin(name).unwrap().dir.clone();
        let version = context
            .project
            .find_plugin(name)
            .unwrap()
            .version()
            .unwrap()
            .to_string();
        crate::prebuilt::InstallMarker {
            kind: crate::prebuilt::MARKER_KIND.into(),
            package: name.into(),
            version,
            engine: engine.into(),
            platform: PLATFORM.into(),
            url: "https://example.com/x.zip".into(),
            sha256: BINARY_SHA.into(),
            installed_at: "2026-10-10T08:00:00Z".into(),
        }
        .write(&dir)
        .unwrap();
        context.git.remove(&name.to_ascii_lowercase());
        context.project.reload().unwrap();
    }

    #[test]
    fn install_kinds_and_prebuilt_availability() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = prebuilt_workspace(dir.path(), true);
        let info = |workspace: &Workspace, name: &str| workspace.package_info(name);

        // Not installed, binaries for UE 5.5 and a launcher engine: available.
        assert!(info(&workspace, "UBotROS").prebuilt_available);
        assert_eq!(info(&workspace, "UBotROS").install_kind, None);
        // The latest UBotCore (0.2.0) has binaries, but a git checkout updates with git.
        assert_eq!(
            info(&workspace, "UBotCore").install_kind,
            Some(InstallKind::Git)
        );
        assert!(!info(&workspace, "UBotCore").prebuilt_available);
        // No binaries for the planned UBotVehicles or for UBotSensor 0.1.0.
        assert!(!info(&workspace, "UBotVehicles").prebuilt_available);
        assert!(!info(&workspace, "UBotSensor").prebuilt_available);

        // A marker makes a prebuilt install; its update would download again.
        mark_prebuilt(&mut workspace, "UBotCore", "5.5");
        assert_eq!(
            info(&workspace, "UBotCore").install_kind,
            Some(InstallKind::Prebuilt)
        );
        assert!(info(&workspace, "UBotCore").prebuilt_available);
        assert_eq!(
            info(&workspace, "UBotCore").prebuilt,
            Some(PrebuiltInfo {
                engine: "5.5".into(),
                platform: "Win64".into()
            })
        );
        assert_eq!(info(&workspace, "UBotROS").prebuilt, None, "not installed");
        // A marker of another package (a copied folder) does not count.
        let sensor_dir = workspace.plugin("UBotSensor").unwrap().dir.clone();
        std::fs::copy(
            workspace
                .plugin("UBotCore")
                .unwrap()
                .dir
                .join(crate::prebuilt::MARKER_FILE),
            sensor_dir.join(crate::prebuilt::MARKER_FILE),
        )
        .unwrap();
        workspace.project.as_mut().unwrap().git.remove("ubotsensor");
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();
        assert_eq!(
            info(&workspace, "UBotSensor").install_kind,
            Some(InstallKind::Other)
        );
        assert_eq!(info(&workspace, "UBotSensor").prebuilt, None);

        // Not with usePrebuilt off, a source engine, another engine version or no engine.
        let check = |change: &dyn Fn(&mut Workspace)| {
            let mut copy = workspace.clone();
            change(&mut copy);
            info(&copy, "UBotROS").prebuilt_available
        };
        assert!(check(&|_| {}));
        assert!(!check(&|w| w.settings.install.use_prebuilt = false));
        assert!(!check(&|w| {
            w.project.as_mut().unwrap().engine.as_mut().unwrap().kind = EngineKind::Source
        }));
        assert!(!check(&|w| {
            w.project.as_mut().unwrap().engine.as_mut().unwrap().kind = EngineKind::Custom
        }));
        assert!(!check(&|w| {
            w.project.as_mut().unwrap().engine.as_mut().unwrap().version = "5.6.0".into()
        }));
        assert!(!check(&|w| w.project.as_mut().unwrap().engine = None));
        assert_eq!(workspace.prebuilt_engine().as_deref(), Some("5.5"));
    }

    /// The marker's engine is reported as written, whatever the project's engine is and whether or
    /// not the package is enabled (a disabled package raises no engine mismatch problem).
    #[test]
    fn a_prebuilt_package_reports_the_engine_it_was_built_for() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = prebuilt_workspace(dir.path(), false);
        assert_eq!(
            workspace.package_info("UBotSensor").state,
            PackageState::Disabled
        );
        mark_prebuilt(&mut workspace, "UBotSensor", "5.4");
        let sensor = workspace.package_info("UBotSensor");
        assert_eq!(sensor.install_kind, Some(InstallKind::Prebuilt));
        assert_eq!(
            sensor.prebuilt,
            Some(PrebuiltInfo {
                engine: "5.4".into(),
                platform: "Win64".into()
            })
        );
        assert_eq!(workspace.engine_major_minor().as_deref(), Some("5.5"));
        // Without an engine the marker still says what it says.
        workspace.project.as_mut().unwrap().engine = None;
        assert_eq!(
            workspace
                .package_info("UBotSensor")
                .prebuilt
                .map(|p| p.engine),
            Some("5.4".to_string())
        );
        // A git checkout has no prebuilt info, even with a marker next to its .git.
        let mut workspace = prebuilt_workspace(dir.path(), false);
        mark_prebuilt(&mut workspace, "UBotCore", "5.4");
        workspace.project.as_mut().unwrap().git.insert(
            "ubotcore".into(),
            crate::git::GitInfo {
                is_checkout: true,
                ..Default::default()
            },
        );
        let core = workspace.package_info("UBotCore");
        assert_eq!(core.install_kind, Some(InstallKind::Git));
        assert_eq!(core.prebuilt, None);
    }

    #[test]
    fn builds_package_infos() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = sample_workspace(dir.path(), false);
        let packages = workspace.package_infos();
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "UBotCore",
                "UBotEnvironment",
                "UBotSensor",
                "UBotVehicles",
                "UBotROS"
            ]
        );

        let core = &packages[0];
        assert_eq!(core.state, PackageState::Enabled);
        assert_eq!(core.friendly_name, "uBot Core");
        assert_eq!(core.description.zh_cn.as_deref(), Some("核心"));
        assert_eq!(core.installed_version.as_deref(), Some("0.1.0"));
        assert_eq!(core.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(core.versions, ["0.2.0", "0.1.0"]);
        assert!(core.update_available);
        assert_eq!(core.location.as_deref(), Some("Plugins/uBot/uBotCore"));
        assert_eq!(
            core.git.as_ref().unwrap().git_ref.as_deref(),
            Some("v0.1.0")
        );
        assert_eq!(
            core.web_url.as_deref(),
            Some("https://github.com/AstreoX/uBotCore")
        );
        assert_eq!(core.index_source.as_deref(), Some("ubot"));
        assert_eq!(
            core.tags,
            Vec::<String>::new(),
            "tags of an installed package come from its descriptor"
        );

        let environment = &packages[1];
        assert_eq!(environment.requires.len(), 2);
        assert_eq!(
            environment.requires[0].status,
            RequirementStatus::VersionMismatch
        );
        assert_eq!(
            environment.requires[0].installed_version.as_deref(),
            Some("0.1.0")
        );
        assert_eq!(environment.requires[1].kind, RequirementKind::EnginePlugin);
        assert_eq!(environment.requires[1].status, RequirementStatus::Disabled);

        let sensor = &packages[2];
        assert_eq!(sensor.state, PackageState::Disabled);
        assert_eq!(
            sensor.git.as_ref().unwrap().git_ref.as_deref(),
            Some("main")
        );

        let vehicles = &packages[3];
        assert_eq!(vehicles.state, PackageState::Planned);
        assert_eq!(vehicles.planned_version.as_deref(), Some("0.1.0"));
        assert_eq!(vehicles.latest_version, None);
        assert!(vehicles.git.is_none() && vehicles.location.is_none());
        assert_eq!(vehicles.requires[1].name, "ChaosVehiclesPlugin");
        assert_eq!(
            vehicles.requires[1].status,
            RequirementStatus::Disabled,
            "engine plugins without EnabledByDefault are off"
        );

        let ros = &packages[4];
        assert_eq!(ros.state, PackageState::NotInstalled);
        assert_eq!(ros.requires[1].status, RequirementStatus::Disabled);
        assert_eq!(ros.install_kind, None);
        assert!(!ros.prebuilt_available, "the index has no binaries");
        assert_eq!(core.install_kind, Some(InstallKind::Git));
        assert_eq!(environment.install_kind, Some(InstallKind::Other));

        let plugins = workspace.plugin_infos();
        let legacy = plugins
            .iter()
            .find(|p| p.name == "AgentSensorCore")
            .unwrap();
        assert!(legacy.enabled && !legacy.is_ubot);
        assert_eq!(legacy.version.as_deref(), Some("0.5.0"));

        let recipes = workspace.recipes();
        assert_eq!(
            recipes
                .iter()
                .map(|r| (r.id.as_str(), r.available, r.user))
                .collect::<Vec<_>>(),
            [
                ("ros", true, false),
                ("veh", false, false),
                ("mine", false, true)
            ]
        );
        assert_eq!(recipes[0].checks, ["rosBridge"]);
    }
}

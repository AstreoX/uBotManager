//! The open project (SPEC §3): its `.uproject`, plugin discovery and UE's enabled rules.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::descriptor::{self, Descriptor};
use crate::engine::EnginePlugins;
use crate::paths;
use crate::prebuilt::InstallMarker;
use crate::uproject::UProject;

/// Directories never searched for plugins.
const SKIPPED_DIRS: &[&str] = &[
    "binaries",
    "intermediate",
    "saved",
    "deriveddatacache",
    ".git",
    "node_modules",
];
const MAX_DEPTH: usize = 16;

/// Every `*.uplugin` below `root`. A directory that contains a descriptor is recorded and not
/// descended into. Results are sorted by path.
pub fn find_descriptors(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, 0, &mut found);
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    let descriptors: Vec<PathBuf> = entries
        .iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("uplugin"))
                && path.is_file()
        })
        .cloned()
        .collect();
    if !descriptors.is_empty() {
        found.extend(descriptors);
        return;
    }
    if depth >= MAX_DEPTH {
        return;
    }
    for path in entries {
        // `.ubot-*` folders are the manager's own: a download being extracted, or the previous
        // version of a package during its replacement.
        let skipped = path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .is_some_and(|name| {
                SKIPPED_DIRS.contains(&name.as_str()) || name.starts_with(".ubot-")
            });
        if !skipped && path.is_dir() {
            walk(&path, depth + 1, found);
        }
    }
}

/// A plugin found under the project's plugin directories.
#[derive(Clone, Debug)]
pub struct DiscoveredPlugin {
    /// The descriptor's file stem, which is the plugin name.
    pub name: String,
    pub descriptor_path: PathBuf,
    pub dir: PathBuf,
    /// `Err` when the descriptor could not be read or parsed.
    pub descriptor: Result<Descriptor, String>,
    /// The raw text mentions a `"UBot"` key; used when it does not parse.
    pub mentions_ubot: bool,
    /// A valid `.ubot-install.json` of this plugin: the manager installed it from prebuilt
    /// binaries.
    pub install_marker: Option<InstallMarker>,
}

impl DiscoveredPlugin {
    pub fn load(descriptor_path: &Path) -> DiscoveredPlugin {
        let name = descriptor_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (descriptor, mentions_ubot) = match paths::read_text(descriptor_path) {
            Ok((text, _)) => (descriptor::parse(&text), text.contains("\"UBot\"")),
            Err(error) => (Err(error.to_string()), false),
        };
        let dir = descriptor_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        DiscoveredPlugin {
            install_marker: InstallMarker::read(&dir, &name),
            name,
            descriptor_path: descriptor_path.to_path_buf(),
            dir,
            descriptor,
            mentions_ubot,
        }
    }

    /// The plugin has code modules (an unreadable descriptor is assumed to have some).
    pub fn has_modules(&self) -> bool {
        self.descriptor
            .as_ref()
            .map_or(true, |descriptor| descriptor.has_modules)
    }

    /// The descriptor has a `"UBot"` object (or is unreadable but mentions one).
    pub fn is_ubot(&self) -> bool {
        match &self.descriptor {
            Ok(descriptor) => descriptor.is_ubot(),
            Err(_) => self.mentions_ubot,
        }
    }

    pub fn version(&self) -> Option<&str> {
        self.descriptor
            .as_ref()
            .ok()?
            .version_name
            .as_deref()
            .filter(|version| !version.is_empty())
    }

    pub fn enabled_by_default(&self) -> Option<bool> {
        self.descriptor.as_ref().ok()?.enabled_by_default
    }

    /// Descriptor problems worth reporting, joined; `None` when clean.
    pub fn issues(&self) -> Option<String> {
        match &self.descriptor {
            Ok(descriptor) if descriptor.issues.is_empty() => None,
            Ok(descriptor) => Some(descriptor.issues.join(" ")),
            Err(error) => Some(error.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    /// Absolute path of the `.uproject`.
    pub path: PathBuf,
    pub name: String,
    pub dir: PathBuf,
    pub uproject: UProject,
    /// `<dir>/Plugins` and the `AdditionalPluginDirectories`.
    pub plugin_roots: Vec<PathBuf>,
    pub plugins: Vec<DiscoveredPlugin>,
}

impl Project {
    pub fn open(path: &Path) -> io::Result<Project> {
        let path = paths::absolute(path);
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("uproject"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a .uproject file", path.display()),
            ));
        }
        let uproject = UProject::load(&path)?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut project = Project {
            path,
            name,
            dir,
            uproject,
            plugin_roots: Vec::new(),
            plugins: Vec::new(),
        };
        project.scan_plugins();
        Ok(project)
    }

    /// Re-reads the `.uproject` and rediscovers plugins.
    pub fn reload(&mut self) -> io::Result<()> {
        self.uproject = UProject::load(&self.path)?;
        self.scan_plugins();
        Ok(())
    }

    fn scan_plugins(&mut self) {
        let mut roots = vec![self.dir.join("Plugins")];
        for additional in self.uproject.additional_plugin_directories() {
            let root = paths::normalize(&self.dir.join(additional));
            if !roots
                .iter()
                .any(|existing| paths::same_path(existing, &root))
            {
                roots.push(root);
            }
        }

        let mut plugins: Vec<DiscoveredPlugin> = Vec::new();
        for root in &roots {
            for descriptor in find_descriptors(root) {
                let plugin = DiscoveredPlugin::load(&descriptor);
                // UE refuses duplicate plugin names; the first one found is the one shown.
                if !plugins
                    .iter()
                    .any(|existing| existing.name.eq_ignore_ascii_case(&plugin.name))
                {
                    plugins.push(plugin);
                }
            }
        }
        self.plugin_roots = roots;
        self.plugins = plugins;
    }

    pub fn find_plugin(&self, name: &str) -> Option<&DiscoveredPlugin> {
        let name = name.trim();
        self.plugins
            .iter()
            .find(|plugin| plugin.name.eq_ignore_ascii_case(name))
    }

    /// UE rules for a project plugin: a `.uproject` entry with `"Enabled"` decides, else the
    /// descriptor's `EnabledByDefault`, else enabled.
    pub fn is_plugin_enabled(&self, plugin: &DiscoveredPlugin) -> bool {
        self.uproject
            .entry_enabled(&plugin.name)
            .or(plugin.enabled_by_default())
            .unwrap_or(true)
    }

    /// Engine plugins: the entry decides, else `EnabledByDefault`, else disabled.
    pub fn is_engine_plugin_enabled(&self, name: &str, engine_plugins: &EnginePlugins) -> bool {
        self.uproject
            .entry_enabled(name)
            .or_else(|| {
                engine_plugins
                    .get(name)
                    .and_then(|plugin| plugin.enabled_by_default)
            })
            .unwrap_or(false)
    }

    /// Project-relative location with '/' separators.
    pub fn location(&self, path: &Path) -> String {
        paths::relative_display(path, &self.dir)
    }

    pub fn has_code(&self) -> bool {
        self.dir.join("Source").is_dir()
    }

    /// The UBT target that builds this project's editor (SPEC section 3).
    pub fn build_target(&self) -> String {
        self.build_target_with(&[])
    }

    /// [`Project::build_target`] when packages are also going to be cloned into `new_plugin_dirs`.
    ///
    /// `<Name>Editor` exists as a target when the project has a `*.Target.cs`, or when UBT makes
    /// a temporary one in `Intermediate/Source` - which it does for a content-only project only
    /// when the plugins enabled differ from the default set, as they do for plugins in an
    /// `AdditionalPluginDirectories` root ("hybrid" projects such as DevHost). A content-only
    /// project whose plugins all sit in `<Project>/Plugins` has no such target, so the stock
    /// `UnrealEditor` target is built with `-Project=`.
    pub fn build_target_with(&self, new_plugin_dirs: &[PathBuf]) -> String {
        let editor = format!("{}Editor", self.name);
        let plugins_dir = self.dir.join("Plugins");
        let outside_plugins_dir = |dir: &Path| !paths::is_within(dir, &plugins_dir);
        let hybrid = self
            .plugins
            .iter()
            .filter(|plugin| self.is_plugin_enabled(plugin))
            .any(|plugin| outside_plugins_dir(&plugin.dir))
            || new_plugin_dirs.iter().any(|dir| outside_plugins_dir(dir));
        if hybrid || self.has_target_rules() || self.has_temporary_target(&editor) {
            editor
        } else {
            "UnrealEditor".to_string()
        }
    }

    /// `Source/*.Target.cs` exists: the project is a code project with its own targets.
    fn has_target_rules(&self) -> bool {
        fs::read_dir(self.dir.join("Source")).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(".target.cs")
            })
        })
    }

    /// UBT left `Intermediate/Source/<Target>.Target.cs` from an earlier build.
    fn has_temporary_target(&self, target: &str) -> bool {
        self.dir
            .join("Intermediate")
            .join("Source")
            .join(format!("{target}.Target.cs"))
            .is_file()
    }

    pub fn file_name(&self) -> String {
        format!("{}.uproject", self.name)
    }

    /// `settings.install.directory` resolved against the project directory.
    pub fn install_dir(&self, directory: &str) -> PathBuf {
        let directory = directory.trim();
        let directory = if directory.is_empty() {
            "Plugins/uBot"
        } else {
            directory
        };
        paths::normalize(&self.dir.join(directory))
    }

    /// True when `path` is a plugin root itself or inside one: `<Project>/Plugins` or an
    /// `AdditionalPluginDirectories` entry. Only there are plugins discovered.
    pub fn is_discoverable(&self, path: &Path) -> bool {
        self.plugin_roots
            .iter()
            .any(|root| paths::is_within(path, root))
    }

    /// True when `dir` is a plugin root itself, which holds other plugins too.
    pub fn is_plugin_root(&self, dir: &Path) -> bool {
        self.plugin_roots
            .iter()
            .any(|root| paths::same_path(root, dir))
    }

    pub fn status_file(&self) -> PathBuf {
        self.dir.join("Saved").join("uBot").join("status.json")
    }

    pub fn merged_index_file(&self) -> PathBuf {
        self.dir.join("Saved").join("uBot").join("index.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn discovers_plugins_and_applies_enabled_rules() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("Host");
        write(
            &project_dir.join("Host.uproject"),
            "{\n\t\"FileVersion\": 3,\n\t\"EngineAssociation\": \"5.5\",\n\t\"AdditionalPluginDirectories\": [ \"../Shared\" ],\n\t\"Plugins\": [ { \"Name\": \"Disabled\", \"Enabled\": false }, { \"Name\": \"ChaosVehiclesPlugin\", \"Enabled\": true } ]\n}",
        );
        write(
            &project_dir.join("Plugins/uBot/uBotCore/UBotCore.uplugin"),
            r#"{ "VersionName": "0.1.0", "UBot": { "Layer": "Foundation" } }"#,
        );
        write(
            &project_dir.join("Plugins/uBot/uBotCore/Nested/Inner.uplugin"),
            "{}",
        );
        write(
            &project_dir.join("Plugins/Disabled/Disabled.uplugin"),
            r#"{ "VersionName": "1.0" }"#,
        );
        write(
            &project_dir.join("Plugins/OffByDefault/OffByDefault.uplugin"),
            r#"{ "EnabledByDefault": false }"#,
        );
        write(
            &project_dir.join("Plugins/Broken/Broken.uplugin"),
            r#"{ "UBot": { "#,
        );
        write(
            &project_dir.join("Plugins/Binaries/Hidden/Hidden.uplugin"),
            "{}",
        );
        write(&project_dir.join("Plugins/x/.git/G/G.uplugin"), "{}");
        write(
            &project_dir.join("Plugins/uBot/.ubot-extract-UBotROS/UBotROS/UBotROS.uplugin"),
            r#"{ "VersionName": "9.0.0", "UBot": { "Layer": "Adapter" } }"#,
        );
        write(
            &root.path().join("Shared/uBotROS/UBotROS.uplugin"),
            r#"{ "VersionName": "0.1.0", "UBot": { "Layer": "Adapter" } }"#,
        );
        write(
            &root.path().join("Shared/ubotcore/UBotCore.uplugin"),
            r#"{ "VersionName": "9.9.9", "UBot": { "Layer": "Foundation" } }"#,
        );

        let project = Project::open(&project_dir.join("Host.uproject")).unwrap();
        assert_eq!(project.name, "Host");
        assert_eq!(
            project.build_target(),
            "HostEditor",
            "plugins in an AdditionalPluginDirectories root: UBT makes the temporary target"
        );
        assert!(!project.has_code());
        let names: Vec<&str> = project.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["Broken", "Disabled", "OffByDefault", "UBotCore", "UBotROS"]
        );

        let core = project.find_plugin("ubotcore").unwrap();
        assert!(core.is_ubot());
        assert_eq!(core.version(), Some("0.1.0"));
        assert_eq!(project.location(&core.dir), "Plugins/uBot/uBotCore");
        assert!(project.is_plugin_enabled(core));
        let ros = project.find_plugin("UBotROS").unwrap();
        assert_eq!(project.location(&ros.dir), "../Shared/uBotROS");
        assert!(project.is_discoverable(&ros.dir));
        assert!(!project.is_plugin_enabled(project.find_plugin("Disabled").unwrap()));
        assert!(!project.is_plugin_enabled(project.find_plugin("OffByDefault").unwrap()));
        let broken = project.find_plugin("Broken").unwrap();
        assert!(broken.is_ubot() && broken.issues().is_some());

        let mut engine_plugins = EnginePlugins::default();
        engine_plugins.plugins.insert(
            "modelingtools".into(),
            crate::engine::EnginePlugin {
                name: "ModelingTools".into(),
                enabled_by_default: Some(true),
            },
        );
        assert!(project.is_engine_plugin_enabled("ChaosVehiclesPlugin", &engine_plugins));
        assert!(project.is_engine_plugin_enabled("ModelingTools", &engine_plugins));
        assert!(!project.is_engine_plugin_enabled("Paper2D", &engine_plugins));

        assert_eq!(
            project.install_dir(""),
            project_dir.join("Plugins").join("uBot")
        );
        assert!(Project::open(&project_dir.join("Plugins/Disabled/Disabled.uplugin")).is_err());
    }

    #[test]
    fn builds_the_target_that_exists() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("Foo");
        let uproject = project_dir.join("Foo.uproject");
        write(
            &uproject,
            "{ \"FileVersion\": 3, \"EngineAssociation\": \"5.5\" }",
        );
        write(
            &project_dir.join("Plugins/uBot/uBotCore/UBotCore.uplugin"),
            r#"{ "VersionName": "0.1.0", "UBot": { "Layer": "Foundation" } }"#,
        );
        let target = |dir: &Path| {
            Project::open(&dir.join("Foo.uproject"))
                .unwrap()
                .build_target()
        };

        // Content only, every plugin in <Project>/Plugins: UBT makes no <Name>Editor target.
        assert_eq!(target(&project_dir), "UnrealEditor");
        // A target of the project's own.
        write(&project_dir.join("Source/Foo.Target.cs"), "// target");
        assert_eq!(target(&project_dir), "FooEditor");
        fs::remove_dir_all(project_dir.join("Source")).unwrap();
        assert_eq!(target(&project_dir), "UnrealEditor");
        // The temporary target of an earlier build.
        write(
            &project_dir.join("Intermediate/Source/FooEditor.Target.cs"),
            "// generated",
        );
        assert_eq!(target(&project_dir), "FooEditor");
        fs::remove_dir_all(project_dir.join("Intermediate")).unwrap();

        // Plugins in an AdditionalPluginDirectories root: UBT makes the temporary target.
        write(
            &uproject,
            "{ \"FileVersion\": 3, \"EngineAssociation\": \"5.5\", \"AdditionalPluginDirectories\": [ \"../Shared\" ] }",
        );
        write(
            &root.path().join("Shared/uBotROS/UBotROS.uplugin"),
            r#"{ "VersionName": "0.1.0", "UBot": { "Layer": "Adapter" } }"#,
        );
        assert_eq!(target(&project_dir), "FooEditor");
        // ... but not when that plugin is disabled in the project.
        write(
            &uproject,
            "{ \"FileVersion\": 3, \"EngineAssociation\": \"5.5\", \"AdditionalPluginDirectories\": [ \"../Shared\" ], \"Plugins\": [ { \"Name\": \"UBotROS\", \"Enabled\": false } ] }",
        );
        assert_eq!(target(&project_dir), "UnrealEditor");

        // A clone that is about to go into such a root decides the target before it exists.
        write(
            &uproject,
            "{ \"FileVersion\": 3, \"EngineAssociation\": \"5.5\" }",
        );
        let project = Project::open(&uproject).unwrap();
        let inside = project_dir.join("Plugins/uBot/uBotSensor");
        let outside = root.path().join("Shared/uBot/uBotSensor");
        assert_eq!(project.build_target_with(&[inside]), "UnrealEditor");
        assert_eq!(project.build_target_with(&[outside]), "FooEditor");
    }

    #[test]
    fn knows_its_plugin_roots() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("Host");
        write(
            &project_dir.join("Host.uproject"),
            "{ \"FileVersion\": 3, \"AdditionalPluginDirectories\": [ \"../Shared\" ] }",
        );
        let project = Project::open(&project_dir.join("Host.uproject")).unwrap();
        assert!(project.is_plugin_root(&project_dir.join("Plugins")));
        assert!(project.is_plugin_root(&root.path().join("Shared")));
        assert!(!project.is_plugin_root(&project_dir.join("Plugins/uBot")));
        assert!(!project.is_plugin_root(&project_dir));
        assert!(project.is_discoverable(&project_dir.join("Plugins/uBot")));
        assert!(project.is_discoverable(&project_dir.join("Plugins")));
        assert!(!project.is_discoverable(&project_dir.join("Content")));
        assert!(!project.is_discoverable(&project_dir));
    }
}

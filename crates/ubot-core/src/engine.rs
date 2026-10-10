//! Unreal Engine installations (SPEC §3): resolving a project's `EngineAssociation`, listing the
//! installed engines and scanning an engine's plugins.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{EngineInfo, EngineKind};
use crate::paths;
use crate::project::find_descriptors;

pub const LAUNCHER_INSTALLED_DAT: &str =
    r"C:\ProgramData\Epic\UnrealEngineLauncher\LauncherInstalled.dat";

/// `Major.Minor.Patch` from `<root>/Engine/Build/Build.version`.
pub fn engine_version(root: &Path) -> Option<String> {
    let (text, _) =
        paths::read_text(&root.join("Engine").join("Build").join("Build.version")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let part = |key: &str| value.get(key).and_then(Value::as_u64);
    Some(format!(
        "{}.{}.{}",
        part("MajorVersion")?,
        part("MinorVersion")?,
        part("PatchVersion").unwrap_or(0)
    ))
}

pub fn is_engine_root(root: &Path) -> bool {
    root.join("Engine")
        .join("Build")
        .join("Build.version")
        .is_file()
}

/// `"5.5"`-style associations name launcher installs; `{GUID}` and other strings name registered builds.
pub fn is_launcher_association(association: &str) -> bool {
    let mut parts = association.split('.');
    let numeric = |part: Option<&str>| {
        part.is_some_and(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
    };
    numeric(parts.next()) && numeric(parts.next()) && parts.next().is_none()
}

/// Launcher installs from `LauncherInstalled.dat` content: `(association, root)` for every `UE_x.y` app.
pub fn parse_launcher_installed(text: &str) -> Vec<(String, PathBuf)> {
    let Ok(value) = serde_json::from_str::<Value>(text.strip_prefix('\u{feff}').unwrap_or(text))
    else {
        return Vec::new();
    };
    value
        .get("InstallationList")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let app = item.get("AppName")?.as_str()?;
                    let association = app.strip_prefix("UE_")?;
                    if !is_launcher_association(association) {
                        return None;
                    }
                    let location = item.get("InstallLocation")?.as_str()?;
                    Some((association.to_string(), PathBuf::from(location)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where engines are registered on this machine. Abstracted so resolution can be tested.
pub trait EngineRegistry {
    /// `(association, root)` of launcher installs.
    fn launcher_installs(&self) -> Vec<(String, PathBuf)>;
    /// `(id, root)` of registered source builds.
    fn registered_builds(&self) -> Vec<(String, PathBuf)>;
}

/// The real registry: `LauncherInstalled.dat`, `HKLM\SOFTWARE\EpicGames\Unreal Engine\<x.y>` and
/// `HKCU\Software\Epic Games\Unreal Engine\Builds`.
pub struct SystemEngineRegistry;

impl EngineRegistry for SystemEngineRegistry {
    fn launcher_installs(&self) -> Vec<(String, PathBuf)> {
        let mut installs = paths::read_text(Path::new(LAUNCHER_INSTALLED_DAT))
            .map(|(text, _)| parse_launcher_installed(&text))
            .unwrap_or_default();
        for (association, root) in registry::launcher_versions() {
            if !installs
                .iter()
                .any(|(existing, _)| existing == &association)
            {
                installs.push((association, root));
            }
        }
        installs
    }

    fn registered_builds(&self) -> Vec<(String, PathBuf)> {
        registry::source_builds()
    }
}

/// Every installed engine: launcher installs, then registered source builds.
pub fn list_engines(registry: &dyn EngineRegistry) -> Vec<EngineInfo> {
    let mut engines: Vec<EngineInfo> = Vec::new();
    let mut push = |id: String, root: PathBuf, kind: EngineKind| {
        let root = paths::normalize(&root);
        if engines
            .iter()
            .any(|engine| paths::same_path(Path::new(&engine.root), &root))
        {
            return;
        }
        if let Some(version) = engine_version(&root) {
            engines.push(EngineInfo {
                id,
                version,
                root: paths::display(&root),
                kind,
            });
        }
    };
    for (association, root) in registry.launcher_installs() {
        push(association, root, EngineKind::Launcher);
    }
    for (id, root) in registry.registered_builds() {
        push(id, root, EngineKind::Source);
    }
    engines
}

/// Resolves the engine of a project. `override_id` (`settings.build.engineId`) is an id from
/// [`list_engines`] or an engine root entered by the user; when it does not resolve, the
/// association is used.
pub fn resolve(
    registry: &dyn EngineRegistry,
    association: &str,
    project_dir: &Path,
    override_id: Option<&str>,
) -> Option<EngineInfo> {
    if let Some(id) = override_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Some(engine) = list_engines(registry)
            .into_iter()
            .find(|engine| engine.id.eq_ignore_ascii_case(id))
        {
            return Some(engine);
        }
        let root = PathBuf::from(id);
        if root.is_absolute() {
            if let Some(version) = engine_version(&root) {
                return Some(EngineInfo {
                    id: id.to_string(),
                    version,
                    root: paths::display(&paths::normalize(&root)),
                    kind: EngineKind::Custom,
                });
            }
        }
    }

    let association = association.trim();
    if association.is_empty() {
        let mut dir = Some(project_dir);
        while let Some(current) = dir {
            if let Some(version) = engine_version(current) {
                let root = paths::display(&paths::normalize(current));
                return Some(EngineInfo {
                    id: root.clone(),
                    version,
                    root,
                    kind: EngineKind::Source,
                });
            }
            dir = current.parent();
        }
        return None;
    }

    let (candidates, kind) = if is_launcher_association(association) {
        (registry.launcher_installs(), EngineKind::Launcher)
    } else {
        (registry.registered_builds(), EngineKind::Source)
    };
    candidates
        .into_iter()
        .filter(|(id, _)| id.eq_ignore_ascii_case(association))
        .find_map(|(_, root)| {
            let version = engine_version(&root)?;
            Some(EngineInfo {
                id: association.to_string(),
                version,
                root: paths::display(&paths::normalize(&root)),
                kind,
            })
        })
}

/// A plugin shipped with the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnginePlugin {
    pub name: String,
    pub enabled_by_default: Option<bool>,
}

/// The plugins under `<root>/Engine/Plugins`, keyed by lower-case name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnginePlugins {
    pub plugins: HashMap<String, EnginePlugin>,
}

impl EnginePlugins {
    pub fn scan(root: &Path) -> EnginePlugins {
        let mut plugins = HashMap::new();
        for descriptor in find_descriptors(&root.join("Engine").join("Plugins")) {
            let Some(name) = descriptor
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
            else {
                continue;
            };
            let enabled_by_default = paths::read_text(&descriptor)
                .ok()
                .and_then(|(text, _)| serde_json::from_str::<Value>(&text).ok())
                .and_then(|value| value.get("EnabledByDefault").and_then(Value::as_bool));
            plugins
                .entry(name.to_ascii_lowercase())
                .or_insert(EnginePlugin {
                    name,
                    enabled_by_default,
                });
        }
        EnginePlugins { plugins }
    }

    pub fn get(&self, name: &str) -> Option<&EnginePlugin> {
        self.plugins.get(&name.trim().to_ascii_lowercase())
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

#[cfg(windows)]
mod registry {
    use std::path::PathBuf;

    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    use super::is_launcher_association;

    pub fn launcher_versions() -> Vec<(String, PathBuf)> {
        let Ok(key) =
            RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\EpicGames\Unreal Engine")
        else {
            return Vec::new();
        };
        key.enum_keys()
            .filter_map(Result::ok)
            .filter(|name| is_launcher_association(name))
            .filter_map(|name| {
                let directory: String = key
                    .open_subkey(&name)
                    .ok()?
                    .get_value("InstalledDirectory")
                    .ok()?;
                Some((name, PathBuf::from(directory)))
            })
            .collect()
    }

    pub fn source_builds() -> Vec<(String, PathBuf)> {
        let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Epic Games\Unreal Engine\Builds")
        else {
            return Vec::new();
        };
        key.enum_values()
            .filter_map(Result::ok)
            .filter_map(|(name, _)| {
                let root: String = key.get_value(&name).ok()?;
                Some((name, PathBuf::from(root)))
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod registry {
    use std::path::PathBuf;

    pub fn launcher_versions() -> Vec<(String, PathBuf)> {
        Vec::new()
    }

    pub fn source_builds() -> Vec<(String, PathBuf)> {
        Vec::new()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    /// A fake registry over temp directories.
    pub struct FakeRegistry {
        pub launcher: Vec<(String, PathBuf)>,
        pub builds: Vec<(String, PathBuf)>,
    }

    impl EngineRegistry for FakeRegistry {
        fn launcher_installs(&self) -> Vec<(String, PathBuf)> {
            self.launcher.clone()
        }
        fn registered_builds(&self) -> Vec<(String, PathBuf)> {
            self.builds.clone()
        }
    }

    pub fn make_engine(root: &Path, version: (u32, u32, u32)) {
        let build = root.join("Engine").join("Build");
        fs::create_dir_all(&build).unwrap();
        fs::write(
            build.join("Build.version"),
            format!(
                "{{ \"MajorVersion\": {}, \"MinorVersion\": {}, \"PatchVersion\": {} }}",
                version.0, version.1, version.2
            ),
        )
        .unwrap();
    }

    #[test]
    fn parses_launcher_installed() {
        let text = r#"{ "InstallationList": [
            { "InstallLocation": "D:\\Epic\\UE_5.5", "AppName": "FabPlugin_5.5" },
            { "InstallLocation": "D:\\Epic\\UE_5.5", "AppName": "UE_5.5" },
            { "InstallLocation": "D:\\Epic\\UE_5.4", "AppName": "UE_5.4" },
            { "InstallLocation": "G:\\Game", "AppName": "8769e240" } ] }"#;
        assert_eq!(
            parse_launcher_installed(text),
            [
                ("5.5".to_string(), PathBuf::from(r"D:\Epic\UE_5.5")),
                ("5.4".to_string(), PathBuf::from(r"D:\Epic\UE_5.4"))
            ]
        );
        assert!(parse_launcher_installed("garbage").is_empty());
        assert!(is_launcher_association("5.5"));
        assert!(!is_launcher_association("{8F5A1B2C-0000}"));
        assert!(!is_launcher_association("5"));
        assert!(!is_launcher_association("5.5.1"));
    }

    #[test]
    fn resolves_associations() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = dir.path().join("UE_5.5");
        let source = dir.path().join("Source");
        let custom = dir.path().join("Custom");
        make_engine(&launcher, (5, 5, 4));
        make_engine(&source, (5, 6, 0));
        make_engine(&custom, (5, 5, 1));
        let registry = FakeRegistry {
            launcher: vec![("5.5".into(), launcher.clone())],
            builds: vec![
                ("{ABC}".into(), source.clone()),
                ("Broken".into(), dir.path().join("missing")),
            ],
        };
        let project_dir = dir.path().join("Project");

        let engine = resolve(&registry, "5.5", &project_dir, None).unwrap();
        assert_eq!(
            (engine.id.as_str(), engine.version.as_str(), engine.kind),
            ("5.5", "5.5.4", EngineKind::Launcher)
        );
        let engine = resolve(&registry, "{abc}", &project_dir, None).unwrap();
        assert_eq!(
            (engine.id.as_str(), engine.version.as_str(), engine.kind),
            ("{abc}", "5.6.0", EngineKind::Source)
        );
        assert!(resolve(&registry, "5.4", &project_dir, None).is_none());
        assert!(resolve(&registry, "Broken", &project_dir, None).is_none());

        let engine = resolve(&registry, "5.4", &project_dir, Some("{ABC}")).unwrap();
        assert_eq!(engine.kind, EngineKind::Source);
        let custom_id = custom.to_string_lossy().into_owned();
        let engine = resolve(&registry, "5.5", &project_dir, Some(&custom_id)).unwrap();
        assert_eq!(
            (engine.version.as_str(), engine.kind),
            ("5.5.1", EngineKind::Custom)
        );
        let engine = resolve(&registry, "5.5", &project_dir, Some("unknown")).unwrap();
        assert_eq!(
            engine.kind,
            EngineKind::Launcher,
            "an unknown override falls back to the association"
        );

        // Empty association: walk up from the project directory.
        let inside = source.join("Games").join("MyGame");
        fs::create_dir_all(&inside).unwrap();
        let engine = resolve(&registry, "", &inside, None).unwrap();
        assert_eq!(
            (engine.version.as_str(), engine.kind),
            ("5.6.0", EngineKind::Source)
        );
        assert!(resolve(&registry, "", &project_dir, None).is_none());

        let engines = list_engines(&registry);
        assert_eq!(
            engines.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["5.5", "{ABC}"]
        );
    }

    #[test]
    fn scans_engine_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("Engine").join("Plugins");
        let write = |relative: &str, text: &str| {
            let path = plugins.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "Runtime/ChaosVehiclesPlugin/ChaosVehiclesPlugin.uplugin",
            r#"{ "EnabledByDefault": false }"#,
        );
        write(
            "Editor/ModelingTools/ModelingTools.uplugin",
            r#"{ "EnabledByDefault": true }"#,
        );
        write("Editor/ModelingTools/Nested/Inner.uplugin", "{}");
        write("Experimental/Broken/Broken.uplugin", "not json");
        let scanned = EnginePlugins::scan(dir.path());
        assert_eq!(scanned.len(), 3);
        assert_eq!(
            scanned
                .get("chaosvehiclesplugin")
                .unwrap()
                .enabled_by_default,
            Some(false)
        );
        assert_eq!(
            scanned.get("ModelingTools").unwrap().enabled_by_default,
            Some(true)
        );
        assert_eq!(scanned.get("Broken").unwrap().enabled_by_default, None);
        assert!(
            scanned.get("Inner").is_none(),
            "no recursion below a plugin"
        );
    }
}

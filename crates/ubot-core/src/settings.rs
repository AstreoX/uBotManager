//! Settings (SPEC §6): `<config>/settings.json` with defaults for missing keys and unknown keys
//! preserved, plus the per-project ROS bridge address in `Config/DefaultUBotROS.ini`.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::model::{
    BuildConfiguration, BuildSettings, IndexSettings, IndexSourceConfig, InstallSettings, Lang,
    Recipe, RosSettings, Settings, Theme,
};
use crate::paths::{self, AppDirs};

pub const REMOTE_INDEX_URL: &str =
    "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json";
pub const LOCAL_INDEX_LOCATION: &str = "<UBotCore>/Index/index.json";
pub const REMOTE_SOURCE_ID: &str = "ubot";
pub const LOCAL_SOURCE_ID: &str = "local";
pub const DEFAULT_INSTALL_DIRECTORY: &str = "Plugins/uBot";
pub const DEFAULT_ROS_HOST: &str = "127.0.0.1";
pub const DEFAULT_ROS_PORT: u16 = 8268;
const MAX_RECENT_PROJECTS: usize = 10;

pub fn builtin_sources() -> Vec<IndexSourceConfig> {
    vec![
        IndexSourceConfig {
            id: REMOTE_SOURCE_ID.into(),
            name: "uBotCore".into(),
            location: REMOTE_INDEX_URL.into(),
            enabled: true,
            builtin: true,
        },
        IndexSourceConfig {
            id: LOCAL_SOURCE_ID.into(),
            name: "Local copy".into(),
            location: LOCAL_INDEX_LOCATION.into(),
            enabled: true,
            builtin: true,
        },
    ]
}

impl Default for IndexSettings {
    fn default() -> Self {
        IndexSettings {
            sources: builtin_sources(),
            sync_on_start: true,
            show_planned: true,
        }
    }
}

impl Default for InstallSettings {
    fn default() -> Self {
        InstallSettings {
            directory: DEFAULT_INSTALL_DIRECTORY.into(),
            git: "git".into(),
            use_prebuilt: true,
        }
    }
}

impl Default for BuildSettings {
    fn default() -> Self {
        BuildSettings {
            engine_id: None,
            configuration: BuildConfiguration::Development,
            wait_for_editor: true,
            reopen_editor: true,
            run_tests: false,
        }
    }
}

impl Default for RosSettings {
    fn default() -> Self {
        RosSettings {
            host: DEFAULT_ROS_HOST.into(),
            port: DEFAULT_ROS_PORT,
            check_on_refresh: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: Lang::En,
            theme: Theme::Dark,
            index: IndexSettings::default(),
            install: InstallSettings::default(),
            build: BuildSettings::default(),
            ros: RosSettings::default(),
            recent_projects: Vec::new(),
            last_project: None,
        }
    }
}

/// zh-CN when the OS UI language is Chinese, else en.
pub fn os_language() -> Lang {
    #[cfg(windows)]
    {
        // SAFETY: GetUserDefaultUILanguage takes no arguments and only returns a LANGID.
        let lang_id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        const LANG_CHINESE: u16 = 0x04;
        if lang_id & 0x3ff == LANG_CHINESE {
            Lang::ZhCn
        } else {
            Lang::En
        }
    }
    #[cfg(not(windows))]
    {
        let locale = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default();
        if locale.to_ascii_lowercase().starts_with("zh") {
            Lang::ZhCn
        } else {
            Lang::En
        }
    }
}

/// The settings file: typed settings over the raw JSON, which keeps unknown keys.
#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
    raw: Map<String, Value>,
    pub settings: Settings,
    /// Ignored problem ids per absolute `.uproject` path.
    pub ignored_problems: BTreeMap<String, Vec<String>>,
    pub user_recipes: Vec<Recipe>,
    /// Why the settings file existed but could not be used (defaults are in effect). The next
    /// `save` copies the file aside first, so nothing the user wrote is lost.
    pub unreadable: Option<String>,
    /// The copy made by `save` after an unreadable file.
    pub backup: Option<PathBuf>,
}

fn field<T: DeserializeOwned>(raw: &Map<String, Value>, key: &str, default: T) -> T {
    raw.get(key)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or(default)
}

/// Keeps the elements of an array that parse as `T`.
fn keep_valid_elements<T: DeserializeOwned>(value: &mut Value) {
    if let Value::Array(elements) = value {
        elements.retain(|element| serde_json::from_value::<T>(element.clone()).is_ok());
    }
}

/// A settings section read key by key: one wrongly typed value only resets its own key, not its
/// siblings (a team index source survives a bad `syncOnStart`). `prepare` may clean up values
/// that hold lists before they are checked.
fn section<T: Serialize + DeserializeOwned>(
    raw: &Map<String, Value>,
    key: &str,
    default: T,
    prepare: impl Fn(&mut Map<String, Value>),
) -> T {
    let Some(Value::Object(incoming)) = raw.get(key) else {
        return default;
    };
    let Ok(Value::Object(mut merged)) = serde_json::to_value(&default) else {
        return default;
    };
    let mut incoming = incoming.clone();
    prepare(&mut incoming);
    for (name, value) in incoming {
        let mut candidate = merged.clone();
        candidate.insert(name, value);
        if serde_json::from_value::<T>(Value::Object(candidate.clone())).is_ok() {
            merged = candidate;
        }
    }
    serde_json::from_value(Value::Object(merged)).unwrap_or(default)
}

/// `ignoredProblems` entry by entry.
fn ignored_problems(raw: &Map<String, Value>) -> BTreeMap<String, Vec<String>> {
    let Some(Value::Object(entries)) = raw.get("ignoredProblems") else {
        return BTreeMap::new();
    };
    entries
        .iter()
        .filter_map(|(project, ids)| {
            serde_json::from_value::<Vec<String>>(ids.clone())
                .ok()
                .map(|ids| (project.clone(), ids))
        })
        .collect()
}

/// `userRecipes` element by element.
fn user_recipes(raw: &Map<String, Value>) -> Vec<Recipe> {
    let Some(Value::Array(elements)) = raw.get("userRecipes") else {
        return Vec::new();
    };
    elements
        .iter()
        .filter_map(|element| serde_json::from_value::<Recipe>(element.clone()).ok())
        .collect()
}

impl SettingsStore {
    /// Loads `<config>/settings.json`. A missing file yields defaults; the language then follows
    /// the OS UI language. A file that exists but cannot be used (invalid JSON, a UTF-16 file, a
    /// file locked by a sync tool) also yields defaults, but is remembered in `unreadable`, and
    /// `save` copies it aside before it is overwritten.
    pub fn load(dirs: &AppDirs) -> SettingsStore {
        let path = dirs.settings_file();
        let (raw, unreadable) = match std::fs::read(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Map::new(), None),
            Err(error) => (Map::new(), Some(format!("could not be read: {error}"))),
            Ok(bytes) => match paths::decode_text(&bytes)
                .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()))
            {
                Ok(Value::Object(map)) => (map, None),
                Ok(_) => (Map::new(), Some("is not a JSON object".to_string())),
                Err(error) => (Map::new(), Some(format!("is not valid: {error}"))),
            },
        };
        let mut store = SettingsStore::from_raw(path, raw);
        store.unreadable = unreadable;
        store
    }

    pub fn from_raw(path: PathBuf, raw: Map<String, Value>) -> SettingsStore {
        let defaults = Settings::default();
        let language = if raw.contains_key("language") {
            field(&raw, "language", Lang::En)
        } else {
            os_language()
        };
        let mut settings = Settings {
            language,
            theme: field(&raw, "theme", defaults.theme),
            index: section(&raw, "index", defaults.index, |index| {
                if let Some(sources) = index.get_mut("sources") {
                    keep_valid_elements::<IndexSourceConfig>(sources);
                }
            }),
            install: section(&raw, "install", defaults.install, |_| {}),
            build: section(&raw, "build", defaults.build, |_| {}),
            ros: section(&raw, "ros", defaults.ros, |_| {}),
            recent_projects: field(&raw, "recentProjects", defaults.recent_projects),
            last_project: field(&raw, "lastProject", defaults.last_project),
        };
        normalize(&mut settings);
        let ignored_problems = ignored_problems(&raw);
        let mut user_recipes = user_recipes(&raw);
        for recipe in &mut user_recipes {
            recipe.user = true;
        }
        SettingsStore {
            path,
            raw,
            settings,
            ignored_problems,
            user_recipes,
            unreadable: None,
            backup: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Replaces the typed settings (normalized: built-in sources stay, empty values get defaults).
    pub fn set(&mut self, mut settings: Settings) {
        normalize(&mut settings);
        self.settings = settings;
    }

    pub fn save(&mut self) -> io::Result<()> {
        if let Some(reason) = &self.unreadable {
            // Defaults are about to replace a file the user wrote; keep a copy of it.
            match paths::backup_unreadable(&self.path) {
                Ok(backup) => {
                    self.backup = Some(backup);
                    self.unreadable = None;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => self.unreadable = None,
                Err(error) => {
                    return Err(io::Error::other(format!(
                        "{} {reason} and could not be backed up ({error}); fix or delete it",
                        self.path.display()
                    )))
                }
            }
        }
        let typed = serde_json::to_value(&self.settings).map_err(io::Error::other)?;
        merge_into(&mut self.raw, typed);
        self.raw.insert(
            "ignoredProblems".into(),
            serde_json::to_value(&self.ignored_problems).map_err(io::Error::other)?,
        );
        let recipes: Vec<Value> = self
            .user_recipes
            .iter()
            .map(|recipe| {
                let mut value = serde_json::to_value(recipe).unwrap_or(Value::Null);
                if let Some(object) = value.as_object_mut() {
                    object.remove("available");
                    object.insert("user".into(), Value::Bool(true));
                }
                value
            })
            .collect();
        self.raw.insert("userRecipes".into(), Value::Array(recipes));
        let text = crate::index::to_pretty_json(&self.raw);
        paths::write_atomic(&self.path, text.as_bytes())
    }

    pub fn ignored_for(&self, project: &Path) -> Vec<String> {
        let key = project_key(project);
        self.ignored_problems
            .iter()
            .find(|(path, _)| path.eq_ignore_ascii_case(&key))
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default()
    }

    pub fn ignore(&mut self, project: &Path, id: &str) {
        let key = project_key(project);
        let existing = self
            .ignored_problems
            .keys()
            .find(|path| path.eq_ignore_ascii_case(&key))
            .cloned();
        let ids = self
            .ignored_problems
            .entry(existing.unwrap_or(key))
            .or_default();
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }

    /// Moves the project to the front of the recent list and makes it the last project.
    pub fn add_recent(&mut self, project: &Path) {
        let key = project_key(project);
        self.settings
            .recent_projects
            .retain(|recent| !recent.eq_ignore_ascii_case(&key));
        self.settings.recent_projects.insert(0, key.clone());
        self.settings.recent_projects.truncate(MAX_RECENT_PROJECTS);
        self.settings.last_project = Some(key);
    }

    /// Adds or replaces a user recipe (by id).
    pub fn save_user_recipe(&mut self, recipe: Recipe) {
        self.user_recipes
            .retain(|existing| !existing.id.eq_ignore_ascii_case(&recipe.id));
        self.user_recipes.push(Recipe {
            user: true,
            ..recipe
        });
    }
}

pub fn project_key(project: &Path) -> String {
    paths::display(&paths::absolute(project))
}

fn normalize(settings: &mut Settings) {
    let defaults = builtin_sources();
    let mut sources: Vec<IndexSourceConfig> = Vec::new();
    for source in std::mem::take(&mut settings.index.sources) {
        if let Some(builtin) = defaults.iter().find(|builtin| builtin.id == source.id) {
            if !sources.iter().any(|existing| existing.id == builtin.id) {
                sources.push(builtin.clone());
            }
            continue;
        }
        let location = source.location.trim().to_string();
        if location.is_empty() {
            continue;
        }
        let mut id = source.id.trim().to_string();
        if id.is_empty()
            || sources.iter().any(|existing| existing.id == id)
            || defaults.iter().any(|builtin| builtin.id == id)
        {
            let mut counter = 1;
            while sources
                .iter()
                .any(|existing| existing.id == format!("user-{counter}"))
            {
                counter += 1;
            }
            id = format!("user-{counter}");
        }
        let name = if source.name.trim().is_empty() {
            location.clone()
        } else {
            source.name.trim().to_string()
        };
        sources.push(IndexSourceConfig {
            id,
            name,
            location,
            enabled: source.enabled,
            builtin: false,
        });
    }
    for builtin in defaults {
        if !sources.iter().any(|existing| existing.id == builtin.id) {
            sources.push(builtin);
        }
    }
    settings.index.sources = sources;

    if settings.install.directory.trim().is_empty() {
        settings.install.directory = DEFAULT_INSTALL_DIRECTORY.into();
    }
    if settings.install.git.trim().is_empty() {
        settings.install.git = "git".into();
    }
    if settings.ros.host.trim().is_empty() {
        settings.ros.host = DEFAULT_ROS_HOST.into();
    }
    if settings.ros.port == 0 {
        settings.ros.port = DEFAULT_ROS_PORT;
    }
    if settings
        .build
        .engine_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
    {
        settings.build.engine_id = None;
    }
    let mut seen: Vec<String> = Vec::new();
    settings.recent_projects.retain(|recent| {
        let key = recent.to_lowercase();
        let keep = !recent.trim().is_empty() && !seen.contains(&key);
        seen.push(key);
        keep
    });
    settings.recent_projects.truncate(MAX_RECENT_PROJECTS);
}

/// Objects merge key by key (keeping keys `from` does not have); everything else is replaced.
fn merge_into(target: &mut Map<String, Value>, from: Value) {
    let Value::Object(from) = from else {
        return;
    };
    for (key, value) in from {
        match (target.get_mut(&key), value) {
            (Some(Value::Object(existing)), Value::Object(incoming)) => {
                merge_into(existing, Value::Object(incoming))
            }
            (_, value) => {
                target.insert(key, value);
            }
        }
    }
}

pub const ROS_SECTION: &str = "/Script/UBotROS.UBotROSSettings";
pub const ROS_HOST_KEY: &str = "BridgeHost";
pub const ROS_PORT_KEY: &str = "BridgePort";

pub fn ros_ini_path(project_dir: &Path) -> PathBuf {
    project_dir.join("Config").join("DefaultUBotROS.ini")
}

fn section_name(line: &str) -> Option<&str> {
    let line = line.trim();
    line.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(str::trim)
}

fn key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

/// `BridgeHost` / `BridgePort` from the ini text (quotes around values are removed).
pub fn read_ros_ini_text(text: &str) -> (Option<String>, Option<u16>) {
    let mut in_section = false;
    let (mut host, mut port) = (None, None);
    for line in text.lines() {
        if let Some(section) = section_name(line) {
            in_section = section.eq_ignore_ascii_case(ROS_SECTION);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = key_value(line) {
            let value = value.trim_matches('"');
            if key.eq_ignore_ascii_case(ROS_HOST_KEY) && !value.is_empty() {
                host = Some(value.to_string());
            } else if key.eq_ignore_ascii_case(ROS_PORT_KEY) {
                port = value.parse().ok();
            }
        }
    }
    (host, port)
}

pub fn read_ros_ini(project_dir: &Path) -> (Option<String>, Option<u16>) {
    paths::read_text(&ros_ini_path(project_dir))
        .map(|(text, _)| read_ros_ini_text(&text))
        .unwrap_or((None, None))
}

/// Sets `BridgeHost` / `BridgePort`, touching only those lines; creates the section when missing.
pub fn write_ros_ini_text(text: &str, host: &str, port: u16) -> String {
    let newline = if text.contains("\r\n") || text.is_empty() {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let host_line = format!("{ROS_HOST_KEY}={host}");
    let port_line = format!("{ROS_PORT_KEY}={port}");

    let section_start = lines.iter().position(|line| {
        section_name(line).is_some_and(|name| name.eq_ignore_ascii_case(ROS_SECTION))
    });
    match section_start {
        None => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{ROS_SECTION}]"));
            lines.push(host_line);
            lines.push(port_line);
        }
        Some(start) => {
            let end = lines[start + 1..]
                .iter()
                .position(|line| section_name(line).is_some())
                .map_or(lines.len(), |offset| start + 1 + offset);
            let (mut has_host, mut has_port) = (false, false);
            for line in &mut lines[start + 1..end] {
                match key_value(line) {
                    Some((key, _)) if key.eq_ignore_ascii_case(ROS_HOST_KEY) => {
                        *line = host_line.clone();
                        has_host = true;
                    }
                    Some((key, _)) if key.eq_ignore_ascii_case(ROS_PORT_KEY) => {
                        *line = port_line.clone();
                        has_port = true;
                    }
                    _ => {}
                }
            }
            // New keys go after the last non-empty line of the section.
            let mut insert_at = end;
            while insert_at > start + 1 && lines[insert_at - 1].trim().is_empty() {
                insert_at -= 1;
            }
            if !has_port {
                lines.insert(insert_at, port_line);
            }
            if !has_host {
                lines.insert(insert_at, host_line);
            }
        }
    }
    let mut result = lines.join(newline);
    if text.is_empty() || text.ends_with('\n') {
        result.push_str(newline);
    }
    result
}

pub fn write_ros_ini(project_dir: &Path, host: &str, port: u16) -> io::Result<()> {
    let path = ros_ini_path(project_dir);
    let (text, bom) = match paths::read_text(&path) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => (String::new(), false),
        Err(error) => return Err(error),
    };
    let mut updated = write_ros_ini_text(&text, host, port);
    if bom {
        updated.insert(0, '\u{feff}');
    }
    paths::write_atomic(&path, updated.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path().to_path_buf());
        let store = SettingsStore::load(&dirs);
        assert_eq!(store.settings.index.sources, builtin_sources());
        assert_eq!(store.settings.install.directory, "Plugins/uBot");
        assert_eq!(store.settings.ros.port, 8268);
        assert!(store.settings.index.sync_on_start);

        std::fs::write(
            dirs.settings_file(),
            json!({
                "language": "zh-CN",
                "theme": "light",
                "future": { "keep": true },
                "index": { "sources": [
                    { "id": "local", "name": "Renamed", "location": "x", "enabled": false, "builtin": true },
                    { "id": "", "name": "", "location": "D:/team/index.json", "enabled": true },
                    { "id": "ubot", "name": "x", "location": "y", "enabled": false, "builtin": true }
                ], "syncOnStart": false, "extra": 1 },
                "build": { "configuration": "DebugGame", "engineId": "" },
                "ros": { "port": "not a number" },
                "ignoredProblems": { "C:\\P\\P.uproject": [ "legacy:A" ] },
                "userRecipes": [ { "id": "mine", "name": { "en": "Mine" }, "description": { "en": "" }, "packages": [ "UBotCore" ] } ]
            })
            .to_string(),
        )
        .unwrap();
        let mut store = SettingsStore::load(&dirs);
        assert_eq!(store.settings.language, Lang::ZhCn);
        assert_eq!(store.settings.theme, Theme::Light);
        let ids: Vec<&str> = store
            .settings
            .index
            .sources
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["local", "user-1", "ubot"],
            "built-ins can be reordered but not changed or removed"
        );
        assert!(store
            .settings
            .index
            .sources
            .iter()
            .all(|s| s.enabled || !s.builtin));
        assert_eq!(store.settings.index.sources[0].name, "Local copy");
        assert_eq!(store.settings.index.sources[1].name, "D:/team/index.json");
        assert!(!store.settings.index.sync_on_start);
        assert_eq!(
            store.settings.build.configuration,
            BuildConfiguration::DebugGame
        );
        assert_eq!(store.settings.build.engine_id, None);
        assert_eq!(
            store.settings.ros,
            RosSettings::default(),
            "a malformed section falls back to defaults"
        );
        if cfg!(windows) {
            assert_eq!(
                store.ignored_for(Path::new("c:\\p\\p.uproject")),
                ["legacy:A"]
            );
        }
        assert!(store.user_recipes[0].user);

        store.add_recent(Path::new(&dir.path().join("A.uproject")));
        store.ignore(Path::new("C:\\P\\P.uproject"), "engine");
        store.save().unwrap();
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dirs.settings_file()).unwrap()).unwrap();
        assert_eq!(saved["future"], json!({ "keep": true }));
        assert_eq!(saved["index"]["extra"], 1);
        assert_eq!(
            saved["ignoredProblems"]["C:\\P\\P.uproject"],
            json!(["legacy:A", "engine"])
        );
        assert_eq!(saved["userRecipes"][0]["user"], true);
        assert!(saved["userRecipes"][0].get("available").is_none());
        assert_eq!(saved["recentProjects"].as_array().unwrap().len(), 1);
        assert_eq!(saved["lastProject"], saved["recentProjects"][0]);
        assert_eq!(saved["build"]["engineId"], Value::Null);
    }

    #[test]
    fn one_bad_value_does_not_reset_its_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path().to_path_buf());
        std::fs::write(
            dirs.settings_file(),
            json!({
                "index": { "sources": [
                    { "id": "team", "name": "Team", "location": "https://t/index.json", "enabled": true },
                    { "id": 7 },
                    "junk"
                ], "syncOnStart": "no", "showPlanned": false },
                "install": { "directory": "Plugins/Mine", "git": 5 },
                "build": { "configuration": "Nope", "waitForEditor": false },
                "ignoredProblems": { "C:\\P\\P.uproject": [ "a" ], "bad": 3 },
                "userRecipes": [
                    { "id": "mine", "name": { "en": "Mine" }, "description": { "en": "" }, "packages": [ "UBotCore" ] },
                    { "id": 1 }
                ],
                "future": 1
            })
            .to_string(),
        )
        .unwrap();
        let mut store = SettingsStore::load(&dirs);
        assert!(store.unreadable.is_none());
        let ids: Vec<&str> = store
            .settings
            .index
            .sources
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["team", "ubot", "local"], "the team source survives");
        assert!(store.settings.index.sync_on_start, "bad value: default");
        assert!(!store.settings.index.show_planned, "good sibling kept");
        assert_eq!(store.settings.install.directory, "Plugins/Mine");
        assert_eq!(store.settings.install.git, "git");
        assert!(
            store.settings.install.use_prebuilt,
            "a file without the key gets the default"
        );
        assert_eq!(
            store.settings.build.configuration,
            BuildConfiguration::Development
        );
        assert!(!store.settings.build.wait_for_editor);
        assert_eq!(store.user_recipes.len(), 1);
        assert_eq!(store.ignored_problems.len(), 1);

        store.save().unwrap();
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dirs.settings_file()).unwrap()).unwrap();
        assert_eq!(saved["future"], 1);
        assert_eq!(saved["index"]["sources"][0]["id"], "team");
        assert_eq!(saved["install"]["directory"], "Plugins/Mine");
    }

    #[test]
    fn prebuilt_binaries_are_used_unless_the_file_says_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path().to_path_buf());
        assert!(SettingsStore::load(&dirs).settings.install.use_prebuilt);

        // An existing file keeps its value, also across a save.
        std::fs::write(
            dirs.settings_file(),
            json!({ "install": { "directory": "Plugins/uBot", "usePrebuilt": false } }).to_string(),
        )
        .unwrap();
        let mut store = SettingsStore::load(&dirs);
        assert!(!store.settings.install.use_prebuilt);
        store.save().unwrap();
        assert!(!SettingsStore::load(&dirs).settings.install.use_prebuilt);

        std::fs::write(
            dirs.settings_file(),
            json!({ "install": { "usePrebuilt": "yes" } }).to_string(),
        )
        .unwrap();
        assert!(
            SettingsStore::load(&dirs).settings.install.use_prebuilt,
            "a value of the wrong type takes the default"
        );
    }

    #[test]
    fn an_unreadable_settings_file_is_backed_up_before_it_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path().to_path_buf());
        let hand_edited =
            "{\n  \"language\": \"zh-CN\",\n  \"userRecipes\": [ { \"id\": \"mine\" } ],\n}\n";
        std::fs::write(dirs.settings_file(), hand_edited).unwrap();

        let mut store = SettingsStore::load(&dirs);
        assert!(store.unreadable.as_deref().unwrap().contains("not valid"));
        assert_eq!(store.settings.index.sources, builtin_sources());
        store.add_recent(Path::new(&dir.path().join("A.uproject")));
        store.save().unwrap();
        let backup = store.backup.clone().unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), hand_edited);
        assert!(store.unreadable.is_none());
        // The replaced file is valid now and is not backed up again.
        store.save().unwrap();
        assert_eq!(store.backup.as_deref(), Some(backup.as_path()));
        assert!(SettingsStore::load(&dirs).unreadable.is_none());

        // UTF-16 (Notepad's "Unicode") is read, not discarded.
        let utf16: Vec<u8> = [0xFFu8, 0xFE]
            .into_iter()
            .chain(
                "{\"theme\":\"light\"}"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        std::fs::write(dirs.settings_file(), utf16).unwrap();
        let store = SettingsStore::load(&dirs);
        assert!(store.unreadable.is_none());
        assert_eq!(store.settings.theme, Theme::Light);

        // A root that is not an object is unreadable too.
        std::fs::write(dirs.settings_file(), "[1, 2]").unwrap();
        assert!(SettingsStore::load(&dirs).unreadable.is_some());
    }

    #[test]
    fn ros_ini_editing() {
        let original = "; comment\r\n[/Script/Engine.Engine]\r\nBridgeHost=other\r\n\r\n[/Script/UBotROS.UBotROSSettings]\r\nBridgeHost=\"10.0.0.2\"\r\nOther=1\r\n\r\n[Next]\r\nA=B\r\n";
        assert_eq!(read_ros_ini_text(original), (Some("10.0.0.2".into()), None));
        let updated = write_ros_ini_text(original, "192.168.1.5", 9000);
        assert_eq!(
            updated,
            "; comment\r\n[/Script/Engine.Engine]\r\nBridgeHost=other\r\n\r\n[/Script/UBotROS.UBotROSSettings]\r\nBridgeHost=192.168.1.5\r\nOther=1\r\nBridgePort=9000\r\n\r\n[Next]\r\nA=B\r\n"
        );
        assert_eq!(
            read_ros_ini_text(&updated),
            (Some("192.168.1.5".into()), Some(9000))
        );

        assert_eq!(
            write_ros_ini_text("[A]\nx=1", "h", 1),
            "[A]\nx=1\n\n[/Script/UBotROS.UBotROSSettings]\nBridgeHost=h\nBridgePort=1"
        );
        assert_eq!(
            write_ros_ini_text("", "h", 2),
            "[/Script/UBotROS.UBotROSSettings]\r\nBridgeHost=h\r\nBridgePort=2\r\n"
        );

        let dir = tempfile::tempdir().unwrap();
        write_ros_ini(dir.path(), "127.0.0.2", 8270).unwrap();
        assert_eq!(
            read_ros_ini(dir.path()),
            (Some("127.0.0.2".into()), Some(8270))
        );
    }
}

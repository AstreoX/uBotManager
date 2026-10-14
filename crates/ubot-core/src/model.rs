//! Data types shared with the frontend (SPEC §7). The TypeScript declarations in
//! `app/src/api/types.ts` are the reference: field names are camelCase, enums serialize to the exact
//! string literals, `Option` is always serialized (`null`), and unions are tagged by `kind`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum Layer {
    Foundation,
    Capability,
    Adapter,
    Content,
    #[default]
    Unknown,
}

impl Layer {
    /// Case-insensitive, whitespace-tolerant parse; anything else is `None`.
    pub fn parse(text: &str) -> Option<Layer> {
        match text.trim().to_ascii_lowercase().as_str() {
            "foundation" => Some(Layer::Foundation),
            "capability" => Some(Layer::Capability),
            "adapter" => Some(Layer::Adapter),
            "content" => Some(Layer::Content),
            "unknown" => Some(Layer::Unknown),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Layer::Foundation => "Foundation",
            Layer::Capability => "Capability",
            Layer::Adapter => "Adapter",
            Layer::Content => "Content",
            Layer::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedText {
    pub en: String,
    #[serde(rename = "zh-CN", default)]
    pub zh_cn: Option<String>,
}

impl LocalizedText {
    pub fn en(text: impl Into<String>) -> Self {
        LocalizedText {
            en: text.into(),
            zh_cn: None,
        }
    }
}

/// Message for the UI: `code` is a key under `msg.` in the locale files; params are plain strings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Msg {
    pub code: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineKind {
    Launcher,
    Source,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub id: String,
    pub version: String,
    pub root: String,
    pub kind: EngineKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorState {
    pub running: bool,
    pub pid: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    pub dir: String,
    pub engine_association: String,
    pub engine: Option<EngineInfo>,
    pub has_code: bool,
    pub build_target: String,
    pub editor: EditorState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageState {
    Enabled,
    Disabled,
    NotInstalled,
    Planned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequirementKind {
    Package,
    EnginePlugin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequirementStatus {
    Satisfied,
    VersionMismatch,
    NotInstalled,
    Disabled,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub name: String,
    pub constraint: String,
    pub optional: bool,
    pub kind: RequirementKind,
    pub status: RequirementStatus,
    pub installed_version: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitState {
    pub is_checkout: bool,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub dirty: bool,
}

/// How an installed package got into the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    /// A git checkout (`.git` exists).
    Git,
    /// Prebuilt binaries installed by the manager (a valid `.ubot-install.json` marker).
    Prebuilt,
    /// Neither: a folder copied in by hand, a zip, a shared project.
    Other,
}

/// What the install marker of a prebuilt package says it was built for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrebuiltInfo {
    /// `Major.Minor` of the engine the binaries were built for ("5.5").
    pub engine: String,
    /// "Win64".
    pub platform: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub name: String,
    pub friendly_name: String,
    pub description: LocalizedText,
    pub layer: Layer,
    pub state: PackageState,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    /// Non-planned index versions, newest first.
    pub versions: Vec<String>,
    pub planned_version: Option<String>,
    pub update_available: bool,
    /// Project-relative with '/' separators ("Plugins/uBot/uBotSensor", "../Plugins/uBot/uBotCore").
    pub location: Option<String>,
    /// `None` when not installed.
    pub git: Option<GitState>,
    /// `None` when not installed.
    pub install_kind: Option<InstallKind>,
    /// An install or update of this package would use prebuilt binaries for this project now.
    pub prebuilt_available: bool,
    /// The engine and platform the installed binaries were built for, from the install marker:
    /// set exactly when `install_kind` is `prebuilt`, `None` otherwise.
    pub prebuilt: Option<PrebuiltInfo>,
    pub repository: Option<String>,
    pub web_url: Option<String>,
    pub docs_url: Option<String>,
    /// Id of the index source that defined the package.
    pub index_source: Option<String>,
    pub requires: Vec<Requirement>,
    pub provides: Vec<String>,
    pub tags: Vec<String>,
    pub problem_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub name: String,
    pub enabled: bool,
    pub location: String,
    pub version: Option<String>,
    #[serde(rename = "isUBot")]
    pub is_ubot: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSourceStatus {
    pub id: String,
    pub name: String,
    pub location: String,
    pub builtin: bool,
    pub enabled: bool,
    pub ok: bool,
    pub from_cache: bool,
    pub error: Option<String>,
    pub packages: u32,
    /// RFC 3339.
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    pub name: LocalizedText,
    pub description: LocalizedText,
    pub packages: Vec<String>,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub user: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsSection {
    Appearance,
    Index,
    Install,
    Build,
    Ros,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemFact {
    pub label: Msg,
    pub value: String,
    pub value_msg: Option<Msg>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FixAction {
    Changes { changes: Vec<Change> },
    Ignore,
    OpenSettings { section: SettingsSection },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub label: Msg,
    pub primary: bool,
    pub action: FixAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProblemScope {
    Packages,
    Session,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub id: String,
    pub scope: ProblemScope,
    pub group: String,
    pub severity: Severity,
    pub message: Msg,
    pub facts: Vec<ProblemFact>,
    pub fixes: Vec<Fix>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKind {
    #[serde(rename = "PIE")]
    Pie,
    Editor,
    Game,
}

/// Parsed `Saved/uBot/status.json` (SPEC §9.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub kind: SessionKind,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub engine_version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub project: Option<ProjectInfo>,
    /// Index + installed packages, sorted by layer order, then name.
    pub packages: Vec<PackageInfo>,
    /// Every discovered project plugin.
    pub plugins: Vec<PluginInfo>,
    /// Ignored problems removed.
    pub problems: Vec<Problem>,
    pub index: Vec<IndexSourceStatus>,
    /// Index recipes, then user recipes.
    pub recipes: Vec<Recipe>,
    pub session: Option<SessionStatus>,
    pub syncing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Install,
    Update,
    Enable,
    Disable,
    Uninstall,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Install => "install",
            ChangeKind::Update => "update",
            ChangeKind::Enable => "enable",
            ChangeKind::Disable => "disable",
            ChangeKind::Uninstall => "uninstall",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub kind: ChangeKind,
    pub package: String,
    #[serde(default)]
    pub version: Option<String>,
}

impl Change {
    pub fn new(kind: ChangeKind, package: impl Into<String>, version: Option<String>) -> Self {
        Change {
            kind,
            package: package.into(),
            version,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepGroup {
    Download,
    Project,
    Build,
    After,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    Clone,
    Checkout,
    Download,
    EnablePackage,
    DisablePackage,
    EnableEnginePlugin,
    RemovePackage,
    Build,
    RunTests,
    OpenEditor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub id: String,
    pub n: u32,
    pub group: StepGroup,
    pub kind: StepKind,
    pub title: Msg,
    pub detail: Msg,
    pub needs_editor_closed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOptions {
    pub build: bool,
    pub reopen_editor: bool,
    pub run_tests: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub changes: Vec<Change>,
    pub steps: Vec<PlanStep>,
    pub needs_build: bool,
    pub editor_running: bool,
    pub errors: Vec<Msg>,
    pub warnings: Vec<Msg>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Pending,
    Waiting,
    Running,
    Done,
    Failed,
    Skipped,
    Canceled,
}

impl StepStatus {
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            StepStatus::Done | StepStatus::Failed | StepStatus::Skipped | StepStatus::Canceled
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStep {
    pub id: String,
    pub title: Msg,
    pub status: StepStatus,
    pub started_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub progress: Option<Progress>,
    pub log_lines: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: Msg,
    pub project: String,
    /// Empty for task history written before project paths were recorded.
    #[serde(default)]
    pub project_path: String,
    pub status: StepStatus,
    pub started_at: String,
    pub duration_ms: Option<u64>,
    pub steps: Vec<TaskStep>,
    pub error: Option<String>,
    pub changes: Vec<Change>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildConfiguration {
    #[default]
    Development,
    DebugGame,
}

impl BuildConfiguration {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildConfiguration::Development => "Development",
            BuildConfiguration::DebugGame => "DebugGame",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSourceConfig {
    pub id: String,
    pub name: String,
    pub location: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IndexSettings {
    pub sources: Vec<IndexSourceConfig>,
    pub sync_on_start: bool,
    pub show_planned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InstallSettings {
    pub directory: String,
    pub git: String,
    pub use_prebuilt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BuildSettings {
    pub engine_id: Option<String>,
    pub configuration: BuildConfiguration,
    pub wait_for_editor: bool,
    pub reopen_editor: bool,
    pub run_tests: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RosSettings {
    pub host: String,
    pub port: u16,
    pub check_on_refresh: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub language: Lang,
    pub theme: Theme,
    pub index: IndexSettings,
    pub install: InstallSettings,
    pub build: BuildSettings,
    pub ros: RosSettings,
    pub recent_projects: Vec<String>,
    pub last_project: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvCheckId {
    Git,
    Vs2022,
    Engine,
    RosBridge,
}

impl EnvCheckId {
    pub fn parse(text: &str) -> Option<EnvCheckId> {
        match text {
            "git" => Some(EnvCheckId::Git),
            "vs2022" => Some(EnvCheckId::Vs2022),
            "engine" => Some(EnvCheckId::Engine),
            "rosBridge" => Some(EnvCheckId::RosBridge),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvCheck {
    pub id: EnvCheckId,
    pub ok: Option<bool>,
    pub detail: Option<String>,
}

/// Result of `test_git`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitTestResult {
    pub ok: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

/// Result of `test_ros`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RosTestResult {
    pub ok: bool,
    pub error: Option<String>,
}

/// Payload of the `ubot://task-log` event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLogEvent {
    pub task_id: String,
    pub step_id: String,
    pub lines: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn keys(value: &Value) -> Vec<String> {
        value.as_object().expect("object").keys().cloned().collect()
    }

    fn msg(code: &str) -> Msg {
        Msg {
            code: code.into(),
            params: BTreeMap::from([("package".to_string(), "UBotCore".to_string())]),
        }
    }

    #[test]
    fn task_without_project_path_loads_with_empty_path() {
        let task: Task = serde_json::from_str(
            r#"{
                "id": "legacy",
                "title": { "code": "task.apply", "params": { "project": "P" } },
                "project": "P",
                "status": "done",
                "startedAt": "2026-10-09T15:20:04Z",
                "durationMs": 112000,
                "steps": [],
                "error": null,
                "changes": []
            }"#,
        )
        .unwrap();
        assert_eq!(task.id, "legacy");
        assert_eq!(task.project, "P");
        assert!(task.project_path.is_empty());
    }

    #[test]
    fn snapshot_matches_the_typescript_contract() {
        let package = PackageInfo {
            name: "UBotSensor".into(),
            friendly_name: "uBot Sensor".into(),
            description: LocalizedText {
                en: "Sensors".into(),
                zh_cn: Some("传感器".into()),
            },
            layer: Layer::Capability,
            state: PackageState::NotInstalled,
            installed_version: None,
            latest_version: Some("0.1.0".into()),
            versions: vec!["0.1.0".into()],
            planned_version: None,
            update_available: false,
            location: None,
            git: Some(GitState {
                is_checkout: true,
                git_ref: Some("v0.1.0".into()),
                dirty: false,
            }),
            install_kind: Some(InstallKind::Git),
            prebuilt_available: false,
            prebuilt: None,
            repository: None,
            web_url: None,
            docs_url: None,
            index_source: Some("ubot".into()),
            requires: vec![Requirement {
                name: "UBotCore".into(),
                constraint: "^0.1.0".into(),
                optional: false,
                kind: RequirementKind::EnginePlugin,
                status: RequirementStatus::VersionMismatch,
                installed_version: None,
            }],
            provides: vec![],
            tags: vec![],
            problem_ids: vec![],
        };
        let snapshot = Snapshot {
            project: Some(ProjectInfo {
                path: "C:/P/P.uproject".into(),
                name: "P".into(),
                dir: "C:/P".into(),
                engine_association: "5.5".into(),
                engine: Some(EngineInfo {
                    id: "5.5".into(),
                    version: "5.5.4".into(),
                    root: "D:/UE".into(),
                    kind: EngineKind::Launcher,
                }),
                has_code: false,
                build_target: "PEditor".into(),
                editor: EditorState {
                    running: false,
                    pid: None,
                },
            }),
            packages: vec![package],
            plugins: vec![PluginInfo {
                name: "X".into(),
                enabled: true,
                location: "Plugins/X".into(),
                version: None,
                is_ubot: false,
            }],
            problems: vec![Problem {
                id: "legacy:X".into(),
                scope: ProblemScope::Session,
                group: "P.uproject".into(),
                severity: Severity::Warning,
                message: msg("problem.legacyConflict"),
                facts: vec![ProblemFact {
                    label: msg("fact.legacy"),
                    value: "X 1.0".into(),
                    value_msg: None,
                }],
                fixes: vec![
                    Fix {
                        label: msg("fix.disable"),
                        primary: true,
                        action: FixAction::Changes {
                            changes: vec![Change::new(ChangeKind::Disable, "X", None)],
                        },
                    },
                    Fix {
                        label: msg("fix.ignore"),
                        primary: false,
                        action: FixAction::Ignore,
                    },
                    Fix {
                        label: msg("fix.openRosSettings"),
                        primary: false,
                        action: FixAction::OpenSettings {
                            section: SettingsSection::Ros,
                        },
                    },
                ],
            }],
            index: vec![IndexSourceStatus {
                id: "ubot".into(),
                name: "uBotCore".into(),
                location: "https://x".into(),
                builtin: true,
                enabled: true,
                ok: true,
                from_cache: false,
                error: None,
                packages: 3,
                fetched_at: None,
            }],
            recipes: vec![Recipe {
                id: "r".into(),
                name: LocalizedText::en("R"),
                description: LocalizedText::en("D"),
                packages: vec![],
                checks: vec![],
                available: true,
                user: false,
            }],
            session: Some(SessionStatus {
                kind: SessionKind::Pie,
                started_at: "a".into(),
                ended_at: None,
                engine_version: "5.5.4".into(),
            }),
            syncing: false,
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            keys(&value),
            [
                "project", "packages", "plugins", "problems", "index", "recipes", "session",
                "syncing"
            ]
        );

        let project = &value["project"];
        assert_eq!(
            keys(project),
            [
                "path",
                "name",
                "dir",
                "engineAssociation",
                "engine",
                "hasCode",
                "buildTarget",
                "editor"
            ]
        );
        assert_eq!(project["engine"]["kind"], "launcher");
        assert_eq!(project["editor"], json!({ "running": false, "pid": null }));

        let package = &value["packages"][0];
        assert_eq!(
            keys(package),
            [
                "name",
                "friendlyName",
                "description",
                "layer",
                "state",
                "installedVersion",
                "latestVersion",
                "versions",
                "plannedVersion",
                "updateAvailable",
                "location",
                "git",
                "installKind",
                "prebuiltAvailable",
                "prebuilt",
                "repository",
                "webUrl",
                "docsUrl",
                "indexSource",
                "requires",
                "provides",
                "tags",
                "problemIds"
            ]
        );
        assert_eq!(
            package["description"],
            json!({ "en": "Sensors", "zh-CN": "传感器" })
        );
        assert_eq!(package["layer"], "Capability");
        assert_eq!(package["state"], "notInstalled");
        assert_eq!(package["installedVersion"], Value::Null);
        assert_eq!(
            package["git"],
            json!({ "isCheckout": true, "ref": "v0.1.0", "dirty": false })
        );
        assert_eq!(package["installKind"], "git");
        assert_eq!(package["prebuiltAvailable"], false);
        assert_eq!(package["prebuilt"], Value::Null);
        assert_eq!(
            serde_json::to_value(PrebuiltInfo {
                engine: "5.5".into(),
                platform: "Win64".into()
            })
            .unwrap(),
            json!({ "engine": "5.5", "platform": "Win64" })
        );
        for (kind, literal) in [
            (InstallKind::Git, "git"),
            (InstallKind::Prebuilt, "prebuilt"),
            (InstallKind::Other, "other"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), literal);
        }
        assert_eq!(
            keys(&package["requires"][0]),
            [
                "name",
                "constraint",
                "optional",
                "kind",
                "status",
                "installedVersion"
            ]
        );
        assert_eq!(package["requires"][0]["kind"], "enginePlugin");
        assert_eq!(package["requires"][0]["status"], "versionMismatch");

        assert_eq!(
            keys(&value["plugins"][0]),
            ["name", "enabled", "location", "version", "isUBot"]
        );

        let problem = &value["problems"][0];
        assert_eq!(
            keys(problem),
            ["id", "scope", "group", "severity", "message", "facts", "fixes"]
        );
        assert_eq!(problem["scope"], "session");
        assert_eq!(problem["severity"], "warning");
        assert_eq!(
            problem["message"],
            json!({ "code": "problem.legacyConflict", "params": { "package": "UBotCore" } })
        );
        assert_eq!(keys(&problem["facts"][0]), ["label", "value", "valueMsg"]);
        assert_eq!(
            problem["fixes"][0]["action"],
            json!({ "kind": "changes", "changes": [ { "kind": "disable", "package": "X", "version": null } ] })
        );
        assert_eq!(problem["fixes"][1]["action"], json!({ "kind": "ignore" }));
        assert_eq!(
            problem["fixes"][2]["action"],
            json!({ "kind": "openSettings", "section": "ros" })
        );

        assert_eq!(
            keys(&value["index"][0]),
            [
                "id",
                "name",
                "location",
                "builtin",
                "enabled",
                "ok",
                "fromCache",
                "error",
                "packages",
                "fetchedAt"
            ]
        );
        assert_eq!(
            keys(&value["recipes"][0]),
            [
                "id",
                "name",
                "description",
                "packages",
                "checks",
                "available",
                "user"
            ]
        );
        assert_eq!(
            value["session"],
            json!({ "kind": "PIE", "startedAt": "a", "endedAt": null, "engineVersion": "5.5.4" })
        );
    }

    #[test]
    fn plan_and_task_match_the_typescript_contract() {
        let plan = Plan {
            changes: vec![Change::new(
                ChangeKind::Uninstall,
                "A",
                Some("1.0.0".into()),
            )],
            steps: vec![PlanStep {
                id: "s1".into(),
                n: 1,
                group: StepGroup::Download,
                kind: StepKind::EnableEnginePlugin,
                title: msg("step.clone"),
                detail: msg("step.clone.detail"),
                needs_editor_closed: false,
            }],
            needs_build: true,
            editor_running: false,
            errors: vec![],
            warnings: vec![],
        };
        let value = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            keys(&value),
            [
                "changes",
                "steps",
                "needsBuild",
                "editorRunning",
                "errors",
                "warnings"
            ]
        );
        assert_eq!(
            value["changes"][0],
            json!({ "kind": "uninstall", "package": "A", "version": "1.0.0" })
        );
        assert_eq!(
            keys(&value["steps"][0]),
            [
                "id",
                "n",
                "group",
                "kind",
                "title",
                "detail",
                "needsEditorClosed"
            ]
        );
        assert_eq!(value["steps"][0]["group"], "download");
        assert_eq!(value["steps"][0]["kind"], "enableEnginePlugin");

        for (kind, literal) in [
            (StepKind::Clone, "clone"),
            (StepKind::Checkout, "checkout"),
            (StepKind::Download, "download"),
            (StepKind::EnablePackage, "enablePackage"),
            (StepKind::DisablePackage, "disablePackage"),
            (StepKind::RemovePackage, "removePackage"),
            (StepKind::Build, "build"),
            (StepKind::RunTests, "runTests"),
            (StepKind::OpenEditor, "openEditor"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), literal);
        }

        let task = Task {
            id: "t1".into(),
            title: msg("task.apply"),
            project: "P".into(),
            project_path: r"E:\Projects\P\P.uproject".into(),
            status: StepStatus::Canceled,
            started_at: "now".into(),
            duration_ms: None,
            steps: vec![TaskStep {
                id: "s1".into(),
                title: msg("step.build"),
                status: StepStatus::Waiting,
                started_at: None,
                duration_ms: Some(5),
                progress: Some(Progress { done: 1, total: 2 }),
                log_lines: 3,
            }],
            error: None,
            changes: vec![],
        };
        let value = serde_json::to_value(&task).unwrap();
        assert_eq!(
            keys(&value),
            [
                "id",
                "title",
                "project",
                "projectPath",
                "status",
                "startedAt",
                "durationMs",
                "steps",
                "error",
                "changes"
            ]
        );
        assert_eq!(value["projectPath"], r"E:\Projects\P\P.uproject");
        assert_eq!(value["status"], "canceled");
        assert_eq!(
            keys(&value["steps"][0]),
            [
                "id",
                "title",
                "status",
                "startedAt",
                "durationMs",
                "progress",
                "logLines"
            ]
        );
        assert_eq!(
            value["steps"][0]["progress"],
            json!({ "done": 1, "total": 2 })
        );
        assert_eq!(value["steps"][0]["status"], "waiting");

        let options: ApplyOptions = serde_json::from_value(
            json!({ "build": true, "reopenEditor": false, "runTests": true }),
        )
        .unwrap();
        assert!(options.build && options.run_tests && !options.reopen_editor);
    }

    #[test]
    fn settings_and_env_match_the_typescript_contract() {
        let settings = Settings {
            language: Lang::ZhCn,
            theme: Theme::System,
            index: IndexSettings {
                sources: vec![IndexSourceConfig {
                    id: "ubot".into(),
                    name: "uBotCore".into(),
                    location: "x".into(),
                    enabled: true,
                    builtin: true,
                }],
                sync_on_start: true,
                show_planned: false,
            },
            install: InstallSettings {
                directory: "Plugins/uBot".into(),
                git: "git".into(),
                use_prebuilt: false,
            },
            build: BuildSettings {
                engine_id: None,
                configuration: BuildConfiguration::DebugGame,
                wait_for_editor: true,
                reopen_editor: true,
                run_tests: false,
            },
            ros: RosSettings {
                host: "127.0.0.1".into(),
                port: 8268,
                check_on_refresh: true,
            },
            recent_projects: vec![],
            last_project: None,
        };
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            keys(&value),
            [
                "language",
                "theme",
                "index",
                "install",
                "build",
                "ros",
                "recentProjects",
                "lastProject"
            ]
        );
        assert_eq!(value["language"], "zh-CN");
        assert_eq!(value["theme"], "system");
        assert_eq!(
            keys(&value["index"]),
            ["sources", "syncOnStart", "showPlanned"]
        );
        assert_eq!(
            keys(&value["index"]["sources"][0]),
            ["id", "name", "location", "enabled", "builtin"]
        );
        assert_eq!(keys(&value["install"]), ["directory", "git", "usePrebuilt"]);
        assert_eq!(
            keys(&value["build"]),
            [
                "engineId",
                "configuration",
                "waitForEditor",
                "reopenEditor",
                "runTests"
            ]
        );
        assert_eq!(value["build"]["configuration"], "DebugGame");
        assert_eq!(value["build"]["engineId"], Value::Null);
        assert_eq!(keys(&value["ros"]), ["host", "port", "checkOnRefresh"]);
        assert_eq!(value["lastProject"], Value::Null);

        let check = EnvCheck {
            id: EnvCheckId::Vs2022,
            ok: None,
            detail: None,
        };
        assert_eq!(
            serde_json::to_value(&check).unwrap(),
            json!({ "id": "vs2022", "ok": null, "detail": null })
        );
        assert_eq!(
            serde_json::to_value(EnvCheckId::RosBridge).unwrap(),
            "rosBridge"
        );
        assert_eq!(
            serde_json::to_value(GitTestResult {
                ok: true,
                version: None,
                error: None
            })
            .unwrap(),
            json!({ "ok": true, "version": null, "error": null })
        );
        assert_eq!(
            serde_json::to_value(RosTestResult {
                ok: false,
                error: Some("x".into())
            })
            .unwrap(),
            json!({ "ok": false, "error": "x" })
        );
        assert_eq!(
            serde_json::to_value(TaskLogEvent {
                task_id: "t".into(),
                step_id: "s".into(),
                lines: vec![]
            })
            .unwrap(),
            json!({ "taskId": "t", "stepId": "s", "lines": [] })
        );
        for (state, literal) in [
            (PackageState::Enabled, "enabled"),
            (PackageState::Disabled, "disabled"),
            (PackageState::Planned, "planned"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), literal);
        }
        for (status, literal) in [
            (StepStatus::Pending, "pending"),
            (StepStatus::Running, "running"),
            (StepStatus::Done, "done"),
            (StepStatus::Failed, "failed"),
            (StepStatus::Skipped, "skipped"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), literal);
        }
        for (section, literal) in [
            (SettingsSection::Appearance, "appearance"),
            (SettingsSection::Index, "index"),
            (SettingsSection::Install, "install"),
            (SettingsSection::Build, "build"),
        ] {
            assert_eq!(serde_json::to_value(section).unwrap(), literal);
        }
        assert_eq!(serde_json::to_value(Severity::Info).unwrap(), "info");
        assert_eq!(serde_json::to_value(Severity::Error).unwrap(), "error");
        assert_eq!(serde_json::to_value(Lang::En).unwrap(), "en");
        assert_eq!(serde_json::to_value(Layer::Unknown).unwrap(), "Unknown");
        assert_eq!(serde_json::to_value(SessionKind::Editor).unwrap(), "Editor");
        assert_eq!(serde_json::to_value(StepGroup::After).unwrap(), "after");
        assert_eq!(
            serde_json::to_value(RequirementStatus::NotInstalled).unwrap(),
            "notInstalled"
        );
        assert_eq!(serde_json::to_value(EngineKind::Custom).unwrap(), "custom");
        assert_eq!(
            serde_json::to_value(ChangeKind::Install).unwrap(),
            "install"
        );
    }
}

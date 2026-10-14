//! The JSON contract with the frontend (SPEC §7). Sample values of everything the commands and
//! events carry, covering every enum literal, are serialized by the Rust model and written to
//! `app/src/api/contract.fixture.ts` as constants typed with `app/src/api/types.ts`. `npm test`
//! (contract.test.ts) and `npm run build` type-check that file, so a key or literal that differs
//! on either side fails: TypeScript rejects missing keys, extra keys (excess property checks on
//! the object literals) and unknown literals.
//!
//! When the model changes, regenerate the fixture with
//! `UBOT_UPDATE_CONTRACT=1 cargo test -p ubot-manager contract`, then run `npm test` in `app/`.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::Serialize;
use ubot_core::model::{
    ApplyOptions, BuildConfiguration, BuildSettings, Change, ChangeKind, EditorState, EngineInfo,
    EngineKind, EnvCheck, EnvCheckId, Fix, FixAction, GitState, GitTestResult, IndexSettings,
    IndexSourceConfig, IndexSourceStatus, InstallKind, InstallSettings, Lang, Layer, LocalizedText,
    Msg, PackageInfo, PackageState, Plan, PlanStep, PluginInfo, PrebuiltInfo, Problem, ProblemFact,
    ProblemScope, Progress, ProjectInfo, Recipe, Requirement, RequirementKind, RequirementStatus,
    RosSettings, RosTestResult, SessionKind, SessionStatus, Settings, SettingsSection, Severity,
    Snapshot, StepGroup, StepKind, StepStatus, Task, TaskLogEvent, TaskStep, Theme,
};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../app/src/api/contract.fixture.ts"
);
const UPDATE_ENV: &str = "UBOT_UPDATE_CONTRACT";

/// Every variant of a field-less enum. The `match` is exhaustive on purpose: a new variant does
/// not compile until it is listed here, so the fixture keeps covering every literal.
macro_rules! variants {
    ($ty:ident: $($variant:ident),+ $(,)?) => {{
        fn _exhaustive(value: $ty) {
            match value {
                $($ty::$variant)|+ => {}
            }
        }
        [$($ty::$variant),+]
    }};
}

fn msg(code: &str, params: &[(&str, &str)]) -> Msg {
    Msg {
        code: code.into(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn text(en: &str, zh_cn: Option<&str>) -> LocalizedText {
    LocalizedText {
        en: en.into(),
        zh_cn: zh_cn.map(Into::into),
    }
}

fn engines() -> Vec<EngineInfo> {
    variants!(EngineKind: Launcher, Source, Custom)
        .into_iter()
        .enumerate()
        .map(|(position, kind)| EngineInfo {
            id: [
                "5.5",
                "{8A1E5B5C-0000-4000-8000-000000000000}",
                "D:/UE/Custom",
            ][position]
                .into(),
            version: "5.5.4".into(),
            root: [
                "D:/Program Files/Epic Games/UE_5.5",
                "D:/UE/Source",
                "D:/UE/Custom",
            ][position]
                .into(),
            kind,
        })
        .collect()
}

fn changes() -> Vec<Change> {
    variants!(ChangeKind: Install, Update, Enable, Disable, Uninstall)
        .into_iter()
        .map(|kind| {
            let version = matches!(kind, ChangeKind::Install | ChangeKind::Update)
                .then(|| "0.2.0".to_string());
            Change::new(kind, "UBotSensor", version)
        })
        .collect()
}

fn packages() -> Vec<PackageInfo> {
    let layers = variants!(Layer: Foundation, Capability, Adapter, Content, Unknown);
    let states = variants!(PackageState: Enabled, Disabled, NotInstalled, Planned);
    let requirement_kinds = variants!(RequirementKind: Package, EnginePlugin);
    let install_kinds = variants!(InstallKind: Git, Prebuilt, Other);
    let requirement_statuses =
        variants!(RequirementStatus: Satisfied, VersionMismatch, NotInstalled, Disabled, Missing);
    let requires: Vec<Requirement> = requirement_statuses
        .into_iter()
        .enumerate()
        .map(|(position, status)| Requirement {
            name: format!("Dependency{position}"),
            constraint: "^0.1.0".into(),
            optional: position % 2 == 1,
            kind: requirement_kinds[position % requirement_kinds.len()],
            status,
            installed_version: (status != RequirementStatus::NotInstalled).then(|| "0.1.0".into()),
        })
        .collect();
    layers
        .into_iter()
        .enumerate()
        .map(|(position, layer)| {
            let state = states[position % states.len()];
            let installed = matches!(state, PackageState::Enabled | PackageState::Disabled);
            PackageInfo {
                name: format!("UBotPackage{position}"),
                friendly_name: format!("uBot Package {position}"),
                description: text(
                    "A package.",
                    (position % 2 == 0).then_some("一个 Package。"),
                ),
                layer,
                state,
                installed_version: installed.then(|| "0.1.0".into()),
                latest_version: (state != PackageState::Planned).then(|| "0.2.0".into()),
                versions: if state == PackageState::Planned {
                    Vec::new()
                } else {
                    vec!["0.2.0".into(), "0.1.0".into()]
                },
                planned_version: (state == PackageState::Planned).then(|| "0.3.0".into()),
                update_available: installed,
                location: installed.then(|| format!("Plugins/uBot/uBotPackage{position}")),
                git: installed.then(|| GitState {
                    is_checkout: install_kinds[position % install_kinds.len()] == InstallKind::Git,
                    git_ref: (position == 0).then(|| "v0.1.0".into()),
                    dirty: position == 1,
                }),
                // Installed: every kind across the list (the fifth package is git again).
                install_kind: installed.then(|| install_kinds[position % install_kinds.len()]),
                prebuilt_available: matches!(position, 1 | 2),
                // Set exactly for the prebuilt installs; the second one is for another engine.
                prebuilt: (installed
                    && install_kinds[position % install_kinds.len()] == InstallKind::Prebuilt)
                    .then(|| PrebuiltInfo {
                        engine: if position == 1 { "5.5" } else { "5.4" }.into(),
                        platform: "Win64".into(),
                    }),
                repository: Some(format!(
                    "https://github.com/AstreoX/uBotPackage{position}.git"
                )),
                web_url: Some(format!("https://github.com/AstreoX/uBotPackage{position}")),
                docs_url: None,
                index_source: Some("ubot".into()),
                requires: if position == 0 {
                    requires.clone()
                } else {
                    Vec::new()
                },
                provides: vec!["Sensor.IMU".into()],
                tags: vec!["sensor".into()],
                problem_ids: if position == 0 {
                    vec!["requires:UBotPackage0:Dependency1".into()]
                } else {
                    Vec::new()
                },
            }
        })
        .collect()
}

fn fix_actions() -> Vec<FixAction> {
    let changes = vec![Change::new(
        ChangeKind::Update,
        "UBotCore",
        Some("0.2.0".into()),
    )];
    let mut actions = vec![FixAction::Changes { changes }, FixAction::Ignore];
    actions.extend(
        variants!(SettingsSection: Appearance, Index, Install, Build, Ros)
            .into_iter()
            .map(|section| FixAction::OpenSettings { section }),
    );
    for action in &actions {
        // Exhaustive on purpose, like `variants!`.
        match action {
            FixAction::Changes { .. } | FixAction::Ignore | FixAction::OpenSettings { .. } => {}
        }
    }
    actions
}

fn problems() -> Vec<Problem> {
    let severities = variants!(Severity: Error, Warning, Info);
    let scopes = variants!(ProblemScope: Packages, Session);
    let mut fixes = fix_actions()
        .into_iter()
        .enumerate()
        .map(|(position, action)| Fix {
            label: msg("fix.ignore", &[]),
            primary: position == 0,
            action,
        });
    severities
        .into_iter()
        .enumerate()
        .map(|(position, severity)| Problem {
            id: format!("problem-{position}"),
            scope: scopes[position % scopes.len()],
            group: "UBotPackage0".into(),
            severity,
            message: msg(
                "problem.requiresVersion",
                &[
                    ("package", "UBotPackage0"),
                    ("dep", "UBotCore"),
                    ("range", "^0.2.0"),
                    ("installed", "0.1.0"),
                ],
            ),
            facts: vec![
                ProblemFact {
                    label: msg("fact.package", &[]),
                    value: "UBotPackage0 0.1.0".into(),
                    value_msg: None,
                },
                ProblemFact {
                    label: msg("fact.time", &[]),
                    value: "2026-10-10T08:00:00Z".into(),
                    value_msg: Some(msg("session.lastPie", &[])),
                },
            ],
            // The last problem takes the remaining fixes, so every action kind is covered.
            fixes: if position + 1 < severities.len() {
                fixes.by_ref().take(2).collect()
            } else {
                fixes.by_ref().collect()
            },
        })
        .collect()
}

fn sessions() -> Vec<SessionStatus> {
    variants!(SessionKind: Pie, Editor, Game)
        .into_iter()
        .enumerate()
        .map(|(position, kind)| SessionStatus {
            kind,
            started_at: "2026-10-10T08:00:00Z".into(),
            ended_at: (position != 1).then(|| "2026-10-10T08:05:00Z".into()),
            engine_version: "5.5.4".into(),
        })
        .collect()
}

fn snapshots() -> Vec<Snapshot> {
    let full = Snapshot {
        project: Some(ProjectInfo {
            path: "E:/Sims/MyRobotSim/MyRobotSim.uproject".into(),
            name: "MyRobotSim".into(),
            dir: "E:/Sims/MyRobotSim".into(),
            engine_association: "5.5".into(),
            engine: engines().into_iter().next(),
            has_code: false,
            build_target: "MyRobotSimEditor".into(),
            editor: EditorState {
                running: true,
                pid: Some(4242),
            },
        }),
        packages: packages(),
        plugins: vec![
            PluginInfo {
                name: "UBotCore".into(),
                enabled: true,
                location: "Plugins/uBot/uBotCore".into(),
                version: Some("0.1.0".into()),
                is_ubot: true,
            },
            PluginInfo {
                name: "AgentSensorCore".into(),
                enabled: false,
                location: "Plugins/AgentSensorCore".into(),
                version: None,
                is_ubot: false,
            },
        ],
        problems: problems(),
        index: vec![
            IndexSourceStatus {
                id: "ubot".into(),
                name: "uBotCore".into(),
                location:
                    "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json"
                        .into(),
                builtin: true,
                enabled: true,
                ok: false,
                from_cache: true,
                error: Some("HTTP 404 Not Found".into()),
                packages: 9,
                fetched_at: Some("2026-10-10T08:00:00Z".into()),
            },
            IndexSourceStatus {
                id: "local".into(),
                name: "Local copy".into(),
                location: "Plugins/uBot/uBotCore/Index/index.json".into(),
                builtin: true,
                enabled: true,
                ok: true,
                from_cache: false,
                error: None,
                packages: 9,
                fetched_at: None,
            },
        ],
        recipes: vec![
            Recipe {
                id: "ros-sensors".into(),
                name: text("ROS 2 sensors", Some("ROS 2 传感器")),
                description: text("Sensors published to ROS 2.", None),
                packages: vec!["UBotCore".into(), "UBotSensor".into(), "UBotROS".into()],
                checks: vec!["rosBridge".into()],
                available: true,
                user: false,
            },
            Recipe {
                id: "mine".into(),
                name: text("Mine", None),
                description: text("", None),
                packages: vec!["UBotCore".into()],
                checks: Vec::new(),
                available: false,
                user: true,
            },
        ],
        session: sessions().into_iter().next(),
        syncing: true,
    };
    vec![full, Snapshot::default()]
}

fn plans() -> Vec<Plan> {
    let groups = variants!(StepGroup: Download, Project, Build, After);
    let kinds = variants!(
        StepKind: Clone,
        Checkout,
        Download,
        EnablePackage,
        DisablePackage,
        EnableEnginePlugin,
        RemovePackage,
        Build,
        RunTests,
        OpenEditor
    );
    let steps = kinds
        .into_iter()
        .enumerate()
        .map(|(position, kind)| PlanStep {
            id: format!("step-{}", position + 1),
            n: position as u32 + 1,
            group: groups[position * groups.len() / kinds.len()],
            kind,
            title: if kind == StepKind::Download {
                msg(
                    "step.download",
                    &[("package", "UBotSensor"), ("version", "0.2.0")],
                )
            } else {
                msg(
                    "step.clone",
                    &[("package", "UBotSensor"), ("version", "0.2.0")],
                )
            },
            detail: if kind == StepKind::Download {
                msg(
                    "step.download.detail",
                    &[
                        ("source", "github.com/…/UBotSensor-0.2.0-UE5.5-Win64.zip"),
                        ("dest", "Plugins/uBot/uBotSensor"),
                    ],
                )
            } else {
                msg(
                    "step.clone.detail",
                    &[
                        ("repo", "https://github.com/AstreoX/uBotSensor.git"),
                        ("dest", "Plugins/uBot/uBotSensor"),
                    ],
                )
            },
            needs_editor_closed: !matches!(kind, StepKind::Clone | StepKind::Download),
        })
        .collect();
    vec![
        Plan {
            changes: changes(),
            steps,
            needs_build: true,
            editor_running: true,
            errors: vec![msg(
                "plan.breaksDependent",
                &[
                    ("package", "UBotCore"),
                    ("dependent", "UBotSensor"),
                    ("range", "^0.1.0"),
                ],
            )],
            warnings: vec![
                msg(
                    "plan.onBranch",
                    &[("package", "UBotCore"), ("branch", "main")],
                ),
                msg(
                    "plan.prebuiltUnavailable",
                    &[
                        ("package", "UBotROS"),
                        ("version", "0.1.0"),
                        ("engine", "5.5"),
                    ],
                ),
                msg("plan.noCompiler", &[]),
            ],
        },
        Plan::default(),
    ]
}

fn tasks() -> Vec<Task> {
    let statuses =
        variants!(StepStatus: Pending, Waiting, Running, Done, Failed, Skipped, Canceled);
    let steps: Vec<TaskStep> = statuses
        .into_iter()
        .enumerate()
        .map(|(position, status)| TaskStep {
            id: format!("step-{}", position + 1),
            title: msg("step.build", &[("target", "MyRobotSimEditor")]),
            status,
            started_at: (status != StepStatus::Pending).then(|| "2026-10-10T08:00:00Z".into()),
            duration_ms: status.is_finished().then_some(1500),
            progress: (status == StepStatus::Running).then_some(Progress {
                done: 15,
                total: 24,
            }),
            log_lines: position as u32 * 10,
        })
        .collect();
    // A download step reports bytes.
    let mut first_steps = steps.clone();
    first_steps.push(TaskStep {
        id: "step-download".into(),
        title: msg(
            "step.download",
            &[("package", "UBotSensor"), ("version", "0.2.0")],
        ),
        status: StepStatus::Running,
        started_at: Some("2026-10-10T08:00:00Z".into()),
        duration_ms: None,
        progress: Some(Progress {
            done: 12_300_000,
            total: 45_600_000,
        }),
        log_lines: 1,
    });
    // The first task carries a step in every status; the others one step each.
    statuses
        .into_iter()
        .enumerate()
        .map(|(position, status)| Task {
            id: format!("task-{status:?}").to_lowercase(),
            title: msg("task.apply", &[("project", "MyRobotSim")]),
            project: "E:/Sims/MyRobotSim/MyRobotSim.uproject".into(),
            project_path: "E:/Sims/MyRobotSim/MyRobotSim.uproject".into(),
            status,
            started_at: "2026-10-10T08:00:00Z".into(),
            duration_ms: status.is_finished().then_some(64_000),
            steps: if position == 0 {
                first_steps.clone()
            } else {
                vec![steps[position].clone()]
            },
            error: (status == StepStatus::Failed).then(|| "Build failed.".into()),
            changes: changes(),
        })
        .collect()
}

fn settings() -> Vec<Settings> {
    let languages = variants!(Lang: En, ZhCn);
    let configurations = variants!(BuildConfiguration: Development, DebugGame);
    variants!(Theme: Dark, Light, System)
        .into_iter()
        .enumerate()
        .map(|(position, theme)| Settings {
            language: languages[position % languages.len()],
            theme,
            index: IndexSettings {
                sources: vec![
                    IndexSourceConfig {
                        id: "ubot".into(),
                        name: "uBotCore".into(),
                        location:
                            "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json"
                                .into(),
                        enabled: true,
                        builtin: true,
                    },
                    IndexSourceConfig {
                        id: "user-1".into(),
                        name: "Team mirror".into(),
                        location: "//server/share/index.json".into(),
                        enabled: position != 1,
                        builtin: false,
                    },
                ],
                sync_on_start: position == 0,
                show_planned: true,
            },
            install: InstallSettings {
                directory: "Plugins/uBot".into(),
                git: "git".into(),
                use_prebuilt: position != 1,
            },
            build: BuildSettings {
                engine_id: (position == 1).then(|| "5.5".into()),
                configuration: configurations[position % configurations.len()],
                wait_for_editor: true,
                reopen_editor: position != 2,
                run_tests: position == 2,
            },
            ros: RosSettings {
                host: "127.0.0.1".into(),
                port: 8268,
                check_on_refresh: true,
            },
            recent_projects: vec!["E:/Sims/MyRobotSim/MyRobotSim.uproject".into()],
            last_project: (position == 0).then(|| "E:/Sims/MyRobotSim/MyRobotSim.uproject".into()),
        })
        .collect()
}

fn env_checks() -> Vec<EnvCheck> {
    variants!(EnvCheckId: Git, Vs2022, Engine, RosBridge)
        .into_iter()
        .map(|id| EnvCheck {
            id,
            ok: match id {
                EnvCheckId::Git | EnvCheckId::Engine => Some(true),
                EnvCheckId::Vs2022 => Some(false),
                EnvCheckId::RosBridge => None,
            },
            detail: (id != EnvCheckId::RosBridge).then(|| "git version 2.47.0".into()),
        })
        .collect()
}

fn git_tests() -> Vec<GitTestResult> {
    vec![
        GitTestResult {
            ok: true,
            version: Some("2.47.0".into()),
            error: None,
        },
        GitTestResult {
            ok: false,
            version: None,
            error: Some("program not found".into()),
        },
    ]
}

fn ros_tests() -> Vec<RosTestResult> {
    vec![
        RosTestResult {
            ok: true,
            error: None,
        },
        RosTestResult {
            ok: false,
            error: Some("connection refused".into()),
        },
    ]
}

fn task_log_events() -> Vec<TaskLogEvent> {
    vec![TaskLogEvent {
        task_id: "task-1".into(),
        step_id: "step-1".into(),
        lines: vec!["[15/24] Compile Module.UBotSensor.cpp".into()],
    }]
}

/// One exported constant per type: (TypeScript name, TypeScript type, JSON).
fn entries() -> Vec<(&'static str, &'static str, String)> {
    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string_pretty(value).expect("the model serializes")
    }
    vec![
        ("snapshots", "Snapshot[]", json(&snapshots())),
        ("sessions", "SessionStatus[]", json(&sessions())),
        ("plans", "Plan[]", json(&plans())),
        ("tasks", "Task[]", json(&tasks())),
        ("taskLogEvents", "TaskLogEvent[]", json(&task_log_events())),
        ("settings", "Settings[]", json(&settings())),
        ("engines", "EngineInfo[]", json(&engines())),
        ("envChecks", "EnvCheck[]", json(&env_checks())),
        ("changes", "Change[]", json(&changes())),
        (
            "applyOptions",
            "ApplyOptions",
            json(&ApplyOptions {
                build: true,
                reopen_editor: true,
                run_tests: false,
            }),
        ),
        ("gitTests", "GitTestResult[]", json(&git_tests())),
        ("rosTests", "RosTestResult[]", json(&ros_tests())),
    ]
}

fn render_fixture() -> String {
    let mut out = String::from(
        "// Generated from the Rust model (crates/ubot-core/src/model.rs) by the `contract` test of src-tauri.\n\
         // Do not edit: run `UBOT_UPDATE_CONTRACT=1 cargo test -p ubot-manager contract` instead.\n\
         // contract.test.ts type-checks this file against types.ts (SPEC §7).\n\n\
         import type { GitTestResult, RosTestResult, TaskLogEvent } from \"./backend\";\n\
         import type {\n  ApplyOptions, Change, EngineInfo, EnvCheck, Plan, SessionStatus, Settings, Snapshot, Task\n\
         } from \"./types\";\n",
    );
    for (name, ty, json) in entries() {
        out.push_str(&format!("\nexport const {name}: {ty} = {json};\n"));
    }
    out
}

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(values: Vec<T>) {
    for value in values {
        let json = serde_json::to_value(&value).unwrap();
        let back: T = serde_json::from_value(json).unwrap();
        assert_eq!(back, value);
    }
}

#[test]
fn contract_fixture_is_current() {
    let expected = render_fixture();
    let current = std::fs::read_to_string(FIXTURE)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if current == expected {
        return;
    }
    if std::env::var_os(UPDATE_ENV).is_some() {
        std::fs::write(FIXTURE, expected).expect("writing the contract fixture");
        return;
    }
    panic!(
        "{FIXTURE} does not match the Rust model. Regenerate it with \
         `{UPDATE_ENV}=1 cargo test -p ubot-manager contract` and run `npm test` in app/."
    );
}

/// What the frontend sends back (changes, options, settings, titles) is read by the same model.
#[test]
fn the_frontend_shapes_read_back() {
    round_trip(snapshots());
    round_trip(plans());
    round_trip(tasks());
    round_trip(settings());
    round_trip(changes());
    round_trip(engines());
    round_trip(env_checks());
    round_trip(vec![ApplyOptions {
        build: false,
        reopen_editor: true,
        run_tests: true,
    }]);
    round_trip(vec![msg(
        "task.recipe",
        &[("recipe", "ROS 2 sensors"), ("project", "P")],
    )]);
}

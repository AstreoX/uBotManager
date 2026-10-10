//! Problems (SPEC §4), computed from a [`Workspace`] on every refresh.

use crate::graph;
use crate::model::{
    Change, ChangeKind, Fix, FixAction, Msg, PackageInfo, Problem, ProblemFact, ProblemScope,
    SessionKind, SettingsSection, Severity,
};
use crate::msg::{is_known_code, msg, msg0, verbatim};
use crate::url::redact_credentials;
use crate::version::{Constraint, SemVer};
use crate::workspace::{PluginPresence, Workspace};

pub const STATUS_FILE_DISPLAY: &str = "Saved/uBot/status.json";

/// The `group` of problems about the package indexes. The frontend translates this fixed name.
pub const INDEX_GROUP: &str = "Package Index";

/// A problem together with the package it is about. `Problem.group` is display text (a project
/// file, "Package Index", a package name) that can collide with a package name, so the package a
/// problem belongs to is tracked explicitly and `problemIds` of a package come from here.
#[derive(Clone, Debug)]
pub struct ComputedProblem {
    pub problem: Problem,
    /// `None` for problems about the project, the engine, the indexes or the whole session.
    pub package: Option<String>,
}

impl ComputedProblem {
    fn general(problem: Problem) -> Self {
        ComputedProblem {
            problem,
            package: None,
        }
    }

    fn of_package(package: &str, problem: Problem) -> Self {
        ComputedProblem {
            problem,
            package: Some(package.to_string()),
        }
    }
}

fn fact(label: &str, value: impl Into<String>) -> ProblemFact {
    ProblemFact {
        label: msg0(label),
        value: value.into(),
        value_msg: None,
    }
}

fn fixes(list: Vec<(Msg, FixAction)>) -> Vec<Fix> {
    list.into_iter()
        .enumerate()
        .map(|(position, (label, action))| Fix {
            label,
            primary: position == 0,
            action,
        })
        .collect()
}

fn changes(kind: ChangeKind, package: &str, version: Option<&str>) -> FixAction {
    FixAction::Changes {
        changes: vec![Change::new(kind, package, version.map(str::to_string))],
    }
}

fn problem(
    id: String,
    group: String,
    severity: Severity,
    message: Msg,
    facts: Vec<ProblemFact>,
    fixes: Vec<Fix>,
) -> Problem {
    Problem {
        id,
        scope: ProblemScope::Packages,
        group,
        severity,
        message,
        facts,
        fixes,
    }
}

fn range_text(constraint: &str) -> String {
    let trimmed = constraint.trim();
    if trimmed.is_empty() {
        "*".into()
    } else {
        trimmed.to_string()
    }
}

fn with_version(name: &str, version: Option<&str>) -> String {
    match version {
        Some(version) if !version.is_empty() => format!("{name} {version}"),
        _ => name.to_string(),
    }
}

/// Every problem, before ignored ones are removed.
pub fn compute(workspace: &Workspace) -> Vec<ComputedProblem> {
    let mut problems = Vec::new();
    let Some(context) = &workspace.project else {
        index_problems(workspace, &mut problems);
        return problems;
    };
    let project = &context.project;
    let project_group = project.file_name();

    if context.engine.is_none() {
        let association = project.uproject.engine_association();
        problems.push(ComputedProblem::general(problem(
            "engine".into(),
            project_group.clone(),
            Severity::Error,
            msg(
                "problem.engineNotFound",
                &[(
                    "association",
                    if association.is_empty() {
                        "(none)"
                    } else {
                        &association
                    },
                )],
            ),
            Vec::new(),
            fixes(vec![(
                msg0("fix.openBuildSettings"),
                FixAction::OpenSettings {
                    section: SettingsSection::Build,
                },
            )]),
        )));
    }
    index_problems(workspace, &mut problems);

    let mut packages: Vec<_> = workspace.ubot_plugins();
    packages.sort_by_key(|plugin| workspace.display_name(&plugin.name).to_lowercase());
    for plugin in &packages {
        let name = workspace.display_name(&plugin.name);
        if let Some(detail) = plugin.issues() {
            problems.push(ComputedProblem::of_package(
                &name,
                problem(
                    format!("descriptor:{name}"),
                    name.clone(),
                    Severity::Warning,
                    msg(
                        "problem.descriptorInvalid",
                        &[("package", &name), ("detail", &detail)],
                    ),
                    vec![fact("fact.file", project.location(&plugin.descriptor_path))],
                    Vec::new(),
                ),
            ));
        }
        if !workspace.is_enabled(plugin) {
            continue;
        }
        let mut found = Vec::new();
        requirement_problems(workspace, &name, plugin.version(), &mut found);
        problems.extend(
            found
                .into_iter()
                .map(|found| ComputedProblem::of_package(&name, found)),
        );
    }

    cycle_problems(workspace, &mut problems);
    legacy_problems(workspace, &project_group, &mut problems);

    if let Some(ros) = context.ros.as_ref().filter(|ros| !ros.reachable) {
        let ros_package = workspace.display_name("UBotROS");
        problems.push(ComputedProblem::of_package(
            &ros_package,
            problem(
                "ros:bridge".into(),
                ros_package.clone(),
                Severity::Info,
                msg("problem.rosBridgeUnreachable", &[("address", &ros.address)]),
                vec![fact("fact.address", ros.address.clone())],
                fixes(vec![(
                    msg0("fix.openRosSettings"),
                    FixAction::OpenSettings {
                        section: SettingsSection::Ros,
                    },
                )]),
            ),
        ));
    }

    session_problems(workspace, &mut problems);
    problems
}

/// One problem per failed index source. They belong to no package, whatever the source is
/// called (the built-in source is named "uBotCore" like a package), and are grouped under
/// [`INDEX_GROUP`].
///
/// A failed source only matters as much as what is lost: `info` when the merged index still has
/// packages (from the source's cache, another source or the embedded copy), `warning` when a
/// source the user added fails without a cache to fall back to, or when no index data is
/// available at all.
fn index_problems(workspace: &Workspace, problems: &mut Vec<ComputedProblem>) {
    let has_index_data = !workspace.index.packages.is_empty();
    for failure in &workspace.source_failures {
        let severity = if failure.from_cache {
            Severity::Info
        } else if !failure.builtin || !has_index_data {
            Severity::Warning
        } else {
            Severity::Info
        };
        problems.push(ComputedProblem::general(problem(
            format!("index:{}", failure.id),
            INDEX_GROUP.to_string(),
            severity,
            msg(
                "problem.indexSourceFailed",
                &[
                    ("source", &redact_credentials(&failure.name)),
                    ("error", &redact_credentials(&failure.error)),
                ],
            ),
            vec![fact("fact.source", failure.location.clone())],
            fixes(vec![(msg0("fix.ignore"), FixAction::Ignore)]),
        )));
    }
}

fn requirement_problems(
    workspace: &Workspace,
    name: &str,
    version: Option<&str>,
    problems: &mut Vec<Problem>,
) {
    let requirements = workspace.requirements(name);
    let package_fact = || fact("fact.package", with_version(name, version));
    let disable_fix = || {
        (
            msg("fix.disable", &[("package", name)]),
            changes(ChangeKind::Disable, name, None),
        )
    };

    for dependency in &requirements.packages {
        let dep = workspace.display_name(&dependency.name);
        let range = range_text(&dependency.version);
        let id = format!("requires:{name}:{dep}");
        let requires_fact = || {
            fact(
                "fact.requires",
                if dependency.version.trim().is_empty() {
                    dep.clone()
                } else {
                    format!("{dep} {range}")
                },
            )
        };
        let constraint = Constraint::parse(&dependency.version).ok();

        match workspace.installed(&dependency.name) {
            None => {
                if dependency.optional {
                    continue;
                }
                let mut list = Vec::new();
                if workspace.index.is_available(&dependency.name) {
                    list.push((
                        msg("fix.install", &[("package", &dep)]),
                        changes(ChangeKind::Install, &dep, None),
                    ));
                }
                list.push(disable_fix());
                problems.push(problem(
                    id,
                    name.to_string(),
                    Severity::Warning,
                    msg(
                        "problem.requiresMissing",
                        &[("package", name), ("dep", &dep), ("range", &range)],
                    ),
                    vec![package_fact(), requires_fact()],
                    fixes(list),
                ));
            }
            Some(target) => {
                let installed = target.version();
                let satisfied = match &constraint {
                    Some(constraint) if !constraint.is_any() => installed
                        .and_then(SemVer::parse)
                        .is_some_and(|version| constraint.is_satisfied_by(&version)),
                    _ => true,
                };
                if !satisfied {
                    let mut list = Vec::new();
                    let candidate = constraint.as_ref().and_then(|constraint| {
                        workspace
                            .index
                            .package(&dependency.name)
                            .and_then(|package| {
                                package
                                    .released_versions()
                                    .into_iter()
                                    .find(|(_, version)| constraint.is_satisfied_by(version))
                                    .map(|(v, _)| v.version.clone())
                            })
                    });
                    if let Some(version) = &candidate {
                        list.push((
                            msg("fix.updateTo", &[("package", &dep), ("version", version)]),
                            changes(ChangeKind::Update, &dep, Some(version)),
                        ));
                    }
                    list.push(disable_fix());
                    let installed_text = with_version(&dep, installed);
                    problems.push(problem(
                        id,
                        name.to_string(),
                        Severity::Warning,
                        msg(
                            "problem.requiresVersion",
                            &[
                                ("package", name),
                                ("dep", &dep),
                                ("range", &range),
                                ("installed", &installed_text),
                            ],
                        ),
                        vec![
                            package_fact(),
                            requires_fact(),
                            fact("fact.installed", installed_text.clone()),
                        ],
                        fixes(list),
                    ));
                } else if !dependency.optional && !workspace.is_enabled(target) {
                    problems.push(problem(
                        id,
                        name.to_string(),
                        Severity::Warning,
                        msg(
                            "problem.requiresDisabled",
                            &[("package", name), ("dep", &dep)],
                        ),
                        vec![package_fact(), requires_fact()],
                        fixes(vec![
                            (
                                msg("fix.enable", &[("package", &dep)]),
                                changes(ChangeKind::Enable, &dep, None),
                            ),
                            disable_fix(),
                        ]),
                    ));
                }
            }
        }
    }

    let engine_version = workspace
        .project
        .as_ref()
        .and_then(|context| context.engine.as_ref())
        .map(|engine| engine.version.clone());
    for plugin in &requirements.engine_plugins {
        let id = format!("enginePlugin:{name}:{plugin}");
        match workspace.engine_plugin_presence(plugin) {
            PluginPresence::Missing => problems.push(problem(
                id,
                name.to_string(),
                Severity::Error,
                msg(
                    "problem.enginePluginMissing",
                    &[("package", name), ("plugin", plugin)],
                ),
                vec![
                    package_fact(),
                    fact("fact.plugin", plugin.clone()),
                    fact("fact.engine", engine_version.clone().unwrap_or_default()),
                ],
                fixes(vec![(msg0("fix.ignore"), FixAction::Ignore)]),
            )),
            PluginPresence::Disabled => problems.push(problem(
                id,
                name.to_string(),
                Severity::Warning,
                msg(
                    "problem.enginePluginDisabled",
                    &[("package", name), ("plugin", plugin)],
                ),
                vec![package_fact(), fact("fact.plugin", plugin.clone())],
                fixes(vec![(
                    msg("fix.enable", &[("package", plugin)]),
                    changes(ChangeKind::Enable, plugin, None),
                )]),
            )),
            PluginPresence::Enabled | PluginPresence::Unknown => {}
        }
    }
}

/// One problem per dependency cycle among the enabled packages.
fn cycle_problems(workspace: &Workspace, problems: &mut Vec<ComputedProblem>) {
    let mut names: Vec<String> = workspace
        .ubot_plugins()
        .into_iter()
        .filter(|plugin| workspace.is_enabled(plugin))
        .map(|plugin| workspace.display_name(&plugin.name))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    let position = |name: &str| {
        names
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(name.trim()))
    };
    let edges: Vec<Vec<usize>> = names
        .iter()
        .map(|name| {
            workspace
                .requirements(name)
                .packages
                .iter()
                .filter(|d| !d.optional)
                .filter_map(|d| position(&d.name))
                .collect()
        })
        .collect();
    for cycle in graph::find_cycles(&edges) {
        let first = names[cycle[0]].clone();
        let text: Vec<&str> = cycle.iter().map(|&node| names[node].as_str()).collect();
        problems.push(ComputedProblem::of_package(
            &first,
            problem(
                format!("cycle:{first}"),
                first.clone(),
                Severity::Error,
                msg("problem.dependencyCycle", &[("cycle", &text.join(" -> "))]),
                Vec::new(),
                Vec::new(),
            ),
        ));
    }
}

fn legacy_problems(
    workspace: &Workspace,
    project_group: &str,
    problems: &mut Vec<ComputedProblem>,
) {
    for replacement in &workspace.index.replacements {
        let (Some(legacy), Some(successor)) = (
            workspace.plugin(&replacement.legacy),
            workspace.installed(&replacement.replacement),
        ) else {
            continue;
        };
        if legacy.is_ubot() || !workspace.is_enabled(legacy) || !workspace.is_enabled(successor) {
            continue;
        }
        let successor_name = workspace.display_name(&successor.name);
        let mut facts = vec![
            fact("fact.legacy", with_version(&legacy.name, legacy.version())),
            fact(
                "fact.replacement",
                with_version(&successor_name, successor.version()),
            ),
        ];
        if let Some(redirects) = &replacement.redirects {
            facts.push(fact("fact.redirects", redirects.clone()));
        }
        // The conflict is shown with the package that replaces the legacy plugin.
        problems.push(ComputedProblem::of_package(
            &successor_name,
            problem(
                format!("legacy:{}", legacy.name),
                project_group.to_string(),
                Severity::Warning,
                msg(
                    "problem.legacyConflict",
                    &[("legacy", &legacy.name), ("replacement", &successor_name)],
                ),
                facts,
                fixes(vec![
                    (
                        msg("fix.disable", &[("package", &legacy.name)]),
                        changes(ChangeKind::Disable, &legacy.name, None),
                    ),
                    (msg0("fix.ignore"), FixAction::Ignore),
                ]),
            ),
        ));
    }
}

fn session_problems(workspace: &Workspace, problems: &mut Vec<ComputedProblem>) {
    let Some(session) = workspace
        .project
        .as_ref()
        .and_then(|context| context.session.as_ref())
    else {
        return;
    };
    let time_label = match session.session.kind {
        SessionKind::Pie => "session.lastPie",
        SessionKind::Editor => "session.lastEditor",
        SessionKind::Game => "session.lastGame",
    };
    let time = session
        .session
        .ended_at
        .clone()
        .unwrap_or_else(|| session.session.started_at.clone());

    for entry in &session.problems {
        let code = format!("problem.{}", entry.code);
        let message = if !entry.code.is_empty() && is_known_code(&code) {
            Msg {
                code,
                params: entry.params.clone(),
            }
        } else {
            verbatim(&entry.message)
        };
        let mut facts = vec![fact("fact.source", STATUS_FILE_DISPLAY)];
        for (key, value) in &entry.params {
            let label = format!("fact.{key}");
            if is_known_code(&label) && key != "source" && key != "time" {
                facts.push(fact(&label, value.clone()));
            }
        }
        facts.push(ProblemFact {
            label: msg0("fact.time"),
            value: time.clone(),
            value_msg: Some(msg0(time_label)),
        });
        let fix_list = match entry.code.as_str() {
            "rosBridgeUnreachable" => fixes(vec![(
                msg0("fix.openRosSettings"),
                FixAction::OpenSettings {
                    section: SettingsSection::Ros,
                },
            )]),
            _ => Vec::new(),
        };
        problems.push(ComputedProblem {
            problem: Problem {
                id: format!("session:{}", entry.id),
                scope: ProblemScope::Session,
                group: if entry.package.is_empty() {
                    workspace
                        .project()
                        .map(|p| p.file_name())
                        .unwrap_or_default()
                } else {
                    entry.package.clone()
                },
                severity: entry.severity,
                message,
                facts,
                fixes: fix_list,
            },
            package: Some(entry.package.clone()).filter(|package| !package.is_empty()),
        });
    }
}

/// Removes ignored problems.
pub fn without_ignored(problems: &[ComputedProblem], ignored: &[String]) -> Vec<ComputedProblem> {
    problems
        .iter()
        .filter(|computed| !ignored.iter().any(|id| id == &computed.problem.id))
        .cloned()
        .collect()
}

/// Fills `problemIds` of each package with the problems that belong to it: its own requirement,
/// descriptor and session problems, and the legacy conflicts it resolves. Problems about the
/// indexes, the project or the engine are never attached to a package.
pub fn attach_problem_ids(packages: &mut [PackageInfo], problems: &[ComputedProblem]) {
    for package in packages.iter_mut() {
        package.problem_ids = problems
            .iter()
            .filter(|computed| {
                computed
                    .package
                    .as_deref()
                    .is_some_and(|owner| owner.eq_ignore_ascii_case(&package.name))
            })
            .map(|computed| computed.problem.id.clone())
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FixAction;
    use crate::sources::SourceFailure;
    use crate::status::{SessionFile, SessionProblem};
    use crate::workspace::tests::sample_workspace;
    use crate::workspace::RosCheck;

    /// The problems as the snapshot carries them.
    fn plain(computed: &[ComputedProblem]) -> Vec<Problem> {
        computed.iter().map(|c| c.problem.clone()).collect()
    }

    fn find<'a>(problems: &'a [Problem], id: &str) -> &'a Problem {
        problems
            .iter()
            .find(|problem| problem.id == id)
            .unwrap_or_else(|| {
                panic!(
                    "no problem {id} in {:#?}",
                    problems.iter().map(|p| &p.id).collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn computes_the_problem_table() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = sample_workspace(dir.path(), false);
        let problems = plain(&compute(&workspace));
        let ids: Vec<&str> = problems.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "requires:UBotEnvironment:UBotCore",
                "enginePlugin:UBotEnvironment:Niagara"
            ],
            "UBotSensor is disabled, so the legacy conflict does not apply"
        );

        let version = find(&problems, "requires:UBotEnvironment:UBotCore");
        assert_eq!(version.severity, Severity::Warning);
        assert_eq!(version.group, "UBotEnvironment");
        assert_eq!(version.message.code, "problem.requiresVersion");
        assert_eq!(version.message.params["installed"], "UBotCore 0.1.0");
        assert_eq!(version.message.params["range"], "^0.2.0");
        let facts: Vec<(&str, &str)> = version
            .facts
            .iter()
            .map(|f| (f.label.code.as_str(), f.value.as_str()))
            .collect();
        assert_eq!(
            facts,
            [
                ("fact.package", "UBotEnvironment 0.2.0"),
                ("fact.requires", "UBotCore ^0.2.0"),
                ("fact.installed", "UBotCore 0.1.0")
            ]
        );
        assert_eq!(version.fixes.len(), 2);
        assert!(version.fixes[0].primary && !version.fixes[1].primary);
        assert_eq!(version.fixes[0].label.code, "fix.updateTo");
        assert_eq!(
            version.fixes[0].action,
            FixAction::Changes {
                changes: vec![Change::new(
                    ChangeKind::Update,
                    "UBotCore",
                    Some("0.2.0".into())
                )]
            }
        );
        assert_eq!(
            version.fixes[1].action,
            FixAction::Changes {
                changes: vec![Change::new(ChangeKind::Disable, "UBotEnvironment", None)]
            }
        );

        let niagara = find(&problems, "enginePlugin:UBotEnvironment:Niagara");
        assert_eq!(niagara.message.code, "problem.enginePluginDisabled");
        assert_eq!(
            niagara.fixes[0].action,
            FixAction::Changes {
                changes: vec![Change::new(ChangeKind::Enable, "Niagara", None)]
            }
        );

        // Enable UBotSensor: legacy conflict appears; add more failures.
        let mut workspace_enabled = sample_workspace(&dir.path().join("b"), true);
        {
            let context = workspace_enabled.project.as_mut().unwrap();
            context.engine = None;
            context.engine_plugins = None;
            context.ros = Some(RosCheck {
                address: "127.0.0.1:8268".into(),
                reachable: false,
            });
            context.session = Some(SessionFile {
                session: crate::model::SessionStatus {
                    kind: SessionKind::Pie,
                    started_at: "2026-10-10T07:00:00Z".into(),
                    ended_at: Some("2026-10-10T08:00:00Z".into()),
                    engine_version: "5.5.4".into(),
                },
                problems: vec![
                    SessionProblem {
                        id: "ros.bridge".into(),
                        package: "UBotROS".into(),
                        severity: Severity::Info,
                        code: "rosBridgeUnreachable".into(),
                        params: [
                            ("address".to_string(), "127.0.0.1:8268".to_string()),
                            ("attempts".into(), "3".into()),
                        ]
                        .into(),
                        message: "Could not connect.".into(),
                    },
                    SessionProblem {
                        id: "x".into(),
                        package: String::new(),
                        severity: Severity::Error,
                        code: "somethingNew".into(),
                        params: Default::default(),
                        message: "Raw text.".into(),
                    },
                ],
            });
        }
        workspace_enabled.source_failures = vec![SourceFailure {
            id: "ubot".into(),
            name: "uBotCore".into(),
            location: "https://x".into(),
            error: "HTTP 404".into(),
            from_cache: true,
            builtin: true,
        }];
        let computed = compute(&workspace_enabled);
        let problems = plain(&computed);
        let ids: Vec<&str> = problems.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "engine",
                "index:ubot",
                "requires:UBotEnvironment:UBotCore",
                "legacy:AgentSensorCore",
                "ros:bridge",
                "session:ros.bridge",
                "session:x"
            ]
        );
        let engine = find(&problems, "engine");
        assert_eq!(
            (engine.severity, engine.group.as_str()),
            (Severity::Error, "MyRobotSim.uproject")
        );
        assert_eq!(engine.message.params["association"], "5.5");
        assert_eq!(
            engine.fixes[0].action,
            FixAction::OpenSettings {
                section: SettingsSection::Build
            }
        );

        let index = find(&problems, "index:ubot");
        assert_eq!(
            (index.severity, index.group.as_str()),
            (Severity::Info, INDEX_GROUP)
        );
        assert_eq!(index.facts[0].value, "https://x");

        let legacy = find(&problems, "legacy:AgentSensorCore");
        let facts: Vec<(&str, &str)> = legacy
            .facts
            .iter()
            .map(|f| (f.label.code.as_str(), f.value.as_str()))
            .collect();
        assert_eq!(
            facts,
            [
                ("fact.legacy", "AgentSensorCore 0.5.0"),
                ("fact.replacement", "UBotSensor 0.1.0"),
                ("fact.redirects", "Config/DefaultUBotSensor.ini")
            ]
        );
        assert_eq!(legacy.fixes[1].action, FixAction::Ignore);

        let ros = find(&problems, "ros:bridge");
        assert_eq!(
            (ros.severity, ros.scope, ros.group.as_str()),
            (Severity::Info, ProblemScope::Packages, "UBotROS")
        );

        let session = find(&problems, "session:ros.bridge");
        assert_eq!(session.scope, ProblemScope::Session);
        assert_eq!(session.message.code, "problem.rosBridgeUnreachable");
        let facts: Vec<(&str, &str, Option<&str>)> = session
            .facts
            .iter()
            .map(|f| {
                (
                    f.label.code.as_str(),
                    f.value.as_str(),
                    f.value_msg.as_ref().map(|m| m.code.as_str()),
                )
            })
            .collect();
        assert_eq!(
            facts,
            [
                ("fact.source", "Saved/uBot/status.json", None),
                ("fact.address", "127.0.0.1:8268", None),
                ("fact.time", "2026-10-10T08:00:00Z", Some("session.lastPie"))
            ]
        );
        assert_eq!(
            session.fixes[0].action,
            FixAction::OpenSettings {
                section: SettingsSection::Ros
            }
        );
        let raw = find(&problems, "session:x");
        assert_eq!(raw.message, verbatim("Raw text."));
        assert_eq!(raw.group, "MyRobotSim.uproject");
        assert!(raw.fixes.is_empty());

        let filtered = without_ignored(&computed, &["legacy:AgentSensorCore".into()]);
        assert_eq!(filtered.len(), computed.len() - 1);

        let mut packages = workspace_enabled.package_infos();
        attach_problem_ids(&mut packages, &computed);
        let sensor = packages.iter().find(|p| p.name == "UBotSensor").unwrap();
        assert_eq!(sensor.problem_ids, ["legacy:AgentSensorCore"]);
        let environment = packages
            .iter()
            .find(|p| p.name == "UBotEnvironment")
            .unwrap();
        assert_eq!(
            environment.problem_ids,
            ["requires:UBotEnvironment:UBotCore"]
        );

        // Missing dependency, missing engine plugin, disabled dependency, cycle, descriptor issue.
        let root = dir.path().join("c");
        workspace = sample_workspace(&root, false);
        let plugins = root.join("MyRobotSim").join("Plugins").join("uBot");
        crate::workspace::tests::write(
            &plugins.join("uBotROS/UBotROS.uplugin"),
            &crate::workspace::tests::descriptor(
                "0.1.0",
                "Adapter",
                &[
                    ("UBotCore", "^0.1.0"),
                    ("UBotSensor", "^0.1.0"),
                    ("UBotMaps", ""),
                ],
                &["Paper2D"],
            ),
        );
        crate::workspace::tests::write(
            &plugins.join("loopA/LoopA.uplugin"),
            &crate::workspace::tests::descriptor("1.0.0", "Content", &[("LoopB", "")], &[]),
        );
        crate::workspace::tests::write(
            &plugins.join("loopB/LoopB.uplugin"),
            &crate::workspace::tests::descriptor("bad", "Content", &[("LoopA", "")], &[]),
        );
        workspace
            .project
            .as_mut()
            .unwrap()
            .project
            .reload()
            .unwrap();
        let problems = plain(&compute(&workspace));
        let ros_missing = find(&problems, "requires:UBotROS:UBotMaps");
        assert_eq!(ros_missing.message.code, "problem.requiresMissing");
        assert_eq!(ros_missing.message.params["range"], "*");
        assert_eq!(
            ros_missing.fixes.len(),
            1,
            "UBotMaps is not in the index, so only disabling is offered"
        );
        let disabled = find(&problems, "requires:UBotROS:UBotSensor");
        assert_eq!(disabled.message.code, "problem.requiresDisabled");
        assert_eq!(
            disabled.fixes[0].action,
            FixAction::Changes {
                changes: vec![Change::new(ChangeKind::Enable, "UBotSensor", None)]
            }
        );
        let paper = find(&problems, "enginePlugin:UBotROS:Paper2D");
        assert_eq!(
            (paper.severity, paper.message.code.as_str()),
            (Severity::Error, "problem.enginePluginMissing")
        );
        assert_eq!(paper.facts[2].value, "5.5.4");
        let cycle = find(&problems, "cycle:LoopA");
        assert_eq!(cycle.message.params["cycle"], "LoopA -> LoopB -> LoopA");
        let descriptor = find(&problems, "descriptor:LoopB");
        assert_eq!(
            descriptor.facts[0].value,
            "Plugins/uBot/loopB/LoopB.uplugin"
        );
        assert!(descriptor.message.params["detail"].contains("'bad'"));
    }

    #[test]
    fn index_source_problems_belong_to_no_package() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = sample_workspace(dir.path(), true);
        let failure = |id: &str, name: &str, from_cache: bool, builtin: bool| SourceFailure {
            id: id.into(),
            name: name.into(),
            location: "https://x".into(),
            error: "HTTP 404".into(),
            from_cache,
            builtin,
        };
        // The built-in remote source is called "uBotCore" - like the package.
        workspace.source_failures = vec![
            failure("ubot", "uBotCore", false, true),
            failure("team", "Team", false, false),
            failure("cached", "UBotSensor", true, false),
        ];
        let computed = compute(&workspace);
        let problems = plain(&computed);

        // The merged index still has packages, so a failed built-in source is only information;
        // a source the user added that failed without a cache is a warning.
        let severity = |id: &str| find(&problems, id).severity;
        assert_eq!(severity("index:ubot"), Severity::Info);
        assert_eq!(severity("index:team"), Severity::Warning);
        assert_eq!(severity("index:cached"), Severity::Info);
        for id in ["index:ubot", "index:team", "index:cached"] {
            assert_eq!(find(&problems, id).group, INDEX_GROUP, "{id}");
            assert_eq!(find(&problems, id).scope, ProblemScope::Packages, "{id}");
        }

        // Whatever the source is called, no package picks the problem up.
        let mut packages = workspace.package_infos();
        attach_problem_ids(&mut packages, &computed);
        for package in &packages {
            assert!(
                package
                    .problem_ids
                    .iter()
                    .all(|id| !id.starts_with("index:")),
                "{} got {:?}",
                package.name,
                package.problem_ids
            );
        }
        let core = packages.iter().find(|p| p.name == "UBotCore").unwrap();
        assert!(core.problem_ids.is_empty());

        // Without any index data a failure of the built-in source matters.
        workspace.index = crate::index::MergedIndex::default();
        let problems = plain(&compute(&workspace));
        assert_eq!(find(&problems, "index:ubot").severity, Severity::Warning);

        // Without a project the index problems are still reported.
        workspace.project = None;
        let problems = plain(&compute(&workspace));
        assert_eq!(problems.len(), 3);
    }

    #[test]
    fn index_source_problems_name_no_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = sample_workspace(dir.path(), true);
        // A source without a name is called by its location; an unreadable cache is not a
        // served copy, so a user's source that fails that way is a warning.
        workspace.source_failures = vec![SourceFailure {
            id: "cli-1".into(),
            name: "https://user:SECRET@h/i.json".into(),
            location: "https://***@h/i.json".into(),
            error: "cached copy: Invalid JSON via https://user:SECRET@h/i.json".into(),
            from_cache: false,
            builtin: false,
        }];
        let problems = plain(&compute(&workspace));
        let problem = find(&problems, "index:cli-1");
        assert_eq!(problem.severity, Severity::Warning);
        assert_eq!(problem.message.params["source"], "https://***@h/i.json");
        assert!(!problem.message.params["error"].contains("SECRET"));
        assert!(!format!("{problem:?}").contains("SECRET"), "{problem:?}");
    }

    #[test]
    fn package_problems_are_attached_by_package_not_by_group_text() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = sample_workspace(dir.path(), false);
        let mut computed = compute(&workspace);
        // A general problem whose group text happens to be a package name stays unattached.
        computed.push(ComputedProblem::general(problem(
            "x:1".into(),
            "UBotCore".into(),
            Severity::Info,
            msg0("fix.ignore"),
            Vec::new(),
            Vec::new(),
        )));
        let mut packages = workspace.package_infos();
        attach_problem_ids(&mut packages, &computed);
        let core = packages.iter().find(|p| p.name == "UBotCore").unwrap();
        assert!(core.problem_ids.is_empty());
        let environment = packages
            .iter()
            .find(|p| p.name == "UBotEnvironment")
            .unwrap();
        assert!(environment
            .problem_ids
            .contains(&"requires:UBotEnvironment:UBotCore".to_string()));
    }
}

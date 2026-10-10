//! Plain-text rendering of snapshots, problems, plans and tasks (English, SPEC §10).

use std::fmt::Write as _;

use ubot_core::index::MergedIndex;
use ubot_core::model::{
    EngineInfo, EngineKind, FixAction, IndexSourceStatus, PackageInfo, PackageState, Plan, Problem,
    Recipe, RequirementKind, RequirementStatus, Snapshot, StepGroup, StepStatus, Task,
};
use ubot_core::msg::render_en;

fn table(rows: &[Vec<String>], indent: &str) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let mut line = String::from(indent);
        for (column, cell) in row.iter().enumerate() {
            if column + 1 == row.len() {
                line.push_str(cell);
            } else {
                let _ = write!(line, "{cell:<width$}  ", width = widths[column]);
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn state_text(state: PackageState) -> &'static str {
    match state {
        PackageState::Enabled => "enabled",
        PackageState::Disabled => "disabled",
        PackageState::NotInstalled => "not installed",
        PackageState::Planned => "planned",
    }
}

fn engine_kind(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Launcher => "launcher",
        EngineKind::Source => "source",
        EngineKind::Custom => "custom",
    }
}

fn git_text(package: &PackageInfo) -> String {
    match &package.git {
        Some(git) if git.is_checkout => {
            let mut text = git.git_ref.clone().unwrap_or_else(|| "?".into());
            if git.dirty {
                text.push_str(" (modified)");
            }
            text
        }
        Some(_) => "not a git checkout".into(),
        None => String::new(),
    }
}

pub fn status(snapshot: &Snapshot, show_planned: bool) -> String {
    let mut out = String::new();
    match &snapshot.project {
        Some(project) => {
            let engine = match &project.engine {
                Some(engine) => format!(
                    "{} ({}) {}",
                    engine.version,
                    engine_kind(engine.kind),
                    engine.root
                ),
                None => format!(
                    "not found (EngineAssociation \"{}\")",
                    project.engine_association
                ),
            };
            let editor = match project.editor.pid {
                Some(pid) => format!("running (pid {pid})"),
                None => "not running".into(),
            };
            out.push_str(&table(
                &[
                    vec![
                        "Project".into(),
                        format!("{}  {}", project.name, project.path),
                    ],
                    vec!["Engine".into(), engine],
                    vec![
                        "Build".into(),
                        format!(
                            "{}{}",
                            project.build_target,
                            if project.has_code {
                                ""
                            } else {
                                " (content only)"
                            }
                        ),
                    ],
                    vec!["Editor".into(), editor],
                ],
                "",
            ));
        }
        None => out.push_str("No project is open. Use --project <path to .uproject>.\n"),
    }

    out.push_str("\nPackages\n");
    let mut rows = vec![vec![
        "NAME".into(),
        "STATE".into(),
        "INSTALLED".into(),
        "LATEST".into(),
        "LOCATION".into(),
        "GIT".into(),
    ]];
    for package in snapshot
        .packages
        .iter()
        .filter(|package| show_planned || package.state != PackageState::Planned)
    {
        let latest = match (&package.latest_version, &package.planned_version) {
            (Some(latest), _) => format!(
                "{latest}{}",
                if package.update_available { " ↑" } else { "" }
            ),
            (None, Some(planned)) => format!("({planned})"),
            (None, None) => "-".into(),
        };
        rows.push(vec![
            package.name.clone(),
            state_text(package.state).into(),
            package
                .installed_version
                .clone()
                .unwrap_or_else(|| "-".into()),
            latest,
            package.location.clone().unwrap_or_else(|| "-".into()),
            git_text(package),
        ]);
    }
    out.push_str(&table(&rows, "  "));

    let others: Vec<_> = snapshot
        .plugins
        .iter()
        .filter(|plugin| !plugin.is_ubot)
        .collect();
    if !others.is_empty() {
        out.push_str("\nOther project plugins\n");
        let rows: Vec<Vec<String>> = others
            .iter()
            .map(|plugin| {
                vec![
                    plugin.name.clone(),
                    if plugin.enabled {
                        "enabled".into()
                    } else {
                        "disabled".into()
                    },
                    plugin.version.clone().unwrap_or_else(|| "-".into()),
                    plugin.location.clone(),
                ]
            })
            .collect();
        out.push_str(&table(&rows, "  "));
    }

    out.push('\n');
    out.push_str(&problem_summary(&snapshot.problems));
    out.push('\n');
    out.push_str(&sources(&snapshot.index));
    out
}

fn problem_summary(problems: &[Problem]) -> String {
    if problems.is_empty() {
        return "No problems.\n".into();
    }
    let mut out = format!("Problems ({})\n", problems.len());
    let rows: Vec<Vec<String>> = problems
        .iter()
        .map(|problem| {
            vec![
                problem.severity.as_str().into(),
                problem.group.clone(),
                render_en(&problem.message),
            ]
        })
        .collect();
    out.push_str(&table(&rows, "  "));
    out
}

pub fn sources(statuses: &[IndexSourceStatus]) -> String {
    let mut out = String::from("Index sources\n");
    let rows: Vec<Vec<String>> = statuses
        .iter()
        .map(|status| {
            let state = if !status.enabled {
                "disabled".to_string()
            } else if status.ok {
                format!(
                    "ok, {} packages{}",
                    status.packages,
                    if status.from_cache { " (cached)" } else { "" }
                )
            } else if status.from_cache {
                format!(
                    "cached copy, {} packages ({})",
                    status.packages,
                    status.error.clone().unwrap_or_default()
                )
            } else {
                format!("not loaded ({})", status.error.clone().unwrap_or_default())
            };
            vec![
                status.id.clone(),
                status.name.clone(),
                state,
                status.location.clone(),
            ]
        })
        .collect();
    out.push_str(&table(&rows, "  "));
    out
}

pub fn problems(problems: &[Problem]) -> String {
    if problems.is_empty() {
        return "No problems.\n".into();
    }
    let mut out = String::new();
    for problem in problems {
        let _ = writeln!(
            out,
            "{}  {}  [{}]  ({})",
            problem.severity.as_str(),
            problem.group,
            problem.id,
            scope(problem)
        );
        let _ = writeln!(out, "  {}", render_en(&problem.message));
        for fact in &problem.facts {
            let value = match &fact.value_msg {
                Some(value_msg) => format!("{} ({})", render_en(value_msg), fact.value),
                None => fact.value.clone(),
            };
            let _ = writeln!(out, "    {}: {}", render_en(&fact.label), value);
        }
        if !problem.fixes.is_empty() {
            let fixes: Vec<String> = problem
                .fixes
                .iter()
                .map(|fix| {
                    let hint = match &fix.action {
                        FixAction::Changes { changes } => {
                            let parts: Vec<String> = changes
                                .iter()
                                .map(|change| match &change.version {
                                    Some(version) => format!(
                                        "{}:{}@{}",
                                        change.kind.as_str(),
                                        change.package,
                                        version
                                    ),
                                    None => format!("{}:{}", change.kind.as_str(), change.package),
                                })
                                .collect();
                            format!(" (ubot apply {})", parts.join(" "))
                        }
                        FixAction::Ignore | FixAction::OpenSettings { .. } => String::new(),
                    };
                    format!("{}{}", render_en(&fix.label), hint)
                })
                .collect();
            let _ = writeln!(out, "    Fixes: {}", fixes.join("; "));
        }
    }
    out
}

fn scope(problem: &Problem) -> &'static str {
    match problem.scope {
        ubot_core::model::ProblemScope::Packages => "packages",
        ubot_core::model::ProblemScope::Session => "last session",
    }
}

pub fn package(package: &PackageInfo, problems: &[Problem]) -> String {
    let mut out = format!(
        "{} ({})  {}\n",
        package.name,
        package.friendly_name,
        package.layer.as_str()
    );
    if !package.description.en.is_empty() {
        let _ = writeln!(out, "  {}", package.description.en);
    }
    let join = |items: &[String]| {
        if items.is_empty() {
            "-".to_string()
        } else {
            items.join(", ")
        }
    };
    let optional = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".into());
    let requires: Vec<String> = package
        .requires
        .iter()
        .map(|requirement| {
            let status = match requirement.status {
                RequirementStatus::Satisfied => "satisfied",
                RequirementStatus::VersionMismatch => "version mismatch",
                RequirementStatus::NotInstalled => "not installed",
                RequirementStatus::Disabled => "disabled",
                RequirementStatus::Missing => "missing",
            };
            let kind = match requirement.kind {
                RequirementKind::Package => "",
                RequirementKind::EnginePlugin => "engine plugin ",
            };
            let constraint = if requirement.constraint.is_empty() {
                String::new()
            } else {
                format!(" {}", requirement.constraint)
            };
            format!(
                "{kind}{}{constraint}{} ({status})",
                requirement.name,
                if requirement.optional {
                    " optional"
                } else {
                    ""
                }
            )
        })
        .collect();
    let rows = vec![
        vec!["State".into(), state_text(package.state).into()],
        vec!["Installed".into(), optional(&package.installed_version)],
        vec![
            "Latest".into(),
            format!(
                "{}{}",
                optional(&package.latest_version),
                if package.update_available {
                    " (update available)"
                } else {
                    ""
                }
            ),
        ],
        vec!["Versions".into(), join(&package.versions)],
        vec!["Planned".into(), optional(&package.planned_version)],
        vec!["Location".into(), optional(&package.location)],
        vec!["Git".into(), {
            let git = git_text(package);
            if git.is_empty() {
                "-".into()
            } else {
                git
            }
        }],
        vec!["Repository".into(), optional(&package.repository)],
        vec!["Web".into(), optional(&package.web_url)],
        vec!["Docs".into(), optional(&package.docs_url)],
        vec!["Index source".into(), optional(&package.index_source)],
        vec!["Requires".into(), join(&requires)],
        vec!["Provides".into(), join(&package.provides)],
        vec!["Tags".into(), join(&package.tags)],
    ];
    out.push_str(&table(&rows, "  "));
    let own: Vec<Problem> = problems
        .iter()
        .filter(|problem| package.problem_ids.contains(&problem.id))
        .cloned()
        .collect();
    if !own.is_empty() {
        out.push('\n');
        out.push_str(&self::problems(&own));
    }
    out
}

pub fn plan(plan: &Plan) -> String {
    let mut out = String::new();
    if plan.steps.is_empty() && plan.errors.is_empty() {
        out.push_str("Nothing to do.\n");
    }
    let mut group = None;
    for step in &plan.steps {
        if group != Some(step.group) {
            group = Some(step.group);
            let name = match step.group {
                StepGroup::Download => "Download",
                StepGroup::Project => "Project",
                StepGroup::Build => "Build",
                StepGroup::After => "After",
            };
            let _ = writeln!(out, "{name}");
        }
        let _ = writeln!(
            out,
            "  {}. {}  ({})",
            step.n,
            render_en(&step.title),
            render_en(&step.detail)
        );
    }
    if plan.editor_running {
        out.push_str("Unreal Editor is running this project; steps that change the project wait until it is closed.\n");
    }
    for warning in &plan.warnings {
        let _ = writeln!(out, "warning: {}", render_en(warning));
    }
    for error in &plan.errors {
        let _ = writeln!(out, "error: {}", render_en(error));
    }
    out
}

pub fn recipes(recipes: &[Recipe]) -> String {
    if recipes.is_empty() {
        return "No recipes.\n".into();
    }
    let rows: Vec<Vec<String>> = recipes
        .iter()
        .map(|recipe| {
            vec![
                recipe.id.clone(),
                recipe.name.en.clone(),
                if recipe.available {
                    "available".into()
                } else {
                    "not available".into()
                },
                format!(
                    "{}{}",
                    recipe.packages.join(", "),
                    if recipe.user { "  (user)" } else { "" }
                ),
            ]
        })
        .collect();
    table(&rows, "")
}

pub fn engines(engines: &[EngineInfo]) -> String {
    if engines.is_empty() {
        return "No Unreal Engine installations were found.\n".into();
    }
    let rows: Vec<Vec<String>> = engines
        .iter()
        .map(|engine| {
            vec![
                engine.id.clone(),
                engine.version.clone(),
                engine_kind(engine.kind).into(),
                engine.root.clone(),
            ]
        })
        .collect();
    table(&rows, "")
}

pub fn index(statuses: &[IndexSourceStatus], merged: &MergedIndex) -> String {
    let mut out = sources(statuses);
    out.push_str("\nPackages\n");
    let rows: Vec<Vec<String>> = merged
        .packages
        .iter()
        .map(|entry| {
            let package = &entry.package;
            let versions: Vec<String> = package
                .versions
                .iter()
                .map(|version| {
                    if version.planned {
                        format!("{} (planned)", version.version)
                    } else {
                        version.version.clone()
                    }
                })
                .collect();
            vec![
                package.name.clone(),
                package.layer.clone().unwrap_or_default(),
                versions.join(", "),
                entry.source.clone(),
            ]
        })
        .collect();
    out.push_str(&table(&rows, "  "));
    if !merged.recipes.is_empty() {
        out.push_str("\nRecipes\n");
        let rows: Vec<Vec<String>> = merged
            .recipes
            .iter()
            .map(|entry| {
                vec![
                    entry.recipe.id.clone(),
                    entry.recipe.packages.join(", "),
                    entry.source.clone(),
                ]
            })
            .collect();
        out.push_str(&table(&rows, "  "));
    }
    if !merged.replacements.is_empty() {
        out.push_str("\nReplacements\n");
        let rows: Vec<Vec<String>> = merged
            .replacements
            .iter()
            .map(|replacement| {
                vec![
                    replacement.legacy.clone(),
                    format!("→ {}", replacement.replacement),
                ]
            })
            .collect();
        out.push_str(&table(&rows, "  "));
    }
    out
}

pub fn step_status(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::Waiting => "waiting for Unreal Editor to close",
        StepStatus::Running => "running",
        StepStatus::Done => "done",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
        StepStatus::Canceled => "canceled",
    }
}

pub fn task_result(task: &Task) -> String {
    let duration = task
        .duration_ms
        .map(|ms| format!(" in {:.1} s", ms as f64 / 1000.0))
        .unwrap_or_default();
    match (&task.error, task.status) {
        (Some(error), _) => format!(
            "{}: {}{duration}. {error}\n",
            render_en(&task.title),
            step_status(task.status)
        ),
        (None, status) => format!(
            "{}: {}{duration}.\n",
            render_en(&task.title),
            step_status(status)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_tables() {
        let text = table(
            &[
                vec!["a".into(), "bbb".into(), "c".into()],
                vec!["dddd".into(), "e".into(), "".into()],
            ],
            "  ",
        );
        assert_eq!(text, "  a     bbb  c\n  dddd  e\n");
    }
}

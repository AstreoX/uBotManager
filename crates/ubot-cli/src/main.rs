//! `ubot`: the command line interface of uBot Manager (SPEC §10).

mod package;
mod render;

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use ubot_core::exec::{PermanentDelete, RecycleBin};
use ubot_core::model::{
    ApplyOptions, Change, ChangeKind, PackageState, Plan, Snapshot, StepStatus, Task,
};
use ubot_core::msg::{msg, render_en};
use ubot_core::paths::{self, AppDirs};
use ubot_core::{index, sources, Manager, ManagerOptions, TaskSink};

/// Set to 1 to delete uninstalled packages permanently instead of moving them to the Recycle Bin
/// (tests and CI).
const PERMANENT_DELETE_ENV: &str = "UBOT_MANAGER_PERMANENT_DELETE";

#[derive(Parser)]
#[command(
    name = "ubot",
    version,
    about = "uBot Manager: install, update and enable uBot packages in an Unreal project."
)]
struct Cli {
    /// The project (.uproject); defaults to the last project opened.
    #[arg(long, global = true, value_name = ".uproject")]
    project: Option<PathBuf>,
    /// Index file or URL to use instead of the configured sources (repeatable).
    #[arg(long = "index", global = true, value_name = "file|url")]
    index: Vec<String>,
    /// Use cached index copies only; never download.
    #[arg(long, global = true)]
    offline: bool,
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Packages and problems of the project.
    Status,
    /// Details of one package.
    Info { package: String },
    /// Problems of the project.
    Problems,
    /// Show what changes would do: install:Name[@ver] update:Name@ver enable:Name disable:Name uninstall:Name
    Plan {
        #[arg(required = true, value_name = "change")]
        changes: Vec<String>,
    },
    /// Apply changes: install:Name[@ver] update:Name@ver enable:Name disable:Name uninstall:Name
    Apply {
        #[arg(required = true, value_name = "change")]
        changes: Vec<String>,
        #[command(flatten)]
        options: RunOptions,
        /// Run the automation tests of the changed packages afterwards.
        #[arg(long)]
        run_tests: bool,
        /// Reopen Unreal Editor afterwards.
        #[arg(long)]
        reopen_editor: bool,
    },
    /// Recipes from the index and saved by you.
    Recipes,
    /// Apply a recipe: install its missing packages and enable the disabled ones.
    Recipe {
        id: String,
        #[command(flatten)]
        options: RunOptions,
    },
    /// Installed Unreal Engine versions.
    Engines,
    /// Build prebuilt binaries: install Name[@version]... from source in a host project under
    /// <out>/_work, build it with a launcher engine and pack each package into
    /// <out>/<Name>-<version>-UE<x.y>-Win64.zip (+ .sha256).
    Package {
        #[arg(required = true, value_name = "Name[@version]")]
        packages: Vec<String>,
        /// Folder for the archives.
        #[arg(long, value_name = "dir")]
        out: PathBuf,
        /// Launcher engine id (5.5) or the root of a launcher engine; default: the newest
        /// launcher engine.
        #[arg(long, value_name = "id|root")]
        engine: Option<String>,
        /// Download URL of an archive; placeholders {repo} {ref} {file} {name} {version} {engine}.
        #[arg(long, value_name = "template", default_value = ubot_core::prebuilt::DEFAULT_URL_TEMPLATE)]
        url_template: String,
        /// Write the binaries entries into this index file (and validate it).
        #[arg(long, value_name = "index file")]
        update_index: Option<PathBuf>,
        /// Keep *.pdb, *.exp and *.lib files in the archives.
        #[arg(long)]
        with_symbols: bool,
        /// Keep the host project in <out>/_work.
        #[arg(long)]
        keep_work: bool,
    },
    /// Package index commands.
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
}

#[derive(Args)]
struct RunOptions {
    /// Do not build the project after changing packages.
    #[arg(long)]
    no_build: bool,
    /// Do not ask for confirmation.
    #[arg(long)]
    yes: bool,
}

#[derive(Subcommand)]
enum IndexCommand {
    /// Validate an index file (exit code 1 on errors).
    Validate { file: PathBuf },
    /// The merged index of all sources.
    Show,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(error) => {
            errln(&format!("error: {error:#}"));
            ExitCode::FAILURE
        }
    }
}

/// Writes to stdout and ignores a failure. `println!` panics when stdout has been closed
/// (`ubot apply ... | head`); in the middle of an `apply` that would end the process, or the task
/// thread, with the project half changed and the task never recorded.
fn out(text: &str) {
    let _ = io::stdout().lock().write_all(text.as_bytes());
}

fn outln(text: &str) {
    out(&format!("{text}\n"));
}

fn errln(text: &str) {
    let _ = writeln!(io::stderr().lock(), "{text}");
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    outln(&serde_json::to_string_pretty(value)?);
    Ok(())
}

/// The `--index` values as index sources: URLs as given, files taken from the current directory
/// (where the user typed them), not from the project's, where configured sources are looked up.
fn index_override(values: &[String]) -> Option<Vec<String>> {
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .map(|value| {
                if sources::is_url(value) {
                    value.trim().to_string()
                } else {
                    paths::display(&paths::absolute(Path::new(value.trim())))
                }
            })
            .collect(),
    )
}

/// Ctrl+C and Ctrl+Break while a task runs cancel the task instead of ending the process: the
/// running build is killed with its children, a partial clone is removed and the history records
/// the task as canceled.
#[cfg(windows)]
mod interrupt {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static REQUESTS: AtomicUsize = AtomicUsize::new(0);

    type Handler = unsafe extern "system" fn(u32) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
    }

    /// CTRL_C_EVENT (0), CTRL_BREAK_EVENT (1) and CTRL_CLOSE_EVENT (2) are counted and handled.
    unsafe extern "system" fn handle(event: u32) -> i32 {
        if event <= 2 {
            REQUESTS.fetch_add(1, Ordering::SeqCst);
            1
        } else {
            0
        }
    }

    pub fn install() {
        // SAFETY: `handle` is a valid handler for the lifetime of the process and only touches
        // an atomic.
        unsafe {
            SetConsoleCtrlHandler(Some(handle), 1);
        }
    }

    pub fn requests() -> usize {
        REQUESTS.load(Ordering::SeqCst)
    }
}

#[cfg(not(windows))]
mod interrupt {
    pub fn install() {}

    pub fn requests() -> usize {
        0
    }
}

/// `kind:Name[@version]`.
fn parse_change(text: &str) -> Result<Change> {
    let (kind, rest) = text
        .split_once(':')
        .ok_or_else(|| anyhow!("'{text}': expected kind:Name, e.g. install:UBotROS"))?;
    let kind = match kind.trim().to_ascii_lowercase().as_str() {
        "install" => ChangeKind::Install,
        "update" => ChangeKind::Update,
        "enable" => ChangeKind::Enable,
        "disable" => ChangeKind::Disable,
        "uninstall" => ChangeKind::Uninstall,
        other => bail!(
            "'{text}': unknown change '{other}' (install, update, enable, disable or uninstall)"
        ),
    };
    let (package, version) = match rest.split_once('@') {
        Some((package, version)) => (
            package.trim(),
            Some(version.trim().to_string()).filter(|v| !v.is_empty()),
        ),
        None => (rest.trim(), None),
    };
    if package.is_empty() {
        bail!("'{text}': the package name is missing");
    }
    if version.is_some() && !matches!(kind, ChangeKind::Install | ChangeKind::Update) {
        bail!("'{text}': only install and update take a version");
    }
    if kind == ChangeKind::Update && version.is_none() {
        bail!("'{text}': update needs a version (update:{package}@1.2.3)");
    }
    Ok(Change {
        kind,
        package: package.to_string(),
        version,
    })
}

fn run(cli: Cli) -> Result<ExitCode> {
    if let Command::Index {
        command: IndexCommand::Validate { file },
    } = &cli.command
    {
        return validate_index(file, cli.json);
    }

    if let Command::Package {
        packages,
        out,
        engine,
        url_template,
        update_index,
        with_symbols,
        keep_work,
    } = cli.command
    {
        return package::run(
            package::PackageArgs {
                packages,
                out,
                engine,
                url_template,
                update_index,
                with_symbols,
                keep_work,
            },
            package::Globals {
                index_override: index_override(&cli.index),
                index: cli.index,
                offline: cli.offline,
                json: cli.json,
            },
        );
    }

    let mutating = matches!(cli.command, Command::Apply { .. } | Command::Recipe { .. });
    let permanent_delete = std::env::var(PERMANENT_DELETE_ENV).is_ok_and(|value| value == "1");
    let options = ManagerOptions {
        dirs: AppDirs::from_env(),
        offline: cli.offline,
        index_override: index_override(&cli.index),
        // Read-only commands leave the project untouched.
        write_merged_index: mutating,
        remover: if permanent_delete {
            Arc::new(PermanentDelete)
        } else {
            Arc::new(RecycleBin)
        },
        ..ManagerOptions::default()
    };
    let manager = Manager::new(options);

    if let Command::Engines = cli.command {
        let engines = manager.list_engines();
        if cli.json {
            print_json(&engines)?;
        } else {
            out(&render::engines(&engines));
        }
        return Ok(ExitCode::SUCCESS);
    }

    let settings = manager.get_settings();
    let sync = settings.index.sync_on_start && !cli.offline;
    let snapshot = match &cli.project {
        Some(path) => manager.open_project_with(&path.to_string_lossy(), sync),
        None => manager.refresh(sync),
    }
    .map_err(|error| anyhow!(error))?;

    // `--index` replaces the configured sources (tests, CI). One that cannot be loaded is an
    // error, not a reason to carry on with the built-in copy of the real index. (`index show`
    // is how such a source is looked at, so it still lists it.)
    if !cli.index.is_empty()
        && !matches!(
            cli.command,
            Command::Index {
                command: IndexCommand::Show
            }
        )
    {
        let failed = unloaded_index_sources(&snapshot);
        if !failed.is_empty() {
            bail!("{}", failed.join("\n"));
        }
    }

    match cli.command {
        Command::Status => {
            if cli.json {
                print_json(&snapshot)?;
            } else {
                out(&render::status(&snapshot, settings.index.show_planned));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Info { package } => {
            let found = snapshot
                .packages
                .iter()
                .find(|candidate| candidate.name.eq_ignore_ascii_case(package.trim()));
            let Some(found) = found else {
                bail!("'{package}' is neither in the index nor installed");
            };
            if cli.json {
                let problems: Vec<_> = snapshot
                    .problems
                    .iter()
                    .filter(|problem| found.problem_ids.contains(&problem.id))
                    .collect();
                print_json(&json!({ "package": found, "problems": problems }))?;
            } else {
                out(&render::package(found, &snapshot.problems));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Problems => {
            if cli.json {
                print_json(&snapshot.problems)?;
            } else {
                out(&render::problems(&snapshot.problems));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Plan { changes } => {
            let changes = changes
                .iter()
                .map(|text| parse_change(text))
                .collect::<Result<Vec<_>>>()?;
            let options = ApplyOptions {
                build: true,
                reopen_editor: false,
                run_tests: false,
            };
            let plan = manager
                .plan(&changes, &options)
                .map_err(|error| anyhow!(error))?;
            if cli.json {
                print_json(&plan)?;
            } else {
                out(&render::plan(&plan));
            }
            Ok(if plan.errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Apply {
            changes,
            options,
            run_tests,
            reopen_editor,
        } => {
            let changes = changes
                .iter()
                .map(|text| parse_change(text))
                .collect::<Result<Vec<_>>>()?;
            let apply_options = ApplyOptions {
                build: !options.no_build,
                reopen_editor,
                run_tests,
            };
            apply(
                &manager,
                &snapshot,
                &changes,
                &apply_options,
                options.yes,
                cli.json,
                None,
            )
        }
        Command::Recipes => {
            if cli.json {
                print_json(&snapshot.recipes)?;
            } else {
                out(&render::recipes(&snapshot.recipes));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Recipe { id, options } => {
            let Some(recipe) = snapshot
                .recipes
                .iter()
                .find(|recipe| recipe.id.eq_ignore_ascii_case(id.trim()))
            else {
                bail!("there is no recipe '{id}' (see `ubot recipes`)");
            };
            if !recipe.available {
                // The GUI does not offer such a recipe; applying "the rest" of it would leave
                // the project without the packages it was chosen for.
                let blocked: Vec<&str> = recipe
                    .packages
                    .iter()
                    .filter(|name| {
                        snapshot
                            .packages
                            .iter()
                            .find(|package| package.name.eq_ignore_ascii_case(name.trim()))
                            .is_none_or(|package| {
                                package.state == PackageState::Planned
                                    || package.index_source.is_none()
                            })
                    })
                    .map(String::as_str)
                    .collect();
                if blocked.is_empty() {
                    bail!(
                        "recipe '{}' is not available: it lists no package",
                        recipe.id
                    );
                }
                bail!(
                    "recipe '{}' is not available: {} {} planned or not in the index",
                    recipe.id,
                    blocked.join(", "),
                    if blocked.len() == 1 { "is" } else { "are" }
                );
            }
            let changes = manager.recipe_changes(Some(&recipe.id), &[]);
            let project = snapshot
                .project
                .as_ref()
                .map(|project| project.name.clone())
                .unwrap_or_default();
            let title = msg(
                "task.recipe",
                &[("recipe", &recipe.name.en), ("project", &project)],
            );
            let apply_options = ApplyOptions {
                build: !options.no_build,
                reopen_editor: false,
                run_tests: false,
            };
            if changes.is_empty() {
                if cli.json {
                    // The same empty plan `apply` prints when there is nothing to do.
                    print_json(&Plan::default())?;
                } else {
                    outln(&format!(
                        "Every package of {} is installed and enabled.",
                        recipe.name.en
                    ));
                }
                return Ok(ExitCode::SUCCESS);
            }
            apply(
                &manager,
                &snapshot,
                &changes,
                &apply_options,
                options.yes,
                cli.json,
                Some(title),
            )
        }
        Command::Index {
            command: IndexCommand::Show,
        } => {
            let merged = manager.merged_index();
            if cli.json {
                // Credentials in repository URLs are never printed (SPEC 5.4).
                print_json(
                    &json!({ "sources": snapshot.index, "index": merged.to_file().redacted() }),
                )?;
            } else {
                out(&render::index(&snapshot.index, &merged));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Engines
        | Command::Package { .. }
        | Command::Index {
            command: IndexCommand::Validate { .. },
        } => unreachable!("handled above"),
    }
}

/// The `--index` sources that could not be loaded (and are not served from a cache).
fn unloaded_index_sources(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .index
        .iter()
        .filter(|source| source.id.starts_with("cli-") && !source.ok && !source.from_cache)
        .map(|source| {
            format!(
                "--index {}: {}",
                source.location,
                source.error.as_deref().unwrap_or("could not be loaded")
            )
        })
        .collect()
}

fn validate_index(file: &PathBuf, json_output: bool) -> Result<ExitCode> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let validation = index::validate_text(&text);
    if json_output {
        print_json(
            &json!({ "ok": validation.is_ok(), "errors": validation.errors, "warnings": validation.warnings }),
        )?;
    } else {
        for error in &validation.errors {
            outln(&format!("error: {error}"));
        }
        for warning in &validation.warnings {
            outln(&format!("warning: {warning}"));
        }
        outln(&format!(
            "{}: {} error(s), {} warning(s).",
            file.display(),
            validation.errors.len(),
            validation.warnings.len()
        ));
    }
    Ok(if validation.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn confirm(question: &str) -> Result<bool> {
    let _ = write!(io::stderr(), "{question} [y/N] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    if io::stdin().lock().read_line(&mut answer)? == 0 {
        return Ok(false);
    }
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn apply(
    manager: &Manager,
    snapshot: &Snapshot,
    changes: &[Change],
    options: &ApplyOptions,
    yes: bool,
    json_output: bool,
    title: Option<ubot_core::model::Msg>,
) -> Result<ExitCode> {
    if snapshot.project.is_none() {
        bail!("{}", render_en(&msg("plan.noProject", &[])));
    }
    let plan = manager
        .plan(changes, options)
        .map_err(|error| anyhow!(error))?;
    if !json_output {
        out(&render::plan(&plan));
    }
    if !plan.errors.is_empty() {
        if json_output {
            print_json(&plan)?;
        }
        return Ok(ExitCode::FAILURE);
    }
    if plan.steps.is_empty() {
        if json_output {
            print_json(&plan)?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    if !yes && !confirm("Apply these changes?")? {
        errln("Canceled.");
        return Ok(ExitCode::FAILURE);
    }

    let task = run_task(manager, changes, options, title, json_output)?;
    if json_output {
        print_json(&task)?;
    } else {
        out(&render::task_result(&task));
    }
    Ok(match task.status {
        StepStatus::Done => ExitCode::SUCCESS,
        StepStatus::Canceled => ExitCode::from(130),
        _ => ExitCode::FAILURE,
    })
}

/// Runs the changes as a task, printing step transitions and output (unless `quiet`), with
/// Ctrl+C canceling it; returns the finished task.
fn run_task(
    manager: &Manager,
    changes: &[Change],
    options: &ApplyOptions,
    title: Option<ubot_core::model::Msg>,
    quiet: bool,
) -> Result<Task> {
    let sink = Arc::new(ConsoleSink {
        quiet,
        ..ConsoleSink::default()
    });
    interrupt::install();
    let task_id = manager
        .apply(changes, options, title, None, sink.clone())
        .map_err(|error| anyhow!(error))?;
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = {
        let manager = manager.clone();
        let task_id = task_id.clone();
        let finished = finished.clone();
        std::thread::spawn(move || {
            let mut canceled = false;
            while !finished.load(Ordering::SeqCst) {
                let requests = interrupt::requests();
                if requests >= 1 && !canceled {
                    errln("Canceling...");
                    let _ = manager.cancel_task(&task_id);
                    canceled = true;
                }
                if requests >= 2 {
                    // The second time means: now.
                    std::process::exit(130);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };
    manager.wait_for_task(&task_id);
    finished.store(true, Ordering::SeqCst);
    let _ = watcher.join();
    manager
        .list_tasks()
        .into_iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| anyhow!("the task disappeared"))
}

/// Prints step transitions and step output as they happen.
#[derive(Default)]
struct ConsoleSink {
    quiet: bool,
    printed: Mutex<HashMap<String, StepStatus>>,
}

impl TaskSink for ConsoleSink {
    fn task_updated(&self, task: &Task) {
        if self.quiet {
            return;
        }
        let mut printed = self
            .printed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let total = task.steps.len();
        for (position, step) in task.steps.iter().enumerate() {
            if printed.get(&step.id) == Some(&step.status) || step.status == StepStatus::Pending {
                continue;
            }
            printed.insert(step.id.clone(), step.status);
            let duration = step
                .duration_ms
                .map(|ms| format!(" ({:.1} s)", ms as f64 / 1000.0))
                .unwrap_or_default();
            outln(&format!(
                "[{}/{total}] {}: {}{duration}",
                position + 1,
                render_en(&step.title),
                render::step_status(step.status)
            ));
        }
    }

    fn log(&self, _task_id: &str, _step_id: &str, lines: &[String]) {
        if self.quiet {
            return;
        }
        let mut text = String::new();
        for line in lines {
            text.push_str(&format!("      {line}\n"));
        }
        out(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_changes() {
        assert_eq!(
            parse_change("install:UBotROS").unwrap(),
            Change::new(ChangeKind::Install, "UBotROS", None)
        );
        assert_eq!(
            parse_change("install:UBotROS@0.1.0").unwrap(),
            Change::new(ChangeKind::Install, "UBotROS", Some("0.1.0".into()))
        );
        assert_eq!(
            parse_change("Update:UBotCore@0.2.0").unwrap(),
            Change::new(ChangeKind::Update, "UBotCore", Some("0.2.0".into()))
        );
        assert_eq!(
            parse_change("disable:UBotSensor").unwrap().kind,
            ChangeKind::Disable
        );
        assert_eq!(
            parse_change("uninstall:UBotROS").unwrap().kind,
            ChangeKind::Uninstall
        );
        assert!(parse_change("UBotROS").is_err());
        assert!(parse_change("remove:UBotROS").is_err());
        assert!(parse_change("install:").is_err());
        assert!(parse_change("update:UBotCore").is_err());
        assert!(parse_change("enable:UBotCore@1.0").is_err());
    }

    #[test]
    fn index_files_are_taken_from_the_current_directory() {
        assert_eq!(index_override(&[]), None);
        let given = vec![
            "idx.json".to_string(),
            "https://example.com/index.json".to_string(),
            " http://127.0.0.1:1/i.json ".to_string(),
        ];
        let resolved = index_override(&given).unwrap();
        let cwd = paths::display(&std::env::current_dir().unwrap());
        assert_eq!(
            resolved[0],
            format!("{cwd}{}idx.json", std::path::MAIN_SEPARATOR)
        );
        assert_eq!(resolved[1], "https://example.com/index.json");
        assert_eq!(resolved[2], "http://127.0.0.1:1/i.json");
        let absolute = if cfg!(windows) {
            "C:\\x\\i.json"
        } else {
            "/x/i.json"
        };
        assert_eq!(
            index_override(&[absolute.to_string()]).unwrap()[0],
            paths::display(Path::new(absolute))
        );
    }

    #[test]
    fn writing_to_a_closed_output_does_not_panic() {
        // The helpers swallow failures; with a pipe that is gone `println!` would panic.
        out("hello");
        outln("hello");
        errln("hello");
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}

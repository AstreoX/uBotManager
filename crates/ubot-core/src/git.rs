//! Git: the state of installed package checkouts (SPEC §3) and the git executable setting.

use std::path::Path;
use std::process::Command;

use crate::model::GitState;
use crate::process;

/// Detailed state of a package directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitInfo {
    pub is_checkout: bool,
    /// Tag exactly at HEAD.
    pub tag: Option<String>,
    /// Current branch; `None` when HEAD is detached.
    pub branch: Option<String>,
    pub short_sha: Option<String>,
    pub dirty: bool,
    /// Why git could not describe the checkout (git missing, "dubious ownership", corrupt
    /// repository). `dirty` and the ref are then unknown, not clean: callers must not treat the
    /// directory as a clean checkout. `None` when every query worked.
    pub error: Option<String>,
}

impl GitInfo {
    /// `ref` = exact tag, else branch, else short SHA.
    pub fn to_state(&self) -> GitState {
        GitState {
            is_checkout: self.is_checkout,
            git_ref: self
                .tag
                .clone()
                .or_else(|| self.branch.clone())
                .or_else(|| self.short_sha.clone()),
            dirty: self.dirty,
        }
    }
}

/// A git command that never prompts for credentials and skips the detached-HEAD advice that
/// every checkout of a tag would print.
///
/// Package folders may have come with the project (a zip, a shared project), so their
/// `.git/config` is not trusted: `core.fsmonitor` would make a plain `git status` run a program
/// named there, and `-c` beats the repository's own configuration. That covers this one key;
/// the other keys that name a program are caught by [`untrusted_config`], which has to be
/// asked before a command runs in an existing checkout.
pub fn command(git: &str) -> Command {
    let mut command = process::command(git);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .args([
            "-c",
            "advice.detachedHead=false",
            "-c",
            "core.fsmonitor=false",
        ]);
    command
}

/// [`command`] for fetch and checkout in a folder that already exists: also runs no hooks (a
/// `post-checkout` hook of an untrusted checkout would otherwise run on every update).
pub fn command_in_checkout(git: &str) -> Command {
    let mut command = command(git);
    // A directory that does not exist holds no hooks.
    let no_hooks = std::env::temp_dir().join("ubot-no-git-hooks");
    command.arg("-c").arg(format!(
        "core.hooksPath={}",
        no_hooks.to_string_lossy().replace('\\', "/")
    ));
    command
}

/// A read-only query in `dir` that must not take optional locks (the user may be working there).
fn query(git: &str, dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = command(git);
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args(args);
    process::output_text(command)
}

/// The `(key, value)` pairs of one scope of the repository configuration in `dir`, in file order.
/// Keys are lower-cased. A key without a value (`[core] bare`) has an empty value.
///
/// Reading configuration runs nothing: no filter, hook, credential helper or fsmonitor, and
/// `--local` and `--worktree` follow no `include.path`.
fn config_entries(git: &str, dir: &Path, scope: &str) -> Result<Vec<(String, String)>, String> {
    let text = query(git, dir, &["config", scope, "--list", "-z"])?;
    Ok(text
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.split_once('\n') {
            Some((key, value)) => (key.to_ascii_lowercase(), value.to_string()),
            None => (entry.to_ascii_lowercase(), String::new()),
        })
        .collect())
}

/// Why the repository configuration of the checkout in `dir` must not be used, or `None` when it
/// names no program that git would run for us.
///
/// A package folder may have come with the project (a zip, a shared project), so its `.git/config`
/// is data from an unknown source. `-c` on the command line beats it for the keys [`command`]
/// pins (`core.fsmonitor`, and `core.hooksPath` for [`command_in_checkout`]), but git has many
/// more keys that name a program (a clean filter on every stat-dirty file in `git status`,
/// `core.sshCommand` and credential helpers in `git fetch`), and pinning those would also
/// override the user's own global setting. So a configuration that defines one is refused
/// instead; see [`is_command_key`]. The message names the key, never its value (it may hold a
/// secret).
///
/// Call it before the first git command that runs in an existing checkout.
pub fn untrusted_config(git: &str, dir: &Path) -> Option<String> {
    let unreadable = |what: &str, error: String| {
        // A plain query names the cause (dubious ownership, a corrupt repository) better than
        // `git config --local` does, which just says that it is not inside a repository.
        let cause = query(git, dir, &["rev-parse", "--git-dir"])
            .err()
            .unwrap_or(error);
        Some(format!("cannot read {what}: {cause}"))
    };
    let local = match config_entries(git, dir, "--local") {
        Ok(entries) => entries,
        Err(error) => return unreadable(".git/config", error),
    };
    let mut worktree = Vec::new();
    if local.iter().any(|(key, value)| {
        key == "extensions.worktreeconfig"
            && matches!(value.as_str(), "" | "true" | "yes" | "on" | "1")
    }) {
        // config.worktree is a second file of the same repository that git reads too.
        let path = match query(git, dir, &["rev-parse", "--git-path", "config.worktree"]) {
            Ok(path) => dir.join(path),
            Err(error) => return unreadable("config.worktree", error),
        };
        if path.exists() {
            match config_entries(git, dir, "--worktree") {
                Ok(entries) => worktree = entries,
                Err(error) => return unreadable("config.worktree", error),
            }
        }
    }
    local
        .iter()
        .chain(&worktree)
        .find(|(key, value)| is_command_key(key, value))
        .map(|(key, _)| format!("untrusted .git/config: {key}"))
}

/// Whether the configuration entry `key` (lower-case) = `value` makes git or one of its helpers
/// run a program or talk to a place the repository picks. Entries of the user's own global
/// configuration are never checked, only those found in a repository.
///
/// Not listed on purpose: `core.fsmonitor` and `core.hooksPath` (pinned with `-c` by
/// [`command`] and [`command_in_checkout`]), `alias.*`, `pager.*`, `core.editor` and the
/// `*tool.*.cmd` keys (git runs none of them in the commands used here: no tty, no merge, no
/// diff tool).
fn is_command_key(key: &str, value: &str) -> bool {
    let Some((section, rest)) = key.split_once('.') else {
        return false;
    };
    // `section.subsection.variable`: the variable is what follows the last dot.
    let variable = rest.rsplit('.').next().unwrap_or(rest);
    match section {
        // Clean and smudge filters run on every file whose .gitattributes selects the driver;
        // the standard `git lfs install --local` entries are fixed strings.
        "filter" => {
            matches!(variable, "clean" | "smudge" | "process")
                && !is_standard_lfs_filter(key, value)
        }
        "diff" => matches!(variable, "textconv" | "command") || rest == "external",
        "merge" => variable == "driver",
        "core" => matches!(
            rest,
            "sshcommand" | "gitproxy" | "askpass" | "alternaterefscommand"
        ),
        // A helper name resolves to git-credential-<name> next to git; anything else (`!cmd`, a
        // path) is a program of the repository's choosing.
        "credential" => variable == "helper" && !is_standard_credential_helper(value),
        "gpg" => matches!(variable, "program" | "defaultkeycommand"),
        "remote" => matches!(variable, "uploadpack" | "receivepack" | "proxy" | "vcs"),
        "http" | "https" => variable == "proxy",
        "url" => matches!(variable, "insteadof" | "pushinsteadof"),
        "protocol" => true,
        // `--local --list` shows these without expanding them, so what they pull in is unknown.
        "include" | "includeif" => true,
        // git-lfs runs the programs of `lfs.customtransfer.*` and `lfs.extension.*`.
        "lfs" => rest.starts_with("customtransfer.") || rest.starts_with("extension."),
        _ => false,
    }
}

/// What `git lfs install --local` writes: LFS is the one filter that ships with real clones.
fn is_standard_lfs_filter(key: &str, value: &str) -> bool {
    matches!(
        (key, value),
        ("filter.lfs.clean", "git-lfs clean -- %f")
            | (
                "filter.lfs.smudge",
                "git-lfs smudge -- %f" | "git-lfs smudge --skip -- %f"
            )
            | (
                "filter.lfs.process",
                "git-lfs filter-process" | "git-lfs filter-process --skip"
            )
    )
}

fn is_standard_credential_helper(value: &str) -> bool {
    let name = value.split_whitespace().next().unwrap_or_default();
    matches!(
        name,
        "" | "store"
            | "cache"
            | "manager"
            | "manager-core"
            | "wincred"
            | "osxkeychain"
            | "libsecret"
    )
}

/// `isCheckout` = `<dir>/.git` exists (a directory, or a file for worktrees and submodules).
///
/// A checkout whose configuration is not trusted ([`untrusted_config`]) is described by `error`
/// alone: no other git command runs in it.
pub fn inspect(git: &str, dir: &Path) -> GitInfo {
    if !dir.join(".git").exists() {
        return GitInfo::default();
    }
    if let Some(message) = untrusted_config(git, dir) {
        return GitInfo {
            is_checkout: true,
            error: Some(message),
            ..GitInfo::default()
        };
    }
    let mut error = None;
    // No tag at HEAD is normal; every other failed query means git cannot read this checkout.
    let mut required = |result: Result<String, String>| match result {
        Ok(text) => Some(text),
        Err(message) => {
            error.get_or_insert(message);
            None
        }
    };
    let tag = query(git, dir, &["describe", "--tags", "--exact-match"])
        .ok()
        .filter(|tag| !tag.is_empty());
    let branch = required(query(git, dir, &["rev-parse", "--abbrev-ref", "HEAD"]))
        .filter(|branch| !branch.is_empty() && branch != "HEAD");
    let short_sha =
        required(query(git, dir, &["rev-parse", "--short", "HEAD"])).filter(|sha| !sha.is_empty());
    // `--ignore-submodules=all` keeps git from running status inside submodules, which have
    // their own (equally untrusted) configuration.
    let dirty = required(query(
        git,
        dir,
        &[
            "status",
            "--porcelain",
            "--untracked-files=no",
            "--ignore-submodules=all",
        ],
    ))
    .map(|output| !output.is_empty())
    .unwrap_or(false);
    GitInfo {
        is_checkout: true,
        tag,
        branch,
        short_sha,
        dirty,
        error,
    }
}

/// The version line of a git executable ("git version 2.49.0.windows.1").
pub fn version(git: &str) -> Result<String, String> {
    validate_executable(git)?;
    let mut command = command(git);
    command.arg("--version");
    let output = process::output_text(command)?;
    if output.starts_with("git version") {
        Ok(output)
    } else {
        Err(format!("unexpected output: {output}"))
    }
}

/// The setting must name git itself: no quotes or control characters, no shell, no batch script
/// (Windows runs those through cmd.exe, which would re-parse the arguments).
pub fn validate_executable(git: &str) -> Result<(), String> {
    let git = git.trim();
    if git.is_empty() {
        return Err("the git executable is empty".into());
    }
    if git.chars().any(|c| c == '"' || c.is_control()) {
        return Err("the git executable must not contain quotes or control characters".into());
    }
    let path = Path::new(git);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(extension.as_str(), "bat" | "cmd" | "ps1")
        || matches!(stem.as_str(), "cmd" | "powershell" | "pwsh" | "sh" | "bash")
    {
        return Err(format!(
            "'{git}' must name git itself, not a shell or a script"
        ));
    }
    Ok(())
}

/// A ref that looks like a commit id; `git clone --branch` only accepts branches and tags.
pub fn is_commit_id(git_ref: &str) -> bool {
    (7..=40).contains(&git_ref.len()) && git_ref.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
        let mut command = command("git");
        command
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
            ])
            .args(args);
        process::output_text(command).unwrap();
    }

    #[test]
    fn inspects_checkouts() {
        if version("git").is_err() {
            eprintln!("git is not available; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(inspect("git", dir.path()), GitInfo::default());
        assert_eq!(
            inspect("git", dir.path()).to_state(),
            GitState {
                is_checkout: false,
                git_ref: None,
                dirty: false
            }
        );

        git(dir.path(), &["init", "-q", "-b", "main"]);
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "one"]);
        let info = inspect("git", dir.path());
        assert!(info.is_checkout && !info.dirty);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.to_state().git_ref.as_deref(), Some("main"));

        git(dir.path(), &["tag", "v0.1.0"]);
        git(dir.path(), &["checkout", "-q", "--detach", "v0.1.0"]);
        fs::write(dir.path().join("a.txt"), "two").unwrap();
        fs::write(dir.path().join("untracked.txt"), "x").unwrap();
        let info = inspect("git", dir.path());
        assert_eq!(info.tag.as_deref(), Some("v0.1.0"));
        assert_eq!(info.branch, None);
        assert!(info.dirty);
        assert_eq!(info.to_state().git_ref.as_deref(), Some("v0.1.0"));
    }

    #[test]
    fn a_failing_git_is_reported_not_treated_as_clean() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let info = inspect("ubot-git-that-does-not-exist", dir.path());
        assert!(info.is_checkout);
        assert!(info.error.is_some(), "{info:?}");
        assert_eq!(info.to_state().git_ref, None);
        // A directory without .git is simply not a checkout.
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(
            inspect("ubot-git-that-does-not-exist", plain.path()).error,
            None
        );
    }

    #[test]
    fn fsmonitor_of_an_untrusted_repository_is_not_run() {
        if version("git").is_err() {
            eprintln!("git is not available; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "one"]);

        let marker = dir.path().join("fsmonitor-ran.txt");
        let script = dir.path().join("fsm.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho pwned > \"{}\"\nexit 1\n",
                marker.to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        let script_path = script.to_string_lossy().replace('\\', "/");
        git(dir.path(), &["config", "core.fsmonitor", &script_path]);

        // Control: a plain git (no -c override) runs the program, so the check below means something.
        let mut plain = process::command("git");
        plain
            .args(["-C"])
            .arg(dir.path())
            .args(["status", "--porcelain", "--untracked-files=no"]);
        let _ = process::output_text(plain);
        let control_ran = marker.exists();
        let _ = fs::remove_file(&marker);

        let info = inspect("git", dir.path());
        assert!(info.is_checkout && info.error.is_none(), "{info:?}");
        if control_ran {
            assert!(!marker.exists(), "inspect ran the repository's fsmonitor");
        } else {
            eprintln!("fsmonitor control did not run on this machine; weak check only");
        }
    }

    /// A repository with `a.txt` committed.
    fn committed_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        fs::write(dir.path().join("a.txt"), "one").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "one"]);
        dir
    }

    /// A shell command (git runs filters and ssh commands through sh) that records that it ran.
    fn marker_command(marker: &Path) -> String {
        format!(
            "echo pwned > \"{}\"; cat",
            marker.to_string_lossy().replace('\\', "/")
        )
    }

    /// Changes the modification time of `a.txt` without changing its content, so that git has to
    /// look at the file again (running its clean filter) on the next `git status`.
    fn make_stat_dirty(dir: &Path) {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(dir.join("a.txt"))
            .unwrap();
        file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
    }

    #[test]
    fn a_clean_filter_of_an_untrusted_repository_is_not_run() {
        if version("git").is_err() {
            eprintln!("git is not available; skipping");
            return;
        }
        let dir = committed_repo();
        fs::write(dir.path().join(".gitattributes"), "* filter=evil\n").unwrap();
        git(dir.path(), &["add", ".gitattributes"]);
        git(dir.path(), &["commit", "-q", "-m", "attributes"]);

        let marker = dir.path().join("filter-ran.txt");
        git(
            dir.path(),
            &["config", "filter.evil.clean", &marker_command(&marker)],
        );

        // Control: a plain `git status` of a stat-dirty file runs the filter, so the check
        // below means something. (`--no-optional-locks`: the control must not refresh the index.)
        make_stat_dirty(dir.path());
        let mut plain = process::command("git");
        plain
            .args(["--no-optional-locks", "-C"])
            .arg(dir.path())
            .args(["status", "--porcelain", "--untracked-files=no"]);
        let _ = process::output_text(plain);
        let control_ran = marker.exists();
        let _ = fs::remove_file(&marker);

        make_stat_dirty(dir.path());
        let info = inspect("git", dir.path());
        assert!(info.is_checkout, "{info:?}");
        let error = info.error.as_deref().expect("the configuration is refused");
        assert!(error.contains("filter.evil.clean"), "{error}");
        assert_eq!(info.to_state().git_ref, None, "nothing was queried");
        assert!(!info.dirty);
        if control_ran {
            assert!(
                !marker.exists(),
                "inspect ran the repository's clean filter"
            );
        } else {
            eprintln!("clean filter control did not run on this machine; weak check only");
        }
    }

    #[test]
    fn programs_in_the_configuration_are_refused() {
        if version("git").is_err() {
            eprintln!("git is not available; skipping");
            return;
        }
        let dir = committed_repo();
        assert_eq!(untrusted_config("git", dir.path()), None);
        // Nothing but the plain settings of a clone, a developer's own and a LFS install.
        for (key, value) in [
            ("remote.origin.url", "https://example.com/team/uBotX.git"),
            ("core.fsmonitor", "true"),
            ("core.hooksPath", ".githooks"),
            ("credential.helper", "manager"),
            ("filter.lfs.clean", "git-lfs clean -- %f"),
            ("filter.lfs.required", "true"),
            ("diff.renames", "true"),
        ] {
            git(dir.path(), &["config", key, value]);
        }
        assert_eq!(untrusted_config("git", dir.path()), None);

        let marker = dir.path().join("ssh-ran.txt");
        for (key, value, refused) in [
            (
                "core.sshCommand",
                marker_command(&marker),
                "core.sshcommand",
            ),
            ("core.gitProxy", "evil".into(), "core.gitproxy"),
            ("credential.helper", "!evil".into(), "credential.helper"),
            ("filter.lfs.smudge", "evil %f".into(), "filter.lfs.smudge"),
            ("include.path", "../elsewhere".into(), "include.path"),
        ] {
            let dir = committed_repo();
            git(dir.path(), &["config", key, &value]);
            let message = untrusted_config("git", dir.path()).expect(key);
            assert!(
                message.starts_with("untrusted .git/config: ") && message.ends_with(refused),
                "{message}"
            );
            // The message names the key, not what it is set to.
            assert!(!message.contains("evil"), "{message}");
        }
        assert!(!marker.exists());
    }

    #[test]
    fn the_worktree_configuration_is_checked_too() {
        if version("git").is_err() {
            eprintln!("git is not available; skipping");
            return;
        }
        let dir = committed_repo();
        git(dir.path(), &["config", "extensions.worktreeConfig", "true"]);
        // No config.worktree yet: nothing to refuse, and no error.
        assert_eq!(untrusted_config("git", dir.path()), None);
        git(
            dir.path(),
            &["config", "--worktree", "filter.evil.clean", "echo x"],
        );
        let message = untrusted_config("git", dir.path()).expect("config.worktree is read");
        assert!(message.ends_with("filter.evil.clean"), "{message}");
    }

    #[test]
    fn command_keys() {
        for (key, value) in [
            ("filter.x.clean", "c"),
            ("filter.x.smudge", "c"),
            ("filter.x.process", "c"),
            ("filter.lfs.clean", "git-lfs clean %f"),
            ("diff.x.textconv", "c"),
            ("diff.x.command", "c"),
            ("diff.external", "c"),
            ("merge.x.driver", "c"),
            ("core.sshcommand", "c"),
            ("core.gitproxy", "c"),
            ("core.askpass", "c"),
            ("core.alternaterefscommand", "c"),
            ("credential.helper", "!c"),
            ("credential.helper", "/usr/bin/c"),
            ("credential.https://example.com.helper", "c:\\x.exe"),
            ("gpg.program", "c"),
            ("gpg.ssh.defaultkeycommand", "c"),
            ("remote.origin.uploadpack", "c"),
            ("remote.origin.receivepack", "c"),
            ("remote.origin.proxy", "http://c"),
            ("remote.origin.vcs", "c"),
            ("http.proxy", "http://c"),
            ("http.https://example.com.proxy", "http://c"),
            ("url.https://c/.insteadof", "https://example.com/"),
            ("protocol.ext.allow", "always"),
            ("protocol.allow", "always"),
            ("include.path", "c"),
            ("includeif.gitdir:c.path", "c"),
            ("lfs.customtransfer.x.path", "c"),
            ("lfs.extension.x.clean", "c"),
        ] {
            assert!(is_command_key(key, value), "{key} = {value}");
        }
        for (key, value) in [
            ("core.repositoryformatversion", "0"),
            ("core.fsmonitor", "c"),
            ("core.hookspath", "c"),
            ("core.bare", ""),
            ("remote.origin.url", "https://example.com/x.git"),
            ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
            ("branch.main.remote", "origin"),
            ("filter.lfs.clean", "git-lfs clean -- %f"),
            ("filter.lfs.smudge", "git-lfs smudge --skip -- %f"),
            ("filter.lfs.process", "git-lfs filter-process"),
            ("filter.lfs.required", "true"),
            ("credential.helper", "manager"),
            ("credential.helper", "store --file x"),
            ("credential.helper", ""),
            ("credential.https://example.com.username", "me"),
            ("diff.renames", "true"),
            ("submodule.x.url", "https://example.com/x.git"),
            ("lfs.url", "https://example.com/lfs"),
            ("alias.st", "status"),
            ("bare", ""),
        ] {
            assert!(!is_command_key(key, value), "{key} = {value}");
        }
    }

    #[test]
    fn validates_the_executable() {
        assert!(validate_executable("git").is_ok());
        assert!(validate_executable(r"C:\Program Files\Git\cmd\git.exe").is_ok());
        for bad in [
            "",
            "cmd",
            r"C:\x\run.bat",
            "powershell.exe",
            "git\"x",
            "bash",
        ] {
            assert!(validate_executable(bad).is_err(), "{bad}");
        }
        assert!(is_commit_id("c35fa58"));
        assert!(!is_commit_id("v0.1.0"));
        assert!(!is_commit_id("abc"));
    }
}

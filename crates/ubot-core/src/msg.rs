//! Message codes (SPEC §8) and their English templates.
//!
//! The backend only emits codes listed in [`EN`]; the frontend translates them from its locale files
//! and the CLI renders them with [`render_en`]. Adding a code here requires adding it to both
//! `app/src/i18n/en.json` and `zh-CN.json` in the same change.

use std::collections::BTreeMap;

use crate::model::Msg;

/// English templates keyed by code, exactly as in SPEC §8.
pub const EN: &[(&str, &str)] = &[
    (
        "problem.requiresVersion",
        "Requires {dep} {range}, but {installed} is installed.",
    ),
    (
        "problem.requiresMissing",
        "Requires {dep} {range}, which is not installed.",
    ),
    (
        "problem.requiresDisabled",
        "Requires {dep}, which is disabled.",
    ),
    (
        "problem.enginePluginMissing",
        "Needs engine plugin {plugin}, which this engine does not have.",
    ),
    (
        "problem.enginePluginDisabled",
        "Needs engine plugin {plugin}, which is disabled.",
    ),
    (
        "problem.legacyConflict",
        "{legacy} and {replacement} are both enabled.",
    ),
    (
        "problem.descriptorInvalid",
        "The descriptor of {package} has errors: {detail}",
    ),
    ("problem.dependencyCycle", "Dependency cycle: {cycle}."),
    (
        "problem.engineNotFound",
        "Engine {association} for this project was not found.",
    ),
    (
        "problem.indexSourceFailed",
        "Could not load {source}: {error}",
    ),
    (
        "problem.rosBridgeUnreachable",
        "Could not connect to ubot_ros_bridge at {address}.",
    ),
    (
        "problem.prebuiltEngineMismatch",
        "{package} was installed prebuilt for UE {built}, but this project uses UE {engine}.",
    ),
    ("fact.package", "Package"),
    ("fact.requires", "Requires"),
    ("fact.installed", "Installed"),
    ("fact.legacy", "Legacy plugin"),
    ("fact.replacement", "Replaced by"),
    ("fact.redirects", "Redirects"),
    ("fact.source", "Source"),
    ("fact.address", "Address"),
    ("fact.time", "Time"),
    ("fact.plugin", "Plugin"),
    ("fact.engine", "Engine"),
    ("fact.file", "File"),
    ("session.lastPie", "last PIE session"),
    ("session.lastEditor", "last editor session"),
    ("session.lastGame", "last game session"),
    ("fix.updateTo", "Update {package} to {version}"),
    ("fix.install", "Install {package}"),
    ("fix.enable", "Enable {package}"),
    ("fix.disable", "Disable {package}"),
    ("fix.ignore", "Ignore"),
    ("fix.openRosSettings", "Open ROS Settings"),
    ("fix.openBuildSettings", "Open Build Settings"),
    ("fix.reinstall", "Reinstall {package}"),
    ("step.clone", "Clone {package} {version}"),
    ("step.clone.detail", "{repo} → {dest}"),
    ("step.checkout", "Check out {package} {version}"),
    ("step.checkout.detail", "{from} → {to}"),
    ("step.download", "Download {package} {version}"),
    ("step.download.detail", "{source} → {dest}"),
    ("step.enablePackage", "Enable {package}"),
    ("step.disablePackage", "Disable {package}"),
    ("step.enableEnginePlugin", "Enable {plugin}"),
    ("step.removePackage", "Remove {package}"),
    ("step.removePackage.detail", "{dest} → Recycle Bin"),
    ("step.detail.project", "{file}"),
    ("step.detail.requiredBy", "required by {package}"),
    ("step.build", "Build {target}"),
    ("step.build.detail", "{platform} · {configuration}"),
    ("step.runTests", "Run package tests"),
    ("step.runTests.detail", "{filter}"),
    ("step.openEditor", "Reopen Unreal Editor"),
    ("step.openEditor.detail", "{project}"),
    ("plan.notInIndex", "{package} is not in any package index."),
    (
        "plan.planned",
        "{package} is planned and cannot be installed yet.",
    ),
    ("plan.noVersion", "No version of {package} matches {range}."),
    ("plan.notInstalled", "{package} is not installed."),
    ("plan.alreadyInstalled", "{package} is already installed."),
    (
        "plan.notGit",
        "{package} is not a git checkout; reinstall it to update.",
    ),
    ("plan.dirty", "{package} has local changes."),
    (
        "plan.onBranch",
        "{package} is on branch {branch}; it will be detached at the new version.",
    ),
    (
        "plan.breaksDependent",
        "{dependent} requires {package} {range}.",
    ),
    (
        "plan.requestedVersionChanged",
        "Requested {package} {requested}, but the plan resolved {resolved}.",
    ),
    (
        "plan.alsoDisable",
        "Disabling {package} also disables {dependents}.",
    ),
    (
        "plan.enginePluginMissing",
        "The engine has no plugin named {plugin}.",
    ),
    ("plan.folderExists", "{dest} already exists."),
    ("plan.unsafeUrl", "Refusing repository URL {url}."),
    (
        "plan.noEngine",
        "No Unreal Engine was found for this project.",
    ),
    (
        "plan.engineVersion",
        "{package} {version} supports UE {range}, this project uses {engine}.",
    ),
    ("plan.outsideProject", "{path} is outside the project."),
    (
        "plan.pluginRoot",
        "{path} is a plugin folder that holds more than this package.",
    ),
    (
        "plan.notPluginDir",
        "{path} is not inside the project's Plugins folder or one of its AdditionalPluginDirectories.",
    ),
    (
        "plan.gitUnreadable",
        "Cannot read the git state of {package}: {error}",
    ),
    ("plan.unsafeRef", "Refusing git ref {ref} of {package}."),
    ("plan.unsafeName", "Refusing package name {package}."),
    (
        "plan.unsafeProjectName",
        "The project name {name} cannot be built; use only letters, digits and underscores in the .uproject file name.",
    ),
    (
        "plan.unsafeBuildPath",
        "{path} contains a % character, which the Windows build scripts would expand.",
    ),
    (
        "plan.projectReadOnly",
        "{file} is read-only; check it out or clear the read-only flag first.",
    ),
    ("plan.noProject", "No project is open."),
    (
        "plan.prebuiltUnavailable",
        "No prebuilt {package} {version} for UE {engine}; it will be built from source.",
    ),
    (
        "plan.noPrebuilt",
        "No prebuilt {package} {version} for UE {engine}; uninstall it and install it again to build from source.",
    ),
    (
        "plan.noCompiler",
        "A C++ build is needed, but Visual Studio 2022 was not found.",
    ),
    ("text", "{text}"),
    ("task.apply", "Apply Changes · {project}"),
    ("task.recipe", "Apply Recipe {recipe} · {project}"),
    ("task.install", "Install {package} {version}"),
    ("task.busy", "Another task is running."),
    (
        "task.editorRunning",
        "Unreal Editor is running this project.",
    ),
];

/// Code whose template is only its `{text}` parameter in every language. Used to show text that
/// has no code of its own (for example a session problem whose code the manager does not know).
pub const VERBATIM_CODE: &str = "text";

pub fn en_template(code: &str) -> Option<&'static str> {
    EN.iter()
        .find(|(key, _)| *key == code)
        .map(|(_, template)| *template)
}

pub fn is_known_code(code: &str) -> bool {
    en_template(code).is_some()
}

/// Builds a message. Debug builds assert that the code is part of the contract.
pub fn msg(code: &str, params: &[(&str, &str)]) -> Msg {
    debug_assert!(is_known_code(code), "unknown message code {code}");
    Msg {
        code: code.to_string(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    }
}

pub fn msg0(code: &str) -> Msg {
    msg(code, &[])
}

/// A message that renders as `text` in every language.
pub fn verbatim(text: &str) -> Msg {
    msg(VERBATIM_CODE, &[("text", text)])
}

/// Renders a message with the English templates. Unknown codes render as the code itself and
/// placeholders without a parameter are kept as written.
pub fn render_en(message: &Msg) -> String {
    match en_template(&message.code) {
        Some(template) => fill(template, &message.params),
        None => message.code.clone(),
    }
}

fn fill(template: &str, params: &BTreeMap<String, String>) -> String {
    let mut result = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        result.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let key = &after[..close];
                match params.get(key) {
                    Some(value) => result.push_str(value),
                    None => {
                        result.push('{');
                        result.push_str(key);
                        result.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                result.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_templates() {
        let message = msg(
            "problem.requiresVersion",
            &[
                ("dep", "UBotCore"),
                ("range", "^0.2.0"),
                ("installed", "UBotCore 0.1.0"),
            ],
        );
        assert_eq!(
            render_en(&message),
            "Requires UBotCore ^0.2.0, but UBotCore 0.1.0 is installed."
        );
        assert_eq!(render_en(&msg0("fix.ignore")), "Ignore");
        assert_eq!(render_en(&verbatim("Hello {x}")), "Hello {x}");
        let missing = Msg {
            code: "fix.enable".into(),
            params: BTreeMap::new(),
        };
        assert_eq!(render_en(&missing), "Enable {package}");
        let unknown = Msg {
            code: "nope".into(),
            params: BTreeMap::new(),
        };
        assert_eq!(render_en(&unknown), "nope");
    }

    #[test]
    fn codes_are_unique() {
        let mut codes: Vec<&str> = EN.iter().map(|(code, _)| *code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
        assert!(is_known_code(VERBATIM_CODE));
        assert_eq!(en_template(VERBATIM_CODE), Some("{text}"));
    }

    #[test]
    fn renders_requested_version_changed() {
        assert_eq!(
            render_en(&msg(
                "plan.requestedVersionChanged",
                &[
                    ("package", "UBotCore"),
                    ("requested", "0.1.3"),
                    ("resolved", "0.1.4"),
                ],
            )),
            "Requested UBotCore 0.1.3, but the plan resolved 0.1.4."
        );
    }
}

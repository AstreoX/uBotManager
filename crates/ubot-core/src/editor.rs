//! Detecting and closing the Unreal Editor that runs a project (SPEC §3).

use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::model::EditorState;
use crate::process;

const EDITOR_NAMES: &[&str] = &[
    "unrealeditor.exe",
    "unrealeditor-cmd.exe",
    "unrealeditor",
    "unrealeditor-cmd",
];

fn normalize_for_match(text: &str) -> String {
    text.replace('/', "\\").to_lowercase()
}

/// True when a command line names `project` (case-insensitive, '/' and '\' equivalent).
pub fn command_line_mentions(command_line: &str, project: &Path) -> bool {
    let project = normalize_for_match(&project.to_string_lossy());
    !project.is_empty() && normalize_for_match(command_line).contains(&project)
}

/// The editor process running `project`, if any.
pub fn find_editor(project: &Path) -> EditorState {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    let mut pids: Vec<u32> = system
        .processes()
        .values()
        .filter(|process| {
            EDITOR_NAMES.contains(&process.name().to_string_lossy().to_lowercase().as_str())
        })
        .filter(|process| {
            let command_line: Vec<String> = process
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            command_line_mentions(&command_line.join(" "), project)
        })
        .map(|process| process.pid().as_u32())
        .collect();
    pids.sort_unstable();
    match pids.first() {
        Some(pid) => EditorState {
            running: true,
            pid: Some(*pid),
        },
        None => EditorState {
            running: false,
            pid: None,
        },
    }
}

/// Asks the editor to close gracefully; it may prompt to save.
pub fn close_editor(project: &Path) -> Result<(), String> {
    match find_editor(project).pid {
        Some(pid) => process::request_close(pid),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_command_lines() {
        let project = Path::new(r"E:\Work\DevHost\UBotDev.uproject");
        assert!(command_line_mentions(
            r#""D:\UE\UnrealEditor.exe" "e:/work/devhost/UBotDev.uproject" -log"#,
            project
        ));
        assert!(command_line_mentions(
            r"UnrealEditor-Cmd.exe E:\Work\DevHost\UBOTDEV.uproject -run=x",
            project
        ));
        assert!(!command_line_mentions(
            r"UnrealEditor.exe E:\Work\Other\UBotDev.uproject",
            project
        ));
        assert!(!find_editor(Path::new(r"Z:\nowhere\none.uproject")).running);
    }
}

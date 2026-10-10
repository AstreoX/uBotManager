//! Command line: `ubot-manager.exe [--project <path>]`.

use std::path::Path;

/// The `--project <path>` (or `--project=<path>`) argument; the last one wins. `args` excludes the
/// program name. Other arguments are ignored, so launchers can pass their own flags.
pub fn project_arg<I: IntoIterator<Item = String>>(args: I) -> Option<String> {
    let mut project = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--project" {
            project = args.next();
        } else if let Some(value) = arg.strip_prefix("--project=") {
            project = Some(value.to_string());
        }
    }
    project.filter(|path| !path.trim().is_empty())
}

/// A project path from another process's command line, made absolute against its working
/// directory.
pub fn resolve_against(path: &str, cwd: &Path) -> String {
    let path = Path::new(path.trim());
    if path.is_absolute() || cwd.as_os_str().is_empty() {
        path.to_string_lossy().into_owned()
    } else {
        cwd.join(path).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn finds_the_project_argument() {
        assert_eq!(project_arg(args(&[])), None);
        assert_eq!(
            project_arg(args(&["--project", r"E:\Sim\Sim.uproject"])),
            Some(r"E:\Sim\Sim.uproject".into())
        );
        assert_eq!(
            project_arg(args(&["--project=E:/My Sim/Sim.uproject"])),
            Some("E:/My Sim/Sim.uproject".into())
        );
        assert_eq!(
            project_arg(args(&[
                "--verbose",
                "--project",
                "a.uproject",
                "--project",
                "b.uproject"
            ])),
            Some("b.uproject".into())
        );
        assert_eq!(project_arg(args(&["--project"])), None);
        assert_eq!(project_arg(args(&["--project", "  "])), None);
        assert_eq!(project_arg(args(&["Sim.uproject"])), None);
    }

    #[cfg(windows)]
    #[test]
    fn resolves_relative_paths_against_the_caller() {
        let cwd = Path::new(r"E:\Projects");
        assert_eq!(
            resolve_against(r"Sim\Sim.uproject", cwd),
            r"E:\Projects\Sim\Sim.uproject"
        );
        assert_eq!(
            resolve_against(r"D:\Other\Other.uproject", cwd),
            r"D:\Other\Other.uproject"
        );
        assert_eq!(
            resolve_against("Sim.uproject", Path::new("")),
            "Sim.uproject"
        );
    }
}

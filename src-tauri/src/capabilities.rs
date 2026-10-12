//! Checks of `capabilities/default.json` that `tauri-build` cannot do: the opener scope must let the
//! frontend open `.uproject` files ("Open in Unreal Editor") and nothing else.

use std::path::{Path, PathBuf};

use serde_json::Value;

const CAPABILITY: &str = include_str!("../capabilities/default.json");

fn open_path_patterns() -> Vec<String> {
    let capability: Value = serde_json::from_str(CAPABILITY).unwrap();
    capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|permission| permission["identifier"] == "opener:allow-open-path")
        .flat_map(|permission| permission["allow"].as_array().cloned().unwrap_or_default())
        .map(|entry| entry["path"].as_str().unwrap().to_string())
        .collect()
}

/// Matches like `tauri::scope::fs::Scope::is_allowed`: patterns and paths are rebuilt from their
/// components, and separators must match literally.
fn allowed(path: &str) -> bool {
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..Default::default()
    };
    let path: PathBuf = Path::new(path).components().collect();
    open_path_patterns().iter().any(|pattern| {
        let pattern: PathBuf = Path::new(pattern).components().collect();
        glob::Pattern::new(&pattern.to_string_lossy())
            .unwrap()
            .matches_path_with(&path, options)
    })
}

#[cfg(windows)]
#[test]
fn opener_may_open_projects_only() {
    assert!(allowed(r"D:\Projects\RobotSim\RobotSim.uproject"));
    // The scope canonicalizes existing files to verbatim paths.
    assert!(allowed(r"\\?\E:\Sims\My Sim\MyRobotSim.uproject"));
    assert!(allowed(r"\\?\UNC\server\share\Sim\Sim.uproject"));
    assert!(!allowed(r"\\?\E:\Sims\My Sim\tool.exe"));
    assert!(!allowed(r"\\?\C:\Windows\System32\cmd.exe"));
    assert!(!allowed(r"\\?\E:\Sims\Sim.uproject.bat"));
}

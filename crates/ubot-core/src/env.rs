//! Environment checks (SPEC §7 `check_environment`, `test_git`, `test_ros`).

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::git;
use crate::model::{EngineInfo, EnvCheck, EnvCheckId, GitTestResult, RosTestResult};
use crate::process;

pub const VSWHERE: &str = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
pub const ROS_TIMEOUT: Duration = Duration::from_secs(2);

pub fn test_git(path: &str) -> GitTestResult {
    match git::version(path) {
        Ok(version) => GitTestResult {
            ok: true,
            version: Some(version),
            error: None,
        },
        Err(error) => GitTestResult {
            ok: false,
            version: None,
            error: Some(error),
        },
    }
}

/// "host:port", with brackets around IPv6 addresses.
pub fn ros_address(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// A TCP connection to the ROS bridge with a 2 s timeout in all. A name such as "localhost"
/// resolves to several addresses (on Windows `::1` first), and a bridge usually listens on only one
/// of them, so every address is tried, IPv4 first (as UBotROS does), until one connects.
pub fn test_ros(host: &str, port: u16) -> RosTestResult {
    let address = ros_address(host, port);
    let mut sockets: Vec<SocketAddr> = match address.to_socket_addrs() {
        Ok(addresses) => addresses.collect(),
        Err(error) => {
            return RosTestResult {
                ok: false,
                error: Some(error.to_string()),
            }
        }
    };
    if sockets.is_empty() {
        return RosTestResult {
            ok: false,
            error: Some(format!("{address} does not resolve")),
        };
    }
    // Stable: the order inside each family is kept.
    sockets.sort_by_key(SocketAddr::is_ipv6);

    let deadline = Instant::now() + ROS_TIMEOUT;
    let mut first_error = None;
    for socket in sockets {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        match TcpStream::connect_timeout(&socket, remaining) {
            Ok(_) => {
                return RosTestResult {
                    ok: true,
                    error: None,
                }
            }
            Err(error) => {
                first_error.get_or_insert(error.to_string());
            }
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    RosTestResult {
        ok: false,
        error: first_error,
    }
}

/// Installation path of Visual Studio 2022 (any edition, Build Tools included).
pub fn find_vs2022() -> Result<String, String> {
    if !Path::new(VSWHERE).is_file() {
        return Err("Visual Studio Installer (vswhere.exe) was not found".into());
    }
    let mut command = process::command(VSWHERE);
    command.args([
        "-version",
        "[17,18)",
        "-products",
        "*",
        "-property",
        "installationPath",
    ]);
    let output = process::output_text(command)?;
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Visual Studio 2022 is not installed".into())
}

/// Runs the requested checks; unknown ids are ignored, `rosBridge` is reported as not checked.
pub fn check_environment(
    checks: &[String],
    git_path: &str,
    engine: Option<&EngineInfo>,
    project_open: bool,
) -> Vec<EnvCheck> {
    let mut results = Vec::new();
    for id in checks.iter().filter_map(|check| EnvCheckId::parse(check)) {
        if results.iter().any(|existing: &EnvCheck| existing.id == id) {
            continue;
        }
        let (ok, detail) = match id {
            EnvCheckId::Git => match git::version(git_path) {
                Ok(version) => (Some(true), Some(version)),
                Err(error) => (Some(false), Some(error)),
            },
            EnvCheckId::Vs2022 => match find_vs2022() {
                Ok(path) => (Some(true), Some(path)),
                Err(error) => (Some(false), Some(error)),
            },
            EnvCheckId::Engine => match (engine, project_open) {
                (Some(engine), _) => (
                    Some(true),
                    Some(format!("{} · {}", engine.version, engine.root)),
                ),
                (None, true) => (
                    Some(false),
                    Some("No Unreal Engine was found for this project.".into()),
                ),
                (None, false) => (None, Some("No project is open.".into())),
            },
            EnvCheckId::RosBridge => (None, None),
        };
        results.push(EnvCheck { id, ok, detail });
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EngineKind;
    use std::net::TcpListener;

    #[test]
    fn ros_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(test_ros("127.0.0.1", port).ok);
        drop(listener);
        let result = test_ros("127.0.0.1", port);
        assert!(!result.ok && result.error.is_some());
        assert!(!test_ros("bad host name", 1).ok);
        // "localhost" can resolve to ::1 before 127.0.0.1; a bridge on IPv4 must still be found.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(test_ros("localhost", port).ok);
        drop(listener);
        assert!(!test_ros("localhost", port).ok);
        assert_eq!(ros_address("::1", 8268), "[::1]:8268");
        assert_eq!(ros_address("127.0.0.1", 8268), "127.0.0.1:8268");
    }

    #[test]
    fn environment_checks() {
        let engine = EngineInfo {
            id: "5.5".into(),
            version: "5.5.4".into(),
            root: "D:\\UE".into(),
            kind: EngineKind::Launcher,
        };
        let checks: Vec<String> = ["engine", "rosBridge", "nope", "engine"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let results = check_environment(&checks, "git", Some(&engine), true);
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0],
            EnvCheck {
                id: EnvCheckId::Engine,
                ok: Some(true),
                detail: Some("5.5.4 · D:\\UE".into())
            }
        );
        assert_eq!(
            results[1],
            EnvCheck {
                id: EnvCheckId::RosBridge,
                ok: None,
                detail: None
            }
        );
        let results = check_environment(&["engine".into()], "git", None, false);
        assert_eq!(results[0].ok, None);
        let results = check_environment(&["git".into()], "cmd", None, false);
        assert_eq!(results[0].ok, Some(false));
        assert!(!test_git("powershell").ok);
    }
}

//! File system helpers: lexical path normalization, project-relative locations, atomic writes and
//! the manager's own directories (SPEC §6).

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

/// Environment variable that moves `<config>`, `<data>` and `<cache>` under one directory.
pub const HOME_ENV: &str = "UBOT_MANAGER_HOME";

/// Where the manager keeps its settings, task history and caches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl AppDirs {
    /// `%APPDATA%\AstreoX\uBot Manager` for settings, `%LOCALAPPDATA%\AstreoX\uBot Manager` for data
    /// and its `cache` subfolder, or everything under `UBOT_MANAGER_HOME` when that is set.
    pub fn from_env() -> AppDirs {
        if let Some(home) = std::env::var_os(HOME_ENV).filter(|value| !value.is_empty()) {
            return AppDirs::under(PathBuf::from(home));
        }
        let config_base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
        let data_base = dirs::data_local_dir().unwrap_or_else(|| config_base.clone());
        let config = config_base.join("AstreoX").join("uBot Manager");
        let data = data_base.join("AstreoX").join("uBot Manager");
        AppDirs {
            config,
            cache: data.join("cache"),
            data,
        }
    }

    /// Everything under one directory (`<home>/settings.json`, `<home>/tasks.json`, `<home>/cache`).
    pub fn under(home: PathBuf) -> AppDirs {
        AppDirs {
            config: home.clone(),
            data: home.clone(),
            cache: home.join("cache"),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    pub fn tasks_file(&self) -> PathBuf {
        self.data.join("tasks.json")
    }
}

/// Resolves "." and ".." without touching the file system.
pub fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = matches!(result.components().next_back(), Some(Component::Normal(_)));
                if can_pop {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Absolute and normalized; relative paths are taken against the current directory.
pub fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        let base = std::env::current_dir().unwrap_or_default();
        normalize(&base.join(path))
    }
}

fn component_key(component: &Component<'_>) -> String {
    component.as_os_str().to_string_lossy().to_lowercase()
}

/// Case-insensitive (Windows rules) prefix test on normalized paths.
pub fn is_within(path: &Path, dir: &Path) -> bool {
    let path = normalize(path);
    let dir = normalize(dir);
    let mut path_components = path.components();
    for dir_component in dir.components() {
        match path_components.next() {
            Some(component) if component_key(&component) == component_key(&dir_component) => {}
            _ => return false,
        }
    }
    true
}

/// True when `path` is inside `dir` and not `dir` itself.
pub fn is_strictly_within(path: &Path, dir: &Path) -> bool {
    is_within(path, dir)
        && normalize(path).components().count() > normalize(dir).components().count()
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    is_within(a, b) && is_within(b, a)
}

/// `path` relative to `base` with '/' separators ("Plugins/uBot/uBotCore", "../Plugins/uBot/x").
/// Falls back to the absolute path (with '/') when they are on different roots.
pub fn relative_display(path: &Path, base: &Path) -> String {
    let path = normalize(path);
    let base = normalize(base);
    let path_components: Vec<Component<'_>> = path.components().collect();
    let base_components: Vec<Component<'_>> = base.components().collect();

    let same_root = match (path_components.first(), base_components.first()) {
        (Some(a), Some(b)) => component_key(a) == component_key(b),
        _ => false,
    };
    if !same_root {
        return to_slash(&path);
    }

    let common = path_components
        .iter()
        .zip(base_components.iter())
        .take_while(|(a, b)| component_key(a) == component_key(b))
        .count();
    let mut parts: Vec<String> = Vec::new();
    for _ in common..base_components.len() {
        parts.push("..".into());
    }
    for component in &path_components[common..] {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

pub fn to_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Platform-native display of a path (backslashes on Windows).
pub fn display(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.replace('/', "\\")
    } else {
        text
    }
}

/// Sets or clears the read-only flag of a file (the only permission Windows has).
fn set_readonly(path: &Path, readonly: bool) {
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() != readonly {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(readonly);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

/// ERROR_NOT_SAME_DEVICE. Windows reports it when an app sandbox redirects part of a path (MSIX) and the
/// rename would cross the redirection. On other systems code 17 means something else.
fn is_not_same_device(error: &io::Error) -> bool {
    cfg!(windows) && error.raw_os_error() == Some(17)
}

/// Moves `temp` over `path`, copying instead when the rename fails with ERROR_NOT_SAME_DEVICE. A read-only
/// target is made writable for the replacement and read-only again afterwards.
fn replace_file(
    temp: &Path,
    path: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let was_readonly = fs::metadata(path).is_ok_and(|meta| meta.permissions().readonly());
    if was_readonly {
        set_readonly(path, false);
    }
    let result = match rename(temp, path) {
        Err(error) if is_not_same_device(&error) => {
            fs::copy(temp, path).and_then(|_| fs::remove_file(temp))
        }
        result => result,
    };
    if was_readonly {
        // Restore the flag even if the rename or fallback copy failed.
        set_readonly(path, true);
    }
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Writes through a temporary file in the same directory and renames it over the target.
/// Falls back to copying if Windows sandbox redirection makes the rename cross devices.
///
/// A read-only target (the default state of a file in a Perforce workspace) cannot be replaced by a
/// rename on Windows. Its flag is cleared for the replacement and set again afterwards, so the file
/// keeps the state its source control gave it.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut file_name = OsString::from(".");
    file_name.push(path.file_name().unwrap_or_default());
    file_name.push(format!(".{}.tmp", std::process::id()));
    let temp = dir.join(file_name);
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temp, path, |temp, path| fs::rename(temp, path))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Reads a text file, dropping a UTF-8 byte order mark. Returns whether one was present.
pub fn read_text(path: &Path) -> io::Result<(String, bool)> {
    let bytes = fs::read(path)?;
    let (bytes, bom) = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
        Some(rest) => (rest.to_vec(), true),
        None => (bytes, false),
    };
    let text = String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((text, bom))
}

/// Decodes a text file that may be UTF-8 (with or without a byte order mark) or UTF-16 with a
/// byte order mark (what Notepad's "Unicode" produces).
pub fn decode_text(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |bytes: &[u8], little_endian: bool| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| "the text is not valid UTF-16".to_string())
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, true)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, false)
    } else {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        String::from_utf8(bytes.to_vec()).map_err(|_| "the text is not valid UTF-8".to_string())
    }
}

/// Copies a file that could not be read as intended to `<name>.bad-<unix time>` next to it, so the
/// next save does not destroy what the user wrote. Returns the copy.
pub fn backup_unreadable(path: &Path) -> io::Result<PathBuf> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    for attempt in 0..100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let backup = path.with_file_name(format!("{name}.bad-{seconds}{suffix}"));
        if !backup.exists() {
            fs::copy(path, &backup)?;
            return Ok(backup);
        }
    }
    Err(io::Error::other("too many backups of an unreadable file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_and_utf16_text() {
        assert_eq!(decode_text(b"{\"a\":1}").unwrap(), "{\"a\":1}");
        assert_eq!(decode_text(b"\xEF\xBB\xBF{}").unwrap(), "{}");
        let utf16: Vec<u8> = [0xFFu8, 0xFE]
            .into_iter()
            .chain("{\"a\":\"仓\"}".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode_text(&utf16).unwrap(), "{\"a\":\"仓\"}");
        let big: Vec<u8> = [0xFEu8, 0xFF]
            .into_iter()
            .chain("[]".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        assert_eq!(decode_text(&big).unwrap(), "[]");
        assert!(decode_text(b"\xFF\xFF\xFF").is_err());
    }

    #[test]
    fn backs_up_an_unreadable_file_without_overwriting_older_backups() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        fs::write(&file, "{ broken").unwrap();
        let first = backup_unreadable(&file).unwrap();
        let second = backup_unreadable(&file).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(&first).unwrap(), "{ broken");
        assert_eq!(fs::read_to_string(&second).unwrap(), "{ broken");
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("settings.json.bad-"));
    }

    #[test]
    fn normalizes_and_relativizes() {
        let project = Path::new("E:/Work/DevHost");
        assert_eq!(
            normalize(&project.join("../Plugins/uBot")),
            PathBuf::from("E:/Work/Plugins/uBot")
        );
        assert_eq!(
            relative_display(Path::new("E:/Work/Plugins/uBot/uBotCore"), project),
            "../Plugins/uBot/uBotCore"
        );
        assert_eq!(
            relative_display(
                Path::new("E:/Work/DevHost/Plugins/uBot/uBotSensor"),
                project
            ),
            "Plugins/uBot/uBotSensor"
        );
        assert_eq!(
            relative_display(Path::new("e:/work/devhost/Plugins/X"), project),
            "Plugins/X"
        );
        assert_eq!(relative_display(project, project), ".");
        assert_eq!(normalize(Path::new("a/../../b")), PathBuf::from("../b"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths() {
        let project = Path::new(r"E:\Work\DevHost");
        assert!(is_within(Path::new(r"e:\work\devhost\Plugins"), project));
        assert!(is_strictly_within(
            Path::new(r"E:\Work\DevHost\Plugins"),
            project
        ));
        assert!(!is_strictly_within(project, project));
        assert!(!is_within(Path::new(r"E:\Work\DevHostOther"), project));
        assert!(same_path(
            Path::new(r"E:\Work\DevHost\."),
            Path::new("e:/work/devhost")
        ));
        assert_eq!(
            relative_display(Path::new(r"D:\Other\X"), project),
            "D:/Other/X"
        );
    }

    #[test]
    fn writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("a.json");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);

        fs::write(&file, b"\xEF\xBB\xBF{}").unwrap();
        assert_eq!(read_text(&file).unwrap(), ("{}".to_string(), true));
    }

    #[test]
    fn writes_over_a_read_only_file_and_keeps_it_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("P.uproject");
        fs::write(&file, b"one").unwrap();
        set_readonly(&file, true);
        write_atomic(&file, b"two").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        assert!(
            fs::metadata(&file).unwrap().permissions().readonly(),
            "the read-only state is restored"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        // Clear the flag so the temporary directory can be removed.
        set_readonly(&file, false);
    }

    #[cfg(windows)]
    #[test]
    fn copies_when_rename_reports_not_same_device() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let temp = dir.path().join(".settings.json.tmp");
        fs::write(&file, b"one").unwrap();
        let entries = fs::read_dir(dir.path()).unwrap().count();
        fs::write(&temp, b"two").unwrap();

        replace_file(&temp, &file, |_, _| Err(io::Error::from_raw_os_error(17))).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        assert!(!temp.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), entries);
    }

    #[cfg(windows)]
    #[test]
    fn copies_over_a_read_only_file_and_keeps_it_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("P.uproject");
        let temp = dir.path().join(".P.uproject.tmp");
        fs::write(&file, b"one").unwrap();
        set_readonly(&file, true);
        let entries = fs::read_dir(dir.path()).unwrap().count();
        fs::write(&temp, b"two").unwrap();

        replace_file(&temp, &file, |_, _| Err(io::Error::from_raw_os_error(17))).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        assert!(!temp.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), entries);
        set_readonly(&file, false);
    }

    #[cfg(windows)]
    #[test]
    fn copies_to_a_missing_file_when_rename_reports_not_same_device() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let temp = dir.path().join(".settings.json.tmp");
        fs::write(&temp, b"two").unwrap();

        replace_file(&temp, &file, |_, _| Err(io::Error::from_raw_os_error(17))).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        assert!(!temp.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn preserves_other_rename_errors_and_cleans_up_temp_file() {
        for readonly in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("settings.json");
            let temp = dir.path().join(".settings.json.tmp");
            fs::write(&file, b"one").unwrap();
            set_readonly(&file, readonly);
            let entries = fs::read_dir(dir.path()).unwrap().count();
            fs::write(&temp, b"two").unwrap();

            let error = replace_file(&temp, &file, |_, _| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "rename denied",
                ))
            })
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(error.to_string(), "rename denied");
            assert_eq!(error.raw_os_error(), None);
            assert_eq!(fs::read_to_string(&file).unwrap(), "one");
            assert_eq!(
                fs::metadata(&file).unwrap().permissions().readonly(),
                readonly
            );
            assert!(!temp.exists());
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), entries);
            set_readonly(&file, false);
        }
    }

    #[cfg(windows)]
    #[test]
    fn restores_read_only_state_when_copy_fails() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("P.uproject");
        let temp = dir.path().join(".P.uproject.tmp");
        fs::write(&file, b"one").unwrap();
        set_readonly(&file, true);
        let entries = fs::read_dir(dir.path()).unwrap().count();
        fs::write(&temp, b"two").unwrap();

        let error = replace_file(&temp, &file, |temp, _| {
            fs::remove_file(temp)?;
            Err(io::Error::from_raw_os_error(17))
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read_to_string(&file).unwrap(), "one");
        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        assert!(!temp.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), entries);
        set_readonly(&file, false);
    }

    #[test]
    fn recognizes_not_same_device_only_on_windows() {
        assert_eq!(
            is_not_same_device(&io::Error::from_raw_os_error(17)),
            cfg!(windows)
        );
        assert!(!is_not_same_device(&io::Error::from_raw_os_error(5)));
        assert!(!is_not_same_device(&io::Error::new(
            io::ErrorKind::PermissionDenied,
            "rename denied"
        )));
    }

    #[test]
    fn home_override() {
        let dirs = AppDirs::under(PathBuf::from("X:/home"));
        assert_eq!(dirs.settings_file(), PathBuf::from("X:/home/settings.json"));
        assert_eq!(dirs.tasks_file(), PathBuf::from("X:/home/tasks.json"));
        assert_eq!(dirs.cache, PathBuf::from("X:/home/cache"));
    }
}

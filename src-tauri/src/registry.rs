//! Registers the running executable so the uBot panel in Unreal Editor can start it (SPEC §9.3, §12):
//! `HKCU\Software\AstreoX\uBot Manager`, value `Executable` (REG_SZ).

use std::io;
use std::path::Path;

pub const KEY: &str = r"Software\AstreoX\uBot Manager";
pub const VALUE: &str = "Executable";

/// Writes the current executable's path, unless it is already there.
pub fn register_current_exe() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    write_executable(KEY, &exe)
}

#[cfg(windows)]
fn write_executable(key: &str, exe: &Path) -> io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(key)?;
    let current: Option<std::ffi::OsString> = key.get_value(VALUE).ok();
    if current.as_deref() == Some(exe.as_os_str()) {
        return Ok(());
    }
    key.set_value(VALUE, &exe.as_os_str())
}

#[cfg(not(windows))]
fn write_executable(_key: &str, _exe: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    #[test]
    fn writes_the_executable_value() {
        // A throwaway key, removed again below.
        let key = format!(r"Software\uBotManagerTest-{}", std::process::id());
        let exe = Path::new(r"E:\Tools\uBot Manager\ubot-manager.exe");
        let result = write_executable(&key, exe).and_then(|()| write_executable(&key, exe));
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let stored: io::Result<String> = hkcu
            .open_subkey_with_flags(&key, KEY_READ)
            .and_then(|opened| opened.get_value(VALUE));
        let other = Path::new(r"D:\Other\ubot-manager.exe");
        let replaced: io::Result<String> = write_executable(&key, other).and_then(|()| {
            hkcu.open_subkey_with_flags(&key, KEY_READ)?
                .get_value(VALUE)
        });
        hkcu.delete_subkey_all(&key).unwrap();

        result.unwrap();
        assert_eq!(stored.unwrap(), r"E:\Tools\uBot Manager\ubot-manager.exe");
        assert_eq!(replaced.unwrap(), r"D:\Other\ubot-manager.exe");
    }
}

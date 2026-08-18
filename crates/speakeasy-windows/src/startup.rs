use std::io;
use std::path::{Path, PathBuf};

pub const STARTUP_VALUE_NAME: &str = "SpeakEasy";
const LEGACY_STARTUP_VALUE_NAME: &str = "SpeakEasy v2 Preview";
const DESKTOP_EXECUTABLE_NAME: &str = "ai-speakeasy-mini.exe";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupStatus {
    pub enabled: bool,
    pub executable: Option<PathBuf>,
}

#[cfg(windows)]
/// Reads only the exact v2 current-user startup value.
///
/// # Errors
///
/// Returns registry access or value-decoding errors.
pub fn startup_status() -> io::Result<StartupStatus> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let key = current_user
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_READ)?;
    match key.get_value::<String, _>(STARTUP_VALUE_NAME) {
        Ok(value) => Ok(StartupStatus {
            enabled: true,
            executable: parse_quoted_command(&value),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(StartupStatus {
            enabled: false,
            executable: None,
        }),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
/// Creates or removes only the exact v2 current-user startup value.
///
/// # Errors
///
/// Returns invalid-executable or registry write/read-back errors.
pub fn set_startup_with_windows(enabled: bool, executable: &Path) -> io::Result<StartupStatus> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    if !executable.is_absolute()
        || executable.file_name().and_then(|name| name.to_str()) != Some(DESKTOP_EXECUTABLE_NAME)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "startup executable identity is invalid",
        ));
    }
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = current_user.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
    if enabled {
        key.set_value(
            STARTUP_VALUE_NAME,
            &format!("\"{}\" --startup", executable.display()),
        )?;
    } else {
        match key.delete_value(STARTUP_VALUE_NAME) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    startup_status()
}

#[cfg(windows)]
/// Converts the legacy startup entry to the production executable and value
/// name. The old value is removed only after the new one is written.
///
/// # Errors
///
/// Returns registry or executable-identity errors.
pub fn migrate_legacy_startup(executable: &Path) -> io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    if !executable.is_absolute()
        || executable.file_name().and_then(|name| name.to_str()) != Some(DESKTOP_EXECUTABLE_NAME)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "startup executable identity is invalid",
        ));
    }
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let key = match current_user.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        KEY_READ | KEY_WRITE,
    ) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if key.get_value::<String, _>(STARTUP_VALUE_NAME).is_ok() {
        return Ok(());
    }
    if key
        .get_value::<String, _>(LEGACY_STARTUP_VALUE_NAME)
        .is_ok()
    {
        key.set_value(
            STARTUP_VALUE_NAME,
            &format!("\"{}\" --startup", executable.display()),
        )?;
        key.delete_value(LEGACY_STARTUP_VALUE_NAME)?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn migrate_legacy_startup(_executable: &Path) -> io::Result<()> {
    Ok(())
}

fn parse_quoted_command(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    let remainder = value.strip_prefix('"')?;
    let end = remainder.find('"')?;
    let path = PathBuf::from(&remainder[..end]);
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_parser_accepts_only_an_absolute_quoted_executable() {
        assert_eq!(
            parse_quoted_command(r#""C:\Apps\SpeakEasy\ai-speakeasy-mini.exe" --startup"#),
            Some(PathBuf::from(r"C:\Apps\SpeakEasy\ai-speakeasy-mini.exe"))
        );
        assert_eq!(parse_quoted_command("relative.exe --startup"), None);
        assert_eq!(parse_quoted_command("\"unterminated"), None);
    }
}

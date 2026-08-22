use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::command_first_line;

pub(super) fn version() -> Option<String> {
    command_first_line("/usr/bin/sw_vers", &["-productVersion"])
        .map(|version| format!("macOS {version}"))
}

pub(super) fn config_root(home: Option<&OsStr>) -> PathBuf {
    home.filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("TimemAi")
        })
        .unwrap_or_else(|| PathBuf::from("/Library/Application Support/TimemAi"))
}

pub(super) fn browser_command(url: &str) -> (OsString, Vec<OsString>) {
    (OsString::from("open"), vec![OsString::from(url)])
}

pub(super) fn terminal_command(path: &Path) -> (OsString, Vec<OsString>) {
    (
        OsString::from("open"),
        vec![
            OsString::from("-a"),
            OsString::from("Terminal"),
            path.as_os_str().to_os_string(),
        ],
    )
}

pub(super) fn graphical_session_available() -> bool {
    true
}

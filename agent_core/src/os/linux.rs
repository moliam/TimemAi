use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn version() -> Option<String> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    for key in ["PRETTY_NAME", "NAME"] {
        if let Some(value) = os_release_value(&content, key) {
            return Some(value);
        }
    }
    None
}

pub(super) fn config_root(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    if let Some(path) = xdg.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join("timem");
    }
    home.filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("timem"))
        .unwrap_or_else(|| PathBuf::from("/etc/xdg/timem"))
}

pub(super) fn browser_command(url: &str) -> (OsString, Vec<OsString>) {
    (OsString::from("xdg-open"), vec![OsString::from(url)])
}

pub(super) fn terminal_command(path: &Path) -> (OsString, Vec<OsString>) {
    (
        OsString::from("x-terminal-emulator"),
        vec![
            OsString::from("--working-directory"),
            path.as_os_str().to_os_string(),
        ],
    )
}

pub(super) fn graphical_session_available() -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY"]
        .into_iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

pub(super) fn os_release_value(content: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    content.lines().find_map(|line| {
        let value = line.strip_prefix(&prefix)?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value)
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
        super::non_empty_one_line(&value)
    })
}

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

#[allow(dead_code)]
pub(super) fn process_identity(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesized and may contain spaces or `)`, so split
    // only after its final closing parenthesis. Linux proc(5) field 22 is the
    // process start time in clock ticks since boot; after removing pid+comm it
    // is index 19 in the remaining field list beginning with state (field 3).
    let tail = stat.rsplit_once(") ")?.1;
    let start_ticks = tail.split_whitespace().nth(19)?;
    Some(format!("linux-start-ticks:{start_ticks}"))
}

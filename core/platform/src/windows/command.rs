use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

pub(crate) fn command_for_script(path: &Path) -> Result<Command, String> {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "ps1" => powershell_script_command(path),
        "cmd" | "bat" => {
            let mut command = Command::new("cmd.exe");
            command.args(["/d", "/s", "/c"]);
            command.arg(path);
            Ok(command)
        }
        "exe" | "com" => Ok(Command::new(path)),
        _ => Err(format!("unsupported_windows_command_extension:{extension}")),
    }
}

pub(crate) fn powershell_script_command(path: &Path) -> Result<Command, String> {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]);
    command.arg(path);
    Ok(command)
}

pub(crate) fn configure_sanitized_child_environment(command: &mut Command) {
    let temp = std::env::temp_dir();
    command.env("TEMP", &temp).env("TMP", &temp);
    for name in [
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "COMMONPROGRAMFILES",
        "COMMONPROGRAMFILES(X86)",
        "PSModulePath",
    ] {
        if let Some(value) = std::env::var_os(name).filter(|value| !value.is_empty()) {
            command.env(name, value);
        }
    }
}

pub(crate) fn configure_child_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

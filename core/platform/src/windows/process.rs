use std::process::{Command, Stdio};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, TerminateProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};

const STILL_ACTIVE_EXIT_CODE: u32 = 259;

pub(crate) fn process_is_alive(pid: u64) -> Option<bool> {
    let pid = u32::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(5) => Some(true),
            Some(87) => Some(false),
            _ => None,
        };
    }
    let mut exit_code = 0_u32;
    let result = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
    unsafe { CloseHandle(handle) };
    (result != 0).then_some(exit_code == STILL_ACTIVE_EXIT_CODE)
}

pub(crate) fn process_identity(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let result =
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    unsafe { CloseHandle(handle) };
    if result == 0 {
        return None;
    }
    let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    Some(format!("windows-creation-time:{ticks}"))
}

pub(crate) fn current_parent_pid() -> Option<u32> {
    parent_pid(std::process::id())
}

pub(crate) fn parent_pid(pid: u32) -> Option<u32> {
    snapshot_processes()
        .into_iter()
        .find_map(|(candidate, parent)| (candidate == pid && parent > 0).then_some(parent))
}

pub(crate) fn child_process_running(pid: u32) -> bool {
    process_is_alive(u64::from(pid)).unwrap_or(false)
}

pub(crate) fn is_runtime_child_process_group(pid: u32) -> bool {
    pid > 0 && pid != std::process::id() && parent_pid(pid) == Some(std::process::id())
}

pub(crate) fn terminate_process(pid: u32) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    let _ = Command::new("taskkill.exe")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if process_is_alive(u64::from(pid)) == Some(true) {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if !handle.is_null() {
            unsafe {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
}

pub(crate) fn process_tree_running(root_pid: u32) -> bool {
    let processes = snapshot_processes();
    let mut pending = vec![root_pid];
    let mut index = 0;
    while index < pending.len() {
        let parent = pending[index];
        index += 1;
        if process_is_alive(u64::from(parent)) == Some(true) {
            return true;
        }
        for (pid, candidate_parent) in &processes {
            if *candidate_parent == parent && !pending.contains(pid) {
                pending.push(*pid);
            }
        }
    }
    false
}

fn snapshot_processes() -> Vec<(u32, u32)> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut rows = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            rows.push((entry.th32ProcessID, entry.th32ParentProcessID));
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snapshot) };
    rows
}

//! Runtime-wide kill-on-close Job Object containment.
//!
//! Every shell child is assigned to a single job object configured with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and without a breakaway limit, so
//! descendants cannot leave the job (a `setsid`-equivalent escape is
//! rejected by the OS) and the kernel terminates the whole job if this
//! runtime ever exits without cleanup.

use std::sync::Mutex;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

// `HANDLE` is a raw pointer; store it as isize for Sync.
static JOB_HANDLE: Mutex<isize> = Mutex::new(0);

fn runtime_job_handle() -> isize {
    let mut handle = JOB_HANDLE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if *handle == 0 {
        let created = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if !created.is_null() {
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = unsafe {
                SetInformationJobObject(
                    created,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok != 0 {
                *handle = created as isize;
            } else {
                unsafe { CloseHandle(created) };
            }
        }
    }
    *handle
}

/// Assign a live process (and therefore its future children) to the runtime
/// kill-on-close job. Returns false when containment could not be applied.
pub(crate) fn contain_process_in_runtime_job(pid: u32) -> bool {
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    let job = runtime_job_handle();
    if job == 0 {
        return false;
    }
    let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
    if process.is_null() {
        return false;
    }
    let assigned = unsafe { AssignProcessToJobObject(job as _, process) };
    unsafe { CloseHandle(process) };
    assigned != 0
}

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};

pub(crate) fn configure_private_file_options(options: &mut std::fs::OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.share_mode(0);
}

pub(crate) fn open_diagnostic_file_lease(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

    std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|error| {
            if matches!(error.raw_os_error(), Some(32 | 33)) {
                std::io::Error::new(std::io::ErrorKind::WouldBlock, error)
            } else {
                error
            }
        })
}

pub(crate) fn fill_secure_random(bytes: &mut [u8]) -> std::io::Result<()> {
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            u32::try_from(bytes.len()).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "secure_random_buffer_too_large",
                )
            })?,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status))
    }
}

pub(crate) fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty())?;
            let path = std::env::var_os("HOMEPATH").filter(|value| !value.is_empty())?;
            let mut home = PathBuf::from(drive);
            home.push(path);
            Some(home)
        })
}

pub(crate) fn local_time(secs: libc::time_t) -> Option<libc::tm> {
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let result = unsafe { libc::localtime_s(tm.as_mut_ptr(), &secs) };
    if result == 0 {
        Some(unsafe { tm.assume_init() })
    } else {
        None
    }
}

pub(crate) fn version() -> Option<String> {
    super::super::command_first_line("cmd.exe", &["/d", "/c", "ver"])
}

pub(crate) fn config_root(_xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|root| root.join("TimemAi"))
        .or_else(|| {
            home.filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|root| root.join("AppData").join("Roaming").join("TimemAi"))
        })
        .unwrap_or_else(|| PathBuf::from("TimemAi"))
}

pub(crate) fn browser_command(url: &str) -> (OsString, Vec<OsString>) {
    (
        OsString::from("rundll32.exe"),
        vec![
            OsString::from("url.dll,FileProtocolHandler"),
            OsString::from(url),
        ],
    )
}

pub(crate) fn terminal_command(path: &Path) -> (OsString, Vec<OsString>) {
    (
        OsString::from("cmd.exe"),
        vec![
            OsString::from("/d"),
            OsString::from("/k"),
            OsString::from("cd"),
            OsString::from("/d"),
            path.as_os_str().to_os_string(),
        ],
    )
}

pub(crate) fn graphical_session_available() -> bool {
    std::env::var_os("SESSIONNAME").is_some_and(|value| !value.is_empty() && value != "Services")
}

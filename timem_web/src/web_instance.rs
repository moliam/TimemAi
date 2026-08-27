use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WebInstanceInfo {
    pub pid: u32,
    #[serde(default)]
    pub launch_parent_pid: Option<u32>,
    pub port: Option<u16>,
    pub token: Option<String>,
    pub browser_url: Option<String>,
    pub public_access: bool,
    pub started_at_ms: u128,
}

impl WebInstanceInfo {
    pub fn starting() -> Self {
        Self {
            pid: std::process::id(),
            launch_parent_pid: crate::os::current_launch_parent_pid(),
            port: None,
            token: None,
            browser_url: None,
            public_access: false,
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct WebInstanceLease {
    file: File,
    #[cfg(test)]
    path: PathBuf,
    info: WebInstanceInfo,
}

impl WebInstanceLease {
    pub fn acquire(instance_path: &Path) -> Result<Self, String> {
        let lock_path = instance_path.to_path_buf();
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options.open(&lock_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                "web_instance_in_use".to_string()
            } else {
                format!("web_instance_lock_open_failed:{error}")
            }
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("web_instance_lock_permissions_failed:{error}"))?;
        }

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
                    "web_instance_in_use".to_string()
                } else {
                    format!("web_instance_lock_failed:{error}")
                });
            }
        }

        let info = WebInstanceInfo::starting();
        let mut instance_lock = Self {
            file,
            #[cfg(test)]
            path: lock_path,
            info: info.clone(),
        };
        instance_lock.publish(&info)?;
        Ok(instance_lock)
    }

    pub fn read_info(path: impl AsRef<Path>) -> Option<WebInstanceInfo> {
        let raw = std::fs::read(path).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    pub fn publish(&mut self, info: &WebInstanceInfo) -> Result<(), String> {
        let encoded = serde_json::to_vec(info)
            .map_err(|error| format!("web_instance_serialize_failed:{error}"))?;
        self.file
            .set_len(0)
            .and_then(|_| self.file.seek(SeekFrom::Start(0)).map(|_| ()))
            .and_then(|_| self.file.write_all(&encoded))
            .and_then(|_| self.file.sync_data())
            .map_err(|error| format!("web_instance_write_failed:{error}"))?;
        self.info = info.clone();
        Ok(())
    }

    pub fn info(&self) -> &WebInstanceInfo {
        &self.info
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

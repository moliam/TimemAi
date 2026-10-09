use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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
        let file = agent_core::os::open_diagnostic_file_lease(&lock_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                "web_instance_in_use".to_string()
            } else {
                format!("web_instance_lock_open_failed:{error}")
            }
        })?;

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

#[derive(Debug)]
pub(crate) struct WebInstanceRegistration {
    path: PathBuf,
    record: agent_core::WebInstanceRegistryRecord,
}

impl WebInstanceRegistration {
    pub fn publish(
        registry_dir: &Path,
        memory_dir: &Path,
        info: &WebInstanceInfo,
    ) -> Result<Self, String> {
        agent_core::create_memory_dir(registry_dir)?;
        let registration_id = format!("{}-{}", info.pid, info.started_at_ms);
        let path = registry_dir.join(format!("{registration_id}.json"));
        let record = agent_core::WebInstanceRegistryRecord {
            registration_id,
            memory_dir: memory_dir.to_path_buf(),
            pid: info.pid,
            started_at_ms: info.started_at_ms,
        };
        write_registration(&path, &record)?;
        Ok(Self { path, record })
    }

    pub fn update_memory_dir(&mut self, memory_dir: &Path) -> Result<(), String> {
        let mut next = self.record.clone();
        next.memory_dir = memory_dir.to_path_buf();
        write_registration(&self.path, &next)?;
        self.record = next;
        Ok(())
    }

    fn remove_if_owned(&self) -> Result<(), String> {
        let current = match std::fs::read(&self.path) {
            Ok(raw) => serde_json::from_slice::<agent_core::WebInstanceRegistryRecord>(&raw).ok(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("web_instance_registry_read_failed:{error}")),
        };
        if current.as_ref() != Some(&self.record) {
            return Ok(());
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("web_instance_registry_remove_failed:{error}")),
        }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WebInstanceRegistration {
    fn drop(&mut self) {
        let _ = self.remove_if_owned();
    }
}

fn write_registration(
    path: &Path,
    record: &agent_core::WebInstanceRegistryRecord,
) -> Result<(), String> {
    let payload = serde_json::to_vec(record)
        .map_err(|error| format!("web_instance_registry_serialize_failed:{error}"))?;
    agent_core::atomic_write_file(path, &payload)
        .map_err(|error| format!("web_instance_registry_write_failed:{error}"))
}

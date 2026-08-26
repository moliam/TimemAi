use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDataLayout {
    data_root: PathBuf,
    space: String,
    direct_memory_dir: Option<PathBuf>,
}

impl RuntimeDataLayout {
    pub fn new(data_root: impl Into<PathBuf>, space: impl Into<String>) -> Self {
        Self {
            data_root: data_root.into(),
            space: space.into(),
            direct_memory_dir: None,
        }
    }

    pub fn from_memory_dir(data_root: impl Into<PathBuf>, memory_dir: impl Into<PathBuf>) -> Self {
        let memory_dir = memory_dir.into();
        Self {
            data_root: data_root.into(),
            space: memory_dir.display().to_string(),
            direct_memory_dir: Some(memory_dir),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn space(&self) -> &str {
        &self.space
    }

    pub fn space_dir(&self) -> PathBuf {
        self.direct_memory_dir
            .clone()
            .unwrap_or_else(|| self.data_root.join(&self.space))
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.direct_memory_dir
            .clone()
            .unwrap_or_else(|| self.space_dir().join("memory"))
    }

    pub fn api_audit_file(&self) -> PathBuf {
        self.space_dir().join("audit").join("api_audit.json")
    }

    pub fn action_audit_file(&self) -> PathBuf {
        self.space_dir().join("audit").join("action_audit.json")
    }

    pub fn workspace_config_file(&self) -> PathBuf {
        workspace_config_file(&self.data_root)
    }
}

pub fn default_memory_dir() -> Result<PathBuf, String> {
    default_memory_dir_from_home(std::env::var_os("HOME").as_deref())
}

fn default_memory_dir_from_home(home: Option<&std::ffi::OsStr>) -> Result<PathBuf, String> {
    let home = home
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "home_directory_unavailable".to_string())?;
    Ok(PathBuf::from(home).join(".timem").join("mem"))
}

pub fn resolve_memory_dir(space: Option<&str>) -> Result<PathBuf, String> {
    let Some(space) = space else {
        return default_memory_dir();
    };
    if space.trim().is_empty() {
        return Err("mem_path_empty".to_string());
    }
    let path = PathBuf::from(space);
    if !path.is_absolute() {
        return Err("space_must_be_absolute_path".to_string());
    }
    Ok(path)
}

pub fn create_memory_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|error| format!("mem_directory_create_failed:{}:{error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                format!(
                    "mem_directory_permissions_failed:{}:{error}",
                    path.display()
                )
            },
        )?;
    }
    Ok(())
}

pub fn default_data_root() -> PathBuf {
    default_memory_dir().unwrap_or_else(|_| PathBuf::from(".timem_data"))
}

pub fn layout_for_space(space: &str) -> RuntimeDataLayout {
    RuntimeDataLayout::from_memory_dir(space, space)
}

pub fn workspace_config_file(data_root: &Path) -> PathBuf {
    data_root.join("workspace.json")
}

#[cfg(test)]
#[path = "../tests/unit/data_layout_tests.rs"]
mod tests;

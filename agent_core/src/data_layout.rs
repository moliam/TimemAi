use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDataLayout {
    data_root: PathBuf,
    space: String,
}

impl RuntimeDataLayout {
    pub fn new(data_root: impl Into<PathBuf>, space: impl Into<String>) -> Self {
        Self {
            data_root: data_root.into(),
            space: space.into(),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn space(&self) -> &str {
        &self.space
    }

    pub fn space_dir(&self) -> PathBuf {
        self.data_root.join(&self.space)
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.space_dir().join("memory")
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

pub fn default_data_root() -> PathBuf {
    std::env::var("TIMEM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
}

pub fn layout_for_space(space: &str) -> RuntimeDataLayout {
    RuntimeDataLayout::new(default_data_root(), space)
}

pub fn workspace_config_file(data_root: &Path) -> PathBuf {
    data_root.join("workspace.json")
}

#[cfg(test)]
#[path = "../tests/unit/data_layout_tests.rs"]
mod tests;

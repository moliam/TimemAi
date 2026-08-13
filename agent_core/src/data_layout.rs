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
        .unwrap_or_else(|_| default_unconfigured_data_root(Path::new(".")))
}

fn default_unconfigured_data_root(current_dir: &Path) -> PathBuf {
    let hidden = current_dir.join(".timem_data");
    let legacy = current_dir.join("data");
    if !hidden.exists() && is_legacy_timem_data_root(&legacy) {
        PathBuf::from("data")
    } else {
        PathBuf::from(".timem_data")
    }
}

fn is_legacy_timem_data_root(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let workspace_is_timem = std::fs::read_to_string(path.join("workspace.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("dirs").and_then(|dirs| dirs.as_array()).cloned())
        .is_some();
    if workspace_is_timem {
        return true;
    }
    std::fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|space| space.is_dir())
        .any(|space| {
            space.join("memory/sessions/index.jsonl").is_file()
                || (space.join("audit/api_audit.json").is_file()
                    && space.join("audit/action_audit.json").is_file())
        })
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

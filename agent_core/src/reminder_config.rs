use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

pub const REMINDER_TIPS_FILE_NAME: &str = "reminder_tips.json";
pub const TIMEM_RESOURCES_DIR_ENV: &str = "TIMEM_RESOURCES_DIR";
const SHIPPED_REMINDER_TIPS: &str = include_str!("../../resources/reminder_tips.json");
const MAX_SCHEDULES: usize = 32;
const MAX_TIPS_PER_SCHEDULE: usize = 128;
const MAX_TIP_BYTES: usize = 4_096;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReminderTipsConfig {
    pub schedules: Vec<ReminderScheduleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReminderScheduleConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_rounds: Option<u32>,
    pub tips: Vec<String>,
}

impl Default for ReminderTipsConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED_REMINDER_TIPS)
            .expect("shipped reminder tips must be valid JSON")
            .validate()
            .expect("shipped reminder tips must satisfy runtime limits")
    }
}

impl ReminderTipsConfig {
    pub fn validate(self) -> Result<Self, String> {
        if self.schedules.len() > MAX_SCHEDULES {
            return Err("reminder_tips_too_many_schedules".to_string());
        }
        for (index, schedule) in self.schedules.iter().enumerate() {
            if schedule.every_minutes.is_some() == schedule.every_rounds.is_some() {
                return Err(format!(
                    "reminder_tips_schedule_{index}_requires_exactly_one_interval"
                ));
            }
            if schedule.every_minutes == Some(0) || schedule.every_rounds == Some(0) {
                return Err(format!("reminder_tips_schedule_{index}_interval_zero"));
            }
            if schedule.tips.is_empty() || schedule.tips.len() > MAX_TIPS_PER_SCHEDULE {
                return Err(format!("reminder_tips_schedule_{index}_invalid_tip_count"));
            }
            if schedule
                .tips
                .iter()
                .any(|tip| tip.trim().is_empty() || tip.len() > MAX_TIP_BYTES)
            {
                return Err(format!("reminder_tips_schedule_{index}_invalid_tip"));
            }
        }
        Ok(self)
    }
}

pub fn default_config_root() -> PathBuf {
    config_root_from_values(
        std::env::var_os("TIMEM_CONFIG_DIR").as_deref(),
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
        cfg!(target_os = "macos"),
    )
}

fn config_root_from_values(
    explicit: Option<&OsStr>,
    xdg: Option<&OsStr>,
    home: Option<&OsStr>,
    macos: bool,
) -> PathBuf {
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    if macos {
        return home
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("TimemAi")
            })
            .unwrap_or_else(|| PathBuf::from("/Library/Application Support/TimemAi"));
    }
    if let Some(path) = xdg.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join("timem");
    }
    home.filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("timem"))
        .unwrap_or_else(|| PathBuf::from("/etc/xdg/timem"))
}

pub fn reminder_tips_config_path(config_root: &Path) -> PathBuf {
    config_root.join(REMINDER_TIPS_FILE_NAME)
}

pub fn default_resources_dir() -> PathBuf {
    resource_dir_candidates_from_values(
        std::env::var_os(TIMEM_RESOURCES_DIR_ENV).as_deref(),
        std::env::current_exe().ok().as_deref(),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources")),
    )
    .into_iter()
    .next()
    .unwrap_or_else(|| PathBuf::from("resources"))
}

fn resource_dir_candidates() -> Vec<PathBuf> {
    resource_dir_candidates_from_values(
        std::env::var_os(TIMEM_RESOURCES_DIR_ENV).as_deref(),
        std::env::current_exe().ok().as_deref(),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources")),
    )
}

fn resource_dir_candidates_from_values(
    explicit: Option<&OsStr>,
    executable: Option<&Path>,
    source_resources: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        candidates.push(PathBuf::from(path));
    }
    if let Some(bin_dir) = executable.and_then(Path::parent) {
        if let Some(prefix) = bin_dir.parent() {
            candidates.push(prefix.join("share").join("timem").join("resources"));
        }
    }
    if let Some(path) = source_resources {
        candidates.push(path);
    }
    candidates.dedup();
    candidates
}

pub fn load_reminder_tips_config(config_root: &Path) -> ReminderTipsConfig {
    load_reminder_tips_config_from_resource_dirs(config_root, &resource_dir_candidates())
}

fn load_reminder_tips_config_from_resource_dirs(
    config_root: &Path,
    resource_dirs: &[PathBuf],
) -> ReminderTipsConfig {
    let user_path = reminder_tips_config_path(config_root);
    match load_config_file(&user_path) {
        Ok(Some(config)) => return config,
        Ok(None) => {}
        Err(error) => {
            eprintln!(
                "[timem_config_warning] {error}; ignoring reminder tips override path={}",
                user_path.display()
            );
        }
    }

    for resource_dir in resource_dirs {
        let path = resource_dir.join(REMINDER_TIPS_FILE_NAME);
        match load_config_file(&path) {
            Ok(Some(config)) => return config,
            Ok(None) => {}
            Err(error) => {
                eprintln!(
                    "[timem_config_warning] {error}; ignoring reminder tips resource path={}",
                    path.display()
                );
            }
        }
    }

    ReminderTipsConfig::default()
}

fn load_config_file(path: &Path) -> Result<Option<ReminderTipsConfig>, String> {
    let text = match read_config_text(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("reminder_tips_read_failed:{error}")),
    };
    serde_json::from_str::<ReminderTipsConfig>(&text)
        .map_err(|error| format!("reminder_tips_parse_failed:{error}"))?
        .validate()
        .map(Some)
}

fn read_config_text(path: &Path) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "reminder_tips_config_too_large",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error.to_string()))
}

#[cfg(test)]
#[path = "../tests/unit/reminder_config_tests.rs"]
mod tests;

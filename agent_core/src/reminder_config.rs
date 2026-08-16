use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

pub const REMINDER_TIPS_FILE_NAME: &str = "reminder_tips.json";
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
        Self {
            schedules: vec![
                ReminderScheduleConfig {
                    every_minutes: Some(10),
                    every_rounds: None,
                    tips: vec![
                        "TIPS: Remind the goal, don't get lost. If you get lost, say so promptly, and stop.".to_string(),
                        "TIPS: Jump out. Take a look at the whole work's state. Make sure the whole picture is still in your control.".to_string(),
                        "TIPS: Don't get stuck in a narrow line of thought. Don't be dragged too far by a superficial observation.".to_string(),
                    ],
                },
                ReminderScheduleConfig {
                    every_minutes: None,
                    every_rounds: Some(8),
                    tips: vec![
                        "TIPS: A good inference has rigid deduction chain. If you say A causes B, there must be no hidden C in the middle.".to_string(),
                        "TIPS: Don't make illusion correlation, such as A happens first, B happends later. Then A is the reason of B. This is super dangerous.".to_string(),
                        "TIPS: A root cause can not only explain the current question, but can also predict real things.".to_string(),
                    ],
                },
            ],
        }
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

pub fn load_reminder_tips_config(config_root: &Path) -> ReminderTipsConfig {
    let path = reminder_tips_config_path(config_root);
    match read_config_text(&path) {
        Ok(text) => match serde_json::from_str::<ReminderTipsConfig>(&text)
            .map_err(|error| format!("reminder_tips_parse_failed:{error}"))
            .and_then(ReminderTipsConfig::validate)
        {
            Ok(config) => config,
            Err(error) => {
                eprintln!(
                    "[timem_config_warning] {error}; using built-in reminder defaults path={}",
                    path.display()
                );
                ReminderTipsConfig::default()
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let config = ReminderTipsConfig::default();
            if let Err(write_error) = write_default_config(&path, &config) {
                eprintln!(
                    "[timem_config_warning] reminder_tips_default_write_failed:{write_error}; using built-in reminder defaults path={}",
                    path.display()
                );
            }
            config
        }
        Err(error) => {
            eprintln!(
                "[timem_config_warning] reminder_tips_read_failed:{error}; using built-in reminder defaults path={}",
                path.display()
            );
            ReminderTipsConfig::default()
        }
    }
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

fn write_default_config(path: &Path, config: &ReminderTipsConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.write_all(b"\n").map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
#[path = "../tests/unit/reminder_config_tests.rs"]
mod tests;

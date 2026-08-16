use super::*;

fn tmp_dir(name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "timem_reminder_config_{name}_{}_{}",
        std::process::id(),
        now
    ))
}

#[test]
fn missing_file_is_created_and_loaded_with_both_schedule_types() {
    let dir = tmp_dir("create_default");
    let config = load_reminder_tips_config(&dir);
    let path = reminder_tips_config_path(&dir);
    assert!(path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(config
        .schedules
        .iter()
        .any(|item| item.every_minutes == Some(10)));
    assert!(config
        .schedules
        .iter()
        .any(|item| item.every_rounds == Some(8)));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn default_config_contains_the_shipped_time_and_reasoning_tips() {
    let config = ReminderTipsConfig::default();
    assert_eq!(config.schedules.len(), 2);
    assert_eq!(config.schedules[0].every_minutes, Some(10));
    assert_eq!(config.schedules[1].every_rounds, Some(8));
    assert_eq!(config.schedules[0].tips.len(), 3);
    assert_eq!(config.schedules[1].tips.len(), 3);
    assert!(config
        .schedules
        .iter()
        .flat_map(|item| &item.tips)
        .all(|tip| tip == "NONE" || (tip.starts_with("TIPS: ") && tip.is_ascii())));
    assert!(config.schedules[0].tips.iter().any(|tip| tip
        == "TIPS: Remind the goal, don't get lost. If you get lost, say so promptly, and stop."));
}

#[test]
fn concurrent_first_start_creates_one_complete_config_without_blocking_any_caller() {
    use std::sync::{Arc, Barrier};

    let dir = tmp_dir("concurrent_first_start");
    let workers = 16;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|_| {
            let dir = dir.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                load_reminder_tips_config(&dir)
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), ReminderTipsConfig::default());
    }
    let text = fs::read_to_string(reminder_tips_config_path(&dir)).unwrap();
    assert_eq!(
        serde_json::from_str::<ReminderTipsConfig>(&text)
            .unwrap()
            .validate()
            .unwrap(),
        ReminderTipsConfig::default()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn unusable_config_root_falls_back_to_defaults_without_blocking_startup() {
    let root_is_a_file = tmp_dir("root_is_file");
    fs::write(&root_is_a_file, "not a directory").unwrap();
    assert_eq!(
        load_reminder_tips_config(&root_is_a_file),
        ReminderTipsConfig::default()
    );
    let _ = fs::remove_file(root_is_a_file);
}

#[test]
fn custom_file_preserves_none_as_a_selectable_noop() {
    let dir = tmp_dir("custom_none");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        reminder_tips_config_path(&dir),
        r#"{"schedules":[{"every_rounds":3,"tips":["NONE","TIPS: custom"]}]}"#,
    )
    .unwrap();
    let config = load_reminder_tips_config(&dir);
    assert_eq!(config.schedules[0].tips, ["NONE", "TIPS: custom"]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn invalid_schedule_shapes_and_bounds_are_rejected() {
    let cases = [
        ReminderTipsConfig {
            schedules: vec![ReminderScheduleConfig {
                every_minutes: Some(1),
                every_rounds: Some(1),
                tips: vec!["TIPS: invalid".to_string()],
            }],
        },
        ReminderTipsConfig {
            schedules: vec![ReminderScheduleConfig {
                every_minutes: Some(0),
                every_rounds: None,
                tips: vec!["TIPS: invalid".to_string()],
            }],
        },
        ReminderTipsConfig {
            schedules: vec![ReminderScheduleConfig {
                every_minutes: None,
                every_rounds: Some(1),
                tips: Vec::new(),
            }],
        },
        ReminderTipsConfig {
            schedules: vec![ReminderScheduleConfig {
                every_minutes: None,
                every_rounds: Some(1),
                tips: vec!["x".repeat(MAX_TIP_BYTES + 1)],
            }],
        },
    ];
    for config in cases {
        assert!(config.validate().is_err());
    }
}

#[test]
fn malformed_file_falls_back_without_preventing_startup() {
    let dir = tmp_dir("malformed");
    fs::create_dir_all(&dir).unwrap();
    fs::write(reminder_tips_config_path(&dir), "not json").unwrap();
    assert_eq!(
        load_reminder_tips_config(&dir),
        ReminderTipsConfig::default()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn oversized_file_falls_back_without_unbounded_read_or_startup_failure() {
    let dir = tmp_dir("oversized");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        reminder_tips_config_path(&dir),
        vec![b' '; MAX_CONFIG_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(
        load_reminder_tips_config(&dir),
        ReminderTipsConfig::default()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn global_config_paths_follow_macos_linux_and_explicit_override_conventions() {
    let explicit = OsStr::new("/opt/timem-config");
    let xdg = OsStr::new("/home/alice/.xdg");
    let home = OsStr::new("/home/alice");
    assert_eq!(
        config_root_from_values(Some(explicit), Some(xdg), Some(home), false),
        PathBuf::from("/opt/timem-config")
    );
    assert_eq!(
        config_root_from_values(None, Some(xdg), Some(home), false),
        PathBuf::from("/home/alice/.xdg/timem")
    );
    assert_eq!(
        config_root_from_values(None, None, Some(home), false),
        PathBuf::from("/home/alice/.config/timem")
    );
    assert_eq!(
        config_root_from_values(None, None, Some(OsStr::new("/Users/alice")), true),
        PathBuf::from("/Users/alice/Library/Application Support/TimemAi")
    );
    assert_eq!(
        config_root_from_values(None, None, None, false),
        PathBuf::from("/etc/xdg/timem")
    );
}

use super::*;
use std::fs;

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

fn write_config(path: &Path, json: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, json).unwrap();
}

#[test]
fn missing_user_override_loads_resource_without_creating_user_file() {
    let root = tmp_dir("resource_default");
    let config_root = root.join("config");
    let resources = root.join("resources");
    write_config(
        &resources.join(REMINDER_TIPS_FILE_NAME),
        r#"{"schedules":[{"every_rounds":3,"tips":["TIPS: installed resource"]}]}"#,
    );

    let config = load_reminder_tips_config_from_resource_dirs(
        &config_root,
        std::slice::from_ref(&resources),
    );

    assert_eq!(config.schedules[0].every_rounds, Some(3));
    assert_eq!(config.schedules[0].tips, ["TIPS: installed resource"]);
    assert!(
        !reminder_tips_config_path(&config_root).exists(),
        "loading shipped resources must not create a user override"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn user_override_takes_precedence_over_installed_resource() {
    let root = tmp_dir("user_precedence");
    let config_root = root.join("config");
    let resources = root.join("resources");
    write_config(
        &reminder_tips_config_path(&config_root),
        r#"{"schedules":[{"every_rounds":2,"tips":["TIPS: user override"]}]}"#,
    );
    write_config(
        &resources.join(REMINDER_TIPS_FILE_NAME),
        r#"{"schedules":[{"every_rounds":3,"tips":["TIPS: installed resource"]}]}"#,
    );

    let config = load_reminder_tips_config_from_resource_dirs(
        &config_root,
        std::slice::from_ref(&resources),
    );

    assert_eq!(config.schedules[0].every_rounds, Some(2));
    assert_eq!(config.schedules[0].tips, ["TIPS: user override"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_user_override_falls_through_to_valid_resource() {
    let root = tmp_dir("invalid_user");
    let config_root = root.join("config");
    let resources = root.join("resources");
    write_config(&reminder_tips_config_path(&config_root), "not json");
    write_config(
        &resources.join(REMINDER_TIPS_FILE_NAME),
        r#"{"schedules":[{"every_minutes":4,"tips":["TIPS: resource fallback"]}]}"#,
    );

    let config = load_reminder_tips_config_from_resource_dirs(
        &config_root,
        std::slice::from_ref(&resources),
    );

    assert_eq!(config.schedules[0].every_minutes, Some(4));
    assert_eq!(config.schedules[0].tips, ["TIPS: resource fallback"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn first_valid_resource_candidate_wins() {
    let root = tmp_dir("resource_precedence");
    let first = root.join("first");
    let second = root.join("second");
    write_config(
        &first.join(REMINDER_TIPS_FILE_NAME),
        r#"{"schedules":[{"every_rounds":5,"tips":["TIPS: first"]}]}"#,
    );
    write_config(
        &second.join(REMINDER_TIPS_FILE_NAME),
        r#"{"schedules":[{"every_rounds":6,"tips":["TIPS: second"]}]}"#,
    );

    let config =
        load_reminder_tips_config_from_resource_dirs(&root.join("config"), &[first, second]);

    assert_eq!(config.schedules[0].every_rounds, Some(5));
    assert_eq!(config.schedules[0].tips, ["TIPS: first"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn default_config_is_parsed_from_the_shipped_resource() {
    let parsed = serde_json::from_str::<ReminderTipsConfig>(SHIPPED_REMINDER_TIPS)
        .unwrap()
        .validate()
        .unwrap();
    let config = ReminderTipsConfig::default();

    assert_eq!(config, parsed);
    assert_eq!(config.schedules.len(), 2);
    assert_eq!(config.schedules[0].every_minutes, Some(10));
    assert_eq!(config.schedules[1].every_rounds, Some(10));
    assert!(config
        .schedules
        .iter()
        .flat_map(|item| &item.tips)
        .all(|tip| tip == "NONE" || (tip.starts_with("TIPS: ") && tip.is_ascii())));
}

#[test]
fn missing_or_unusable_sources_fall_back_without_blocking_startup() {
    let root = tmp_dir("fallback");
    let config_root_is_file = root.join("config-is-file");
    fs::create_dir_all(&root).unwrap();
    fs::write(&config_root_is_file, "not a directory").unwrap();

    assert_eq!(
        load_reminder_tips_config_from_resource_dirs(&config_root_is_file, &[]),
        ReminderTipsConfig::default()
    );
    assert_eq!(
        load_reminder_tips_config_from_resource_dirs(&root.join("missing-config"), &[]),
        ReminderTipsConfig::default()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn custom_file_preserves_none_as_a_selectable_noop() {
    let root = tmp_dir("custom_none");
    let config_root = root.join("config");
    write_config(
        &reminder_tips_config_path(&config_root),
        r#"{"schedules":[{"every_rounds":3,"tips":["NONE","TIPS: custom"]}]}"#,
    );

    let config = load_reminder_tips_config_from_resource_dirs(&config_root, &[]);

    assert_eq!(config.schedules[0].tips, ["NONE", "TIPS: custom"]);
    let _ = fs::remove_dir_all(root);
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
fn oversized_override_falls_through_without_unbounded_read() {
    let root = tmp_dir("oversized");
    let config_root = root.join("config");
    write_config(
        &reminder_tips_config_path(&config_root),
        &" ".repeat(MAX_CONFIG_BYTES as usize + 1),
    );

    assert_eq!(
        load_reminder_tips_config_from_resource_dirs(&config_root, &[]),
        ReminderTipsConfig::default()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resource_candidates_follow_override_install_prefix_and_source_order() {
    let explicit = OsStr::new("/opt/timem-resources");
    let executable = Path::new("/home/alice/.local/bin/timem-web");
    let source = PathBuf::from("/checkout/resources");

    assert_eq!(
        resource_dir_candidates_from_values(Some(explicit), Some(executable), Some(source.clone())),
        vec![
            PathBuf::from("/opt/timem-resources"),
            PathBuf::from("/home/alice/.local/share/timem/resources"),
            source,
        ]
    );
    assert_eq!(
        resource_dir_candidates_from_values(None, Some(executable), None),
        vec![PathBuf::from("/home/alice/.local/share/timem/resources")]
    );
}

#[test]
fn global_config_path_honors_explicit_override() {
    let explicit = OsStr::new("/opt/timem-config");
    let xdg = OsStr::new("/home/alice/.xdg");
    let home = OsStr::new("/home/alice");

    assert_eq!(
        config_root_from_values(Some(explicit), Some(xdg), Some(home)),
        PathBuf::from("/opt/timem-config")
    );
}

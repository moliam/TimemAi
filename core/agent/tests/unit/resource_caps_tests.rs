use crate::resource_caps::{
    SystemResourceCapConfigLoad, DEFAULT_SYSTEM_CPU_CAP_PERCENT, DEFAULT_SYSTEM_MEM_CAP_PERCENT,
    SYSTEM_CPU_CAP_ENV, SYSTEM_MEM_CAP_ENV,
};
use std::collections::HashMap;

fn env_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn defaults_apply_without_env() {
    let load = SystemResourceCapConfigLoad::from_env_map(&HashMap::new());
    assert_eq!(load.config.cpu_percent, DEFAULT_SYSTEM_CPU_CAP_PERCENT);
    assert_eq!(load.config.mem_percent, DEFAULT_SYSTEM_MEM_CAP_PERCENT);
    assert_eq!(DEFAULT_SYSTEM_CPU_CAP_PERCENT, 85.0);
    assert_eq!(DEFAULT_SYSTEM_MEM_CAP_PERCENT, 85.0);
    assert!(load.warnings.is_empty());
}

#[test]
fn valid_values_override_defaults_without_warnings() {
    let load = SystemResourceCapConfigLoad::from_env_map(&env_map(&[
        (SYSTEM_CPU_CAP_ENV, "60.5"),
        (SYSTEM_MEM_CAP_ENV, " 100 "),
    ]));
    assert_eq!(load.config.cpu_percent, 60.5);
    assert_eq!(load.config.mem_percent, 100.0);
    assert!(load.warnings.is_empty());
}

#[test]
fn invalid_values_warn_and_fall_back_to_defaults() {
    for bad in ["abc", "0", "-5", "150", "nan", "inf", ""] {
        let load =
            SystemResourceCapConfigLoad::from_env_map(&env_map(&[(SYSTEM_CPU_CAP_ENV, bad)]));
        assert_eq!(
            load.config.cpu_percent, DEFAULT_SYSTEM_CPU_CAP_PERCENT,
            "cpu falls back for '{bad}'"
        );
        assert_eq!(load.config.mem_percent, DEFAULT_SYSTEM_MEM_CAP_PERCENT);
        assert_eq!(load.warnings.len(), 1, "one warning for '{bad}'");
        assert!(
            load.warnings[0].contains(SYSTEM_CPU_CAP_ENV),
            "{:?}",
            load.warnings
        );
        assert!(
            load.warnings[0].contains("using default 85"),
            "{:?}",
            load.warnings
        );
    }
}

#[test]
fn independent_dimensions_warn_independently() {
    let load = SystemResourceCapConfigLoad::from_env_map(&env_map(&[
        (SYSTEM_CPU_CAP_ENV, "70"),
        (SYSTEM_MEM_CAP_ENV, "bogus"),
    ]));
    assert_eq!(load.config.cpu_percent, 70.0);
    assert_eq!(load.config.mem_percent, DEFAULT_SYSTEM_MEM_CAP_PERCENT);
    assert_eq!(load.warnings.len(), 1);
    assert!(load.warnings[0].contains(SYSTEM_MEM_CAP_ENV));
}

use crate::os::{InstanceResourceCaps, ResourceCapStatus, ResourceCapsApplication};
use crate::resource_caps::{
    containment_note_for_application, SystemResourceCapConfigLoad, DEFAULT_SYSTEM_CPU_CAP_PERCENT,
    DEFAULT_SYSTEM_MEM_CAP_PERCENT, INSTANCE_CONTAINMENT_NOTE, SYSTEM_CPU_CAP_ENV,
    SYSTEM_MEM_CAP_ENV,
};
use std::collections::HashMap;

fn caps_application(cpu: ResourceCapStatus, mem: ResourceCapStatus) -> ResourceCapsApplication {
    ResourceCapsApplication {
        caps: InstanceResourceCaps {
            cpu_enabled: true,
            cpu_percent: 85.0,
            mem_enabled: true,
            mem_percent: 85.0,
        },
        cpu,
        mem,
        scope: "/sys/fs/cgroup/user.slice/timem-scope-test".to_string(),
    }
}

#[test]
fn containment_note_requires_a_platform_facility() {
    // Nothing recorded yet: nothing to say.
    assert_eq!(containment_note_for_application(None), None);
    // A platform without any kernel-level containment facility: no note.
    let unsupported = caps_application(
        ResourceCapStatus::Unsupported,
        ResourceCapStatus::Unsupported,
    );
    assert_eq!(containment_note_for_application(Some(&unsupported)), None);
    // The facility exists even when the bound is off or failed: keep the
    // note, because scope isolation and usage accounting keep working.
    for (cpu, mem) in [
        (
            ResourceCapStatus::Applied {
                limit: "850000".to_string(),
            },
            ResourceCapStatus::Disabled,
        ),
        (
            ResourceCapStatus::Disabled,
            ResourceCapStatus::Unavailable {
                reason: "cpu caps need root".to_string(),
            },
        ),
        (
            ResourceCapStatus::Unavailable {
                reason: "temporary".to_string(),
            },
            ResourceCapStatus::Applied {
                limit: "max".to_string(),
            },
        ),
    ] {
        let application = caps_application(cpu, mem);
        assert_eq!(
            containment_note_for_application(Some(&application)),
            Some(INSTANCE_CONTAINMENT_NOTE)
        );
    }
    // The note guides the model to spawn tool work in the most constrained
    // way inside the managed tree, leaving only when truly unavoidable or
    // the user explicitly asks for it.
    assert!(INSTANCE_CONTAINMENT_NOTE.contains("managed process tree"));
    assert!(INSTANCE_CONTAINMENT_NOTE.contains("cgroup"));
    assert!(INSTANCE_CONTAINMENT_NOTE.contains("spawn work in the most constrained way possible"));
    assert!(
        INSTANCE_CONTAINMENT_NOTE.contains("truly unavoidable or the user explicitly asks for it")
    );
}

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

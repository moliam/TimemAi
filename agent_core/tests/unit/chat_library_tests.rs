use super::*;

#[test]
fn capacity_warns_above_ninety_percent_and_blocks_only_above_limit() {
    assert!(!capacity_is_nearing_limit(ChatLibraryCapacity {
        used_bytes: 90,
        limit_bytes: Some(100),
        used_percent: Some(90),
    }));
    assert!(capacity_is_nearing_limit(ChatLibraryCapacity {
        used_bytes: 91,
        limit_bytes: Some(100),
        used_percent: Some(91),
    }));
    assert!(!capacity_is_nearing_limit(ChatLibraryCapacity {
        used_bytes: u64::MAX,
        limit_bytes: None,
        used_percent: None,
    }));
    assert!(!capacity_exceeds_limit(ChatLibraryCapacity {
        used_bytes: 100,
        limit_bytes: Some(100),
        used_percent: Some(100),
    }));
    assert!(capacity_exceeds_limit(ChatLibraryCapacity {
        used_bytes: 101,
        limit_bytes: Some(100),
        used_percent: Some(100),
    }));
}

#[test]
fn capacity_limit_accepts_only_supported_tiers() {
    assert!(validate_capacity_limit(Some(FAVORITES_LIMIT_256_MB)).is_ok());
    assert!(validate_capacity_limit(Some(FAVORITES_LIMIT_1_GB)).is_ok());
    assert!(validate_capacity_limit(None).is_ok());
    assert_eq!(
        validate_capacity_limit(Some(512)),
        Err("favorite_capacity_limit_invalid".to_string()),
    );
}

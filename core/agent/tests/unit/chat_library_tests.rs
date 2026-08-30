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

#[test]
fn rolling_favorite_rewrite_evicts_oldest_and_keeps_newest_record_complete() {
    let root = std::env::temp_dir().join(format!(
        "timem_favorite_rollover_{}_{}",
        std::process::id(),
        NEXT_LIBRARY_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("favorites.jsonl");
    let favorite = |id: &str, created_at_ms: i64, content: &str| ChatFavorite {
        id: id.to_string(),
        source_key: format!("source-{id}"),
        session_id: "session-a".to_string(),
        session_display_name: "Session A".to_string(),
        turn_id: format!("turn-{id}"),
        title: id.to_string(),
        content_snapshot: content.to_string(),
        source_created_at_ms: created_at_ms,
        created_at_ms,
        updated_at_ms: created_at_ms,
        version: 1,
        deleted: false,
    };
    let old = favorite("old", 1, &"old payload ".repeat(16));
    let new = favorite("new", 2, &"new payload ".repeat(16));
    let records = vec![
        serialized_favorite_bytes(&old).unwrap(),
        serialized_favorite_bytes(&new).unwrap(),
    ];
    let stable = records[1].len() as u64;
    let slice = stable.max(1);
    let capacity = RollingCapacity::with_slice_bytes(stable + slice, slice).unwrap();

    rewrite_segmented_records(&path, &records, capacity, slice).unwrap();

    let retained = read_segmented_records(&path).unwrap();
    assert_eq!(retained.len(), 1);
    let retained: ChatFavorite = serde_json::from_slice(&retained[0]).unwrap();
    assert_eq!(retained.id, "new");
    assert_eq!(retained.content_snapshot, new.content_snapshot);
    let _ = fs::remove_dir_all(root);
}

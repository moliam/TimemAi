use super::*;

#[test]
fn capacity_reserves_exactly_one_slice() {
    let capacity = RollingCapacity::from_total_bytes(16 * DEFAULT_ROLLING_SLICE_BYTES).unwrap();
    assert_eq!(capacity.stable_bytes, 15 * DEFAULT_ROLLING_SLICE_BYTES);
    assert_eq!(capacity.reserved_bytes, DEFAULT_ROLLING_SLICE_BYTES);
    assert!(RollingCapacity::from_total_bytes(DEFAULT_ROLLING_SLICE_BYTES).is_err());
    let audit =
        RollingCapacity::with_slice_bytes(512 * 1024 * 1024, AUDIT_ROLLING_SLICE_BYTES).unwrap();
    assert_eq!(audit.stable_bytes, 496 * 1024 * 1024);
    assert!(RollingCapacity::from_total_bytes(65 * 1024 * 1024).is_err());
}

#[test]
fn eviction_never_splits_records() {
    assert_eq!(newest_records_start(&[4, 5, 6], 11).unwrap(), 1);
    assert_eq!(newest_records_start(&[4, 5, 6], 15).unwrap(), 0);
    assert_eq!(
        newest_records_start(&[4, 12], 11),
        Err("rolling_record_exceeds_capacity")
    );
}

#[test]
fn segmented_rewrite_migrates_legacy_file_and_keeps_complete_records() {
    let root = std::env::temp_dir().join(format!(
        "timem_rolling_segments_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("favorites.jsonl");
    std::fs::write(&path, b"legacy-one\nlegacy-two\n").unwrap();
    assert_eq!(read_segmented_records(&path).unwrap().len(), 2);

    let records = vec![
        b"first\n".to_vec(),
        b"second\n".to_vec(),
        b"third\n".to_vec(),
    ];
    let capacity = RollingCapacity::with_slice_bytes(24, 8).unwrap();
    let result = rewrite_segmented_records(&path, &records, capacity, 8).unwrap();
    assert_eq!(result.evicted_records, 1);
    assert!(!path.exists());
    assert!(segmented_directory(&path).exists());
    assert_eq!(read_segmented_records(&path).unwrap(), records[1..]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn segmented_read_recovers_an_interrupted_directory_swap() {
    let root = std::env::temp_dir().join(format!(
        "timem_rolling_recovery_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("favorites.jsonl");
    let backup = root.join(".rolling-segments.old-test");
    let stale = root.join(".rolling-segments.tmp-test");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(backup.join("segment-00000000.jsonl"), b"safe-record\n").unwrap();
    std::fs::write(stale.join("segment-00000000.jsonl"), b"unfinished\n").unwrap();

    assert_eq!(
        read_segmented_records(&path).unwrap(),
        vec![b"safe-record\n".to_vec()]
    );
    assert!(segmented_directory(&path).exists());
    assert!(!backup.exists());
    assert!(!stale.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn incremental_append_keeps_closed_segments_unchanged() {
    let root = std::env::temp_dir().join(format!(
        "timem_rolling_incremental_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("audit.jsonl");
    let capacity = RollingCapacity::with_slice_bytes(32, 8).unwrap();

    append_rolling_record(&path, b"first\n", capacity, 8).unwrap();
    append_rolling_record(&path, b"second\n", capacity, 8).unwrap();
    let before = rolling_segments(&path).unwrap();
    assert_eq!(before.len(), 2);
    let closed_path = before[0].path.clone();
    let closed_bytes = std::fs::read(&closed_path).unwrap();
    let closed_modified = std::fs::metadata(&closed_path).unwrap().modified().unwrap();

    append_rolling_record(&path, b"third\n", capacity, 8).unwrap();

    assert_eq!(std::fs::read(&closed_path).unwrap(), closed_bytes);
    assert_eq!(
        std::fs::metadata(&closed_path).unwrap().modified().unwrap(),
        closed_modified
    );
    assert_eq!(
        read_segmented_records(&path).unwrap(),
        vec![
            b"first\n".to_vec(),
            b"second\n".to_vec(),
            b"third\n".to_vec()
        ]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn incremental_capacity_evicts_only_complete_oldest_segments() {
    let root = std::env::temp_dir().join(format!(
        "timem_rolling_evict_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("audit.jsonl");
    let capacity = RollingCapacity::with_slice_bytes(24, 8).unwrap();

    for record in [b"one---\n", b"two---\n", b"three-\n"] {
        append_rolling_record(&path, record, capacity, 8).unwrap();
    }
    let before = rolling_segments(&path).unwrap();
    assert_eq!(before.len(), 2);
    let oldest_path = before[0].path.clone();

    assert_eq!(
        append_rolling_record(&path, b"four--\n", capacity, 8).unwrap(),
        1
    );
    assert!(!oldest_path.exists());
    assert_eq!(
        read_segmented_records(&path).unwrap(),
        vec![b"three-\n".to_vec(), b"four--\n".to_vec()]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn trimming_shared_budget_may_remove_the_last_complete_segment() {
    let root = std::env::temp_dir().join(format!(
        "timem_rolling_trim_last_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("audit.jsonl");
    let capacity = RollingCapacity::with_slice_bytes(24, 8).unwrap();
    append_rolling_record(&path, b"only--\n", capacity, 8).unwrap();

    assert_eq!(trim_rolling_segments(&path, 0, 8).unwrap(), 1);
    assert!(rolling_segments(&path).unwrap().is_empty());
    assert!(read_segmented_records(&path).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn append_repairs_an_active_segment_length_left_ahead_of_its_manifest() {
    let root = std::env::temp_dir().join(format!(
        "timem_rolling_manifest_repair_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("audit.jsonl");
    let capacity = RollingCapacity::with_slice_bytes(32, 8).unwrap();
    append_rolling_record(&path, b"one\n", capacity, 8).unwrap();
    let active = rolling_segments(&path)
        .unwrap()
        .last()
        .unwrap()
        .path
        .clone();

    std::fs::OpenOptions::new()
        .append(true)
        .open(&active)
        .unwrap()
        .write_all(b"two\n")
        .unwrap();
    append_rolling_record(&path, b"three\n", capacity, 8).unwrap();

    assert_eq!(
        read_segmented_records(&path).unwrap(),
        vec![b"one\n".to_vec(), b"two\n".to_vec(), b"three\n".to_vec()]
    );
    assert_eq!(rolling_segments(&path).unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn append_rebuilds_a_corrupt_manifest_without_losing_segments() {
    let root = std::env::temp_dir().join(format!(
        "timem_rolling_corrupt_manifest_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("audit.jsonl");
    let capacity = RollingCapacity::with_slice_bytes(32, 8).unwrap();
    append_rolling_record(&path, b"one---\n", capacity, 8).unwrap();
    append_rolling_record(&path, b"two---\n", capacity, 8).unwrap();
    std::fs::write(rolling_manifest_path(&path), b"not-json").unwrap();

    append_rolling_record(&path, b"three-\n", capacity, 8).unwrap();

    assert_eq!(
        read_segmented_records(&path).unwrap(),
        vec![
            b"one---\n".to_vec(),
            b"two---\n".to_vec(),
            b"three-\n".to_vec(),
        ]
    );
    let repaired: RollingManifest =
        serde_json::from_slice(&std::fs::read(rolling_manifest_path(&path)).unwrap()).unwrap();
    assert_eq!(repaired.version, 1);
    assert_eq!(repaired.segments.len(), 3);
    let _ = std::fs::remove_dir_all(root);
}

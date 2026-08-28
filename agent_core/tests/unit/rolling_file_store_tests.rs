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

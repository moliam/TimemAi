use super::*;

const GB: u64 = 1024 * 1024 * 1024;
const CAP: u64 = 10 * GB;
// threshold = min(200MB, 8% * 10GB) = 200MB; the implementation uses
// integer math capacity / 100 * 8, so mirror it exactly.
const THRESHOLD: u64 = 200 * 1024 * 1024;

fn sample(free: u64, capacity: u64) -> DiskSample {
    DiskSample {
        free,
        capacity,
        devices: vec![(1, capacity)],
    }
}

fn tracker() -> DiskPressureTracker {
    DiskPressureTracker::new()
}

fn advance_to_sample(t: &mut DiskPressureTracker, free: u64) -> Option<DiskPressureEvent> {
    let mut event = None;
    for _ in 0..DISK_SAMPLE_INTERVAL {
        event = t.observe(Some(sample(free, CAP)));
    }
    assert_eq!(t.pending_observations(), 0, "sample must reset count gate");
    event
}

#[test]
fn sample_callback_is_lazy_until_a_gate_is_due() {
    let start = Instant::now();
    let mut t = tracker();
    t.seed_baseline_at(Some(sample(5 * GB, CAP)), start);
    let mut sample_calls = 0;
    for step in 1..DISK_SAMPLE_INTERVAL {
        assert!(t
            .observe_with_at(
                || {
                    sample_calls += 1;
                    Some(sample(5 * GB, CAP))
                },
                start + Duration::from_secs(step as u64),
            )
            .is_none());
    }
    assert_eq!(
        sample_calls, 0,
        "sampling must stay lazy before a gate is due"
    );
    assert!(t
        .observe_with_at(
            || {
                sample_calls += 1;
                Some(sample(5 * GB, CAP))
            },
            start + Duration::from_secs(DISK_SAMPLE_INTERVAL as u64),
        )
        .is_none());
    assert_eq!(sample_calls, 1, "the due observation samples exactly once");
}

#[test]
fn count_gate_samples_on_tenth_observation_not_before() {
    let start = Instant::now();
    let mut t = tracker();
    t.seed_baseline_at(Some(sample(5 * GB, CAP)), start);
    for step in 1..DISK_SAMPLE_INTERVAL {
        assert!(
            t.observe_at(
                Some(sample(5 * GB - THRESHOLD - 1, CAP)),
                start + Duration::from_secs(step as u64)
            )
            .is_none(),
            "observation {step} must not sample early"
        );
        assert_eq!(t.pending_observations(), step);
    }
    assert!(
        t.observe_at(
            Some(sample(5 * GB - THRESHOLD - 1, CAP)),
            start + Duration::from_secs(DISK_SAMPLE_INTERVAL as u64),
        )
        .is_some(),
        "tenth observation must sample"
    );
    assert_eq!(t.pending_observations(), 0);
}

#[test]
fn time_gate_samples_after_three_minutes_with_one_observation() {
    let start = Instant::now();
    let mut t = tracker();
    t.seed_baseline_at(Some(sample(5 * GB, CAP)), start);
    assert!(
        t.observe_at(
            Some(sample(5 * GB - THRESHOLD - 1, CAP)),
            start + DISK_SAMPLE_MAX_AGE - Duration::from_millis(1),
        )
        .is_none(),
        "time gate must not fire early"
    );
    assert_eq!(t.pending_observations(), 1);
    assert!(
        t.observe_at(
            Some(sample(5 * GB - THRESHOLD - 1, CAP)),
            start + DISK_SAMPLE_MAX_AGE,
        )
        .is_some(),
        "first observation at three minutes must sample"
    );
    assert_eq!(t.pending_observations(), 0);
}

#[test]
fn successful_sample_resets_both_count_and_time_gates() {
    let start = Instant::now();
    let mut t = tracker();
    t.seed_baseline_at(Some(sample(5 * GB, CAP)), start);
    // Time gate closes the first window with only one observation.
    assert!(t
        .observe_at(Some(sample(5 * GB, CAP)), start + DISK_SAMPLE_MAX_AGE)
        .is_none());
    assert_eq!(t.pending_observations(), 0);
    // Nine observations and just under three minutes from the new sample
    // must not close the next window.
    for step in 1..DISK_SAMPLE_INTERVAL {
        assert!(t
            .observe_at(
                Some(sample(5 * GB - THRESHOLD - 1, CAP)),
                start + DISK_SAMPLE_MAX_AGE + Duration::from_secs(step as u64),
            )
            .is_none());
    }
    assert_eq!(t.pending_observations(), DISK_SAMPLE_INTERVAL - 1);
    // The tenth observation closes it via the count gate.
    assert!(t
        .observe_at(
            Some(sample(5 * GB - THRESHOLD - 1, CAP)),
            start + DISK_SAMPLE_MAX_AGE + Duration::from_secs(10),
        )
        .is_some());
    assert_eq!(t.pending_observations(), 0);
}

#[test]
fn unavailable_sample_does_not_reset_due_window() {
    let start = Instant::now();
    let mut t = tracker();
    t.seed_baseline_at(Some(sample(5 * GB, CAP)), start);
    for step in 1..DISK_SAMPLE_INTERVAL {
        assert!(t
            .observe_at(None, start + Duration::from_secs(step as u64))
            .is_none());
    }
    assert!(
        t.observe_at(None, start + Duration::from_secs(10))
            .is_none(),
        "missing sample cannot close the due window"
    );
    assert_eq!(t.pending_observations(), DISK_SAMPLE_INTERVAL);
    assert!(
        t.observe_at(
            Some(sample(5 * GB - THRESHOLD - 1, CAP)),
            start + Duration::from_secs(11),
        )
        .is_some(),
        "next available sample must service the overdue window"
    );
    assert_eq!(t.pending_observations(), 0);
}

#[test]
fn event_render_includes_before_after_and_disk_table() {
    let event = DiskPressureEvent {
        dropped: 255_400_000,
        base: 91_000_000_000,
        new: 90_700_000_000,
    };
    let filesystems = vec![FilesystemUsage {
        device_id: 1,
        path: "/".into(),
        total_bytes: 982_862_268 * 1024,
        free_bytes: 84_600_000_000,
    }];
    let text = event.render(&filesystems);
    assert!(text.contains("dropped by 243.6 MB"), "{text}");
    assert!(text.contains("(84.8 GB -> 84.5 GB)"), "{text}");
    assert!(text.contains("current disk info:"), "{text}");
    assert!(text.contains("| `/` | 937.3 GB | 78.8 GB |"), "{text}");
}

#[test]
fn seeded_baseline_makes_first_window_compare_immediately() {
    let mut t = tracker();
    t.seed_baseline(Some(sample(5 * GB, CAP)));
    assert_eq!(t.baseline(), Some(5 * GB));
    // First window right after startup can now trigger: no blind window.
    let notice = advance_to_sample(&mut t, 5 * GB - THRESHOLD - 1);
    assert!(
        notice.is_some(),
        "startup-seeded baseline must compare in the first window"
    );
}

#[test]
fn seed_is_ignored_when_baseline_already_exists() {
    let mut t = tracker();
    assert!(advance_to_sample(&mut t, 5 * GB).is_none());
    t.seed_baseline(Some(sample(GB, CAP)));
    assert_eq!(
        t.baseline(),
        Some(5 * GB),
        "seed must not overwrite an existing baseline"
    );
}

#[test]
fn first_sample_only_sets_baseline() {
    let mut t = tracker();
    assert!(advance_to_sample(&mut t, 5 * GB).is_none());
    assert_eq!(t.baseline(), Some(5 * GB));
}

#[test]
fn drop_below_capped_threshold_keeps_baseline() {
    let mut t = tracker();
    assert!(advance_to_sample(&mut t, 5 * GB).is_none());
    // 150MB drop: below the 200MB capped threshold, no notice, and the
    // baseline stays so consumption accumulates.
    assert!(advance_to_sample(&mut t, 5 * GB - 150 * 1024 * 1024).is_none());
    assert_eq!(t.baseline(), Some(5 * GB));
}

#[test]
fn drop_just_above_capped_threshold_triggers() {
    let mut t = tracker();
    assert!(advance_to_sample(&mut t, 5 * GB).is_none());
    // A 250MB drop on a large disk must trigger: the threshold is
    // capped at 200MB, not scaled to 8% of the capacity.
    let notice = advance_to_sample(&mut t, 5 * GB - 250 * 1024 * 1024);
    assert!(
        notice.is_some(),
        "250MB drop must trigger the 200MB-capped threshold"
    );
}

#[test]
fn large_drop_triggers_once_and_rebases() {
    let mut t = tracker();
    assert!(advance_to_sample(&mut t, 5 * GB).is_none());
    let notice = advance_to_sample(&mut t, 5 * GB - THRESHOLD - 1);
    assert!(notice.is_some(), "drop just above threshold must trigger");
    assert_eq!(t.baseline(), Some(5 * GB - THRESHOLD - 1));
    // Same level again: no notice.
    assert!(advance_to_sample(&mut t, 5 * GB - THRESHOLD - 1).is_none());
}

#[test]
fn small_disks_scale_below_the_150mb_cap() {
    // A 1GB disk scales to only 80MB (8%), which is now below the cap.
    let small_cap = GB;
    let scaled = small_cap / 100 * 8;
    assert!(scaled < 200 * 1024 * 1024);
    let mut t = tracker();
    for _ in 0..DISK_SAMPLE_INTERVAL {
        assert!(t.observe(Some(sample(small_cap, small_cap))).is_none());
    }
    assert_eq!(t.pending_observations(), 0);
    // Just above the scaled threshold must trigger.
    let drop = scaled + 1;
    let mut notice = None;
    for _ in 0..DISK_SAMPLE_INTERVAL {
        notice = t.observe(Some(sample(small_cap - drop, small_cap)));
    }
    assert_eq!(t.pending_observations(), 0);
    assert!(
        notice.is_some(),
        "drop just above the 8% scaled threshold must trigger"
    );
    // Just below the scaled threshold must not.
    let mut t2 = tracker();
    for _ in 0..DISK_SAMPLE_INTERVAL {
        assert!(t2.observe(Some(sample(small_cap, small_cap))).is_none());
    }
    assert_eq!(t2.pending_observations(), 0);
    let mut notice = None;
    for _ in 0..DISK_SAMPLE_INTERVAL {
        notice = t2.observe(Some(sample(small_cap - scaled + 1, small_cap)));
    }
    assert_eq!(t2.pending_observations(), 0);
    assert!(
        notice.is_none(),
        "drop below the 8% scaled threshold must not trigger"
    );
}

#[test]
fn reclaiming_space_rebases_without_notice() {
    let mut t = tracker();
    assert!(advance_to_sample(&mut t, 5 * GB).is_none());
    assert!(advance_to_sample(&mut t, 6 * GB).is_none());
    assert_eq!(t.baseline(), Some(6 * GB));
}

#[test]
fn removing_a_disk_does_not_report_its_free_space_as_consumed() {
    let mut t = tracker();
    t.seed_baseline(Some(sample(100 * GB, 200 * GB)));
    for _ in 0..DISK_SAMPLE_INTERVAL {
        assert!(t.observe(Some(sample(150 * GB, 300 * GB))).is_none());
    }
    for _ in 0..DISK_SAMPLE_INTERVAL {
        assert!(
            t.observe(Some(sample(100 * GB, 200 * GB))).is_none(),
            "removing a disk is not consuming 50GiB"
        );
    }
}

fn disks(entries: &[(u64, u64, u64)]) -> DiskSample {
    DiskSample::from_filesystems(
        &entries
            .iter()
            .map(|&(id, capacity, free)| FilesystemUsage {
                device_id: id,
                path: format!("/disk/{id}"),
                total_bytes: capacity,
                free_bytes: free,
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn sample_disks(
    t: &mut DiskPressureTracker,
    entries: &[(u64, u64, u64)],
) -> Option<DiskPressureEvent> {
    let mut event = None;
    for _ in 0..DISK_SAMPLE_INTERVAL {
        event = t.observe(Some(disks(entries)));
    }
    event
}

#[test]
fn topology_change_rebases_then_real_consumption_still_alerts() {
    let mut t = tracker();
    assert!(sample_disks(&mut t, &[(1, 200 * GB, 100 * GB)]).is_none());
    assert!(sample_disks(&mut t, &[(1, 200 * GB, 100 * GB), (2, 100 * GB, 50 * GB)]).is_none());
    assert!(sample_disks(&mut t, &[(1, 200 * GB, 100 * GB)]).is_none());
    assert_eq!(t.baseline(), Some(100 * GB));
    let event = sample_disks(&mut t, &[(1, 200 * GB, 99 * GB)]).unwrap();
    assert_eq!(event.dropped, GB);
}

#[test]
fn equal_capacity_replacement_is_not_the_same_disk() {
    let mut t = tracker();
    assert!(sample_disks(&mut t, &[(1, CAP, 5 * GB)]).is_none());
    assert!(sample_disks(&mut t, &[(2, CAP, GB)]).is_none());
    assert_eq!(t.baseline(), Some(GB));
}

#[test]
fn enumeration_order_and_path_alias_do_not_reset_accumulated_drop() {
    let mut t = tracker();
    assert!(sample_disks(&mut t, &[(1, CAP, 5 * GB), (2, CAP, GB)]).is_none());
    assert!(sample_disks(&mut t, &[(2, CAP, GB), (1, CAP, 5 * GB - THRESHOLD)]).is_none());
    assert_eq!(t.baseline(), Some(6 * GB));
    let mut fs = vec![
        FilesystemUsage {
            device_id: 1,
            path: "/another/alias".into(),
            total_bytes: CAP,
            free_bytes: 5 * GB - THRESHOLD - 1,
        },
        FilesystemUsage {
            device_id: 2,
            path: "/disk/two".into(),
            total_bytes: CAP,
            free_bytes: GB,
        },
    ];
    let mut event = None;
    for _ in 0..DISK_SAMPLE_INTERVAL {
        event = t.observe(DiskSample::from_filesystems(&fs));
        fs.reverse();
    }
    assert_eq!(event.unwrap().dropped, THRESHOLD + 1);
}

#[test]
fn empty_sample_keeps_due_window_and_original_baseline() {
    let mut t = tracker();
    assert!(sample_disks(&mut t, &[(1, CAP, 5 * GB)]).is_none());
    for _ in 0..DISK_SAMPLE_INTERVAL {
        assert!(t.observe(DiskSample::from_filesystems(&[])).is_none());
    }
    assert_eq!(t.baseline(), Some(5 * GB));
    assert!(t.observe(Some(disks(&[(1, CAP, 4 * GB)]))).is_some());
}

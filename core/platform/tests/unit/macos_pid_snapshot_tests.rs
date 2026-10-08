use super::macos_pid_snapshot;

#[test]
fn return_value_is_pid_count_not_byte_count() {
    let pids = macos_pid_snapshot(|buffer| {
        if buffer.is_empty() {
            return 4;
        }
        buffer[..4].copy_from_slice(&[11, 22, 33, 44]);
        4
    })
    .unwrap();
    assert_eq!(pids, vec![11, 22, 33, 44]);
}

#[test]
fn full_positive_buffer_requires_regrowth() {
    let mut calls = 0;
    let pids = macos_pid_snapshot(|buffer| {
        calls += 1;
        match calls {
            1 => 1,
            2 => {
                assert_eq!(buffer.len(), 33);
                33
            }
            3 => {
                assert_eq!(buffer.len(), 66);
                buffer[..2].copy_from_slice(&[7, 8]);
                2
            }
            _ => panic!("unexpected retry"),
        }
    });
    assert_eq!(pids, Some(vec![7, 8]));
    assert_eq!(calls, 3);
}

#[test]
fn incomplete_or_failed_snapshot_is_unknown_not_empty() {
    for failure in [0, -1, i32::MAX] {
        assert_eq!(macos_pid_snapshot(|_| failure), None);
    }
    let mut calls = 0;
    assert_eq!(
        macos_pid_snapshot(|buffer| {
            calls += 1;
            if buffer.is_empty() {
                1
            } else {
                buffer.len() as i32
            }
        }),
        None
    );
    assert_eq!(calls, 4, "one sizing call and at most three fills");
    let mut calls = 0;
    assert_eq!(
        macos_pid_snapshot(|_| {
            calls += 1;
            if calls == 1 {
                4
            } else {
                -1
            }
        }),
        None
    );
}

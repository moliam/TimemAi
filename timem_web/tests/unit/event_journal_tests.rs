use super::*;
use serde_json::json;
use std::io::Write;
use std::sync::{Arc, Barrier, Mutex};

fn journal_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "timem_web_event_journal_{name}_{}_{}.ndjson",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

#[test]
fn append_restart_and_cursor_replay_preserve_exact_order() {
    let path = journal_path("restart");
    let mut journal = EventJournal::open(&path).unwrap();
    assert_eq!(journal.cursor(), 0);
    assert_eq!(
        journal
            .append(json!({"type":"turn_updated"}))
            .unwrap()
            .event_seq,
        1
    );
    assert_eq!(
        journal
            .append(json!({"type":"core_topic"}))
            .unwrap()
            .event_seq,
        2
    );
    drop(journal);

    let mut recovered = EventJournal::open(&path).unwrap();
    assert_eq!(recovered.cursor(), 2);
    assert_eq!(
        recovered.replay_after(1).unwrap(),
        vec![JournalEvent {
            event_seq: 2,
            event: json!({"type":"core_topic"})
        }]
    );
    assert_eq!(
        recovered
            .append(json!({"type":"turn_finished"}))
            .unwrap()
            .event_seq,
        3
    );
    assert_eq!(
        recovered
            .replay_after(0)
            .unwrap()
            .iter()
            .map(|event| event.event_seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn snapshot_cursor_plus_replay_covers_an_event_during_snapshot_without_a_gap() {
    let path = journal_path("snapshot_gap");
    let mut journal = EventJournal::open(&path).unwrap();
    journal.append(json!({"type":"before_snapshot"})).unwrap();

    // The connection captures this cursor before constructing its snapshot.
    // An event occurring during snapshot construction is then replayed after it.
    let snapshot_cursor = journal.cursor();
    journal.append(json!({"type":"during_snapshot"})).unwrap();
    let replay = journal.replay_after(snapshot_cursor).unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].event["type"], "during_snapshot");
    let _ = std::fs::remove_file(path);
}

#[test]
fn reconnect_from_last_ack_replays_each_later_semantic_event_once() {
    let path = journal_path("reconnect");
    let mut journal = EventJournal::open(&path).unwrap();
    for ordinal in 1..=5 {
        journal.append(json!({"ordinal": ordinal})).unwrap();
    }

    let replay = journal.replay_after(3).unwrap();
    assert_eq!(
        replay
            .iter()
            .map(|entry| (entry.event_seq, entry.event["ordinal"].as_u64().unwrap()))
            .collect::<Vec<_>>(),
        vec![(4, 4), (5, 5)]
    );
    assert!(journal.replay_after(5).unwrap().is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn partial_last_write_does_not_hide_the_prior_durable_prefix() {
    let path = journal_path("partial_tail");
    let mut journal = EventJournal::open(&path).unwrap();
    journal.append(json!({"type":"durable"})).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap();
    file.write_all(br#"{"event_seq":2,"event":{"type":"partial"}"#)
        .unwrap();
    file.sync_data().unwrap();
    drop(journal);

    let recovered = EventJournal::open(&path).unwrap();
    assert_eq!(recovered.cursor(), 1);
    assert_eq!(recovered.replay_after(0).unwrap().len(), 1);
    drop(recovered);
    let mut continued = EventJournal::open(&path).unwrap();
    assert_eq!(
        continued
            .append(json!({"type":"after_recovery"}))
            .unwrap()
            .event_seq,
        2
    );
    assert_eq!(continued.replay_after(0).unwrap().len(), 2);
    let _ = std::fs::remove_file(path);
}

#[test]
fn complete_json_without_newline_is_preserved_and_separated_before_append() {
    let path = journal_path("missing_newline");
    std::fs::write(&path, br#"{"event_seq":1,"event":{"type":"durable"}}"#).unwrap();
    let mut recovered = EventJournal::open(&path).unwrap();
    assert_eq!(recovered.cursor(), 1);
    assert_eq!(
        recovered.append(json!({"type":"next"})).unwrap().event_seq,
        2
    );
    assert_eq!(recovered.replay_after(0).unwrap().len(), 2);
    let _ = std::fs::remove_file(path);
}

#[test]
fn corruption_between_durable_entries_is_rejected_instead_of_skipped() {
    let path = journal_path("middle_corruption");
    std::fs::write(
        &path,
        concat!(
            "{\"event_seq\":1,\"event\":{\"ok\":1}}\n",
            "not-json\n",
            "{\"event_seq\":3,\"event\":{\"ok\":3}}\n"
        ),
    )
    .unwrap();
    assert!(EventJournal::open(&path)
        .unwrap_err()
        .contains("event_journal_parse_failed"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn concurrent_publishers_receive_one_global_strictly_increasing_sequence() {
    const PUBLISHERS: usize = 12;
    const EVENTS_PER_PUBLISHER: usize = 20;
    let path = journal_path("concurrent_publish");
    let journal = Arc::new(Mutex::new(EventJournal::open(&path).unwrap()));
    let barrier = Arc::new(Barrier::new(PUBLISHERS));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let threads = (0..PUBLISHERS)
        .map(|publisher| {
            let journal = Arc::clone(&journal);
            let barrier = Arc::clone(&barrier);
            let observed = Arc::clone(&observed);
            std::thread::spawn(move || {
                barrier.wait();
                for ordinal in 0..EVENTS_PER_PUBLISHER {
                    let entry = journal
                        .lock()
                        .unwrap()
                        .append(json!({"session_id": format!("session_{publisher}"), "ordinal": ordinal}))
                        .unwrap();
                    observed.lock().unwrap().push(entry.event_seq);
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }

    let replay = journal.lock().unwrap().replay_after(0).unwrap();
    let expected = (1..=(PUBLISHERS * EVENTS_PER_PUBLISHER) as u64).collect::<Vec<_>>();
    assert_eq!(
        replay
            .iter()
            .map(|entry| entry.event_seq)
            .collect::<Vec<_>>(),
        expected
    );
    let mut allocated = observed.lock().unwrap().clone();
    allocated.sort_unstable();
    assert_eq!(
        allocated, expected,
        "no concurrent publisher may reuse or skip a sequence"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn replay_while_an_append_is_blocked_has_a_precise_cursor_boundary() {
    let path = journal_path("concurrent_replay");
    let journal = Arc::new(Mutex::new(EventJournal::open(&path).unwrap()));
    journal
        .lock()
        .unwrap()
        .append(json!({"ordinal": 1}))
        .unwrap();
    let append_started = Arc::new(Barrier::new(2));
    let allow_append = Arc::new(Barrier::new(2));
    let publisher = {
        let journal = Arc::clone(&journal);
        let append_started = Arc::clone(&append_started);
        let allow_append = Arc::clone(&allow_append);
        std::thread::spawn(move || {
            append_started.wait();
            allow_append.wait();
            journal
                .lock()
                .unwrap()
                .append(json!({"ordinal": 2}))
                .unwrap()
        })
    };
    append_started.wait();
    assert!(journal.lock().unwrap().replay_after(1).unwrap().is_empty());
    allow_append.wait();
    let appended = publisher.join().unwrap();
    assert_eq!(appended.event_seq, 2);
    assert_eq!(
        journal.lock().unwrap().replay_after(1).unwrap(),
        vec![appended]
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn compaction_preserves_monotonic_sequences_and_exposes_a_replay_floor() {
    let path = journal_path("compaction");
    let mut journal = EventJournal::open(&path).unwrap();
    for ordinal in 0..100 {
        journal.append(json!({"ordinal": ordinal})).unwrap();
    }
    journal.compact_to_last(32).unwrap();
    let next = journal
        .append(json!({"ordinal": "after_compaction"}))
        .unwrap();
    assert_eq!(next.event_seq, 101);
    assert!(journal.replay_floor() > 0);
    assert!(journal
        .replay_after(journal.replay_floor() - 1)
        .unwrap_err()
        .contains("cursor_before_floor"));
    let replay = journal.replay_after(journal.replay_floor()).unwrap();
    assert_eq!(replay.last().unwrap().event_seq, next.event_seq);
    assert!(replay.len() <= 33);

    let recovered = EventJournal::open(&path).unwrap();
    assert_eq!(recovered.cursor(), next.event_seq);
    assert_eq!(recovered.replay_floor(), journal.replay_floor());
    let _ = std::fs::remove_file(path);
}

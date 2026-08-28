use super::*;
use serde_json::json;

#[test]
fn command_tool_background_job_can_be_polled_until_finished() {
    let dir = temp_case_dir("command_tool_background");
    let script = dir.join("echo_payload.sh");
    fs::write(
            &script,
            "#!/bin/sh\npython3 -c 'import sys,json; data=json.load(sys.stdin); print(data[\"args\"][\"message\"])'\n",
        )
        .unwrap();
    let store = FileToolJobStore::new(&dir);

    let started = store.spawn(
        "local_echo",
        &script,
        &json!({"args":{"message":"background payload ok"}}),
    );
    assert!(started.contains("status: background_started"), "{started}");
    let job_id = started
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .expect("job id");
    let status = store.status(job_id, 3000);

    assert!(
        status.contains("Action result: capmgr\nop: job_status"),
        "{status}"
    );
    assert!(status.contains("state: finished"), "{status}");
    assert!(status.contains("action: local_echo"), "{status}");
    assert!(status.contains("background payload ok"), "{status}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn background_job_ids_are_unique_even_when_created_quickly() {
    let first = unique_job_id("tool_job");
    let second = unique_job_id("tool_job");

    assert_ne!(first, second);
    assert!(first.starts_with("tool_job_"));
    assert!(second.starts_with("tool_job_"));
}

#[test]
fn command_tool_background_job_can_be_cancelled() {
    let dir = temp_case_dir("command_tool_cancel");
    let script = dir.join("sleep_payload.sh");
    fs::write(
            &script,
            "#!/bin/sh\npython3 -c 'import time; print(\"started\", flush=True); time.sleep(10); print(\"done\")'\n",
        )
        .unwrap();
    let store = FileToolJobStore::new(&dir);

    let started = store.spawn("local_sleep", &script, &json!({"args":{}}));
    let job_id = started
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .expect("job id");
    let cancelled = store.cancel(job_id);

    assert!(
        cancelled.contains("Action result: capmgr\nop: job_cancel"),
        "{cancelled}"
    );
    assert!(cancelled.contains("state: cancelled"), "{cancelled}");
    let status = store.status(job_id, 0);
    assert!(status.contains("state: cancelled"), "{status}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn session_cancel_is_idempotent_and_does_not_touch_other_sessions() {
    let dir = temp_case_dir("command_tool_session_cancel");
    let script = dir.join("sleep_payload.sh");
    fs::write(
        &script,
        "#!/bin/sh\npython3 -c 'import time; print(\"started\", flush=True); time.sleep(30)'\n",
    )
    .unwrap();
    let store = FileToolJobStore::new(&dir);

    let started_a = store.spawn_outcome("session-a", "local_sleep", &script, &json!({"args":{}}));
    let started_b = store.spawn_outcome("session-b", "local_sleep", &script, &json!({"args":{}}));
    let job_id_a = started_a
        .text
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .unwrap()
        .to_string();
    let job_id_b = started_b
        .text
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .unwrap()
        .to_string();

    assert_eq!(
        store.cancel_unfinished_for_session("session-a"),
        vec![job_id_a.clone()]
    );
    assert!(store.status(&job_id_a, 0).contains("state: cancelled"));
    assert!(store.status(&job_id_b, 0).contains("state: running"));
    assert!(store.cancel_unfinished_for_session("session-a").is_empty());
    assert_eq!(
        store.cancel_unfinished_for_session("session-b"),
        vec![job_id_b.clone()]
    );
    assert!(store.status(&job_id_b, 0).contains("state: cancelled"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn shutdown_terminates_only_background_jobs_owned_by_this_process() {
    let dir = temp_case_dir("command_tool_shutdown");
    let script = dir.join("sleep_payload.sh");
    fs::write(
        &script,
        "#!/bin/sh\npython3 -c 'import time; print(\"started\", flush=True); time.sleep(30)'\n",
    )
    .unwrap();
    let store = FileToolJobStore::new(&dir);

    let started = store.spawn("local_sleep", &script, &json!({"args":{}}));
    let job_id = started
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .expect("job id");
    assert_eq!(store.terminate_owned_running(), 1);
    assert!(store.status(job_id, 0).contains("state: cancelled"));
    assert_eq!(store.terminate_owned_running(), 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn shutdown_does_not_signal_a_job_record_owned_by_another_process() {
    let dir = temp_case_dir("foreign_command_tool_shutdown");
    let store = FileToolJobStore::new(&dir);
    let status_file = dir.join("foreign.status");
    let record = ToolJobRecord {
        id: "foreign-job".to_string(),
        created_at_ms: now_ms(),
        pid: std::process::id(),
        owner_id: Some("foreign-runtime-owner".to_string()),
        session_id: "foreign-session".to_string(),
        action: "foreign".to_string(),
        command_path: "/tmp/foreign".to_string(),
        payload_file: dir.join("foreign.payload").display().to_string(),
        output_file: dir.join("foreign.out").display().to_string(),
        status_file: status_file.display().to_string(),
    };
    store.append(&record).unwrap();

    assert_eq!(store.terminate_owned_running(), 0);
    assert!(!status_file.exists());

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn tool_job_terminate_ignores_missing_pid_without_signalling_broadly() {
    let missing_pid = i32::MAX as u32;
    terminate_process(missing_pid);
    assert_eq!(unsafe { libc::kill(libc::getpid(), 0) }, 0);
}

fn temp_case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "timem_tool_job_{name}_{}_{}",
        std::process::id(),
        now_ms()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

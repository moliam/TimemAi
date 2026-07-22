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

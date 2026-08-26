#![cfg(unix)]

use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn temporary_mem() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "timem-shell-workspace-lock-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

#[test]
fn shell_process_rejects_mem_workspace_owned_by_another_host() {
    let mem = temporary_mem();
    fs::create_dir_all(&mem).unwrap();
    let lock = agent_core::WorkspaceInstanceLock::acquire(&mem, "timem-web-test").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_timem-native-rs"))
        .args(["--space", mem.to_str().unwrap(), "--once-json", "test"])
        .env_remove("TIMEM_DATA_DIR")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("workspace_already_in_use"), "{text}");

    drop(lock);
    fs::remove_dir_all(mem).unwrap();
}

#[test]
fn removed_data_dir_option_is_rejected_before_mem_access() {
    let removed = temporary_mem();
    let output = Command::new(env!("CARGO_BIN_EXE_timem-native-rs"))
        .args(["--data-dir", removed.to_str().unwrap()])
        .env_remove("TIMEM_DATA_DIR")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("unsupported_option:--data-dir"), "{text}");
    assert!(!removed.exists());
}

use std::{
    backtrace::Backtrace,
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    panic::PanicHookInfo,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

const EVENT_LIMIT: usize = 64;
const ERROR_LIMIT: usize = 4096;
const BACKTRACE_LIMIT: usize = 128 * 1024;
const DIAGNOSTICS_DIR: &str = "diagnostics/timem-web";
const CURRENT_RUNS_DIR: &str = "current-runs";
const RUN_ARCHIVE_DIR: &str = "run-archive";
const ARCHIVE_LIMIT: usize = 32;
const LAST_EXIT_FILE: &str = "last-exit.json";
const LAST_PANIC_FILE: &str = "last-panic.txt";
const PREVIOUS_ABNORMAL_FILE: &str = "previous-abnormal-exit.json";

#[derive(Debug, Clone)]
pub(crate) struct LifecycleDiagnostics {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    root: PathBuf,
    current_path: PathBuf,
    run_id: String,
    started_at_ms: u128,
    argument_options: Vec<String>,
    events: Mutex<VecDeque<LifecycleEvent>>,
    finished: Mutex<bool>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct LifecycleEvent {
    at_ms: u128,
    name: String,
    details: serde_json::Value,
}

impl LifecycleDiagnostics {
    pub(crate) fn install_for(memory_root: &Path, args: &[String]) -> Result<Self, String> {
        Self::install_in(memory_root, args, true)
    }

    #[cfg(test)]
    pub(crate) fn install_for_test(memory_root: &Path) -> Result<Self, String> {
        Self::install_in(memory_root, &[], false)
    }

    fn install_in(data_root: &Path, args: &[String], install_hook: bool) -> Result<Self, String> {
        let root = data_root.join(DIAGNOSTICS_DIR);
        create_private_dir(&root)?;
        create_private_dir(&root.join(CURRENT_RUNS_DIR))?;
        create_private_dir(&root.join(RUN_ARCHIVE_DIR))?;
        promote_interrupted_runs(&root)?;
        prune_run_archive(&root)?;

        let started_at_ms = now_ms();
        let run_id = format!("{}-{started_at_ms}-{}", std::process::id(), now_ns());
        let current_path = root.join(CURRENT_RUNS_DIR).join(format!("{run_id}.json"));
        let current = serde_json::json!({
            "schema_version": 1,
            "status": "running",
            "run_id": run_id,
            "pid": std::process::id(),
            "process_identity": agent_core::os::process_identity(std::process::id()),
            "version": env!("CARGO_PKG_VERSION"),
            "started_at_ms": started_at_ms,
            "argument_options": sanitized_option_names(args),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });
        atomic_json_write(&current_path, &current)?;

        let diagnostics = Self {
            inner: Arc::new(Inner {
                root,
                current_path,
                run_id,
                started_at_ms,
                argument_options: sanitized_option_names(args),
                events: Mutex::new(VecDeque::with_capacity(EVENT_LIMIT)),
                finished: Mutex::new(false),
            }),
        };
        diagnostics.event("process_started", serde_json::Value::Null);
        if install_hook {
            install_panic_hook(Arc::clone(&diagnostics.inner));
        }
        Ok(diagnostics)
    }

    pub(crate) fn disabled() -> Self {
        Self {
            inner: Arc::new(Inner {
                root: PathBuf::new(),
                current_path: PathBuf::new(),
                run_id: "diagnostics-disabled".to_string(),
                started_at_ms: now_ms(),
                argument_options: Vec::new(),
                events: Mutex::new(VecDeque::with_capacity(EVENT_LIMIT)),
                finished: Mutex::new(true),
            }),
        }
    }

    pub(crate) fn root(&self) -> Option<&Path> {
        (!self.inner.root.as_os_str().is_empty()).then_some(self.inner.root.as_path())
    }

    pub(crate) fn event(&self, name: impl Into<String>, details: serde_json::Value) {
        if self.inner.root.as_os_str().is_empty() {
            return;
        }
        push_event(&self.inner, name.into(), details);
    }

    pub(crate) fn checkpoint(&self, name: impl Into<String>, details: serde_json::Value) {
        if self.inner.root.as_os_str().is_empty() {
            return;
        }
        push_event(&self.inner, name.into(), details);
        let current = serde_json::json!({
            "schema_version": 1,
            "status": "running",
            "run_id": self.inner.run_id,
            "pid": std::process::id(),
            "process_identity": agent_core::os::process_identity(std::process::id()),
            "version": env!("CARGO_PKG_VERSION"),
            "started_at_ms": self.inner.started_at_ms,
            "argument_options": self.inner.argument_options,
            "last_checkpoint_at_ms": now_ms(),
            "recent_lifecycle_events": snapshot_events(&self.inner),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });
        if let Err(error) = atomic_json_write(&self.inner.current_path, &current) {
            eprintln!("[timem_web_diagnostics_checkpoint_error] {error}");
        }
    }

    pub(crate) fn finish(&self, reason: &str, graceful: bool, error: Option<&str>) {
        if self.inner.root.as_os_str().is_empty() {
            return;
        }
        let mut finished = self
            .inner
            .finished
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *finished {
            return;
        }
        let finished_at_ms = now_ms();
        let events = snapshot_events(&self.inner);
        let record = serde_json::json!({
            "schema_version": 1,
            "status": "exited",
            "run_id": self.inner.run_id,
            "pid": std::process::id(),
            "started_at_ms": self.inner.started_at_ms,
            "finished_at_ms": finished_at_ms,
            "duration_ms": finished_at_ms.saturating_sub(self.inner.started_at_ms),
            "exit_reason": reason,
            "graceful": graceful,
            "error": error.map(redact_and_bound),
            "recent_lifecycle_events": events,
        });
        let archive_path = self
            .inner
            .root
            .join(RUN_ARCHIVE_DIR)
            .join(format!("{}-exit.json", self.inner.run_id));
        if let Err(write_error) = atomic_json_write(&archive_path, &record)
            .and_then(|_| atomic_json_write(&self.inner.root.join(LAST_EXIT_FILE), &record))
        {
            eprintln!("[timem_web_diagnostics_write_error] {write_error}");
            return;
        }
        if let Err(remove_error) = remove_if_exists(&self.inner.current_path) {
            eprintln!("[timem_web_diagnostics_cleanup_error] {remove_error}");
            return;
        }
        *finished = true;
    }
}

fn install_panic_hook(inner: Arc<Inner>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        try_push_panic_event(&inner);
        if let Err(error) = write_panic_report(&inner, info) {
            eprintln!("[timem_web_panic_diagnostics_error] {error}");
        }
        previous(info);
    }));
}

fn try_push_panic_event(inner: &Inner) {
    if let Ok(mut events) = inner.events.try_lock() {
        if events.len() == EVENT_LIMIT {
            events.pop_front();
        }
        events.push_back(LifecycleEvent {
            at_ms: now_ms(),
            name: "panic".to_string(),
            details: serde_json::Value::Null,
        });
    }
}

fn push_event(inner: &Inner, name: String, details: serde_json::Value) {
    let mut events = inner
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if events.len() == EVENT_LIMIT {
        events.pop_front();
    }
    events.push_back(LifecycleEvent {
        at_ms: now_ms(),
        name,
        details,
    });
}

fn snapshot_events(inner: &Inner) -> Vec<LifecycleEvent> {
    inner
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .cloned()
        .collect()
}

fn write_panic_report(inner: &Inner, info: &PanicHookInfo<'_>) -> Result<(), String> {
    let current_thread = std::thread::current();
    let location = info
        .location()
        .map(|value| format!("{}:{}:{}", value.file(), value.line(), value.column()))
        .unwrap_or_else(|| "unknown".to_string());
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload");
    let events = inner
        .events
        .try_lock()
        .ok()
        .and_then(|events| serde_json::to_string_pretty(&*events).ok())
        .unwrap_or_else(|| "[]".to_string());
    let report = render_panic_report(
        &inner.run_id,
        std::process::id(),
        now_ms(),
        current_thread.name().unwrap_or("unnamed"),
        &location,
        message,
        &events,
        &Backtrace::force_capture().to_string(),
    );
    let archive_path = inner
        .root
        .join(RUN_ARCHIVE_DIR)
        .join(format!("{}-panic.txt", inner.run_id));
    atomic_private_write(&archive_path, report.as_bytes())?;
    atomic_private_write(&inner.root.join(LAST_PANIC_FILE), report.as_bytes())
}

#[allow(clippy::too_many_arguments)]
fn render_panic_report(
    run_id: &str,
    pid: u32,
    timestamp_ms: u128,
    thread: &str,
    location: &str,
    message: &str,
    events: &str,
    backtrace: &str,
) -> String {
    format!(
        "TIMEM WEB PANIC REPORT\nschema_version: 1\nrun_id: {run_id}\npid: {pid}\ntimestamp_ms: {timestamp_ms}\nthread: {thread}\nlocation: {location}\nmessage: {}\n\nRECENT LIFECYCLE EVENTS\n{events}\n\nBACKTRACE\n{}\n",
        redact_and_bound(message),
        bound_text(backtrace, BACKTRACE_LIMIT),
    )
}

fn promote_interrupted_runs(root: &Path) -> Result<(), String> {
    let current_root = root.join(CURRENT_RUNS_DIR);
    let entries = fs::read_dir(&current_root)
        .map_err(|error| format!("diagnostics_current_runs_read_failed:{error}"))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("diagnostics_current_run_read_failed:{error}"))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| format!("diagnostics_current_run_type_failed:{error}"))?
            .is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        let parsed = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if parsed.as_ref().is_some_and(marker_owner_may_still_be_alive) {
            continue;
        }
        let mut record = parsed.unwrap_or_else(|| serde_json::json!({ "schema_version": 1 }));
        if let Some(object) = record.as_object_mut() {
            object.insert(
                "status".to_string(),
                serde_json::Value::String("abnormal_exit_detected_on_next_start".to_string()),
            );
            object.insert("detected_at_ms".to_string(), serde_json::json!(now_ms()));
            object.insert(
                "exact_cause".to_string(),
                serde_json::Value::String("unknown".to_string()),
            );
        }
        let run_id = record
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("unknown-{}", now_ms()));
        let archive = root
            .join(RUN_ARCHIVE_DIR)
            .join(format!("{run_id}-abnormal.json"));
        atomic_json_write(&archive, &record)?;
        atomic_json_write(&root.join(PREVIOUS_ABNORMAL_FILE), &record)?;
        remove_if_exists(&path)?;
    }
    Ok(())
}

fn marker_owner_may_still_be_alive(record: &serde_json::Value) -> bool {
    let Some(pid) = record
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
    else {
        return false;
    };
    if agent_core::os::process_is_definitely_dead(pid) {
        return false;
    }
    match (
        record
            .get("process_identity")
            .and_then(serde_json::Value::as_str),
        agent_core::os::process_identity(pid),
    ) {
        (Some(recorded), Some(current)) => recorded == current,
        _ => agent_core::os::process_may_be_alive(pid),
    }
}

fn prune_run_archive(root: &Path) -> Result<(), String> {
    let archive_root = root.join(RUN_ARCHIVE_DIR);
    let mut entries = fs::read_dir(&archive_root)
        .map_err(|error| format!("diagnostics_archive_read_failed:{error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH)
    });
    let remove_count = entries.len().saturating_sub(ARCHIVE_LIMIT);
    for entry in entries.into_iter().take(remove_count) {
        remove_if_exists(&entry.path())?;
    }
    Ok(())
}

pub(crate) fn memory_root_from_args(args: &[String]) -> Result<PathBuf, String> {
    if std::env::var_os("TIMEM_DATA_DIR").is_some() {
        return Err("unsupported_env:TIMEM_DATA_DIR; MEM is the complete workspace".to_string());
    }
    for (index, argument) in args.iter().enumerate() {
        if argument == "--data-dir" || argument.starts_with("--data-dir=") {
            return Err("unsupported_option:--data-dir; MEM is the complete workspace".to_string());
        }
        if argument == "--space" {
            return agent_core::resolve_memory_dir(args.get(index + 1).map(String::as_str));
        }
        if let Some(value) = argument.strip_prefix("--space=") {
            return agent_core::resolve_memory_dir(Some(value));
        }
    }
    agent_core::resolve_memory_dir(std::env::var("TIMEM_SPACE").ok().as_deref())
}

fn sanitized_option_names(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|argument| argument.starts_with('-'))
        .map(|argument| argument.split('=').next().unwrap_or(argument).to_string())
        .collect()
}

fn redact_and_bound(text: &str) -> String {
    let mut text = bound_text(text, ERROR_LIMIT);
    for marker in ["Bearer ", "api_key=", "api-key=", "sk-"] {
        while let Some(start) = text.find(marker) {
            let value_start = start + marker.len();
            let end = text[value_start..]
                .find(char::is_whitespace)
                .map(|offset| value_start + offset)
                .unwrap_or(text.len());
            text.replace_range(start..end, "[REDACTED]");
        }
    }
    text
}

fn bound_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("diagnostics_dir_create_failed:{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("diagnostics_dir_permissions_failed:{error}"))?;
    }
    Ok(())
}

fn atomic_json_write(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("diagnostics_serialize_failed:{error}"))?;
    atomic_private_write(path, &bytes)
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("tmp-{}-{}", std::process::id(), now_ms()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("diagnostics_file_open_failed:{error}"))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("diagnostics_file_write_failed:{error}"));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("diagnostics_file_replace_failed:{error}"));
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("diagnostics_file_remove_failed:{error}")),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
#[path = "../tests/unit/lifecycle_diagnostics_tests.rs"]
mod tests;

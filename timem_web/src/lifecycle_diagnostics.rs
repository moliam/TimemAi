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
const CURRENT_RUN_FILE: &str = "current-run.json";
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
    pub(crate) fn install_from_env() -> Result<Self, String> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        Self::install_in(&data_root_from_args(&args), &args, true)
    }

    fn install_in(data_root: &Path, args: &[String], install_hook: bool) -> Result<Self, String> {
        let root = data_root.join(DIAGNOSTICS_DIR);
        create_private_dir(&root)?;
        promote_interrupted_run(&root)?;

        let started_at_ms = now_ms();
        let run_id = format!("{}-{started_at_ms}", std::process::id());
        let current = serde_json::json!({
            "schema_version": 1,
            "status": "running",
            "run_id": run_id,
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"),
            "started_at_ms": started_at_ms,
            "argument_options": sanitized_option_names(args),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });
        atomic_json_write(&root.join(CURRENT_RUN_FILE), &current)?;

        let diagnostics = Self {
            inner: Arc::new(Inner {
                root,
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
            "version": env!("CARGO_PKG_VERSION"),
            "started_at_ms": self.inner.started_at_ms,
            "argument_options": self.inner.argument_options,
            "last_checkpoint_at_ms": now_ms(),
            "recent_lifecycle_events": snapshot_events(&self.inner),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });
        if let Err(error) = atomic_json_write(&self.inner.root.join(CURRENT_RUN_FILE), &current) {
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
        if let Err(write_error) = atomic_json_write(&self.inner.root.join(LAST_EXIT_FILE), &record)
        {
            eprintln!("[timem_web_diagnostics_write_error] {write_error}");
            return;
        }
        if let Err(remove_error) = remove_if_exists(&self.inner.root.join(CURRENT_RUN_FILE)) {
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

fn promote_interrupted_run(root: &Path) -> Result<(), String> {
    let current_path = root.join(CURRENT_RUN_FILE);
    if !current_path.is_file() {
        return Ok(());
    }
    let mut record = fs::read(&current_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({ "schema_version": 1 }));
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
    atomic_json_write(&root.join(PREVIOUS_ABNORMAL_FILE), &record)?;
    remove_if_exists(&current_path)
}

fn data_root_from_args(args: &[String]) -> PathBuf {
    for (index, argument) in args.iter().enumerate() {
        if argument == "--data-dir" {
            if let Some(value) = args.get(index + 1) {
                return PathBuf::from(value);
            }
        } else if let Some(value) = argument.strip_prefix("--data-dir=") {
            return PathBuf::from(value);
        }
    }
    agent_core::default_data_root()
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

#[cfg(test)]
#[path = "../tests/unit/lifecycle_diagnostics_tests.rs"]
mod tests;

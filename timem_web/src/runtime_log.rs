use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(crate) const MAX_RUNTIME_LOG_BYTES: u64 = 20 * 1024 * 1024;
#[derive(Clone, Default)]
pub(crate) struct RuntimeLog {
    inner: Option<Arc<LogWriter>>,
}

struct LogWriter {
    path: PathBuf,
    max_bytes: u64,
    write_lock: Mutex<()>,
    _diagnostic_root: Option<Arc<crate::debug_session::TemporaryDebugRoot>>,
}

impl RuntimeLog {
    pub(crate) fn with_diagnostic_root(
        root: Arc<crate::debug_session::TemporaryDebugRoot>,
    ) -> Self {
        Self::with_path_limit_and_owner(
            root.path().join("runtime.log"),
            MAX_RUNTIME_LOG_BYTES,
            Some(root),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_path_and_limit(path: PathBuf, max_bytes: u64) -> Self {
        Self::with_path_limit_and_owner(path, max_bytes, None)
    }

    fn with_path_limit_and_owner(
        path: PathBuf,
        max_bytes: u64,
        diagnostic_root: Option<Arc<crate::debug_session::TemporaryDebugRoot>>,
    ) -> Self {
        Self {
            inner: Some(Arc::new(LogWriter {
                path,
                max_bytes,
                write_lock: Mutex::new(()),
                _diagnostic_root: diagnostic_root,
            })),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.inner.as_ref().map(|inner| inner.path.as_path())
    }

    pub(crate) fn record(&self, stage: &str, fields: Value) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        inner.record(stage, fields);
    }

    pub(crate) fn record_client(
        &self,
        stage: &str,
        session_id: String,
        command_id: String,
        turn_id: Option<String>,
        elapsed_ms: Option<f64>,
        event_count: Option<usize>,
    ) -> Result<(), &'static str> {
        let invalid_id = |value: &str| {
            value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
        };
        if !is_allowed_client_stage(stage)
            || invalid_id(&session_id)
            || invalid_id(&command_id)
            || turn_id.as_deref().is_some_and(invalid_id)
            || elapsed_ms
                .is_some_and(|value| !value.is_finite() || !(0.0..=86_400_000.0).contains(&value))
            || event_count.is_some_and(|value| value > 1_000_000)
        {
            return Err("invalid_performance_trace");
        }
        self.record(
            stage,
            json!({
                "session_id": session_id,
                "command_id": command_id,
                "turn_id": turn_id,
                "elapsed_ms": elapsed_ms,
                "event_count": event_count,
            }),
        );
        Ok(())
    }
}

impl LogWriter {
    fn record(&self, stage: &str, fields: Value) {
        let Ok(_guard) = self.write_lock.lock() else {
            return;
        };
        let Some(parent) = self.path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let Some(encoded) = encode_record(stage, fields) else {
            return;
        };
        if encoded.len() as u64 > self.max_bytes {
            return;
        }
        let current_bytes = fs::metadata(&self.path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let wrapped = current_bytes.saturating_add(encoded.len() as u64) > self.max_bytes;
        let mut options = OpenOptions::new();
        options.create(true).write(true);
        if wrapped {
            options.truncate(true);
        } else {
            options.append(true);
        }
        let Ok(mut file) = options.open(&self.path) else {
            return;
        };
        if wrapped {
            let marker = encode_record("log_wrapped", json!({ "max_bytes": self.max_bytes }));
            if let Some(marker) = marker.filter(|marker| {
                marker.len().saturating_add(encoded.len()) as u64 <= self.max_bytes
            }) {
                let _ = file.write_all(&marker);
            }
        }
        let _ = file.write_all(&encoded);
    }
}

fn encode_record(stage: &str, fields: Value) -> Option<Vec<u8>> {
    let mut encoded = serde_json::to_vec(&json!({
        "timestamp_ms": now_ms(),
        "pid": std::process::id(),
        "stage": stage,
        "fields": fields,
    }))
    .ok()?;
    encoded.push(b'\n');
    Some(encoded)
}

fn is_allowed_client_stage(stage: &str) -> bool {
    matches!(
        stage,
        "browser_send"
            | "browser_turn_updated"
            | "browser_painted"
            | "browser_session_selected"
            | "browser_session_painted"
    )
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
#[path = "../tests/unit/runtime_log_tests.rs"]
mod tests;

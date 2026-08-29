use crate::{
    atomic_write_file, bash_approval_mode_label,
    rolling_file_store::{
        append_rolling_record, migrate_legacy_file, read_segmented_records, rolling_segments,
        segment_metadata_path, segmented_directory, trim_rolling_segments, RollingCapacity,
        AUDIT_ROLLING_SLICE_BYTES,
    },
    ApiProtocol, ApprovalRequest, BashApprovalMode, MemGuard, TurnStopSummary, UsageStats,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(test)]
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
};

pub const DEFAULT_AUDIT_DIRECTORY_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const DEBUG_AUDIT_DIRECTORY_MAX_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const ACTION_AUDIT_MAX_BYTES: u64 = AUDIT_ROLLING_SLICE_BYTES;
const API_AUDIT_BASE_MAX_BYTES: u64 = AUDIT_ROLLING_SLICE_BYTES;
const REPAIR_OUTPUT_MAX_BYTES: u64 = AUDIT_ROLLING_SLICE_BYTES;
static AUDIT_DIRECTORY_MAX_BYTES: AtomicU64 = AtomicU64::new(DEFAULT_AUDIT_DIRECTORY_MAX_BYTES);

pub fn configure_audit_storage(debug: bool) {
    AUDIT_DIRECTORY_MAX_BYTES.store(
        if debug {
            DEBUG_AUDIT_DIRECTORY_MAX_BYTES
        } else {
            DEFAULT_AUDIT_DIRECTORY_MAX_BYTES
        },
        Ordering::Release,
    );
}

fn audit_directory_max_bytes() -> u64 {
    AUDIT_DIRECTORY_MAX_BYTES.load(Ordering::Acquire)
}

fn audit_stable_max_bytes() -> u64 {
    audit_directory_max_bytes().saturating_sub(AUDIT_ROLLING_SLICE_BYTES)
}

fn api_audit_jsonl_max_bytes() -> u64 {
    audit_directory_max_bytes().saturating_sub(AUDIT_ROLLING_SLICE_BYTES.saturating_mul(3))
}
const AUDIT_EVENT_MAX_BYTES: usize = AUDIT_ROLLING_SLICE_BYTES as usize - 1;
#[cfg(test)]
const AUDIT_COPY_BUFFER_BYTES: usize = 64 * 1024;
const REPAIR_OUTPUT_RESPONSE_LIMIT_CHARS: usize = 12_000;

pub fn append_audit_event(path: &Path, event: &Value) -> std::io::Result<()> {
    MemGuard::for_audit_file(path)
        .with_write(|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let now_ms = audit_now_ms();
            let event = bounded_audit_event(timestamp_audit_event(event, now_ms))?;
            if !path.exists() {
                let text = serde_json::to_string_pretty(&empty_audit_doc())
                    .map_err(std::io::Error::other)?;
                atomic_write_file(path, format!("{text}\n").as_bytes())?;
            }
            append_audit_jsonl(&audit_sidecar_path(path), &event)
        })
        .map_err(std::io::Error::other)?
}

pub fn append_repair_output_event(api_audit_file: &Path, event: &Value) -> std::io::Result<()> {
    let repair_file = repair_output_file_for_api_audit(api_audit_file);
    MemGuard::for_audit_file(&repair_file)
        .with_write(|| {
            if let Some(parent) = repair_file.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut doc = read_repair_output_doc(&repair_file)?;
            doc["records"]
                .as_array_mut()
                .expect("repair output doc records must be an array")
                .push(event.clone());
            doc["updated_at_ms"] = json!(audit_now_ms());
            shrink_repair_output_doc(&mut doc)?;
            let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
            atomic_write_file(&repair_file, format!("{text}\n").as_bytes())
        })
        .map_err(std::io::Error::other)??;
    Ok(())
}

fn bounded_audit_event(event: Value) -> std::io::Result<Value> {
    let encoded = serde_json::to_vec(&event).map_err(std::io::Error::other)?;
    if encoded.len() <= AUDIT_EVENT_MAX_BYTES {
        return Ok(event);
    }
    let mut summary = serde_json::Map::new();
    if let Some(object) = event.as_object() {
        for key in [
            "type",
            "time_ms",
            "session",
            "turn_id",
            "model",
            "api_protocol",
            "endpoint",
            "status",
            "error_kind",
        ] {
            if let Some(value) = object.get(key) {
                summary.insert(key.to_string(), value.clone());
            }
        }
    }
    summary.insert("payload_omitted".into(), json!(true));
    summary.insert("payload_bytes".into(), json!(encoded.len()));
    summary.insert("payload_limit_bytes".into(), json!(AUDIT_EVENT_MAX_BYTES));
    Ok(Value::Object(summary))
}

fn shrink_repair_output_doc(doc: &mut Value) -> std::io::Result<()> {
    loop {
        let encoded_len = serde_json::to_vec(doc)
            .map_err(std::io::Error::other)?
            .len() as u64;
        if encoded_len <= REPAIR_OUTPUT_MAX_BYTES {
            return Ok(());
        }
        let Some(records) = doc.get_mut("records").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        if records.is_empty() {
            return Ok(());
        }
        records.remove(0);
    }
}

pub fn enforce_audit_storage_budget(path: &Path) -> std::io::Result<()> {
    MemGuard::for_audit_file(path)
        .with_write(|| enforce_audit_storage_budget_unlocked(path))
        .map_err(std::io::Error::other)?
}

fn enforce_audit_storage_budget_unlocked(path: &Path) -> std::io::Result<()> {
    let Some(audit_dir) = path.parent() else {
        return Ok(());
    };
    cleanup_stale_audit_temps(audit_dir)?;
    if path.exists() {
        compact_json_array_document(path, "events", API_AUDIT_BASE_MAX_BYTES)?;
    }
    let sidecar = audit_sidecar_path(path);
    if sidecar.exists() || segmented_directory(&sidecar).exists() {
        trim_rolling_segments(
            &sidecar,
            api_audit_jsonl_max_bytes(),
            AUDIT_ROLLING_SLICE_BYTES,
        )?;
    }
    let legacy_sidecar = audit_dir
        .parent()
        .map(|space_dir| space_dir.join("api_audit.jsonl"));
    if let Some(legacy) = legacy_sidecar
        .as_ref()
        .filter(|legacy| legacy.exists() || segmented_directory(legacy).exists())
    {
        trim_rolling_segments(
            legacy,
            api_audit_jsonl_max_bytes(),
            AUDIT_ROLLING_SLICE_BYTES,
        )?;
    }

    // Fixed per-file budgets leave room for metadata. If an old layout and the
    // current sidecar coexist, discard oldest legacy bytes before current bytes.
    let mut total = audit_storage_bytes(audit_dir, legacy_sidecar.as_deref())?;
    for candidate in [legacy_sidecar.as_deref(), Some(sidecar.as_path())]
        .into_iter()
        .flatten()
    {
        if total <= audit_stable_max_bytes()
            || (!candidate.exists() && !segmented_directory(candidate).exists())
        {
            continue;
        }
        let record_bytes = rolling_record_bytes(candidate)?;
        let excess = total.saturating_sub(audit_stable_max_bytes());
        trim_rolling_segments(
            candidate,
            record_bytes.saturating_sub(excess),
            AUDIT_ROLLING_SLICE_BYTES,
        )?;
        total = audit_storage_bytes(audit_dir, legacy_sidecar.as_deref())?;
    }
    if total > audit_stable_max_bytes() || total > audit_directory_max_bytes() {
        return Err(std::io::Error::other("audit_directory_budget_exceeded"));
    }
    Ok(())
}

fn cleanup_stale_audit_temps(audit_dir: &Path) -> std::io::Result<()> {
    let Ok(entries) = fs::read_dir(audit_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            && (name.contains(".retention.tmp-") || name.contains(".audit-compact.tmp-"))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

fn rolling_path_bytes(path: &Path) -> std::io::Result<u64> {
    let mut total = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let mut directories = vec![segmented_directory(path)];
    if path
        .file_name()
        .is_some_and(|name| name == "action_audit.json")
    {
        directories.push(path.with_file_name("action_audit.json.turns"));
    }
    for directory in directories {
        match fs::read_dir(directory) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_file() {
                            total = total.saturating_add(metadata.len());
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(total)
}

fn rolling_record_bytes(path: &Path) -> std::io::Result<u64> {
    if segmented_directory(path).exists() {
        return Ok(rolling_segments(path)?
            .iter()
            .map(|segment| segment.bytes)
            .sum());
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Ok(_) => Ok(0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

fn audit_storage_bytes(audit_dir: &Path, legacy_sidecar: Option<&Path>) -> std::io::Result<u64> {
    let mut total = 0u64;
    for path in [
        audit_dir.join("api_audit.json"),
        audit_dir.join("api_audit.jsonl"),
        audit_dir.join("action_audit.json"),
        audit_dir.join("api_output_repair.json"),
    ]
    .into_iter()
    .chain(legacy_sidecar.map(Path::to_path_buf))
    {
        total = total.saturating_add(rolling_path_bytes(&path)?);
    }
    Ok(total)
}

fn compact_json_array_document(
    path: &Path,
    array_key: &str,
    max_bytes: u64,
) -> std::io::Result<()> {
    if fs::metadata(path)?.len() <= max_bytes {
        return Ok(());
    }
    let bytes = fs::read(path)?;
    let mut doc = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(_) if array_key == "events" => read_audit_doc_single(path)?,
        Err(_) => return Ok(()),
    };
    loop {
        let encoded = serde_json::to_vec_pretty(&doc).map_err(std::io::Error::other)?;
        if encoded.len() as u64 <= max_bytes {
            let mut bytes = encoded;
            bytes.push(b'\n');
            return atomic_write_file(path, &bytes);
        }
        let Some(records) = doc.get_mut(array_key).and_then(Value::as_array_mut) else {
            return Ok(());
        };
        if records.is_empty() {
            let mut bytes = serde_json::to_vec_pretty(&doc).map_err(std::io::Error::other)?;
            bytes.push(b'\n');
            return atomic_write_file(path, &bytes);
        }
        let remove_count = (records.len() / 8).max(1);
        records.drain(..remove_count.min(records.len()));
    }
}

#[cfg(test)]
fn compact_jsonl_tail_in_place(path: &Path, max_bytes: u64) -> std::io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_len = file.metadata()?.len();
    if file_len <= max_bytes {
        return Ok(());
    }
    if max_bytes == 0 {
        file.set_len(0)?;
        return file.sync_all();
    }

    let approximate_start = file_len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(approximate_start))?;
    let mut reader = BufReader::new(file.try_clone()?);
    let mut discarded = Vec::new();
    reader.read_until(b'\n', &mut discarded)?;
    let source_start = reader.stream_position()?;
    if source_start >= file_len {
        file.set_len(0)?;
        return file.sync_all();
    }

    let mut buffer = vec![0u8; AUDIT_COPY_BUFFER_BYTES];
    let mut read_offset = source_start;
    let mut write_offset = 0u64;
    while read_offset < file_len {
        let wanted = buffer.len().min((file_len - read_offset) as usize);
        file.seek(SeekFrom::Start(read_offset))?;
        file.read_exact(&mut buffer[..wanted])?;
        file.seek(SeekFrom::Start(write_offset))?;
        file.write_all(&buffer[..wanted])?;
        read_offset += wanted as u64;
        write_offset += wanted as u64;
    }
    file.set_len(write_offset)?;
    file.sync_all()
}

fn timestamp_audit_event(event: &Value, now_ms: i64) -> Value {
    let mut event = event.clone();
    if let Some(object) = event.as_object_mut() {
        let valid_time = object
            .get("time_ms")
            .and_then(Value::as_i64)
            .is_some_and(|time_ms| time_ms <= now_ms);
        if !valid_time {
            object.insert("time_ms".into(), json!(now_ms));
        }
        return event;
    }
    json!({"time_ms": now_ms, "event": event})
}

/// Removes API-audit events older than `cutoff_ms` from the JSON document and
/// its JSONL sidecar. Events at the cutoff are retained.
pub fn prune_api_audit_before(path: &Path, cutoff_ms: i64, now_ms: i64) -> std::io::Result<usize> {
    MemGuard::for_audit_file(path)
        .with_write(|| prune_api_audit_before_unlocked(path, cutoff_ms, now_ms))
        .map_err(std::io::Error::other)?
}

fn prune_api_audit_before_unlocked(
    path: &Path,
    cutoff_ms: i64,
    now_ms: i64,
) -> std::io::Result<usize> {
    let mut removed = 0usize;
    if path.exists() {
        let mut doc = read_audit_doc_single(path)?;
        let events = doc["events"]
            .as_array_mut()
            .expect("audit doc events must be an array");
        let before = events.len();
        events.retain(|event| retained_audit_time_ms(event, cutoff_ms, now_ms).is_some());
        removed = removed.saturating_add(before.saturating_sub(events.len()));
        let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
        atomic_write_file(path, format!("{text}\n").as_bytes())?;
    }
    let sidecar = audit_sidecar_path(path);
    if sidecar != path && (sidecar.exists() || segmented_directory(&sidecar).exists()) {
        removed = removed.saturating_add(prune_audit_jsonl(&sidecar, cutoff_ms, now_ms)?);
    }
    Ok(removed)
}

fn retained_audit_time_ms(event: &Value, cutoff_ms: i64, now_ms: i64) -> Option<i64> {
    let time_ms = event.get("time_ms").and_then(Value::as_i64)?;
    if !(cutoff_ms..=now_ms).contains(&time_ms) {
        return None;
    }
    Some(time_ms)
}

fn audit_record_time(record: &[u8]) -> Option<i64> {
    serde_json::from_slice::<Value>(record)
        .ok()?
        .get("time_ms")?
        .as_i64()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct AuditSegmentSummary {
    bytes: u64,
    records: usize,
    min_time_ms: i64,
    max_time_ms: i64,
}

fn summarize_audit_segment(path: &Path) -> std::io::Result<Option<AuditSegmentSummary>> {
    let bytes = fs::metadata(path)?.len();
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut records = 0usize;
    let mut min_time_ms = i64::MAX;
    let mut max_time_ms = i64::MIN;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let Some(time_ms) = audit_record_time(&line) else {
            return Ok(None);
        };
        records = records.saturating_add(1);
        min_time_ms = min_time_ms.min(time_ms);
        max_time_ms = max_time_ms.max(time_ms);
    }
    if records == 0 {
        return Ok(None);
    }
    Ok(Some(AuditSegmentSummary {
        bytes,
        records,
        min_time_ms,
        max_time_ms,
    }))
}

fn write_audit_segment_summary(path: &Path, summary: AuditSegmentSummary) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(&summary).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    atomic_write_file(&segment_metadata_path(path), &bytes)
}

fn audit_segment_summary(path: &Path) -> std::io::Result<Option<AuditSegmentSummary>> {
    let current_bytes = fs::metadata(path)?.len();
    let metadata_path = segment_metadata_path(path);
    if let Ok(bytes) = fs::read(&metadata_path) {
        if let Ok(summary) = serde_json::from_slice::<AuditSegmentSummary>(&bytes) {
            if summary.bytes == current_bytes {
                return Ok(Some(summary));
            }
        }
    }
    let summary = summarize_audit_segment(path)?;
    if let Some(summary) = summary {
        write_audit_segment_summary(path, summary)?;
    }
    Ok(summary)
}

fn update_active_audit_segment_summary(
    path: &Path,
    event_time_ms: i64,
    record_bytes: u64,
) -> std::io::Result<()> {
    let Some(segment) = rolling_segments(path)?.last().cloned() else {
        return Ok(());
    };
    let metadata_path = segment_metadata_path(&segment.path);
    let previous = fs::read(&metadata_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AuditSegmentSummary>(&bytes).ok());
    let summary = match previous {
        Some(previous) if previous.bytes.saturating_add(record_bytes) == segment.bytes => {
            AuditSegmentSummary {
                bytes: segment.bytes,
                records: previous.records.saturating_add(1),
                min_time_ms: previous.min_time_ms.min(event_time_ms),
                max_time_ms: previous.max_time_ms.max(event_time_ms),
            }
        }
        None if segment.bytes == record_bytes => AuditSegmentSummary {
            bytes: segment.bytes,
            records: 1,
            min_time_ms: event_time_ms,
            max_time_ms: event_time_ms,
        },
        _ => match summarize_audit_segment(&segment.path)? {
            Some(summary) => summary,
            None => return Ok(()),
        },
    };
    write_audit_segment_summary(&segment.path, summary)
}

fn remove_audit_segment(path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)?;
    match fs::remove_file(segment_metadata_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn prune_audit_segment(path: &Path, cutoff_ms: i64, now_ms: i64) -> std::io::Result<usize> {
    let input = fs::File::open(path)?;
    let mut reader = BufReader::new(input);
    let mut retained = Vec::new();
    let mut removed = 0usize;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let keep = serde_json::from_slice::<Value>(&line)
            .ok()
            .and_then(|event| retained_audit_time_ms(&event, cutoff_ms, now_ms))
            .is_some();
        if keep {
            retained.extend_from_slice(&line);
        } else {
            removed = removed.saturating_add(1);
        }
    }
    if removed == 0 {
        return Ok(0);
    }
    if retained.is_empty() {
        remove_audit_segment(path)?;
    } else {
        atomic_write_file(path, &retained)?;
        match summarize_audit_segment(path)? {
            Some(summary) => write_audit_segment_summary(path, summary)?,
            None => {
                let _ = fs::remove_file(segment_metadata_path(path));
            }
        }
    }
    Ok(removed)
}

fn prune_audit_jsonl(path: &Path, cutoff_ms: i64, now_ms: i64) -> std::io::Result<usize> {
    migrate_legacy_file(path, AUDIT_ROLLING_SLICE_BYTES)?;
    let mut removed = 0usize;
    for segment in rolling_segments(path)? {
        match audit_segment_summary(&segment.path)? {
            Some(summary) if summary.max_time_ms < cutoff_ms || summary.min_time_ms > now_ms => {
                removed = removed.saturating_add(summary.records);
                remove_audit_segment(&segment.path)?;
            }
            Some(summary) if summary.min_time_ms >= cutoff_ms && summary.max_time_ms <= now_ms => {}
            _ => {
                removed =
                    removed.saturating_add(prune_audit_segment(&segment.path, cutoff_ms, now_ms)?);
            }
        }
    }
    Ok(removed)
}

fn repair_output_file_for_api_audit(api_audit_file: &Path) -> std::path::PathBuf {
    api_audit_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("api_output_repair.json")
}

fn read_repair_output_doc(path: &Path) -> std::io::Result<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(empty_repair_output_doc());
    };
    if text.trim().is_empty() {
        return Ok(empty_repair_output_doc());
    }
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return Ok(empty_repair_output_doc());
    };
    if value.get("records").and_then(Value::as_array).is_none() {
        value["records"] = json!([]);
    }
    if value.get("version").is_none() {
        value["version"] = json!(1);
    }
    Ok(value)
}

fn empty_repair_output_doc() -> Value {
    json!({
        "version": 1,
        "kind": "timem_realtime_repair_output_log",
        "notes": [
            "Realtime model-output protocol repair diagnostics.",
            "Each record includes the malformed assistant response and the RUNTIME repair message shown to the model.",
            "assistant_response may be capped to avoid unbounded diagnostic growth."
        ],
        "records": []
    })
}

pub fn read_audit_doc(path: &Path) -> std::io::Result<Value> {
    let mut doc = read_audit_doc_single(path)?;
    let sidecar = audit_sidecar_path(path);
    if sidecar != path {
        let sidecar_doc = read_audit_doc_single(&sidecar)?;
        if let (Some(base), Some(extra)) = (
            doc["events"].as_array_mut(),
            sidecar_doc["events"].as_array(),
        ) {
            base.extend(extra.iter().cloned());
        }
    }
    Ok(doc)
}

fn read_audit_doc_single(path: &Path) -> std::io::Result<Value> {
    if segmented_directory(path).exists() {
        let events = read_segmented_records(path)?
            .into_iter()
            .filter_map(|record| serde_json::from_slice::<Value>(&record).ok())
            .collect::<Vec<_>>();
        return Ok(json!({"version": 1, "events": events}));
    }
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(empty_audit_doc());
    };
    if text.trim().is_empty() {
        return Ok(empty_audit_doc());
    }
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if value.get("events").and_then(Value::as_array).is_some() {
            return Ok(value);
        }
    }
    let events = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    Ok(json!({"version": 1, "events": events}))
}

fn append_audit_jsonl(path: &Path, event: &Value) -> std::io::Result<()> {
    let mut record = serde_json::to_vec(event).map_err(std::io::Error::other)?;
    record.push(b'\n');
    let capacity = RollingCapacity::with_slice_bytes(
        api_audit_jsonl_max_bytes().saturating_add(AUDIT_ROLLING_SLICE_BYTES),
        AUDIT_ROLLING_SLICE_BYTES,
    )
    .map_err(std::io::Error::other)?;
    append_rolling_record(path, &record, capacity, AUDIT_ROLLING_SLICE_BYTES)?;
    let event_time_ms = event
        .get("time_ms")
        .and_then(Value::as_i64)
        .ok_or_else(|| std::io::Error::other("audit_event_time_missing"))?;
    update_active_audit_segment_summary(path, event_time_ms, record.len() as u64)
}

fn audit_sidecar_path(path: &Path) -> std::path::PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.with_extension("jsonl");
    };
    path.with_file_name(format!("{file_name}l"))
}

fn empty_audit_doc() -> Value {
    json!({"version": 1, "events": []})
}

#[allow(clippy::too_many_arguments)]
pub fn host_start_audit_event(
    host: &str,
    session: &str,
    space: &str,
    base_url: &str,
    api_protocol: &ApiProtocol,
    model: &str,
    max_llm_input_tokens: u32,
    bash_approval: BashApprovalMode,
) -> Value {
    json!({
        "type": format!("{host}_start"),
        "session": session,
        "space": space,
        "base_url": base_url,
        "api_protocol": api_protocol.label(),
        "model": model,
        "max_llm_input_tokens": max_llm_input_tokens,
        "bash_approval": bash_approval_mode_label(bash_approval),
    })
}

pub fn turn_start_audit_event(session: &str, turn_id: &str, user_input: &str) -> Value {
    json!({
        "type": "turn_start",
        "session": session,
        "turn_id": turn_id,
        "user_input": user_input,
    })
}

pub fn user_supplement_audit_event(session: &str, turn_id: &str, text: &str) -> Value {
    json!({
        "type": "user_supplement",
        "session": session,
        "turn_id": turn_id,
        "text": text,
    })
}

pub fn max_llm_output_increased_audit_event(
    session: &str,
    turn_id: &str,
    max_llm_output_tokens: u32,
) -> Value {
    json!({
        "type": "max_llm_output_increased",
        "session": session,
        "turn_id": turn_id,
        "max_llm_output_tokens": max_llm_output_tokens,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn model_repair_request_audit_event(
    session: &str,
    turn_id: &str,
    issue: Option<&str>,
    model: &str,
    usage: &UsageStats,
    truncated: bool,
    repair_calls: u32,
    repair_calls_delta: u32,
) -> Value {
    json!({
        "type": "model_repair_request",
        "session": session,
        "turn_id": turn_id,
        "issue": issue,
        "model": model,
        "usage": usage,
        "truncated": truncated,
        "repair_calls": repair_calls,
        "repair_calls_delta": repair_calls_delta,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn model_repair_output_event(
    session: &str,
    turn_id: &str,
    issue: Option<&str>,
    assistant_name: &str,
    assistant_response: &str,
    system_message: &str,
    model: &str,
    usage: &UsageStats,
    truncated: bool,
    repair_calls: u32,
    repair_calls_delta: u32,
    spec: &crate::response_protocol::PromptBoundarySpec,
) -> Value {
    let (assistant_response, capped) =
        cap_repair_output_text(assistant_response, REPAIR_OUTPUT_RESPONSE_LIMIT_CHARS);
    let time_ms = audit_now_ms();
    let issue_text = issue.unwrap_or("unknown_repair_issue");
    json!({
        "kind": "model_output_repair",
        "time_ms": time_ms,
        "session": session,
        "turn_id": turn_id,
        "issue": issue,
        "assistant_name": assistant_name,
        "assistant_response": assistant_response,
        "assistant_response_capped": capped,
        "system_message": system_message,
        "model": model,
        "usage": usage,
        "truncated": truncated,
        "repair_calls": repair_calls,
        "repair_calls_delta": repair_calls_delta,
        "rendered": format!(
            "---- {} / {} ----\n## {}:\n{}\n\n## {}\n{}",
            time_ms, turn_id, spec.assistant_role, assistant_response, spec.runtime_role, system_message
        ),
        "summary": format!("{} repair for {}", issue_text, turn_id),
    })
}

fn cap_repair_output_text(text: &str, limit: usize) -> (String, bool) {
    if text.chars().count() <= limit {
        return (text.to_string(), false);
    }
    let head_count = limit / 2;
    let tail_count = limit.saturating_sub(head_count);
    let head = text.chars().take(head_count).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    (
        format!(
            "{head}\n[TRUNCATED repair output: omitted middle chars; original_chars={}]\n{tail}",
            text.chars().count()
        ),
        true,
    )
}

fn audit_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn turn_error_audit_event(session: &str, turn_id: &str, error: &str) -> Value {
    json!({
        "type": "turn_error",
        "session": session,
        "turn_id": turn_id,
        "error": error,
    })
}

pub fn user_approval_audit_event(
    session: &str,
    turn_id: &str,
    approval: &ApprovalRequest,
    approved: bool,
) -> Value {
    json!({
        "type": "user_approval",
        "session": session,
        "turn_id": turn_id,
        "approval_id": approval.approval_id,
        "action": approval.action,
        "command": approval.command,
        "risk": approval.risk,
        "reason": approval.reason,
        "approved": approved,
    })
}

pub fn round_limit_audit_event(
    session: &str,
    turn_id: &str,
    max_rounds: u32,
    continued: bool,
) -> Value {
    json!({
        "type": "round_limit",
        "session": session,
        "turn_id": turn_id,
        "max_rounds": max_rounds,
        "continued": continued,
    })
}

pub fn stale_context_choice_audit_event(
    session: &str,
    idle: Duration,
    dynamic_context_tokens: u32,
    continue_old_context: bool,
) -> Value {
    json!({
        "type": "stale_context_choice",
        "session": session,
        "idle_secs": idle.as_secs(),
        "dynamic_context_tokens": dynamic_context_tokens,
        "continue_old_context": continue_old_context,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn turn_final_audit_event(
    session: &str,
    turn_id: &str,
    assistant_output: &str,
    stats: &UsageStats,
    latest_usage: Option<&UsageStats>,
    repair_issue: Option<&str>,
    stop_summary: Option<&TurnStopSummary>,
    elapsed: Duration,
) -> Value {
    json!({
        "type": "turn_final",
        "session": session,
        "turn_id": turn_id,
        "assistant_output": assistant_output,
        "stats": stats,
        "latest_usage": latest_usage,
        "repair_issue": repair_issue,
        "stop_summary": stop_summary,
        "elapsed_ms": elapsed.as_millis(),
    })
}

pub fn model_retry_audit_event(
    session: &str,
    turn_id: &str,
    attempt: u32,
    max_attempts: u32,
    delay: Duration,
    error: &str,
) -> Value {
    json!({
        "type": "model_retry",
        "session": session,
        "turn_id": turn_id,
        "attempt": attempt,
        "max_attempts": max_attempts,
        "delay_ms": delay.as_millis(),
        "error": error,
    })
}

pub fn model_input_overflow_recovery_audit_event(
    session: &str,
    turn_id: &str,
    removed_delta_id: &str,
    removed_action_output_bytes: usize,
    error: &str,
) -> Value {
    json!({
        "type": "model_input_overflow_recovery",
        "session": session,
        "turn_id": turn_id,
        "removed_delta_id": removed_delta_id,
        "removed_action_output_bytes": removed_action_output_bytes,
        "error": error,
    })
}

#[cfg(test)]
#[path = "../tests/unit/audit_tests.rs"]
mod tests;

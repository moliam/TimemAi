//! MEM-scoped settings, temporary-data retention, and idle capacity maintenance.
//!
//! Functional scope:
//! - persist and validate MEM retention/capacity settings;
//! - enumerate and safely delete bounded temporary-file candidates;
//! - prune temporary history/audit data and old conversation turns;
//! - checkpoint active runtime and run maintenance only while the MEM is idle.
//!
//! Constraints:
//! - file formats, error codes, capacity choices, scan bounds, and eviction order
//!   are compatibility contracts and must not change during internal refactors;
//! - filesystem scans run only on explicit commands or the existing idle worker;
//! - the global Host mutation barrier and a second idle check prevent maintenance
//!   from overlapping accepted work or a MEM switch;
//! - this module does not own Session/Turn lifecycle semantics.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct PersistedTemporaryMaintenanceState {
    #[serde(default)]
    accumulated_runtime_ms: u64,
}

#[derive(Debug, Clone)]
pub(super) struct TemporaryMaintenanceRuntimeState {
    pub(super) accumulated_runtime: Duration,
    pub(super) checkpoint_started_at: Instant,
}

impl TemporaryMaintenanceRuntimeState {
    fn from_persisted(state: PersistedTemporaryMaintenanceState) -> Self {
        Self {
            accumulated_runtime: Duration::from_millis(state.accumulated_runtime_ms),
            checkpoint_started_at: Instant::now(),
        }
    }

    pub(super) fn checkpoint(&mut self, now: Instant) {
        self.accumulated_runtime = self
            .accumulated_runtime
            .saturating_add(now.saturating_duration_since(self.checkpoint_started_at));
        self.checkpoint_started_at = now;
    }

    fn persisted(&self) -> PersistedTemporaryMaintenanceState {
        PersistedTemporaryMaintenanceState {
            accumulated_runtime_ms: u64::try_from(self.accumulated_runtime.as_millis())
                .unwrap_or(u64::MAX),
        }
    }
}

fn temporary_maintenance_state_path(memory_dir: &Path) -> PathBuf {
    memory_dir.join("temporary_maintenance_state.json")
}

pub(super) fn load_temporary_maintenance_runtime_state(
    memory_dir: &Path,
) -> Result<TemporaryMaintenanceRuntimeState, String> {
    let path = temporary_maintenance_state_path(memory_dir);
    let persisted = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "temporary_maintenance_state_parse_failed".to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            PersistedTemporaryMaintenanceState::default()
        }
        Err(_) => return Err("temporary_maintenance_state_read_failed".to_string()),
    };
    Ok(TemporaryMaintenanceRuntimeState::from_persisted(persisted))
}

pub(super) fn save_temporary_maintenance_runtime_state(
    memory_dir: &Path,
    state: &TemporaryMaintenanceRuntimeState,
) -> Result<(), String> {
    let mut payload = serde_json::to_vec_pretty(&state.persisted())
        .map_err(|_| "temporary_maintenance_state_serialize_failed".to_string())?;
    payload.push(b'\n');
    agent_core::atomic_write_file(&temporary_maintenance_state_path(memory_dir), &payload)
        .map_err(|_| "temporary_maintenance_state_write_failed".to_string())
}

pub(super) fn checkpoint_temporary_maintenance_runtime(
    mem: &mut WebMemState,
    now: Instant,
) -> Result<Duration, String> {
    mem.temporary_maintenance.checkpoint(now);
    save_temporary_maintenance_runtime_state(&mem.layout.memory_dir(), &mem.temporary_maintenance)?;
    Ok(mem.temporary_maintenance.accumulated_runtime)
}

fn temporary_maintenance_hint_exists(mem: &WebMemState) -> bool {
    agent_core::api_audit_maintenance_hint_path(&mem.layout.api_audit_file()).exists()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct WebMemSettings {
    #[serde(
        default = "default_mem_temporary_retention_days",
        alias = "history_retention_days"
    )]
    pub(super) temporary_retention_days: Option<u16>,
    #[serde(default)]
    pub(super) temporary_capacity_bytes: Option<u64>,
    #[serde(default)]
    pub(super) conversation_capacity_bytes: Option<u64>,
    #[serde(default)]
    pub(super) claude_codex_tool_discovery: bool,
}

impl Default for WebMemSettings {
    fn default() -> Self {
        Self::for_debug(false)
    }
}

impl WebMemSettings {
    fn for_debug(debug: bool) -> Self {
        Self {
            temporary_retention_days: default_mem_temporary_retention_days(),
            temporary_capacity_bytes: Some(if debug {
                MEM_CAPACITY_512_MB
            } else {
                MEM_CAPACITY_128_MB
            }),
            conversation_capacity_bytes: Some(MEM_CAPACITY_128_MB),
            claude_codex_tool_discovery: false,
        }
    }
}

pub(super) fn default_mem_temporary_retention_days() -> Option<u16> {
    Some(DEFAULT_MEM_TEMPORARY_RETENTION_DAYS)
}

pub(super) fn validate_mem_temporary_retention_days(days: Option<u16>) -> Result<(), String> {
    if matches!(days, None | Some(1 | 5 | 10)) {
        Ok(())
    } else {
        Err("mem_temporary_retention_days_invalid".to_string())
    }
}

pub(super) fn validate_mem_temporary_capacity_bytes(bytes: Option<u64>) -> Result<(), String> {
    if matches!(
        bytes,
        None | Some(
            MEM_CAPACITY_128_MB
                | MEM_CAPACITY_256_MB
                | MEM_CAPACITY_512_MB
                | MEM_CAPACITY_1_GB
                | MEM_CAPACITY_5_GB
        )
    ) {
        Ok(())
    } else {
        Err("mem_temporary_capacity_bytes_invalid".to_string())
    }
}

pub(super) fn validate_mem_conversation_capacity_bytes(bytes: Option<u64>) -> Result<(), String> {
    if matches!(
        bytes,
        None | Some(
            MEM_CAPACITY_128_MB
                | MEM_CAPACITY_512_MB
                | MEM_CAPACITY_1_GB
                | MEM_CAPACITY_5_GB
                | MEM_CAPACITY_20_GB
        )
    ) {
        Ok(())
    } else {
        Err("mem_conversation_capacity_bytes_invalid".to_string())
    }
}

fn validate_web_mem_settings(settings: &WebMemSettings) -> Result<(), String> {
    validate_mem_temporary_retention_days(settings.temporary_retention_days)?;
    validate_mem_temporary_capacity_bytes(settings.temporary_capacity_bytes)?;
    validate_mem_conversation_capacity_bytes(settings.conversation_capacity_bytes)
}

pub(super) fn web_mem_settings_path(memory_dir: &Path) -> PathBuf {
    memory_dir.join("mem_settings.json")
}

pub(super) fn load_web_mem_settings(
    memory_dir: &Path,
    debug: bool,
) -> Result<WebMemSettings, String> {
    let path = web_mem_settings_path(memory_dir);
    let settings = match std::fs::read(&path) {
        Ok(bytes) => {
            let value = serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| "mem_settings_parse_failed".to_string())?;
            let has_temporary_capacity = value
                .as_object()
                .ok_or_else(|| "mem_settings_parse_failed".to_string())?
                .contains_key("temporary_capacity_bytes");
            let has_conversation_capacity = value
                .as_object()
                .expect("object checked above")
                .contains_key("conversation_capacity_bytes");
            let mut settings = serde_json::from_value::<WebMemSettings>(value)
                .map_err(|_| "mem_settings_parse_failed".to_string())?;
            let defaults = WebMemSettings::for_debug(debug);
            if !has_temporary_capacity {
                settings.temporary_capacity_bytes = defaults.temporary_capacity_bytes;
            }
            if !has_conversation_capacity {
                settings.conversation_capacity_bytes = defaults.conversation_capacity_bytes;
            }
            settings
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            WebMemSettings::for_debug(debug)
        }
        Err(_) => return Err("mem_settings_read_failed".to_string()),
    };
    validate_web_mem_settings(&settings)?;
    Ok(settings)
}

pub(super) fn save_web_mem_settings(
    memory_dir: &Path,
    settings: &WebMemSettings,
) -> Result<(), String> {
    validate_web_mem_settings(settings)?;
    let mut payload = serde_json::to_vec_pretty(settings)
        .map_err(|_| "mem_settings_serialize_failed".to_string())?;
    payload.push(b'\n');
    agent_core::atomic_write_file(&web_mem_settings_path(memory_dir), &payload)
        .map_err(|_| "mem_settings_write_failed".to_string())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct TemporaryRetentionResult {
    pub(super) history_events: usize,
    pub(super) api_audit_events: usize,
}

fn temporary_file_name(name: &str) -> bool {
    name.ends_with(".tmp") || name.contains(".tmp-") || name.starts_with("tmp-")
}

fn modified_at_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn mem_temporary_item_precedes(left: &MemTemporaryItem, right: &MemTemporaryItem) -> bool {
    left.bytes > right.bytes || (left.bytes == right.bytes && left.path < right.path)
}

fn retain_top_mem_temporary_item(items: &mut Vec<MemTemporaryItem>, item: MemTemporaryItem) {
    if items.len() < MAX_MEM_TEMPORARY_ITEMS {
        items.push(item);
        return;
    }
    let Some((smallest_index, smallest)) =
        items.iter().enumerate().min_by(|(_, left), (_, right)| {
            left.bytes
                .cmp(&right.bytes)
                .then_with(|| right.path.cmp(&left.path))
        })
    else {
        return;
    };
    if mem_temporary_item_precedes(&item, smallest) {
        items[smallest_index] = item;
    }
}

fn collect_named_temporary_files(root: &Path) -> Result<Vec<MemTemporaryItem>, String> {
    fn visit(
        root: &Path,
        dir: &Path,
        depth: usize,
        visited: &mut usize,
        out: &mut Vec<MemTemporaryItem>,
    ) -> Result<(), String> {
        if depth > MAX_MEM_TEMPORARY_SCAN_DEPTH || *visited >= MAX_MEM_TEMPORARY_SCAN_ENTRIES {
            return Ok(());
        }
        for entry in
            std::fs::read_dir(dir).map_err(|_| "mem_temporary_items_read_failed".to_string())?
        {
            if *visited >= MAX_MEM_TEMPORARY_SCAN_ENTRIES {
                break;
            }
            *visited = visited.saturating_add(1);
            let entry = entry.map_err(|_| "mem_temporary_items_read_failed".to_string())?;
            let file_type = entry
                .file_type()
                .map_err(|_| "mem_temporary_items_metadata_failed".to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if entry.file_name().to_str() != Some("shell_jobs") {
                    visit(root, &path, depth.saturating_add(1), visited, out)?;
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !temporary_file_name(name) {
                continue;
            }
            // Metadata is the comparatively expensive filesystem operation. Read it only
            // after the cheap directory-entry type and filename checks identify a candidate.
            let metadata = entry
                .metadata()
                .map_err(|_| "mem_temporary_items_metadata_failed".to_string())?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "mem_temporary_item_path_invalid".to_string())?;
            let relative = relative
                .to_str()
                .ok_or_else(|| "mem_temporary_item_path_invalid".to_string())?
                .replace('\\', "/");
            out.push(MemTemporaryItem {
                id: format!("file:{relative}"),
                path: relative,
                kind: "temporary_file".to_string(),
                bytes: metadata.len(),
                modified_at_ms: modified_at_ms(&metadata),
                deletable: true,
                delete_reason: None,
            });
        }
        Ok(())
    }
    let mut items = Vec::new();
    let mut visited = 0usize;
    if root.exists() {
        visit(root, root, 0, &mut visited, &mut items)?;
    }
    Ok(items)
}

fn all_mem_temporary_items_at(memory_dir: &Path) -> Result<Vec<MemTemporaryItem>, String> {
    collect_named_temporary_files(memory_dir)
}

pub(super) fn list_mem_temporary_items_at(
    memory_dir: &Path,
) -> Result<Vec<MemTemporaryItem>, String> {
    let mut all = all_mem_temporary_items_at(memory_dir)?;
    let mut items = Vec::with_capacity(MAX_MEM_TEMPORARY_ITEMS);
    for item in all.drain(..) {
        retain_top_mem_temporary_item(&mut items, item);
    }
    items.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(items)
}

fn safe_temporary_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("mem_temporary_item_path_invalid".to_string());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "mem_temporary_item_path_invalid".to_string())?;
    if !temporary_file_name(name) {
        return Err("mem_temporary_item_not_deletable".to_string());
    }
    Ok(path.to_path_buf())
}

pub(super) fn delete_mem_temporary_items_at(
    memory_dir: &Path,
    ids: &[String],
) -> Result<usize, String> {
    if ids.is_empty() || ids.len() > 100 {
        return Err("mem_temporary_item_selection_invalid".to_string());
    }
    let selected = ids.iter().collect::<BTreeSet<_>>();
    if selected.len() != ids.len() {
        return Err("mem_temporary_item_selection_invalid".to_string());
    }
    let current = all_mem_temporary_items_at(memory_dir)?;
    let available = current
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<std::collections::BTreeMap<_, _>>();
    let canonical_root = std::fs::canonicalize(memory_dir)
        .map_err(|_| "mem_temporary_item_path_invalid".to_string())?;
    let mut file_paths = Vec::new();
    for id in ids {
        let item = available
            .get(id.as_str())
            .ok_or_else(|| "mem_temporary_item_not_found".to_string())?;
        if !item.deletable {
            return Err("mem_temporary_item_not_deletable".to_string());
        }
        let relative = id
            .strip_prefix("file:")
            .ok_or_else(|| "mem_temporary_item_id_invalid".to_string())?;
        let path = memory_dir.join(safe_temporary_relative_path(relative)?);
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "mem_temporary_item_not_found".to_string()
            } else {
                "mem_temporary_item_metadata_failed".to_string()
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("mem_temporary_item_not_deletable".to_string());
        }
        let canonical = std::fs::canonicalize(&path)
            .map_err(|_| "mem_temporary_item_path_invalid".to_string())?;
        if !canonical.starts_with(&canonical_root) {
            return Err("mem_temporary_item_path_invalid".to_string());
        }
        file_paths.push(path);
    }
    for path in &file_paths {
        std::fs::remove_file(path).map_err(|_| "mem_temporary_item_delete_failed".to_string())?;
    }
    Ok(file_paths.len())
}

pub(super) fn oldest_temporary_items_to_evict(
    mut items: Vec<MemTemporaryItem>,
    stable_bytes: u64,
) -> Vec<String> {
    let mut used = items.iter().map(|item| item.bytes).sum::<u64>();
    items.retain(|item| item.deletable);
    items.sort_by_key(|item| (item.modified_at_ms, item.path.clone()));
    let mut delete = Vec::new();
    for item in items {
        if used <= stable_bytes {
            break;
        }
        used = used.saturating_sub(item.bytes);
        delete.push(item.id);
    }
    delete
}

pub(super) fn apply_temporary_retention(
    layout: &RuntimeDataLayout,
    store: &SessionStore,
    days: Option<u16>,
    max_bytes: Option<u64>,
    now_ms: i64,
) -> Result<TemporaryRetentionResult, String> {
    let mut result = TemporaryRetentionResult::default();
    if let Some(days) = days {
        validate_mem_temporary_retention_days(Some(days))?;
        let cutoff_ms = now_ms.saturating_sub(i64::from(days).saturating_mul(MILLIS_PER_DAY));
        for session in store.list_sessions_resilient()?.sessions {
            result.history_events = result.history_events.saturating_add(
                store.prune_temporary_history_events_before(&session.session_id, cutoff_ms)?,
            );
        }
        result.api_audit_events =
            agent_core::prune_api_audit_before(&layout.api_audit_file(), cutoff_ms, now_ms)
                .map_err(|_| "api_audit_retention_failed".to_string())?;
    }
    if let Some(total_bytes) = max_bytes {
        let capacity = RollingCapacity::from_total_bytes(total_bytes)
            .map_err(|_| "mem_temporary_capacity_bytes_invalid".to_string())?;
        let items = all_mem_temporary_items_at(&layout.memory_dir())?;
        let delete = oldest_temporary_items_to_evict(items, capacity.stable_bytes);
        for ids in delete.chunks(100) {
            delete_mem_temporary_items_at(&layout.memory_dir(), ids)?;
        }
    }
    Ok(result)
}

pub(super) fn apply_conversation_capacity(
    state: &AppState,
    store: &SessionStore,
    max_bytes: Option<u64>,
) -> Result<u64, String> {
    let Some(total_bytes) = max_bytes else {
        return Ok(0);
    };
    let capacity = RollingCapacity::from_total_bytes(total_bytes)
        .map_err(|_| "mem_conversation_capacity_bytes_invalid".to_string())?;
    apply_conversation_stable_capacity(state, store, capacity.stable_bytes)
}

pub(super) fn apply_conversation_stable_capacity(
    state: &AppState,
    store: &SessionStore,
    stable_bytes: u64,
) -> Result<u64, String> {
    let active = state
        .sessions
        .lock()
        .map_err(|_| "session_state_poisoned".to_string())?
        .iter()
        .filter_map(|(id, session)| {
            (session.active_turn_id.is_some() || session.pending_turn_id.is_some())
                .then_some(id.clone())
        })
        .collect::<BTreeSet<_>>();
    let mut sessions = store.list_sessions_resilient()?.sessions;
    sessions.sort_by_key(|session| (session.updated_at_ms, session.session_id.clone()));
    let mut used = sessions
        .iter()
        .map(|session| {
            std::fs::metadata(store.history_path_for_session(&session.session_id))
                .map(|meta| meta.len())
                .unwrap_or(0)
        })
        .sum::<u64>();
    let mut removed = 0u64;
    for session in sessions {
        if used <= stable_bytes {
            break;
        }
        if active.contains(&session.session_id) {
            continue;
        }
        let reclaimed = store
            .prune_oldest_history_turns(&session.session_id, used.saturating_sub(stable_bytes))?;
        used = used.saturating_sub(reclaimed);
        removed = removed.saturating_add(reclaimed);
    }
    Ok(removed)
}

pub(super) fn has_live_mem_work(state: &AppState) -> Result<bool, String> {
    Ok(state
        .sessions
        .lock()
        .map_err(|_| "session_state_poisoned".to_string())?
        .values()
        .any(session_has_live_work_for_mem_switch))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TemporaryMaintenanceAttempt {
    Completed,
    Busy,
}

pub(super) async fn run_idle_temporary_maintenance(
    state: AppState,
) -> Result<TemporaryMaintenanceAttempt, String> {
    if has_live_mem_work(&state)? {
        return Ok(TemporaryMaintenanceAttempt::Busy);
    }
    let (result_tx, result_rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("timem-web-idle-maintenance".to_string())
        .spawn(move || {
            // Serialize the maintenance pass against every browser mutation.
            // Recheck idleness after acquiring the barrier so a newly queued or
            // started task can never overlap the scan.
            let result = state
                .command_global_barrier
                .write()
                .map_err(|_| "command_global_barrier_poisoned".to_string())
                .and_then(|_guard| {
                    if has_live_mem_work(&state)? {
                        return Ok(TemporaryMaintenanceAttempt::Busy);
                    }
                    let (layout, store, days, temporary_max_bytes, conversation_max_bytes) = {
                        let mem = state
                            .mem
                            .lock()
                            .map_err(|_| "mem_state_poisoned".to_string())?;
                        (
                            mem.layout.clone(),
                            mem.session_store.clone(),
                            mem.settings.temporary_retention_days,
                            mem.settings.temporary_capacity_bytes,
                            mem.settings.conversation_capacity_bytes,
                        )
                    };
                    apply_temporary_retention(
                        &layout,
                        &store,
                        days,
                        temporary_max_bytes,
                        now_ms_i64(),
                    )?;
                    apply_conversation_stable_capacity_if_configured(
                        &store,
                        conversation_max_bytes,
                    )?;
                    // Reset the same MEM while the global barrier is still held.
                    // This prevents a concurrent MEM switch from redirecting the
                    // completion state or hint removal to a different MEM.
                    complete_temporary_maintenance(&state, Instant::now())?;
                    Ok(TemporaryMaintenanceAttempt::Completed)
                });
            let _ = result_tx.send(result);
        })
        .map_err(|error| format!("temporary_maintenance_worker_spawn_failed:{error}"))?;
    result_rx
        .await
        .map_err(|_| "temporary_maintenance_worker_stopped".to_string())?
}

fn apply_conversation_stable_capacity_if_configured(
    store: &SessionStore,
    max_bytes: Option<u64>,
) -> Result<(), String> {
    let Some(total_bytes) = max_bytes else {
        return Ok(());
    };
    let capacity = RollingCapacity::from_total_bytes(total_bytes)
        .map_err(|_| "mem_conversation_capacity_bytes_invalid".to_string())?;
    // The periodic pass runs only when no Session is active, so no active-work
    // exclusion is required after the idle check.
    let mut sessions = store.list_sessions_resilient()?.sessions;
    sessions.sort_by_key(|session| (session.updated_at_ms, session.session_id.clone()));
    let mut used = sessions
        .iter()
        .map(|session| {
            std::fs::metadata(store.history_path_for_session(&session.session_id))
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .sum::<u64>();
    for session in sessions {
        if used <= capacity.stable_bytes {
            break;
        }
        let reclaimed = store.prune_oldest_history_turns(
            &session.session_id,
            used.saturating_sub(capacity.stable_bytes),
        )?;
        used = used.saturating_sub(reclaimed);
    }
    Ok(())
}

pub(super) fn checkpoint_and_get_temporary_maintenance_trigger(
    state: &AppState,
    now: Instant,
) -> Result<bool, String> {
    let mut mem = state
        .mem
        .lock()
        .map_err(|_| "mem_state_poisoned".to_string())?;
    let accumulated = checkpoint_temporary_maintenance_runtime(&mut mem, now)?;
    Ok(accumulated >= TEMPORARY_MAINTENANCE_INTERVAL || temporary_maintenance_hint_exists(&mem))
}

pub(super) fn complete_temporary_maintenance(state: &AppState, now: Instant) -> Result<(), String> {
    let hint_path = {
        let mut mem = state
            .mem
            .lock()
            .map_err(|_| "mem_state_poisoned".to_string())?;
        let completed = TemporaryMaintenanceRuntimeState {
            accumulated_runtime: Duration::ZERO,
            checkpoint_started_at: now,
        };
        // Persist first and only then update memory, so a failed tiny-state write
        // cannot accidentally suppress a still-due maintenance request.
        save_temporary_maintenance_runtime_state(&mem.layout.memory_dir(), &completed)?;
        mem.temporary_maintenance = completed;
        agent_core::api_audit_maintenance_hint_path(&mem.layout.api_audit_file())
    };
    match std::fs::remove_file(hint_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("temporary_maintenance_hint_clear_failed".to_string()),
    }
}

pub(super) fn spawn_idle_temporary_maintenance_loop(state: AppState) {
    tokio::spawn(async move {
        loop {
            sleep(TEMPORARY_MAINTENANCE_CHECKPOINT_INTERVAL).await;
            let triggered =
                match checkpoint_and_get_temporary_maintenance_trigger(&state, Instant::now()) {
                    Ok(triggered) => triggered,
                    Err(error) => {
                        eprintln!(
                        "[timem_web_warning] temporary_maintenance_checkpoint_failed error={error}"
                    );
                        continue;
                    }
                };
            if !triggered {
                continue;
            }
            loop {
                match run_idle_temporary_maintenance(state.clone()).await {
                    Ok(TemporaryMaintenanceAttempt::Completed) => break,
                    Ok(TemporaryMaintenanceAttempt::Busy) => {
                        // Retry only the cheap live-work check. No checkpoint write and no
                        // full scan occurs while work is active.
                        sleep(TEMPORARY_MAINTENANCE_BUSY_RETRY_INTERVAL).await;
                    }
                    Err(error) => {
                        eprintln!(
                            "[timem_web_warning] idle_temporary_maintenance_failed error={error}"
                        );
                        break;
                    }
                }
            }
        }
    });
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct MemTemporaryItem {
    pub(super) id: String,
    pub(super) path: String,
    pub(super) kind: String,
    pub(super) bytes: u64,
    pub(super) modified_at_ms: i64,
    pub(super) deletable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) delete_reason: Option<String>,
}

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_JOURNAL_EVENTS: usize = 20_000;
const RETAINED_JOURNAL_EVENTS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct JournalEvent {
    pub event_seq: u64,
    pub event: Value,
}

/// Durable append-only ordering for semantic host events.
///
/// The caller serializes the public event into `Value`; this keeps recovery
/// independent from the Rust enum version used by a newer binary. An append is
/// acknowledged only after the line and its metadata reach stable storage.
#[derive(Debug)]
pub(crate) struct EventJournal {
    instance_lock: JournalInstanceLock,
    path: PathBuf,
    next_seq: u64,
    first_seq: Option<u64>,
    entry_count: usize,
    bytes: u64,
}

impl EventJournal {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("event_journal_dir_failed:{error}"))?;
        }
        let instance_lock = JournalInstanceLock::acquire(&path)?;
        repair_partial_tail(&path)?;
        let entries = match read_entries(&path, 0) {
            Ok(entries) => entries,
            Err(error) if error == "event_journal_sequence_not_increasing" => {
                let backup = repair_non_increasing_sequences(&path)?;
                eprintln!(
                    "[timem_web_warning] event_journal_sequence_repaired backup={}",
                    backup.display()
                );
                read_entries(&path, 0)?
            }
            Err(error) if error.starts_with("event_journal_parse_failed:") => {
                let backup = quarantine_corrupt_journal(&path)?;
                eprintln!(
                    "[timem_web_warning] event_journal_corruption_quarantined error={error} backup={}",
                    backup.display()
                );
                Vec::new()
            }
            Err(error) => return Err(error),
        };
        let next_seq = entries
            .last()
            .map(|entry| entry.event_seq)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "event_journal_sequence_exhausted".to_string())?;
        let mut journal = Self {
            instance_lock,
            bytes: std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
            first_seq: entries.first().map(|entry| entry.event_seq),
            entry_count: entries.len(),
            path,
            next_seq,
        };
        if journal.needs_compaction() {
            journal.compact_to_last(RETAINED_JOURNAL_EVENTS)?;
        }
        Ok(journal)
    }

    pub fn cursor(&self) -> u64 {
        self.next_seq.saturating_sub(1)
    }

    pub fn publish_instance_info(&mut self, info: &JournalInstanceInfo) -> Result<(), String> {
        self.instance_lock.write_info(info)
    }

    pub fn read_instance_info(path: impl AsRef<Path>) -> Option<JournalInstanceInfo> {
        let raw = std::fs::read(journal_lock_path(path.as_ref())).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    /// The oldest cursor for which every later event is still replayable.
    /// Clients behind this floor must use the current snapshot as their new
    /// baseline instead of attempting to reconstruct discarded history.
    pub fn replay_floor(&self) -> u64 {
        self.first_seq
            .map(|sequence| sequence.saturating_sub(1))
            .unwrap_or_else(|| self.cursor())
    }

    pub fn append(&mut self, event: Value) -> Result<JournalEvent, String> {
        if self.needs_compaction() {
            self.compact_to_last(RETAINED_JOURNAL_EVENTS)?;
        }
        let entry = JournalEvent {
            event_seq: self.next_seq,
            event,
        };
        let mut encoded = serde_json::to_vec(&entry)
            .map_err(|error| format!("event_journal_serialize_failed:{error}"))?;
        encoded.push(b'\n');

        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|error| format!("event_journal_open_failed:{error}"))?;
        file.write_all(&encoded)
            .and_then(|_| file.sync_data())
            .map_err(|error| format!("event_journal_write_failed:{error}"))?;
        self.first_seq.get_or_insert(entry.event_seq);
        self.entry_count = self.entry_count.saturating_add(1);
        self.bytes = self.bytes.saturating_add(encoded.len() as u64);
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or_else(|| "event_journal_sequence_exhausted".to_string())?;
        Ok(entry)
    }

    pub fn replay_after(&self, cursor: u64) -> Result<Vec<JournalEvent>, String> {
        if cursor < self.replay_floor() {
            return Err(format!(
                "event_journal_cursor_before_floor:{cursor}:{}",
                self.replay_floor()
            ));
        }
        read_entries(&self.path, cursor)
    }

    fn needs_compaction(&self) -> bool {
        self.entry_count > MAX_JOURNAL_EVENTS || self.bytes > MAX_JOURNAL_BYTES
    }

    fn compact_to_last(&mut self, retain: usize) -> Result<(), String> {
        let entries = read_entries(&self.path, 0)?;
        let retained = entries
            .get(entries.len().saturating_sub(retain)..)
            .unwrap_or(&entries);
        let temporary = self.path.with_extension("ndjson.tmp");
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("event_journal_compact_open_failed:{error}"))?;
        let mut bytes = 0_u64;
        for entry in retained {
            let mut encoded = serde_json::to_vec(entry)
                .map_err(|error| format!("event_journal_compact_serialize_failed:{error}"))?;
            encoded.push(b'\n');
            file.write_all(&encoded)
                .map_err(|error| format!("event_journal_compact_write_failed:{error}"))?;
            bytes = bytes.saturating_add(encoded.len() as u64);
        }
        file.sync_all()
            .map_err(|error| format!("event_journal_compact_sync_failed:{error}"))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|error| format!("event_journal_compact_replace_failed:{error}"))?;
        sync_parent_directory(&self.path)?;
        self.first_seq = retained.first().map(|entry| entry.event_seq);
        self.entry_count = retained.len();
        self.bytes = bytes;
        Ok(())
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct JournalInstanceInfo {
    pub pid: u32,
    #[serde(default)]
    pub launch_parent_pid: Option<u32>,
    pub port: Option<u16>,
    pub token: Option<String>,
    pub browser_url: Option<String>,
    pub public_access: bool,
    pub started_at_ms: u128,
}

impl JournalInstanceInfo {
    pub fn starting() -> Self {
        Self {
            pid: std::process::id(),
            launch_parent_pid: current_launch_parent_pid(),
            port: None,
            token: None,
            browser_url: None,
            public_access: false,
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        }
    }
}

fn current_launch_parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        u32::try_from(unsafe { libc::getppid() })
            .ok()
            .filter(|pid| *pid > 1)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[derive(Debug)]
struct JournalInstanceLock {
    file: File,
}

impl JournalInstanceLock {
    fn acquire(journal_path: &Path) -> Result<Self, String> {
        let lock_path = journal_lock_path(journal_path);
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options.open(&lock_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                "event_journal_in_use".to_string()
            } else {
                format!("event_journal_lock_open_failed:{error}")
            }
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("event_journal_lock_permissions_failed:{error}"))?;
        }

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
                    "event_journal_in_use".to_string()
                } else {
                    format!("event_journal_lock_failed:{error}")
                });
            }
        }

        let mut instance_lock = Self { file };
        instance_lock.write_info(&JournalInstanceInfo::starting())?;
        Ok(instance_lock)
    }

    fn write_info(&mut self, info: &JournalInstanceInfo) -> Result<(), String> {
        let encoded = serde_json::to_vec(info)
            .map_err(|error| format!("event_journal_instance_serialize_failed:{error}"))?;
        self.file
            .set_len(0)
            .and_then(|_| self.file.seek(SeekFrom::Start(0)).map(|_| ()))
            .and_then(|_| self.file.write_all(&encoded))
            .and_then(|_| self.file.sync_data())
            .map_err(|error| format!("event_journal_instance_write_failed:{error}"))
    }
}

fn journal_lock_path(journal_path: &Path) -> PathBuf {
    let mut lock_path = journal_path.as_os_str().to_os_string();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

fn repair_non_increasing_sequences(path: &Path) -> Result<PathBuf, String> {
    let mut entries = parse_entries(path)?;
    let mut last_seq = 0_u64;
    let mut changed = false;
    for entry in &mut entries {
        if entry.event_seq <= last_seq {
            entry.event_seq = last_seq
                .checked_add(1)
                .ok_or_else(|| "event_journal_sequence_exhausted".to_string())?;
            changed = true;
        }
        last_seq = entry.event_seq;
    }
    if !changed {
        return Err("event_journal_sequence_repair_not_needed".to_string());
    }

    let raw =
        std::fs::read(path).map_err(|error| format!("event_journal_repair_read_failed:{error}"))?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup_path = path_with_suffix(path, &format!(".sequence-backup-{suffix}"));
    write_new_synced_file(&backup_path, &raw, "event_journal_backup")?;

    let temporary_path = path_with_suffix(
        path,
        &format!(".sequence-repair-{}-{suffix}.tmp", std::process::id()),
    );
    let mut repaired = Vec::with_capacity(raw.len());
    for entry in &entries {
        serde_json::to_writer(&mut repaired, entry)
            .map_err(|error| format!("event_journal_repair_serialize_failed:{error}"))?;
        repaired.push(b'\n');
    }
    write_new_synced_file(&temporary_path, &repaired, "event_journal_repair_temporary")?;
    if let Err(error) = std::fs::rename(&temporary_path, path) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(format!("event_journal_repair_replace_failed:{error}"));
    }
    sync_parent_directory(path)?;
    Ok(backup_path)
}

/// A semantic-event journal is a replay cache, not the authoritative session
/// store. If a complete record is corrupt, preserve the exact bytes for
/// diagnosis and start a fresh journal so a damaged cache cannot prevent the
/// Web UI (and therefore the user's agent) from starting.
fn quarantine_corrupt_journal(path: &Path) -> Result<PathBuf, String> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup_path = path_with_suffix(path, &format!(".corrupt-backup-{suffix}"));
    std::fs::rename(path, &backup_path)
        .map_err(|error| format!("event_journal_quarantine_failed:{error}"))?;
    sync_parent_directory(path)?;
    Ok(backup_path)
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut suffixed = path.as_os_str().to_os_string();
    suffixed.push(suffix);
    PathBuf::from(suffixed)
}

fn write_new_synced_file(path: &Path, body: &[u8], label: &str) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("{label}_open_failed:{error}"))?;
    file.write_all(body)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("{label}_write_failed:{error}"))
}

fn repair_partial_tail(path: &Path) -> Result<(), String> {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("event_journal_read_failed:{error}")),
    };
    if raw.is_empty() || raw.last() == Some(&b'\n') {
        return Ok(());
    }
    let tail_start = raw
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let tail = &raw[tail_start..];
    let mut options = OpenOptions::new();
    options.write(true);
    let mut file = options
        .open(path)
        .map_err(|error| format!("event_journal_open_failed:{error}"))?;
    if serde_json::from_slice::<JournalEvent>(tail).is_ok() {
        // The JSON body reached disk but its line terminator did not. Preserve
        // the event and make the next append start on a distinct record.
        file.seek(SeekFrom::End(0))
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_data())
            .map_err(|error| format!("event_journal_repair_failed:{error}"))?;
    } else {
        // Only an invalid final record can be discarded. Invalid data between
        // complete records remains a hard error in `read_entries`.
        file.set_len(tail_start as u64)
            .and_then(|_| file.sync_data())
            .map_err(|error| format!("event_journal_repair_failed:{error}"))?;
    }
    Ok(())
}

fn sync_parent_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("event_journal_dir_sync_failed:{error}"))?;
    }
    Ok(())
}

fn read_entries(path: &Path, cursor: u64) -> Result<Vec<JournalEvent>, String> {
    let parsed = parse_entries(path)?;
    let mut entries = Vec::new();
    let mut last_seq = 0;
    for entry in parsed {
        if entry.event_seq <= last_seq {
            return Err("event_journal_sequence_not_increasing".to_string());
        }
        last_seq = entry.event_seq;
        if entry.event_seq > cursor {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn parse_entries(path: &Path) -> Result<Vec<JournalEvent>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("event_journal_read_failed:{error}")),
    };
    let mut entries = Vec::new();
    let lines = BufReader::new(file).split(b'\n');
    for line in lines {
        let line = line.map_err(|error| format!("event_journal_read_failed:{error}"))?;
        if line.is_empty() {
            continue;
        }
        let entry: JournalEvent = serde_json::from_slice(&line)
            .map_err(|error| format!("event_journal_parse_failed:{error}"))?;
        entries.push(entry);
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "../tests/unit/event_journal_tests.rs"]
mod tests;

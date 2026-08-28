//! Shared bounded rolling-file storage primitives.
//!
//! Capacity is divided into fixed-size slices. One slice is reserved for safe
//! replacement. Callers define record boundaries; only complete oldest records
//! are evicted.

use crate::atomic_write_file;
use std::fs;
use std::path::Path;

pub const DEFAULT_ROLLING_SLICE_BYTES: u64 = 4 * 1024 * 1024;
pub const AUDIT_ROLLING_SLICE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollingCapacity {
    pub total_bytes: u64,
    pub stable_bytes: u64,
    pub reserved_bytes: u64,
}

impl RollingCapacity {
    pub fn from_total_bytes(total_bytes: u64) -> Result<Self, &'static str> {
        Self::with_slice_bytes(total_bytes, DEFAULT_ROLLING_SLICE_BYTES)
    }

    pub fn with_slice_bytes(total_bytes: u64, slice_bytes: u64) -> Result<Self, &'static str> {
        if slice_bytes == 0
            || total_bytes < slice_bytes.saturating_mul(2)
            || total_bytes % slice_bytes != 0
        {
            return Err("rolling_capacity_invalid");
        }
        Ok(Self {
            total_bytes,
            stable_bytes: total_bytes - slice_bytes,
            reserved_bytes: slice_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RollingRewriteResult {
    pub original_records: usize,
    pub retained_records: usize,
    pub evicted_records: usize,
    pub retained_bytes: u64,
}

pub fn newest_records_start(
    record_sizes: &[u64],
    stable_bytes: u64,
) -> Result<usize, &'static str> {
    if record_sizes.last().is_some_and(|size| *size > stable_bytes) {
        return Err("rolling_record_exceeds_capacity");
    }
    let mut retained = 0u64;
    for (index, size) in record_sizes.iter().enumerate().rev() {
        if retained.saturating_add(*size) > stable_bytes {
            return Ok(index + 1);
        }
        retained = retained.saturating_add(*size);
    }
    Ok(0)
}

/// Returns the directory used by the physical segmented representation.
pub fn segmented_directory(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("records");
    path.with_file_name(format!("{name}.segments"))
}

fn recover_segmented_directory(path: &Path) -> std::io::Result<()> {
    let directory = segmented_directory(path);
    let Some(parent) = directory.parent() else {
        return Ok(());
    };
    let mut backups = Vec::new();
    let mut temporaries = Vec::new();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".rolling-segments.old-") {
                backups.push(entry.path());
            } else if name.starts_with(".rolling-segments.tmp-") {
                temporaries.push(entry.path());
            }
        }
    }
    backups.sort();
    temporaries.sort();
    if !directory.exists() {
        if let Some(backup) = backups.pop() {
            fs::rename(backup, &directory)?;
        } else if let Some(temporary) = temporaries.pop() {
            fs::rename(temporary, &directory)?;
        }
    }
    for stale in backups.into_iter().chain(temporaries) {
        let _ = fs::remove_dir_all(stale);
    }
    Ok(())
}

/// Reads complete records from physical segment files, falling back to the
/// legacy newline-delimited file when no segmented representation exists.
pub fn read_segmented_records(path: &Path) -> std::io::Result<Vec<Vec<u8>>> {
    recover_segmented_directory(path)?;
    let directory = segmented_directory(path);
    let bytes = if directory.exists() {
        let mut entries = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut bytes = Vec::new();
        for entry in entries {
            if entry.file_type()?.is_file()
                && entry.file_name().to_string_lossy().starts_with("segment-")
            {
                bytes.extend_from_slice(&fs::read(entry.path())?);
            }
        }
        bytes
    } else {
        match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        }
    };
    let mut records = Vec::new();
    let mut start = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            records.push(bytes[start..=index].to_vec());
            start = index + 1;
        }
    }
    if start < bytes.len() {
        records.push(bytes[start..].to_vec());
    }
    Ok(records)
}

/// Rewrites complete records into real physical segment files. Each segment is
/// bounded by `slice_bytes`; one capacity slice remains reserved. Installation
/// uses a sibling temporary directory and rename, and legacy single-file data
/// is removed only after the segmented representation is committed.
pub fn rewrite_segmented_records(
    path: &Path,
    records: &[Vec<u8>],
    capacity: RollingCapacity,
    slice_bytes: u64,
) -> std::io::Result<RollingRewriteResult> {
    if slice_bytes == 0 || capacity.reserved_bytes != slice_bytes {
        return Err(std::io::Error::other("rolling_capacity_invalid"));
    }
    let sizes = records
        .iter()
        .map(|record| record.len() as u64)
        .collect::<Vec<_>>();
    if sizes.iter().any(|size| *size > slice_bytes) {
        return Err(std::io::Error::other("rolling_record_exceeds_slice"));
    }
    let start =
        newest_records_start(&sizes, capacity.stable_bytes).map_err(std::io::Error::other)?;
    let retained_bytes = sizes[start..].iter().copied().sum();
    recover_segmented_directory(path)?;
    let directory = segmented_directory(path);
    let parent = directory.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".rolling-segments.tmp-{}-{nonce}",
        std::process::id()
    ));
    let backup = parent.join(format!(
        ".rolling-segments.old-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&temporary)?;
    let write_result = (|| {
        let mut segment = Vec::new();
        let mut segment_index = 0usize;
        let flush = |payload: &mut Vec<u8>, index: usize| -> std::io::Result<()> {
            if payload.is_empty() {
                return Ok(());
            }
            let segment_path = temporary.join(format!("segment-{index:08}.jsonl"));
            atomic_write_file(&segment_path, payload)?;
            payload.clear();
            Ok(())
        };
        for record in &records[start..] {
            if !segment.is_empty() && segment.len() as u64 + record.len() as u64 > slice_bytes {
                flush(&mut segment, segment_index)?;
                segment_index += 1;
            }
            segment.extend_from_slice(record);
        }
        flush(&mut segment, segment_index)?;
        if directory.exists() {
            fs::rename(&directory, &backup)?;
        }
        if let Err(error) = fs::rename(&temporary, &directory) {
            if backup.exists() {
                let _ = fs::rename(&backup, &directory);
            }
            return Err(error);
        }
        let _ = fs::remove_dir_all(&backup);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    write_result?;
    Ok(RollingRewriteResult {
        original_records: records.len(),
        retained_records: records.len().saturating_sub(start),
        evicted_records: start,
        retained_bytes,
    })
}

#[cfg(test)]
#[path = "../tests/unit/rolling_file_store_tests.rs"]
mod tests;

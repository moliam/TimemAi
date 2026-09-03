//! Shared bounded rolling-file storage primitives.
//!
//! Capacity is divided into fixed-size slices. One slice is reserved for safe
//! replacement. Callers define record boundaries; only complete oldest records
//! are evicted.

use crate::atomic_write_file;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

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
            || total_bytes.checked_rem(slice_bytes) != Some(0)
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
pub struct RollingAppendResult {
    pub removed_segments: usize,
    /// True only when an append rolls from an existing segment into a new one.
    /// The first segment created for an empty store is not a rollover.
    pub rolled_segment: bool,
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
    if directory.exists() {
        return Ok(());
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollingSegment {
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RollingManifest {
    version: u32,
    next_index: u64,
    segments: Vec<RollingManifestSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RollingManifestSegment {
    file_name: String,
    bytes: u64,
}

fn rolling_manifest_path(path: &Path) -> PathBuf {
    segmented_directory(path).join("manifest.json")
}

fn rolling_manifest_dirty_path(path: &Path) -> PathBuf {
    segmented_directory(path).join(".manifest-dirty")
}

fn write_rolling_manifest(path: &Path, manifest: &RollingManifest) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(manifest).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    atomic_write_file(&rolling_manifest_path(path), &bytes)
}

fn manifest_from_segments(segments: &[RollingSegment]) -> RollingManifest {
    let next_index = segments
        .last()
        .and_then(|segment| segment.path.file_stem())
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.strip_prefix("segment-"))
        .and_then(|index| index.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    RollingManifest {
        version: 1,
        next_index,
        segments: segments
            .iter()
            .filter_map(|segment| {
                Some(RollingManifestSegment {
                    file_name: segment.path.file_name()?.to_str()?.to_string(),
                    bytes: segment.bytes,
                })
            })
            .collect(),
    }
}

fn load_or_rebuild_manifest(path: &Path) -> std::io::Result<RollingManifest> {
    recover_segmented_directory(path)?;
    if rolling_manifest_dirty_path(path).exists() {
        refresh_rolling_manifest(path)?;
    }
    if let Ok(bytes) = fs::read(rolling_manifest_path(path)) {
        if let Ok(manifest) = serde_json::from_slice::<RollingManifest>(&bytes) {
            if manifest.version == 1 {
                return Ok(manifest);
            }
        }
    }
    let segments = segment_entries(path)?;
    let manifest = manifest_from_segments(&segments);
    write_rolling_manifest(path, &manifest)?;
    Ok(manifest)
}

/// Marks a segmented stream before a caller edits physical slices outside the
/// rolling append path. A surviving marker makes the next load reconcile disk
/// state before trusting the manifest.
pub(crate) fn mark_rolling_manifest_dirty(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(segmented_directory(path))?;
    atomic_write_file(
        &rolling_manifest_dirty_path(path),
        b"manifest update pending\n",
    )
}

/// Rebuilds the segmented-stream manifest from the committed files on disk.
/// Callers that remove or rewrite physical slices outside the rolling append
/// path must refresh before considering their operation complete.
pub(crate) fn refresh_rolling_manifest(path: &Path) -> std::io::Result<()> {
    if !segmented_directory(path).exists() {
        return Ok(());
    }
    let previous_next_index = fs::read(rolling_manifest_path(path))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RollingManifest>(&bytes).ok())
        .filter(|manifest| manifest.version == 1)
        .map(|manifest| manifest.next_index);
    let segments = segment_entries(path)?;
    let mut manifest = manifest_from_segments(&segments);
    if let Some(previous_next_index) = previous_next_index {
        manifest.next_index = manifest.next_index.max(previous_next_index);
    }
    write_rolling_manifest(path, &manifest)?;
    match fs::remove_file(rolling_manifest_dirty_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn manifest_segments(path: &Path, manifest: &RollingManifest) -> Vec<RollingSegment> {
    let directory = segmented_directory(path);
    manifest
        .segments
        .iter()
        .map(|segment| RollingSegment {
            path: directory.join(&segment.file_name),
            bytes: segment.bytes,
        })
        .collect()
}

/// Returns the companion metadata path for a physical segment.
pub fn segment_metadata_path(segment: &Path) -> PathBuf {
    let name = segment
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("segment.jsonl");
    segment.with_file_name(format!(".{name}.meta.json"))
}

fn remove_segment(segment: &Path) -> std::io::Result<()> {
    fs::remove_file(segment)?;
    match fs::remove_file(segment_metadata_path(segment)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn segment_entries(path: &Path) -> std::io::Result<Vec<RollingSegment>> {
    recover_segmented_directory(path)?;
    let directory = segmented_directory(path);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    entries
        .into_iter()
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
                && entry.file_name().to_string_lossy().starts_with("segment-")
                && entry.file_name().to_string_lossy().ends_with(".jsonl")
        })
        .map(|entry| {
            Ok(RollingSegment {
                bytes: entry.metadata()?.len(),
                path: entry.path(),
            })
        })
        .collect()
}

/// Returns the physical slices in oldest-to-newest order.
pub fn rolling_segments(path: &Path) -> std::io::Result<Vec<RollingSegment>> {
    if !segmented_directory(path).exists() {
        return Ok(Vec::new());
    }
    let manifest = load_or_rebuild_manifest(path)?;
    Ok(manifest_segments(path, &manifest))
}

/// Migrates a legacy newline-delimited file into bounded physical slices. This
/// is a single sequential copy and never parses or reserializes records.
pub fn migrate_legacy_file(path: &Path, slice_bytes: u64) -> std::io::Result<()> {
    recover_segmented_directory(path)?;
    if segmented_directory(path).exists() || !path.exists() {
        return Ok(());
    }
    if slice_bytes == 0 {
        return Err(std::io::Error::other("rolling_capacity_invalid"));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let directory = segmented_directory(path);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".rolling-segments.tmp-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&temporary)?;
    let result = (|| {
        let mut reader = BufReader::new(fs::File::open(path)?);
        let mut index = 1u64;
        let mut current_bytes = 0u64;
        let mut output: Option<fs::File> = None;
        let mut record = Vec::new();
        loop {
            record.clear();
            if reader.read_until(b'\n', &mut record)? == 0 {
                break;
            }
            if record.len() as u64 > slice_bytes {
                return Err(std::io::Error::other("rolling_record_exceeds_slice"));
            }
            // `slice_bytes` is a soft rollover threshold. Keep each logical
            // record whole in one physical slice, even when that record makes
            // the slice slightly exceed the target. Roll only before the next
            // record once the current slice has already reached the threshold.
            if output.is_none() || current_bytes >= slice_bytes {
                output = Some(
                    OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(temporary.join(format!("segment-{index:016}.jsonl")))?,
                );
                index = index.saturating_add(1);
                current_bytes = 0;
            }
            output
                .as_mut()
                .expect("segment file exists")
                .write_all(&record)?;
            current_bytes = current_bytes.saturating_add(record.len() as u64);
        }
        if let Some(file) = output.as_mut() {
            file.sync_all()?;
        }
        // Windows prevents renaming a directory that contains an open file and
        // deleting the legacy source while its reader is still alive. Release
        // both handles before publishing the segmented directory.
        drop(output);
        drop(reader);
        let mut entries = fs::read_dir(&temporary)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        let segments = entries
            .into_iter()
            .filter(|entry| {
                entry
                    .file_type()
                    .map(|kind| kind.is_file())
                    .unwrap_or(false)
            })
            .map(|entry| {
                Ok(RollingSegment {
                    bytes: entry.metadata()?.len(),
                    path: entry.path(),
                })
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        let manifest = manifest_from_segments(&segments);
        let mut manifest_bytes = serde_json::to_vec(&manifest).map_err(std::io::Error::other)?;
        manifest_bytes.push(b'\n');
        atomic_write_file(&temporary.join("manifest.json"), &manifest_bytes)?;
        fs::rename(&temporary, &directory)?;
        fs::remove_file(path)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

/// Appends one complete record to the active slice and evicts only complete
/// oldest slices when the stable capacity is exceeded.
pub fn append_rolling_record(
    path: &Path,
    record: &[u8],
    capacity: RollingCapacity,
    slice_bytes: u64,
) -> std::io::Result<usize> {
    Ok(append_rolling_record_with_result(path, record, capacity, slice_bytes)?.removed_segments)
}

/// Appends one complete record and reports the infrequent segment-roll boundary.
pub fn append_rolling_record_with_result(
    path: &Path,
    record: &[u8],
    capacity: RollingCapacity,
    slice_bytes: u64,
) -> std::io::Result<RollingAppendResult> {
    if record.is_empty()
        || record.len() as u64 > slice_bytes
        || capacity.reserved_bytes != slice_bytes
    {
        return Err(std::io::Error::other("rolling_record_exceeds_slice"));
    }
    migrate_legacy_file(path, slice_bytes)?;
    let directory = segmented_directory(path);
    fs::create_dir_all(&directory)?;
    let mut manifest = load_or_rebuild_manifest(path)?;
    if let Some(last) = manifest.segments.last_mut() {
        let actual_bytes = fs::metadata(directory.join(&last.file_name))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        last.bytes = actual_bytes;
    }
    let had_segment = !manifest.segments.is_empty();
    let mut created_segment = false;
    let target_index = match manifest.segments.last() {
        Some(last) if last.bytes < slice_bytes => manifest.segments.len().saturating_sub(1),
        _ => {
            let file_name = format!("segment-{:016}.jsonl", manifest.next_index.max(1));
            manifest.next_index = manifest.next_index.max(1).saturating_add(1);
            manifest.segments.push(RollingManifestSegment {
                file_name,
                bytes: 0,
            });
            created_segment = true;
            manifest.segments.len().saturating_sub(1)
        }
    };
    if created_segment {
        write_rolling_manifest(path, &manifest)?;
    }
    let target = directory.join(&manifest.segments[target_index].file_name);
    let mut file = OpenOptions::new().create(true).append(true).open(&target)?;
    file.write_all(record)?;
    file.sync_data()?;
    manifest.segments[target_index].bytes = manifest.segments[target_index]
        .bytes
        .saturating_add(record.len() as u64);
    let mut total = manifest
        .segments
        .iter()
        .map(|segment| segment.bytes)
        .sum::<u64>();
    let mut removed = 0usize;
    while total > capacity.stable_bytes && manifest.segments.len() > 1 {
        let oldest = manifest.segments.remove(0);
        remove_segment(&directory.join(&oldest.file_name))?;
        total = total.saturating_sub(oldest.bytes);
        removed = removed.saturating_add(1);
    }
    write_rolling_manifest(path, &manifest)?;
    Ok(RollingAppendResult {
        removed_segments: removed,
        rolled_segment: had_segment && created_segment,
    })
}

/// Removes complete oldest slices until at most `max_bytes` remain.
pub fn trim_rolling_segments(
    path: &Path,
    max_bytes: u64,
    slice_bytes: u64,
) -> std::io::Result<usize> {
    migrate_legacy_file(path, slice_bytes)?;
    if !segmented_directory(path).exists() {
        return Ok(0);
    }
    let directory = segmented_directory(path);
    let mut manifest = load_or_rebuild_manifest(path)?;
    let mut total = manifest
        .segments
        .iter()
        .map(|segment| segment.bytes)
        .sum::<u64>();
    let mut removed = 0usize;
    while total > max_bytes && !manifest.segments.is_empty() {
        let oldest = manifest.segments.remove(0);
        remove_segment(&directory.join(&oldest.file_name))?;
        total = total.saturating_sub(oldest.bytes);
        removed = removed.saturating_add(1);
    }
    write_rolling_manifest(path, &manifest)?;
    Ok(removed)
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
                && entry.file_name().to_string_lossy().ends_with(".jsonl")
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

/// Rewrites complete records into real physical segment files. `slice_bytes`
/// is a soft rollover threshold: a complete record may make a segment exceed
/// it, and the following record starts a new segment. One capacity slice remains reserved. Installation
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
            if !segment.is_empty() && segment.len() as u64 >= slice_bytes {
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

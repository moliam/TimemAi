use crate::UsageStats;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeProfiler {
    models: BTreeMap<String, ModelProfile>,
    model_wait: Duration,
    local_work: Duration,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelProfile {
    pub llm_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cache_created_tokens: u64,
    pub wait: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageProfile {
    pub durable_entries: usize,
    pub durable_bytes: u64,
    pub scratch_entries: usize,
    pub scratch_bytes: u64,
    pub api_audit_bytes: u64,
    pub action_audit_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProfileReport {
    pub models: Vec<ModelProfileReport>,
    pub model_wait: Duration,
    pub local_work: Duration,
    pub storage: StorageProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProfileReport {
    pub model: String,
    pub llm_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cache_created_tokens: u64,
    pub wait: Duration,
}

impl ModelProfileReport {
    pub fn cache_hit_percent_tenths(&self) -> Option<u32> {
        profile_cache_hit_percent_tenths(self.cached_tokens, self.input_tokens)
    }

    pub fn wait_per_1k_output(&self) -> Option<Duration> {
        profile_wait_per_1k_output(self.wait, self.output_tokens)
    }
}

impl RuntimeProfiler {
    pub fn record_model_wait(
        &mut self,
        provider: &str,
        model: &str,
        usage: &UsageStats,
        wait: Duration,
    ) {
        let key = format!("{}:{}", provider, model);
        let profile = self.models.entry(key).or_default();
        profile.llm_calls = profile.llm_calls.saturating_add(usage.llm_calls);
        profile.input_tokens = profile
            .input_tokens
            .saturating_add(usage.prompt_tokens as u64);
        profile.output_tokens = profile
            .output_tokens
            .saturating_add(usage.completion_tokens as u64);
        profile.cached_tokens = profile
            .cached_tokens
            .saturating_add(usage.cached_tokens as u64);
        profile.cache_created_tokens = profile
            .cache_created_tokens
            .saturating_add(usage.cache_created_tokens as u64);
        profile.wait = profile.wait.saturating_add(wait);
        self.model_wait = self.model_wait.saturating_add(wait);
    }

    pub fn record_turn(&mut self, elapsed: Duration, model_wait: Duration) {
        let local = elapsed.saturating_sub(model_wait);
        self.local_work = self.local_work.saturating_add(local);
    }

    pub fn model_wait(&self) -> Duration {
        self.model_wait
    }

    pub fn local_work(&self) -> Duration {
        self.local_work
    }

    pub fn models(&self) -> &BTreeMap<String, ModelProfile> {
        &self.models
    }

    pub fn report(&self, storage: StorageProfile) -> RuntimeProfileReport {
        RuntimeProfileReport {
            models: self
                .models
                .iter()
                .map(|(model, profile)| ModelProfileReport {
                    model: model.clone(),
                    llm_calls: profile.llm_calls,
                    input_tokens: profile.input_tokens,
                    output_tokens: profile.output_tokens,
                    cached_tokens: profile.cached_tokens,
                    cache_created_tokens: profile.cache_created_tokens,
                    wait: profile.wait,
                })
                .collect(),
            model_wait: self.model_wait,
            local_work: self.local_work,
            storage,
        }
    }
}

pub fn collect_storage_profile(
    memory_dir: &Path,
    api_audit_file: &Path,
    action_audit_file: &Path,
) -> StorageProfile {
    let durable_file = memory_dir.join("memory.jsonl");
    let scratch_file = memory_dir.join("scratch_notes.jsonl");
    StorageProfile {
        durable_entries: count_jsonl_entries(&durable_file),
        durable_bytes: file_size(&durable_file),
        scratch_entries: count_jsonl_entries(&scratch_file),
        scratch_bytes: file_size(&scratch_file),
        api_audit_bytes: file_size(api_audit_file),
        action_audit_bytes: file_size(action_audit_file),
    }
}

pub fn runtime_profile_report(
    profiler: &RuntimeProfiler,
    memory_dir: &Path,
    api_audit_file: &Path,
    action_audit_file: &Path,
) -> RuntimeProfileReport {
    profiler.report(collect_storage_profile(
        memory_dir,
        api_audit_file,
        action_audit_file,
    ))
}

fn count_jsonl_entries(path: &Path) -> usize {
    fs::File::open(path)
        .map(|file| {
            BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

pub fn profile_cache_hit_percent_tenths(cached_tokens: u64, input_tokens: u64) -> Option<u32> {
    if input_tokens == 0 {
        return None;
    }
    Some(((cached_tokens as u128 * 1000) / input_tokens as u128) as u32)
}

pub fn profile_wait_per_1k_output(wait: Duration, output_tokens: u64) -> Option<Duration> {
    if output_tokens == 0 {
        return None;
    }
    let millis = wait.as_millis().saturating_mul(1000) / output_tokens as u128;
    Some(Duration::from_millis(millis as u64))
}

#[cfg(test)]
#[path = "../tests/unit/profiler_tests.rs"]
mod tests;

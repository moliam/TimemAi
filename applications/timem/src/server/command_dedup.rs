//! Bounded process-local browser command correlation.
//!
//! This cache only prevents duplicate execution while the Host process is
//! alive. It is not a durable command ledger and does not own Session/Turn
//! lifecycle state.

use super::WireEvent;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

pub(super) const COMMAND_DEDUP_CAPACITY: usize = 4_096;
pub(super) const MAX_COMMAND_DEDUP_RESULT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub(super) enum CommandDedupState {
    Accepted,
    Committed {
        event: Option<Box<WireEvent>>,
        serialized_event: Option<Value>,
    },
    Rejected {
        error: String,
    },
}

#[derive(Debug, Default)]
pub(super) struct CommandDedupCache {
    records: HashMap<String, CommandDedupState>,
    insertion_order: VecDeque<String>,
}

impl CommandDedupCache {
    /// Reserves a correlation id without evicting commands still in flight.
    pub(super) fn reserve(
        &mut self,
        command_id: &str,
    ) -> Result<Option<CommandDedupState>, String> {
        if let Some(existing) = self.records.get(command_id) {
            return Ok(Some(existing.clone()));
        }
        while self.records.len() >= COMMAND_DEDUP_CAPACITY {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if matches!(self.records.get(&oldest), Some(CommandDedupState::Accepted)) {
                self.insertion_order.push_back(oldest);
                if self
                    .insertion_order
                    .iter()
                    .all(|id| matches!(self.records.get(id), Some(CommandDedupState::Accepted)))
                {
                    break;
                }
                continue;
            }
            self.records.remove(&oldest);
        }
        if self.records.len() >= COMMAND_DEDUP_CAPACITY {
            return Err("command_dedup_capacity_exhausted".to_string());
        }
        self.records
            .insert(command_id.to_string(), CommandDedupState::Accepted);
        self.insertion_order.push_back(command_id.to_string());
        Ok(None)
    }

    pub(super) fn finish(&mut self, command_id: &str, state: CommandDedupState) {
        self.records.insert(command_id.to_string(), state);
    }

    pub(super) fn unreserve(&mut self, command_id: &str) {
        self.records.remove(command_id);
        self.insertion_order.retain(|id| id != command_id);
    }

    pub(super) fn clear(&mut self) {
        self.records.clear();
        self.insertion_order.clear();
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.records.len()
    }

    #[cfg(test)]
    pub(super) fn contains(&self, command_id: &str) -> bool {
        self.records.contains_key(command_id)
    }
}

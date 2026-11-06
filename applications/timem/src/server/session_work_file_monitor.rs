//! Session work-file update notices.
//!
//! `Timem.md` is the Session's durable working note. When the model or the
//! user rewrites it, the conversation work stream gets a small machine
//! generated notice so the update is visible next to the other system notices
//! (memo, context compression). The notice is UI-only: it is persisted to the
//! chat history and broadcast as a turn event, but it never becomes model
//! input. The work-file content already reaches the model through supporting
//! context, so injecting the notice again would only add prompt noise.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime};

use agent_core::session_store::{ChatHistoryEventKind, ChatHistoryRecord, ChatHistoryRole};
use serde_json::Value;

use super::{
    append_history_record, current_turn_id, history_event_content, next_turn_timeline_seq, now_ms,
    persist_web_session, publish_semantic, session_work_file_path, unique_web_id, AppState,
    WebTurn, WebTurnEvent, WireEvent,
};

/// Legacy user-entry kind written before the notice moved to the work stream.
/// Restores migrate those records into notice events so both shapes render the
/// same way.
pub(super) const LEGACY_WORK_FILE_UPDATE_ENTRY_KIND: &str = "work_file_update";
/// Activity kind of the work-file update notice.
pub(super) const WORK_FILE_NOTICE_ACTIVITY_KIND: &str = "work_file_notice";
/// Presentation-only turn event source: rendered by the interface, never used
/// as model input.
const WORK_FILE_NOTICE_SOURCE: &str = "ui_activity";

const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// A single edit often touches the file more than once (truncate + write).
/// Coalesce a burst into one notice instead of spamming the timeline.
const NOTICE_COOLDOWN: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WorkFileStamp {
    pub(super) modified: Option<SystemTime>,
    pub(super) len: u64,
}

#[derive(Default)]
pub(super) struct WorkFileWatchState {
    /// `None` records an explicitly missing file. A later creation is a real
    /// change; a session never observed before is only a baseline.
    stamps: HashMap<String, Option<WorkFileStamp>>,
    last_notice_at: HashMap<String, Instant>,
}

impl WorkFileWatchState {
    /// Returns true when the caller should publish one update notice. The first
    /// observation only establishes a baseline; an unchanged stamp stays quiet,
    /// and writes inside the cooldown window coalesce into the earlier notice.
    pub(super) fn observe(
        &mut self,
        session_id: &str,
        stamp: Option<WorkFileStamp>,
        now: Instant,
    ) -> bool {
        let previous = self.stamps.insert(session_id.to_string(), stamp);
        match previous {
            None => false,
            Some(previous) if previous == stamp => false,
            // Deletion is not an "updated" notice, but it must reset the
            // baseline so recreation has something to compare against.
            Some(_) if stamp.is_none() => false,
            Some(_) => {
                if let Some(last) = self.last_notice_at.get(session_id) {
                    if now.saturating_duration_since(*last) < NOTICE_COOLDOWN {
                        return false;
                    }
                }
                self.last_notice_at.insert(session_id.to_string(), now);
                true
            }
        }
    }

    pub(super) fn retain_sessions(&mut self, alive: &HashSet<String>) {
        self.stamps
            .retain(|session_id, _| alive.contains(session_id));
        self.last_notice_at
            .retain(|session_id, _| alive.contains(session_id));
    }

    fn mark_absent(&mut self, session_id: &str) {
        self.stamps.insert(session_id.to_string(), None);
    }
}

pub(super) fn spawn_session_work_file_monitor(state: AppState) {
    tokio::spawn(async move {
        let mut watch = WorkFileWatchState::default();
        loop {
            sample_work_files(&state, &mut watch);
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

fn sample_work_files(state: &AppState, watch: &mut WorkFileWatchState) {
    if state
        .session_restore_in_progress
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let session_ids: Vec<String> = {
        let Ok(sessions) = state.sessions.lock() else {
            return;
        };
        sessions
            .values()
            .map(|session| session.session_id.clone())
            .collect()
    };
    let alive: HashSet<String> = session_ids.iter().cloned().collect();
    watch.retain_sessions(&alive);
    let now = Instant::now();
    for session_id in session_ids {
        let Ok(path) = session_work_file_path(state, &session_id) else {
            continue;
        };
        let stamp = match std::fs::metadata(&path) {
            Ok(metadata) => WorkFileStamp {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            },
            // A missing work file is not an update. Remember the missing
            // state so a later creation is detected as a real change.
            Err(_) => {
                watch.mark_absent(&session_id);
                continue;
            }
        };
        if watch.observe(&session_id, Some(stamp), now) {
            publish_work_file_update(state, &session_id);
        }
    }
}

fn publish_work_file_update(state: &AppState, session_id: &str) {
    match append_work_file_update_notice(state, session_id) {
        Ok(Some(turn)) => {
            publish_semantic(
                state,
                WireEvent::TurnUpdated {
                    session_id: session_id.to_string(),
                    turn,
                },
            );
            state.runtime_log.record(
                "session_work_file_update",
                serde_json::json!({ "session_id": session_id }),
            );
        }
        // A Session without any Turn has no conversation position for the
        // notice; there is nothing for the UI to attach it to.
        Ok(None) => {}
        Err(error) => {
            state.runtime_log.record(
                "session_work_file_update_failed",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error,
                }),
            );
        }
    }
}

/// Activity payload for the work-stream notice. The interface owns the copy so
/// the notice follows the active language; the payload only carries semantics.
pub(super) fn work_file_notice_payload() -> Value {
    serde_json::json!({
        "kind": WORK_FILE_NOTICE_ACTIVITY_KIND,
        "tone": "notice",
        "title": "",
    })
}

/// Append a UI-only system notice to the active Turn, or to the newest Turn
/// when the Session is idle. The notice rides the work-stream event chain so
/// the interface renders it exactly like memo and context-compression notices.
pub(super) fn append_work_file_update_notice(
    state: &AppState,
    session_id: &str,
) -> Result<Option<WebTurn>, String> {
    let (turn_id, event_id, created_at_ms, snapshot) = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        let turn_id = current_turn_id(session)
            .map(str::to_string)
            .or_else(|| session.turns.last().map(|turn| turn.turn_id.clone()));
        let Some(turn_id) = turn_id else {
            return Ok(None);
        };
        let turn = session
            .turns
            .iter_mut()
            .find(|turn| turn.turn_id == turn_id)
            .ok_or_else(|| "work_file_update_turn_not_found".to_string())?;
        let created_at_ms = now_ms();
        let timeline_seq = next_turn_timeline_seq(turn);
        let event_id = unique_web_id("work_file_notice");
        turn.events.push(WebTurnEvent {
            event_id: event_id.clone(),
            source: WORK_FILE_NOTICE_SOURCE.to_string(),
            payload: work_file_notice_payload(),
            created_at_ms,
            timeline_seq: Some(timeline_seq),
        });
        (turn_id, event_id, created_at_ms, turn.clone())
    };
    if let Err(error) = persist_work_file_update_record(state, session_id, &turn_id, created_at_ms)
    {
        // The UI mutation is not durable; remove it so a reload cannot show a
        // notice that the history file never recorded.
        if let Ok(mut sessions) = state.sessions.lock() {
            if let Some(session) = sessions.get_mut(session_id) {
                if let Some(turn) = session
                    .turns
                    .iter_mut()
                    .find(|turn| turn.turn_id == turn_id)
                {
                    turn.events.retain(|event| event.event_id != event_id);
                }
            }
        }
        return Err(error);
    }
    if let Err(error) = persist_web_session(state, session_id) {
        // The chat history already owns the durable record. Keep the live
        // event and surface the index write failure instead of hiding it.
        state.runtime_log.record(
            "session_work_file_update_persist_pending",
            serde_json::json!({
                "session_id": session_id,
                "error": error,
            }),
        );
    }
    Ok(Some(snapshot))
}

fn persist_work_file_update_record(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    created_at_ms: u128,
) -> Result<(), String> {
    let payload = work_file_notice_payload();
    let mut extra = BTreeMap::new();
    extra.insert(
        "source".to_string(),
        Value::String(WORK_FILE_NOTICE_SOURCE.to_string()),
    );
    extra.insert("payload".to_string(), payload.clone());
    append_history_record(
        state,
        session_id,
        &ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: turn_id.to_string(),
            created_at_ms: created_at_ms as i64,
            kind: ChatHistoryEventKind::RuntimeNotice,
            content: history_event_content(WORK_FILE_NOTICE_SOURCE, &payload),
            extra,
        },
    )
}

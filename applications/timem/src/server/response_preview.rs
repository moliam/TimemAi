//! Bounded ephemeral projection: never append streaming chunks to chat history.
use super::*;
use agent_core::CoreTopicEvent;

/// Minimum spacing between broadcast frames of one streaming preview. Frames
/// inside the window are withheld as pending and only the newest one is
/// broadcast when the window elapses (checked on the next topic event). Every
/// frame carries the full cumulative preview text, so dropping withheld
/// intermediate frames loses no content while capping the client re-render
/// rate at ~7 frames/s instead of the model's native tick rate.
pub(super) const PREVIEW_THROTTLE_WINDOW_MS: u128 = 150;

/// Streaming-preview throttle for one session. A session publishes at most one
/// preview at a time (publish() only accepts the primary worker's active
/// turn), so a single slot keeps the state bounded.
#[derive(Debug, Clone, Default)]
pub(super) struct PreviewThrottleState {
    pub(super) pending: Option<PendingPreview>,
    /// First withholding moment of the current window; deliberately not
    /// refreshed by later withheld frames so the window always elapses.
    window_started_ms: u128,
    last_flush_ms: u128,
}

#[derive(Debug, Clone)]
pub(super) struct PendingPreview {
    turn_id: String,
    timeline_seq: u64,
    wire_event: Value,
}

impl PendingPreview {
    fn into_wire_event(self, session_id: &str) -> WireEvent {
        WireEvent::CoreTopic {
            session_id: session_id.to_string(),
            turn_id: Some(self.turn_id),
            turn_event_id: None,
            timeline_seq: Some(self.timeline_seq),
            created_at_ms: None,
            event: self.wire_event,
        }
    }
}

pub(super) fn publish(state: &AppState, session_id: &str, worker_id: &str, event: &CoreTopicEvent) {
    let Some(turn_id) = event.payload.get("turn_id").and_then(Value::as_str) else {
        return;
    };
    let Some(revision) = event.payload.get("revision").and_then(Value::as_u64) else {
        return;
    };
    let Ok(mut sessions) = state.sessions.lock() else {
        return;
    };
    let Some(session) = sessions.get_mut(session_id) else {
        return;
    };
    if session.primary_worker_id != worker_id {
        return;
    }
    let Some(current) = session.turn_projection.current() else {
        return;
    };
    let TurnProjection::Active(active) = current.projection else {
        return;
    };
    if active.token.turn_id != turn_id || active.token.session_id != session_id {
        return;
    }
    let target_turn_id = current_turn_id(session).map(str::to_string);
    let Some(turn) = session
        .turns
        .iter_mut()
        .find(|turn| Some(turn.turn_id.as_str()) == target_turn_id.as_deref())
    else {
        return;
    };
    if turn
        .preview
        .as_ref()
        .and_then(|p| p.get("revision"))
        .and_then(Value::as_u64)
        .is_some_and(|old| old >= revision)
    {
        return;
    }
    let attempt = event.payload.get("attempt").and_then(Value::as_u64);
    let timeline_seq = turn
        .preview
        .as_ref()
        .filter(|preview| preview.get("attempt").and_then(Value::as_u64) == attempt)
        .and_then(|preview| preview.get("timeline_seq"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| next_turn_timeline_seq(turn));
    let mut preview = event.payload.clone();
    preview["timeline_seq"] = json!(timeline_seq);
    let web_turn_id = turn.turn_id.clone();
    // A settled Final frame is the answer itself, not a streaming tick. It must
    // reach the wire before the terminal turn event, whose flush drops withheld
    // previews as stale. Track that before the payload is moved into the wire
    // event.
    let final_preview = preview
        .get("response")
        .and_then(|response| response.get("status"))
        .and_then(Value::as_str)
        == Some("final");
    // Thinking frames carry only a bounded reasoning tail and no answer text,
    // so bypass the 150ms answer-preview window to keep the strip fluid.
    let live_thinking = !final_preview
        && preview.get("stream_phase").and_then(Value::as_str) == Some("thinking")
        && preview
            .get("thinking_tail")
            .and_then(Value::as_str)
            .is_some_and(|tail| !tail.is_empty());
    // The server-side projection always tracks the newest frame; only the
    // broadcast is throttled.
    turn.preview = Some(preview.clone());
    let mut wire_event = event.wire_payload();
    wire_event["payload"] = preview;
    // Throttle decision, still under the session lock.
    let due = {
        let now = now_ms();
        let throttle = &mut session.preview_throttle;
        let mut due: Vec<PendingPreview> = Vec::new();
        if final_preview || live_thinking {
            // Terminal frames never wait: flush any withheld streaming frame
            // first so wire order stays monotonic, then publish the Final
            // frame immediately. Otherwise the following terminal turn event
            // would discard it as a stale pending preview.
            if let Some(pending) = throttle.pending.take() {
                due.push(pending);
            }
            throttle.last_flush_ms = now;
            due.push(PendingPreview {
                turn_id: web_turn_id,
                timeline_seq,
                wire_event,
            });
        } else {
            // An elapsed window first releases the withheld frame so the wire
            // order (timeline_seq) stays monotonic before any newer frame.
            if let Some(pending) = throttle.pending.take() {
                if now.saturating_sub(throttle.window_started_ms) >= PREVIEW_THROTTLE_WINDOW_MS {
                    throttle.last_flush_ms = now;
                    due.push(pending);
                } else {
                    throttle.pending = Some(pending);
                }
            }
            if now.saturating_sub(throttle.last_flush_ms) >= PREVIEW_THROTTLE_WINDOW_MS {
                throttle.last_flush_ms = now;
                due.push(PendingPreview {
                    turn_id: web_turn_id,
                    timeline_seq,
                    wire_event,
                });
            } else {
                if throttle.pending.is_none() {
                    throttle.window_started_ms = now;
                }
                throttle.pending = Some(PendingPreview {
                    turn_id: web_turn_id,
                    timeline_seq,
                    wire_event,
                });
            }
        }
        due
    };
    drop(sessions);
    for pending in due {
        publish_core_semantic(state, session_id, pending.into_wire_event(session_id));
    }
}

/// Broadcasts a withheld preview whose window has elapsed, or drops it once
/// the turn reached a terminal answer (the real message replaces the
/// preview). Runs before the preview branch of the worker topic loop, so
/// every incoming event also drains due withholdings; the check itself is
/// O(1) and only takes the session lock.
pub(super) fn flush_pending(state: &AppState, session_id: &str) {
    let due = {
        let Ok(mut sessions) = state.sessions.lock() else {
            return;
        };
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        // The preview stream of a turn only lives while that turn stays the
        // session's authoritative active projection. Once the projection
        // leaves it (finished, cancelled, failed, or a newer turn), the real
        // message has replaced the preview and the withheld frame must be
        // dropped, never broadcast late on top of the terminal answer.
        let projection_still_active = session
            .turn_projection
            .current()
            .is_some_and(|current| matches!(current.projection, TurnProjection::Active(_)));
        let active_turn_web_id = if projection_still_active {
            current_turn_id(session).map(str::to_string)
        } else {
            None
        };
        let throttle = &mut session.preview_throttle;
        let Some(pending) = throttle.pending.as_ref() else {
            return;
        };
        if active_turn_web_id.as_deref() != Some(pending.turn_id.as_str()) {
            throttle.pending = None;
            return;
        }
        let now = now_ms();
        if now.saturating_sub(throttle.window_started_ms) < PREVIEW_THROTTLE_WINDOW_MS {
            return;
        }
        throttle.last_flush_ms = now;
        throttle.pending.take()
    };
    if let Some(pending) = due {
        publish_core_semantic(state, session_id, pending.into_wire_event(session_id));
    }
}

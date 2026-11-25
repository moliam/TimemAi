//! Bounded ephemeral projection: never append streaming chunks to chat history.
use super::*;
use agent_core::CoreTopicEvent;

/// Minimum spacing between broadcast frames of one streaming preview. Frames
/// inside the window are withheld as pending and only the newest one is
/// broadcast when the window elapses (checked on the next topic event). Every
/// frame carries the cumulative preview text up to the newest withheld one, so
/// dropping intermediate frames loses no content while capping the client
/// re-render rate at ~7 frames/s instead of the model's native tick rate. On
/// the wire that cumulative text is sent as a delta against the previous frame
/// (see `encode_preview_payload`).
pub(super) const PREVIEW_THROTTLE_WINDOW_MS: u128 = 150;

/// A client that connects mid-attempt, or that drops one frame, cannot join an
/// arbitrary delta chain: it has no copy matching the watermark. Re-sending the
/// full text every `MAX_DELTA_RUN` frames rebuilds that base within a couple of
/// seconds, and still keeps a turn's wire cost linear instead of quadratic.
pub(super) const MAX_DELTA_RUN: u32 = 16;

/// The last frame actually broadcast for a session preview. Deltas are measured
/// against this watermark, never against a withheld frame, so dropping
/// intermediate frames inside the throttle window loses no content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SentPreview {
    attempt: Option<u64>,
    text_bytes: usize,
    /// UTF-16 code units, the unit of the browser string length clients use to
    /// verify that a delta continues their local copy.
    text_utf16: usize,
    status: Option<String>,
    /// Delta frames sent since the last full-text base; bounded so a client
    /// that missed the base can always resynchronize.
    delta_run: u32,
}

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
    /// Wire watermark of the newest broadcast frame.
    pub(super) sent: Option<SentPreview>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingPreview {
    turn_id: String,
    timeline_seq: u64,
    wire_event: Value,
}

impl PendingPreview {
    /// Applies the wire delta/snapshot decision at broadcast time. Encoding a
    /// withheld frame would advance the watermark past content that never
    /// reached the wire, so this runs only for frames that are actually sent.
    fn encode(&mut self, sent: &mut Option<SentPreview>) {
        let Some(payload) = self.wire_event.get_mut("payload") else {
            return;
        };
        if let Some(encoded) = encode_preview_payload(payload, sent) {
            *payload = encoded;
        }
    }

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
    // previews as stale. A settled Intermediate frame is also a one-shot state
    // transition: no later preview tick is guaranteed (a foreground tool may run
    // for minutes), so withholding it would leave clients on the last streaming
    // frame for the whole tool run. Track both before the payload is moved into
    // the wire event.
    let response_status = preview
        .get("response")
        .and_then(|response| response.get("status"))
        .and_then(Value::as_str);
    let final_preview = response_status == Some("final");
    let settled_intermediate = response_status == Some("intermediate");
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
        if final_preview || settled_intermediate || live_thinking {
            // Settled frames never wait: flush any withheld streaming frame
            // first so wire order stays monotonic, then publish the settled
            // frame immediately. Final must also precede the terminal turn
            // event; Intermediate must not wait for a next topic event that a
            // long-running foreground tool may not produce.
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
        for pending in due.iter_mut() {
            pending.encode(&mut throttle.sent);
        }
        due
    };
    drop(sessions);
    for pending in due {
        publish_core_semantic(state, session_id, pending.into_wire_event(session_id));
    }
}

/// Rewrites one preview frame for the wire.
///
/// Only the first frame of an attempt, a status change, or a text rewind
/// carries the full `text`, because that frame becomes the client's new base.
/// Every other frame carries just `text_delta` plus `text_base_len`, the UTF-16
/// offset the slice continues from. Nothing already delivered is re-sent, so a
/// turn's wire cost stays linear in the streamed text instead of quadratic.
fn encode_preview_payload(payload: &Value, sent: &mut Option<SentPreview>) -> Option<Value> {
    let response = payload.get("response")?;
    if !response.is_object() {
        // Retracted, cleared, or not-yet-visible previews carry no text.
        *sent = None;
        return None;
    }
    let attempt = response.get("attempt").and_then(Value::as_u64);
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string);
    let text = response
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text_bytes = text.len();
    let text_utf16 = text.encode_utf16().count();
    // `ResponsePreviewState::append` only appends within one attempt and clears
    // the text when the attempt changes, so equal attempt, equal status and a
    // non-shrinking length mean the client's local copy is still a prefix.
    let base = sent.as_ref().filter(|previous| {
        previous.attempt == attempt
            && previous.status == status
            && previous.text_bytes <= text_bytes
            && previous.delta_run < MAX_DELTA_RUN
    });
    let mut wire = payload.clone();
    let wire_response = wire.get_mut("response")?.as_object_mut()?;
    let mut delta_run = 0;
    if let Some(previous) = base {
        if let Some(delta) = text.get(previous.text_bytes..) {
            // A frame with unchanged text still clears `text`: re-sending the
            // cumulative copy is exactly what this encoding removes.
            wire_response.remove("text");
            wire_response.insert("text_delta".into(), Value::String(delta.to_string()));
            wire_response.insert("text_base_len".into(), json!(previous.text_utf16));
            delta_run = previous.delta_run + 1;
        }
    }
    *sent = Some(SentPreview {
        attempt,
        text_bytes,
        text_utf16,
        status,
        delta_run,
    });
    Some(wire)
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
        // An interruption frame is the only carrier of the Stopped /
        // network-error banner on the client. Stop usually lands right after
        // the last streaming chunk, so the cleared frame is withheld inside
        // the throttle window exactly when the terminal turn event races it;
        // dropping it there would silence the interruption notice forever.
        // It is safe to broadcast late: interrupt() keeps the partial answer
        // and clears the thinking tail, so the frame cannot resurrect a stale
        // strip and its revision is still the newest one.
        let interruption_frame = pending
            .wire_event
            .get("payload")
            .and_then(|payload| payload.get("interruption"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty());
        if !interruption_frame && active_turn_web_id.as_deref() != Some(pending.turn_id.as_str()) {
            throttle.pending = None;
            return;
        }
        let now = now_ms();
        if !interruption_frame
            && now.saturating_sub(throttle.window_started_ms) < PREVIEW_THROTTLE_WINDOW_MS
        {
            return;
        }
        throttle.last_flush_ms = now;
        let mut pending = throttle.pending.take();
        if let Some(frame) = pending.as_mut() {
            frame.encode(&mut throttle.sent);
        }
        pending
    };
    if let Some(pending) = due {
        publish_core_semantic(state, session_id, pending.into_wire_event(session_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(attempt: u64, revision: u64, text: &str, status: &str) -> Value {
        json!({
            "turn_id": "turn",
            "attempt": attempt,
            "revision": revision,
            "response": {
                "attempt": attempt,
                "revision": revision,
                "text": text,
                "status": status,
            },
        })
    }

    /// Applies one wire frame the way the browser does: a frame that still
    /// carries `text` is a new base, otherwise the slice is appended at the
    /// declared UTF-16 offset.
    fn apply(wire: &Value, local: &mut String) {
        let response = &wire["response"];
        if let Some(text) = response["text"].as_str() {
            *local = text.to_string();
            return;
        }
        let base = response["text_base_len"].as_u64().unwrap() as usize;
        assert_eq!(base, local.encode_utf16().count(), "delta base drifted");
        local.push_str(response["text_delta"].as_str().unwrap());
    }

    #[test]
    fn first_frame_is_the_base_and_later_frames_are_pure_deltas() {
        let mut sent = None;
        let first = encode_preview_payload(&frame(1, 1, "hello", "streaming"), &mut sent).unwrap();
        assert_eq!(first["response"]["text"], "hello");

        let second =
            encode_preview_payload(&frame(1, 2, "hello world", "streaming"), &mut sent).unwrap();
        assert!(
            second["response"].get("text").is_none(),
            "a delta frame must not repeat the cumulative text"
        );
        assert_eq!(second["response"]["text_delta"], " world");
        assert_eq!(second["response"]["text_base_len"], 5);
    }

    /// The regression this guards: a cumulative snapshot per frame costs O(n)
    /// bytes per frame and O(n^2) per turn. Pure deltas must stay a small
    /// fraction of that for the same 256 KiB of streamed text.
    #[test]
    fn pure_deltas_keep_a_turn_linear() {
        let chunk = "x".repeat(512);
        let mut sent = None;
        let mut full = String::new();
        let mut local = String::new();
        let mut wire_bytes = 0usize;
        let mut snapshot_bytes = 0usize;
        for revision in 1..=512u64 {
            full.push_str(&chunk);
            let wire = encode_preview_payload(&frame(1, revision, &full, "streaming"), &mut sent)
                .expect("streaming frame");
            wire_bytes += wire["response"].to_string().len();
            snapshot_bytes += full.len();
            apply(&wire, &mut local);
        }
        assert_eq!(local, full, "client copy must match the streamed text");
        assert!(
            wire_bytes * 10 < snapshot_bytes,
            "wire {wire_bytes} bytes vs snapshot semantics {snapshot_bytes} bytes"
        );
    }

    #[test]
    fn utf16_base_offsets_survive_cjk_and_astral_text() {
        let chunks = ["中文流式输出", "🙂🙂", "混合 ASCII 与 emoji 🙂 结尾"];
        let mut sent = None;
        let mut full = String::new();
        let mut local = String::new();
        let mut wire_bytes = 0usize;
        let mut snapshot_bytes = 0usize;
        for index in 0..4000u64 {
            full.push_str(chunks[(index % chunks.len() as u64) as usize]);
            let wire = encode_preview_payload(&frame(7, index + 1, &full, "streaming"), &mut sent)
                .expect("frame");
            wire_bytes += wire["response"].to_string().len();
            snapshot_bytes += full.encode_utf16().count();
            apply(&wire, &mut local);
        }
        assert_eq!(local, full, "multibyte text must round-trip exactly");
        assert!(
            wire_bytes * 5 < snapshot_bytes,
            "wire {wire_bytes} bytes vs snapshot semantics {snapshot_bytes} bytes"
        );
    }

    #[test]
    fn settled_frames_and_new_attempts_rebase_the_client() {
        let text = "y".repeat(4096);
        let mut sent = None;
        let first = encode_preview_payload(&frame(1, 1, &text, "streaming"), &mut sent).unwrap();
        assert_eq!(first["response"]["text"], text);

        let final_frame = encode_preview_payload(&frame(1, 2, &text, "final"), &mut sent).unwrap();
        assert_eq!(
            final_frame["response"]["text"], text,
            "a settled frame must be self-contained"
        );

        let retried =
            encode_preview_payload(&frame(2, 3, "short again", "streaming"), &mut sent).unwrap();
        assert_eq!(retried["response"]["text"], "short again");
    }

    /// A client that missed one frame (reconnect mid-attempt, dropped delivery)
    /// must not stay stuck in a broken delta chain: the bounded run guarantees a
    /// self-contained frame soon enough, and the client copy then matches again.
    #[test]
    fn a_missed_frame_recovers_within_the_bounded_delta_run() {
        let chunk = "y".repeat(64);
        let mut sent = None;
        let mut full = String::new();
        let mut wire = Vec::new();
        for revision in 1..=(MAX_DELTA_RUN * 2 + 2) as u64 {
            full.push_str(&chunk);
            let frame = encode_preview_payload(&frame(1, revision, &full, "streaming"), &mut sent)
                .expect("frame");
            wire.push(frame);
        }
        // The client applies every frame except the second one, i.e. it misses a
        // delta and its local copy stops matching the watermark.
        let mut local = String::new();
        let mut recovered_at = None;
        for (index, frame) in wire.iter().enumerate() {
            let response = &frame["response"];
            if let Some(text) = response["text"].as_str() {
                local = text.to_string();
                if index >= 1 && recovered_at.is_none() {
                    recovered_at = Some(index);
                }
                continue;
            }
            if index == 1 {
                continue;
            }
            let base = response["text_base_len"].as_u64().unwrap() as usize;
            if base != local.encode_utf16().count() {
                continue;
            }
            local.push_str(response["text_delta"].as_str().unwrap());
        }
        let recovered_at = recovered_at.expect("a bounded run must emit a full-text base");
        assert!(
            recovered_at <= 1 + MAX_DELTA_RUN as usize,
            "recovery took {recovered_at} frames, more than the bounded run"
        );
        assert_eq!(local, full, "recovered client must match the streamed text");
    }

    #[test]
    fn retracted_previews_reset_the_base() {
        let mut sent = None;
        encode_preview_payload(&frame(1, 1, &"z".repeat(4096), "streaming"), &mut sent).unwrap();
        let retracted = json!({"turn_id": "turn", "attempt": 1, "revision": 2, "response": null});
        assert!(encode_preview_payload(&retracted, &mut sent).is_none());
        assert!(
            sent.is_none(),
            "cleared previews must not keep a stale base"
        );
        let next = encode_preview_payload(&frame(1, 3, "fresh", "streaming"), &mut sent).unwrap();
        assert_eq!(next["response"]["text"], "fresh");
    }
}

//! Bounded ephemeral projection: never append streaming chunks to chat history.
use super::*;
use agent_core::CoreTopicEvent;

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
    let web_turn_id = turn.turn_id.clone();
    turn.preview = Some(event.payload.clone());
    drop(sessions);
    publish_core_semantic(
        state,
        session_id,
        WireEvent::CoreTopic {
            turn_id: Some(web_turn_id),
            turn_event_id: None,
            event: event.wire_payload(),
        },
    );
}

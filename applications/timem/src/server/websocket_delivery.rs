//! Product-side WebSocket command delivery.
//!
//! Wire framing lives in `timem_http_websocket`; this module adapts decoded
//! browser commands to authoritative Host/Core operations without changing
//! command ordering, deduplication, or disconnect ownership.

use super::*;

/// Runs one authenticated browser connection.
///
/// Functional scope:
/// - establish a snapshot/event-sequence baseline;
/// - accept bounded decoded commands and preserve per-Session ordering;
/// - forward direct results, ACKs, and semantic broadcasts;
/// - recover broadcast lag with a fresh authoritative snapshot.
///
/// Constraints:
/// - no replay journal or persistent browser outbox is introduced;
/// - disconnecting the socket does not cancel commands already accepted by the
///   Host, so the detached worker continues draining its bounded queue;
/// - this adapter does not infer or own Session/Turn lifecycle state.
pub(super) async fn websocket_session(socket: WebSocket, state: AppState, port: u16) {
    let (mut sender, mut receiver) = split_json_websocket(socket);
    // Subscribe before taking the snapshot. The sequence baseline makes every
    // buffered event at or below it redundant with the full snapshot.
    let mut events = state.events.subscribe();
    let mut last_sent_event_seq = state.semantic_delivery.baseline();
    if send_event(
        &mut sender,
        &WireEvent::Hello {
            snapshot: snapshot_for(&state, port),
            event_cursor: last_sent_event_seq,
            event_replay_floor: last_sent_event_seq,
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let (command_tx, command_rx) =
        tokio_mpsc::channel::<BrowserCommand>(BROWSER_COMMAND_QUEUE_CAPACITY);
    let (command_result_tx, mut command_result_rx) = tokio_mpsc::unbounded_channel();
    let command_state = state.clone();
    let command_worker = tokio::spawn(run_ordered_blocking_queue(
        command_rx,
        command_result_tx,
        move |command| Ok(execute_browser_command(&command_state, port, command)),
    ));
    loop {
        tokio::select! {
            maybe_command = receiver.receive::<BrowserCommand>(MAX_BROWSER_COMMAND_BYTES) => {
                match maybe_command {
                    InboundJson::Item(mut command) => {
                                command.accepted_at_ms = now_ms();
                                let (trace_kind, trace_session_id) = client_command_trace_fields(&command.command);
                                if trace_kind != "other" {
                                    state.runtime_log.record("server_received", json!({
                                        "kind": trace_kind,
                                        "session_id": trace_session_id,
                                        "command_id": command.command_id,
                                        "browser_sent_at_ms": command.performance_sent_at_ms,
                                    }));
                                }
                                // Acceptance and mem switching share this barrier. A switch
                                // cannot advance the epoch between stamping and queueing a
                                // command, and the non-Send guard is dropped before any await.
                                let enqueue_outcome = match state.mem_epoch.read() {
                                    Err(_) => BrowserCommandEnqueueOutcome::Rejected {
                                        command_id: command.command_id.clone(),
                                        error: "mem_epoch_poisoned".to_string(),
                                    },
                                    Ok(epoch) => {
                                        command.accepted_mem_epoch = *epoch;
                                        let mut rejection = None;
                                        let mut cached = None;
                                        if let Some(command_id) = command.command_id.as_deref() {
                                            if let Err(error) = validate_command_id(command_id) {
                                                rejection = Some(error);
                                            } else {
                                                match reserve_command_dedup(&state, command_id) {
                                                    Ok(Some(CommandDedupState::Accepted))
                                                        if command.command.waits_for_core_acceptance() =>
                                                    {
                                                        // An accepted TurnSubmit is a durable
                                                        // intent, not proof of Core delivery.
                                                        // Re-drive it; Core deduplicates by ID.
                                                    }
                                                    Ok(Some(previous)) => {
                                                        cached = Some((command_id.to_string(), previous));
                                                    }
                                                    Ok(None) => {}
                                                    Err(error) => rejection = Some(error),
                                                }
                                            }
                                        }
                                        if let Some(error) = rejection {
                                            BrowserCommandEnqueueOutcome::Rejected {
                                                command_id: command.command_id.clone(),
                                                error,
                                            }
                                        } else if let Some((command_id, state)) = cached {
                                            BrowserCommandEnqueueOutcome::Cached { command_id, state: Box::new(state) }
                                        } else {
                                            enqueue_reserved_browser_command(&state, &command_tx, command)
                                        }
                                    }
                                };
                                match enqueue_outcome {
                                    BrowserCommandEnqueueOutcome::Accepted(Some(command_id)) => {
                                        if send_event(&mut sender, &command_ack(&command_id, CommandAckStatus::Accepted, None)).await.is_err() { break; }
                                    }
                                    BrowserCommandEnqueueOutcome::Accepted(None) => {}
                                    BrowserCommandEnqueueOutcome::Cached { command_id, state: cached } => {
                                        if send_cached_command_state(&mut sender, &command_id, *cached).await.is_err() { break; }
                                    }
                                    BrowserCommandEnqueueOutcome::Rejected { command_id: Some(command_id), error } => {
                                        if send_event(&mut sender, &command_ack(&command_id, CommandAckStatus::Rejected, Some(error))).await.is_err() { break; }
                                    }
                                    BrowserCommandEnqueueOutcome::Rejected { command_id: None, error } => {
                                        if send_event(&mut sender, &WireEvent::HostError { message: error }).await.is_err() { break; }
                                    }
                                }
                    }
                    InboundJson::TooLarge => {
                        if send_event(&mut sender, &WireEvent::HostError { message: "browser_command_too_large".to_string() }).await.is_err() {
                            break;
                        }
                    }
                    InboundJson::InvalidJson(error) => {
                        if send_event(&mut sender, &WireEvent::HostError { message: format!("invalid_browser_command:{error}") }).await.is_err() {
                            break;
                        }
                    }
                    InboundJson::Closed => break,
                }
            }
            result = command_result_rx.recv() => {
                match result {
                    Some(Ok(completion)) => {
                        if let Some(event) = completion.event {
                            if send_event(&mut sender, &event).await.is_err() { break; }
                        }
                        if let Some(ack) = completion.ack {
                            if send_event(&mut sender, &ack).await.is_err() { break; }
                        }
                        if let Some(error) = completion.legacy_error {
                            if send_event(&mut sender, &WireEvent::HostError { message: error }).await.is_err() { break; }
                        }
                    }
                    Some(Err(error)) => if send_event(&mut sender, &WireEvent::HostError { message: error }).await.is_err() {
                        break;
                    },
                    None => {
                        let _ = send_event(&mut sender, &WireEvent::HostError { message: "browser_command_worker_stopped".to_string() }).await;
                        break;
                    }
                }
            }
            event = events.recv() => match event {
                Ok(WireEvent::SemanticEvent { event_seq, .. }) if event_seq <= last_sent_event_seq => {}
                Ok(event) => {
                    let sent_seq = match &event {
                        WireEvent::SemanticEvent { event_seq, .. } => Some(*event_seq),
                        WireEvent::Hello { event_cursor, .. } => Some(*event_cursor),
                        _ => None,
                    };
                    if send_event(&mut sender, &event).await.is_err() { break; }
                    if let Some(sent_seq) = sent_seq {
                        last_sent_event_seq = sent_seq;
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // There is deliberately no replay journal. A lagging client
                    // pays the uncommon recovery cost and establishes a fresh
                    // snapshot baseline while connected clients keep a zero-I/O
                    // semantic event path.
                    last_sent_event_seq = state.semantic_delivery.baseline();
                    let hello = WireEvent::Hello {
                        snapshot: snapshot_for(&state, port),
                        event_cursor: last_sent_event_seq,
                        event_replay_floor: last_sent_event_seq,
                    };
                    if send_event(&mut sender, &hello).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    drop(command_tx);
    // Dropping a JoinHandle detaches the worker. It must drain commands that this
    // connection already received even when the browser disconnects before ACK.
    drop(command_worker);
}

/// Direct completion returned by the ordered command worker.
///
/// Visibility is restricted to the parent `server` module so existing boundary
/// tests can assert ACK/result behavior without exposing a crate-level API.
#[derive(Debug)]
pub(super) struct BrowserCommandCompletion {
    pub(super) event: Option<WireEvent>,
    pub(super) ack: Option<WireEvent>,
    pub(super) legacy_error: Option<String>,
}

enum BrowserCommandEnqueueOutcome {
    Accepted(Option<String>),
    Cached {
        command_id: String,
        state: Box<CommandDedupState>,
    },
    Rejected {
        command_id: Option<String>,
        error: String,
    },
}

fn enqueue_reserved_browser_command(
    state: &AppState,
    command_tx: &tokio_mpsc::Sender<BrowserCommand>,
    mut command: BrowserCommand,
) -> BrowserCommandEnqueueOutcome {
    if let Some((key, lane)) = command_lane(state, &command.command) {
        match lane.issue() {
            Ok(ticket) => {
                command.accepted_lane = Some(AcceptedCommandLane {
                    key,
                    lane,
                    lanes: Arc::clone(&state.command_lanes),
                    ticket,
                })
            }
            Err(error) => {
                return reject_reserved_enqueue(state, command, error);
            }
        }
    }
    let command_id = command.command_id.clone();
    match command_tx.try_send(command) {
        Ok(()) => BrowserCommandEnqueueOutcome::Accepted(command_id),
        Err(error) => {
            let (message, command) = match error {
                tokio_mpsc::error::TrySendError::Full(command) => {
                    ("browser_command_queue_full", command)
                }
                tokio_mpsc::error::TrySendError::Closed(command) => {
                    ("browser_command_worker_stopped", command)
                }
            };
            reject_reserved_enqueue(state, command, message.to_string())
        }
    }
}

fn reject_reserved_enqueue(
    state: &AppState,
    command: BrowserCommand,
    mut error: String,
) -> BrowserCommandEnqueueOutcome {
    if let Some(accepted) = command.accepted_lane.as_ref() {
        let _ = accepted.lane.skip(accepted.ticket);
    }
    if let Some(command_id) = command.command_id.as_deref() {
        if let Err(persist_error) = finish_command_dedup(
            state,
            command_id,
            CommandDedupState::Rejected {
                error: error.clone(),
            },
        ) {
            error = persist_error;
        }
    }
    BrowserCommandEnqueueOutcome::Rejected {
        command_id: command.command_id,
        error,
    }
}

/// Executes one already-accepted browser command under the original Host
/// barriers, lane ordering, MEM epoch, and correlation rules.
///
/// This function performs no transport I/O. Sensitive direct replies are never
/// retained in the dedup cache, and Core-acceptance commands remain nonterminal
/// until the parent callback confirms authoritative intake.
pub(super) fn execute_browser_command(
    state: &AppState,
    port: u16,
    browser_command: BrowserCommand,
) -> BrowserCommandCompletion {
    let BrowserCommand {
        command_id,
        command,
        accepted_mem_epoch,
        accepted_lane,
        accepted_at_ms,
        performance_sent_at_ms,
    } = browser_command;
    let execute_started_ms = now_ms();
    let (trace_kind, trace_session_id) = client_command_trace_fields(&command);
    if trace_kind != "other" {
        state.runtime_log.record(
            "server_execute_start",
            json!({
                "kind": trace_kind,
                "session_id": trace_session_id,
                "command_id": command_id,
                "queue_ms": execute_started_ms.saturating_sub(accepted_at_ms),
                "browser_sent_at_ms": performance_sent_at_ms,
            }),
        );
    }
    let sensitive_result = command.result_is_sensitive();
    let direct_result = command.result_is_direct();
    let waits_for_core_acceptance = command.waits_for_core_acceptance();
    let mutation_lane = command.mutation_lane();
    let _global_write_guard;
    let _global_read_guard;
    if mutation_lane.is_some() && command.uses_global_mutation_barrier() {
        _global_write_guard = match state.command_global_barrier.write() {
            Ok(guard) => Some(guard),
            Err(_) => {
                return rejected_browser_command(
                    state,
                    command_id.as_deref(),
                    "command_global_barrier_poisoned".to_string(),
                )
            }
        };
        _global_read_guard = None;
    } else if mutation_lane.is_some() {
        _global_read_guard = match state.command_global_barrier.read() {
            Ok(guard) => Some(guard),
            Err(_) => {
                return rejected_browser_command(
                    state,
                    command_id.as_deref(),
                    "command_global_barrier_poisoned".to_string(),
                )
            }
        };
        _global_write_guard = None;
    } else {
        _global_write_guard = None;
        _global_read_guard = None;
    }
    let _lane_guard = match accepted_lane
        .as_ref()
        .map(|accepted| accepted.lane.enter(accepted.ticket))
        .transpose()
    {
        Ok(guard) => guard,
        Err(error) => return rejected_browser_command(state, command_id.as_deref(), error),
    };
    let is_mem_switch = matches!(command, ClientCommand::MemSwitch { .. });
    let mem_epoch_guard = if is_mem_switch {
        None
    } else {
        state.mem_epoch.read().ok()
    };
    if !is_mem_switch
        && mem_epoch_guard
            .as_deref()
            .is_none_or(|epoch| *epoch != accepted_mem_epoch)
    {
        return rejected_browser_command(
            state,
            command_id.as_deref(),
            "command_mem_epoch_stale".to_string(),
        );
    }
    let handled = handle_command_with_id(state, port, command_id.as_deref(), command);
    if trace_kind != "other" {
        state.runtime_log.record(
            "server_execute_handled",
            json!({
                "kind": trace_kind,
                "session_id": trace_session_id,
                "command_id": command_id,
                "execute_ms": now_ms().saturating_sub(execute_started_ms),
                "ok": handled.is_ok(),
            }),
        );
    }
    match handled {
        Ok(event) => {
            let direct_event = direct_result.then(|| event.clone()).flatten();
            if let Some(command_id) = command_id.as_deref() {
                if sensitive_result {
                    if let Ok(mut cache) = state.command_dedup.lock() {
                        cache.unreserve(command_id);
                    }
                    return BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(command_id, CommandAckStatus::Committed, None)),
                        legacy_error: None,
                    };
                }
                if waits_for_core_acceptance {
                    // The user entry is durable, but the command is not terminal
                    // until Core reports that it dequeued the matching intent.
                    return BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(command_id, CommandAckStatus::Accepted, None)),
                        legacy_error: None,
                    };
                }
                match finish_command_dedup(
                    state,
                    command_id,
                    CommandDedupState::Committed {
                        serialized_event: direct_event.as_ref().and_then(durable_command_result),
                        event: direct_event.clone().map(Box::new),
                    },
                ) {
                    Ok(()) => BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(command_id, CommandAckStatus::Committed, None)),
                        legacy_error: None,
                    },
                    Err(error) => BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(
                            command_id,
                            CommandAckStatus::Accepted,
                            Some(format!("command_terminal_persist_pending:{error}")),
                        )),
                        legacy_error: None,
                    },
                }
            } else {
                BrowserCommandCompletion {
                    event: direct_event,
                    ack: None,
                    legacy_error: None,
                }
            }
        }
        Err(error) => {
            if let Some(command_id) = command_id.as_deref() {
                let persist_error = finish_command_dedup(
                    state,
                    command_id,
                    CommandDedupState::Rejected {
                        error: error.clone(),
                    },
                )
                .err();
                BrowserCommandCompletion {
                    event: None,
                    ack: Some(command_ack(
                        command_id,
                        CommandAckStatus::Rejected,
                        Some(persist_error.unwrap_or(error)),
                    )),
                    legacy_error: None,
                }
            } else {
                BrowserCommandCompletion {
                    event: None,
                    ack: None,
                    legacy_error: Some(error),
                }
            }
        }
    }
}

fn rejected_browser_command(
    state: &AppState,
    command_id: Option<&str>,
    error: String,
) -> BrowserCommandCompletion {
    if let Some(command_id) = command_id {
        let persisted = finish_command_dedup(
            state,
            command_id,
            CommandDedupState::Rejected {
                error: error.clone(),
            },
        )
        .err();
        BrowserCommandCompletion {
            event: None,
            ack: Some(command_ack(
                command_id,
                CommandAckStatus::Rejected,
                Some(persisted.unwrap_or(error)),
            )),
            legacy_error: None,
        }
    } else {
        BrowserCommandCompletion {
            event: None,
            ack: None,
            legacy_error: Some(error),
        }
    }
}

/// Returns the bounded FIFO lane for a typed mutation scope.
///
/// Session mutations share a Session lane, global mutations share the global
/// barrier, and unrelated Sessions may proceed independently.
pub(super) fn command_lane(
    state: &AppState,
    command: &ClientCommand,
) -> Option<(String, Arc<TicketCommandLane>)> {
    command.mutation_lane().and_then(|key| {
        state.command_lanes.lock().ok().map(|mut lanes| {
            let lane = lanes.entry(key.clone()).or_default().clone();
            (key, lane)
        })
    })
}

fn durable_command_result(event: &WireEvent) -> Option<Value> {
    if matches!(
        event,
        WireEvent::SessionApiKeyRevealed { .. }
            | WireEvent::McpServerSecretsRevealed { .. }
            | WireEvent::ModelEndpointSecretRevealed { .. }
    ) {
        return None;
    }
    let value = serde_json::to_value(event).ok()?;
    (serde_json::to_vec(&value).ok()?.len() <= MAX_COMMAND_DEDUP_RESULT_BYTES).then_some(value)
}

/// Reserves a bounded process-local correlation id.
///
/// Accepted entries are never evicted while in flight. Capacity exhaustion is
/// explicit rather than growing memory or creating a persistent command ledger.
pub(super) fn reserve_command_dedup(
    state: &AppState,
    command_id: &str,
) -> Result<Option<CommandDedupState>, String> {
    let mut cache = state
        .command_dedup
        .lock()
        .map_err(|_| "command_dedup_poisoned".to_string())?;
    if !cache.records.contains_key(command_id)
        && cache.records.len() >= COMMAND_DEDUP_CAPACITY
        && cache
            .records
            .values()
            .all(|record| matches!(record, CommandDedupState::Accepted))
    {
        // Accepted entries cannot be evicted safely while this Host process is
        // still handling them. Reject new correlation ids instead of growing
        // the in-memory cache without bound.
        return Err("command_dedup_capacity_exhausted".to_string());
    }
    Ok(cache.reserve(command_id))
}

/// Finalizes the bounded process-local command correlation record.
///
/// This is shared with the Core-acceptance callback in the parent Host module:
/// a queued browser command becomes terminal only after Core confirms intake.
/// The function must remain in-memory, bounded, and non-blocking; it must not
/// create a durable per-command ledger or reinterpret Core lifecycle state.
pub(super) fn finish_command_dedup(
    state: &AppState,
    command_id: &str,
    terminal: CommandDedupState,
) -> Result<(), String> {
    let mut cache = state
        .command_dedup
        .lock()
        .map_err(|_| "command_dedup_poisoned".to_string())?;
    cache.finish(command_id, terminal);
    Ok(())
}

/// Builds the stable browser command acknowledgement envelope.
///
/// ACK status describes command delivery only. It must never be treated as a
/// replacement for the authoritative Session/Turn projection emitted by Core.
pub(super) fn command_ack(
    command_id: &str,
    status: CommandAckStatus,
    error: Option<String>,
) -> WireEvent {
    WireEvent::CommandAck {
        command_id: command_id.to_string(),
        status,
        error,
    }
}

fn validate_command_id(command_id: &str) -> Result<(), String> {
    if command_id.is_empty() {
        return Err("command_id_empty".to_string());
    }
    if command_id.len() > MAX_COMMAND_ID_BYTES {
        return Err("command_id_too_large".to_string());
    }
    if command_id.chars().any(char::is_control) {
        return Err("command_id_invalid".to_string());
    }
    Ok(())
}

async fn send_cached_command_state(
    sender: &mut JsonWebSocketSender,
    command_id: &str,
    state: CommandDedupState,
) -> Result<(), ()> {
    match state {
        CommandDedupState::Accepted => {
            send_event(
                sender,
                &command_ack(command_id, CommandAckStatus::Accepted, None),
            )
            .await
        }
        CommandDedupState::Committed {
            event,
            serialized_event,
        } => {
            if let Some(event) = event {
                send_event(sender, &event).await?;
            } else if let Some(event) = serialized_event {
                sender.send(&event).await?;
            }
            send_event(
                sender,
                &command_ack(command_id, CommandAckStatus::Committed, None),
            )
            .await
        }
        CommandDedupState::Rejected { error } => {
            send_event(
                sender,
                &command_ack(command_id, CommandAckStatus::Rejected, Some(error)),
            )
            .await
        }
    }
}

/// Drains accepted commands in FIFO order on the blocking pool.
///
/// The result channel is best-effort because the socket may already be gone;
/// accepted Host mutations still run to completion to preserve ownership and
/// exactly-once correlation semantics within the current process.
pub(super) async fn run_ordered_blocking_queue<T, R, F>(
    mut receiver: tokio_mpsc::Receiver<T>,
    results: tokio_mpsc::UnboundedSender<Result<R, String>>,
    handler: F,
) where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Result<R, String> + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    while let Some(item) = receiver.recv().await {
        let handler = handler.clone();
        let result = tokio::task::spawn_blocking(move || handler(item))
            .await
            .unwrap_or_else(|error| Err(format!("browser_command_worker_failed:{error}")));
        // The WebSocket may disappear after accepting a command. Continue
        // draining the queue so accepted mutations are not cancelled with it.
        let _ = results.send(result);
    }
}

async fn send_event(sender: &mut JsonWebSocketSender, event: &WireEvent) -> Result<(), ()> {
    sender.send(event).await
}

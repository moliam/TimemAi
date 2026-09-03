//! Bounded-lifetime FIFO serialization for product-side browser mutations.
//!
//! A lane orders already accepted commands for one mutation scope. It owns no
//! Session or Turn semantics: tickets only serialize Host delivery, and the
//! RAII guard advances the queue even when command execution returns early.

use std::{
    collections::BTreeSet,
    sync::{Condvar, Mutex},
};

#[derive(Debug, Default)]
pub(super) struct TicketCommandLane {
    state: Mutex<TicketCommandLaneState>,
    ready: Condvar,
}

#[derive(Debug, Default)]
struct TicketCommandLaneState {
    next_ticket: u64,
    serving_ticket: u64,
    skipped_tickets: BTreeSet<u64>,
    active: bool,
}

impl TicketCommandLane {
    pub(super) fn issue(&self) -> Result<u64, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "command_lane_poisoned".to_string())?;
        let ticket = state.next_ticket;
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| "command_lane_ticket_exhausted".to_string())?;
        Ok(ticket)
    }

    pub(super) fn enter(&self, ticket: u64) -> Result<TicketCommandLaneGuard<'_>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "command_lane_poisoned".to_string())?;
        while state.serving_ticket != ticket {
            state = self
                .ready
                .wait(state)
                .map_err(|_| "command_lane_poisoned".to_string())?;
        }
        state.active = true;
        Ok(TicketCommandLaneGuard { lane: self })
    }

    pub(super) fn skip(&self, ticket: u64) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "command_lane_poisoned".to_string())?;
        state.skipped_tickets.insert(ticket);
        if !state.active {
            skip_cancelled_tickets(&mut state);
        }
        self.ready.notify_all();
        Ok(())
    }

    /// A lane can be reclaimed only after every issued ticket has settled and
    /// no skipped ticket remains to be observed by a future guard.
    pub(super) fn is_idle(&self) -> bool {
        self.state
            .lock()
            .map(|state| {
                !state.active
                    && state.serving_ticket == state.next_ticket
                    && state.skipped_tickets.is_empty()
            })
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub(super) fn serving_ticket(&self) -> u64 {
        self.state.lock().unwrap().serving_ticket
    }

    #[cfg(test)]
    pub(super) fn next_ticket(&self) -> u64 {
        self.state.lock().unwrap().next_ticket
    }
}

fn advance_ticket_lane(state: &mut TicketCommandLaneState) {
    state.active = false;
    state.serving_ticket = state.serving_ticket.saturating_add(1);
    skip_cancelled_tickets(state);
}

fn skip_cancelled_tickets(state: &mut TicketCommandLaneState) {
    while state.skipped_tickets.remove(&state.serving_ticket) {
        state.serving_ticket = state.serving_ticket.saturating_add(1);
    }
}

pub(super) struct TicketCommandLaneGuard<'a> {
    lane: &'a TicketCommandLane,
}

impl Drop for TicketCommandLaneGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.lane.state.lock() {
            advance_ticket_lane(&mut state);
            self.lane.ready.notify_all();
        }
    }
}

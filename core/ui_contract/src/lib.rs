//! UI-neutral semantic contracts shared by Core, Bridges, and Interfaces.
//!
//! This crate owns data shapes only. Lifecycle transitions, token allocation,
//! transport metadata, and presentation remain in their respective layers.

pub mod commands;
pub mod message_fifo;
pub mod preferences;
pub mod projections;

use serde::{Deserialize, Serialize};

/// Stable identity for one accepted Session turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TurnToken {
    pub session_id: String,
    pub turn_id: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnInputAdmission {
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnActivity {
    Running,
    WaitingModel { round: u32 },
    WaitingUser,
    RunningTools,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnProjectionOutcome {
    Completed,
    Cancelled,
    Failed { code: String },
    Interrupted { code: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveTurnProjection {
    pub token: TurnToken,
    pub stop_requested: bool,
    pub input_admission: TurnInputAdmission,
    pub activity: TurnActivity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinishedTurnProjection {
    pub token: TurnToken,
    pub outcome: TurnProjectionOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TurnProjection {
    Active(ActiveTurnProjection),
    Finished(FinishedTurnProjection),
}

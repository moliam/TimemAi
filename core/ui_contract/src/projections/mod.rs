use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Aggregate worker activity projected without exposing scheduler internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreGlobalWorkerStatus {
    pub working_worker_count: usize,
    pub session_working_worker_count: usize,
}

impl CoreGlobalWorkerStatus {
    pub fn new(working_worker_count: usize) -> Self {
        Self {
            working_worker_count,
            session_working_worker_count: working_worker_count,
        }
    }

    pub fn with_session_working_worker_count(
        working_worker_count: usize,
        session_working_worker_count: usize,
    ) -> Self {
        Self {
            working_worker_count,
            session_working_worker_count,
        }
    }
}

/// Stable UI-neutral identity for a worker in one Session and Context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSessionWorkerIdentity {
    pub session_id: String,
    pub context_id: String,
    pub worker_id: String,
    pub display_name: String,
    pub ordinal: u32,
    pub parent_worker_id: Option<String>,
}

impl CoreSessionWorkerIdentity {
    pub fn new(
        session_id: impl Into<String>,
        ordinal: u32,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
    ) -> Self {
        let session_id = session_id.into();
        Self::new_scoped(
            session_id.clone(),
            "context_0",
            session_id,
            ordinal,
            display_name,
            parent_worker_id,
        )
    }

    pub fn new_scoped(
        session_id: impl Into<String>,
        context_id: impl Into<String>,
        worker_id: impl Into<String>,
        ordinal: u32,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            context_id: context_id.into(),
            worker_id: worker_id.into(),
            display_name: session_worker_default_display_name(ordinal, display_name),
            ordinal,
            parent_worker_id,
        }
    }

    pub fn rename(&mut self, display_name: impl Into<String>) {
        let display_name = display_name.into();
        if !display_name.trim().is_empty() {
            self.display_name = display_name.trim().to_string();
        }
    }
}

/// Application lifecycle state for a managed Session worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreSessionWorkerLifecycleState {
    Running,
    Stopping,
    Stopped,
}

/// UI-neutral status of one managed Session worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSessionWorkerStatus {
    pub identity: CoreSessionWorkerIdentity,
    pub state: CoreSessionWorkerLifecycleState,
}

pub fn session_worker_default_display_name(ordinal: u32, requested: Option<String>) -> String {
    requested
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("ID{ordinal}"))
}

/// Host-neutral workspace/configuration values associated with one worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSessionWorkerWorkspace {
    pub current_dir: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub audit_file: PathBuf,
    pub runtime: String,
    pub run_bash_target: String,
    pub env: BTreeMap<String, String>,
    pub workspace_dirs: Vec<PathBuf>,
}

impl CoreSessionWorkerWorkspace {
    pub fn new(
        data_dir: impl Into<PathBuf>,
        audit_file: impl Into<PathBuf>,
        runtime: impl Into<String>,
        run_bash_target: impl Into<String>,
    ) -> Self {
        Self {
            current_dir: None,
            data_dir: data_dir.into(),
            audit_file: audit_file.into(),
            runtime: runtime.into(),
            run_bash_target: run_bash_target.into(),
            env: BTreeMap::new(),
            workspace_dirs: Vec::new(),
        }
    }
}

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

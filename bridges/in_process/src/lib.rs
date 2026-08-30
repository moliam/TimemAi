//! Zero-transport Bridge for hosts embedded in the same process as Timem Core.

use agent_core::{
    AgentCore, ModelClient, ModelServiceConfig, RuntimeProfiler, TurnInput, TurnOutcome, TurnUi,
};

/// Runs one synchronous Turn through direct calls and callbacks.
pub fn run_turn(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    input: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
) -> TurnOutcome {
    agent_core::run_session_turn(core, config, input, ui, profiler)
}

/// Runs one synchronous Turn with a caller-supplied model client.
pub fn run_turn_with_model_client(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    input: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
) -> TurnOutcome {
    agent_core::run_session_turn_with_model_client(core, config, input, ui, profiler, model_client)
}

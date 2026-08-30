//! Zero-transport Bridge for Interfaces embedded in the same process as Timem Core.
//!
//! It adapts typed calls to the Session-owned orchestration boundary and is reusable by Shell,
//! future Rust-native desktop Interfaces, embedded hosts, and tests.

pub use timem_session::agent_api;
pub use timem_ui_contract as ui_contract;

use agent_api::{
    AgentCore, ModelClient, ModelServiceConfig, RuntimeProfiler, TurnInput, TurnOutcome, TurnUi,
};

pub fn run_turn(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    input: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
) -> TurnOutcome {
    timem_session::run_synchronous_turn(core, config, input, ui, profiler)
}

pub fn run_turn_with_model_client(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    input: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
) -> TurnOutcome {
    timem_session::run_synchronous_turn_with_model_client(
        core,
        config,
        input,
        ui,
        profiler,
        model_client,
    )
}

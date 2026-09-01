use std::path::{Path, PathBuf};
use timem_in_process::agent_api::{
    ApiProtocol, CoreProfile, CoreTopicEvent, LlmResponse, ModelClient, ModelServiceConfig,
    NoopTurnUi, OpenAiCompatibleOptions, ResponseProtocolKind, TurnInput, TurnProjection, TurnUi,
    UsageStats,
};

struct FinalAnswerModel {
    prompts: Vec<String>,
}

impl ModelClient for FinalAnswerModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.prompts.push(prompt.to_string());
        Ok(LlmResponse {
            content: r#"{"status":"ALL_FINISHED","final_answer":"bridge complete"}"#.to_string(),
            tool_calls: Vec::new(),
            model_name: "test-model".to_string(),
            usage: UsageStats {
                llm_calls: 1,
                prompt_tokens: 12,
                completion_tokens: 3,
                total_tokens: 15,
                ..UsageStats::zero()
            },
            truncated: false,
        })
    }
}

#[derive(Default)]
struct RecordingUi {
    projections: Vec<TurnProjection>,
    model_responses: Vec<String>,
    topic_names: Vec<String>,
}

impl TurnUi for RecordingUi {
    fn on_turn_projection(&mut self, projection: &TurnProjection) {
        self.projections.push(projection.clone());
    }

    fn on_model_response(&mut self, _round: u32, _usage: &UsageStats, content: &str) {
        self.model_responses.push(content.to_string());
    }

    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        self.topic_names
            .extend(events.iter().map(|event| event.topic.name.clone()));
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "timem_in_process_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn config() -> ModelServiceConfig {
    ModelServiceConfig {
        model: "test-model".to_string(),
        base_url: "http://127.0.0.1:9/v1".to_string(),
        api_key: "dummy".to_string(),
        http_headers: Default::default(),
        request_fields: Default::default(),
        timeout_secs: 1,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol: ApiProtocol::OpenAiCompatible,
        response_protocol: ResponseProtocolKind::Json,
        interaction: Default::default(),
        openai_compatible: OpenAiCompatibleOptions::default(),
        http_transport: Default::default(),
    }
}

fn run_test_turn(
    root: &Path,
    input: &str,
    session: &str,
    ui: &mut dyn TurnUi,
    model: &mut dyn ModelClient,
) -> timem_in_process::agent_api::TurnOutcome {
    let mut core = timem_in_process::agent_api::AgentCore::new(
        "STATIC {{ response_protocol }} {{ capability_catalog }}",
        CoreProfile {
            model: "test-model".to_string(),
        },
        root,
    );
    let mut config = config();
    timem_in_process::run_turn_with_model_client(
        &mut core,
        &mut config,
        TurnInput {
            input,
            session,
            audit_file: &root.join("audit.json"),
            runtime: "test-interface",
            run_bash_target: "test-host",
            additional_context: Some("bridge context"),
        },
        ui,
        None,
        model,
    )
}

#[test]
fn direct_turn_forwards_input_projection_topics_and_outcome_without_transport() {
    let root = temp_dir("turn");
    let mut model = FinalAnswerModel {
        prompts: Vec::new(),
    };
    let mut ui = RecordingUi::default();
    let outcome = run_test_turn(&root, "bridge task", "session_a", &mut ui, &mut model);

    assert_eq!(outcome.text, "bridge complete");
    assert_eq!(outcome.stats.total_tokens, 15);
    assert_eq!(model.prompts.len(), 1);
    assert!(model.prompts[0].contains("bridge task"));
    assert!(model.prompts[0].contains("bridge context"));
    assert!(matches!(
        ui.projections.first(),
        Some(TurnProjection::Active(_))
    ));
    assert!(matches!(
        ui.projections.last(),
        Some(TurnProjection::Finished(_))
    ));
    assert_eq!(ui.model_responses.len(), 1);
    assert!(ui.model_responses[0].contains("bridge complete"));
    assert!(!ui.topic_names.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_turn_accepts_the_core_noop_ui_contract() {
    let root = temp_dir("noop");
    let mut model = FinalAnswerModel {
        prompts: Vec::new(),
    };
    let outcome = run_test_turn(
        &root,
        "headless task",
        "session_b",
        &mut NoopTurnUi,
        &mut model,
    );
    assert_eq!(outcome.text, "bridge complete");
    std::fs::remove_dir_all(root).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
enum ProjectionSemantics {
    Active {
        session_id: String,
        stop_requested: bool,
        input_admission: String,
        activity: String,
    },
    Finished {
        session_id: String,
        outcome: String,
    },
}

fn stable_prompt_semantics(prompt: &str) -> String {
    prompt
        .lines()
        .map(|line| {
            if line.starts_with("[BEGIN DELTA delta_id:") {
                "[BEGIN DELTA <runtime-generated>]".to_string()
            } else if line.starts_with("[BEGIN TURN turn_id:") {
                "[BEGIN TURN <runtime-generated>]".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_single_turn_identity(projections: &[TurnProjection]) {
    let mut identity: Option<(&str, &str, u64)> = None;
    for projection in projections {
        let token = match projection {
            TurnProjection::Active(active) => &active.token,
            TurnProjection::Finished(finished) => &finished.token,
        };
        let current = (
            token.session_id.as_str(),
            token.turn_id.as_str(),
            token.epoch,
        );
        if let Some(expected) = identity {
            assert_eq!(
                current, expected,
                "one call changed Turn identity mid-stream"
            );
        } else {
            identity = Some(current);
        }
    }
    assert!(identity.is_some(), "one call emitted no Turn projection");
}

fn projection_semantics(projections: &[TurnProjection]) -> Vec<ProjectionSemantics> {
    projections
        .iter()
        .map(|projection| match projection {
            TurnProjection::Active(active) => ProjectionSemantics::Active {
                session_id: active.token.session_id.clone(),
                stop_requested: active.stop_requested,
                input_admission: format!("{:?}", active.input_admission),
                activity: format!("{:?}", active.activity),
            },
            TurnProjection::Finished(finished) => ProjectionSemantics::Finished {
                session_id: finished.token.session_id.clone(),
                outcome: format!("{:?}", finished.outcome),
            },
        })
        .collect()
}

#[test]
fn in_process_bridge_is_semantically_equivalent_to_direct_core_call() {
    let root = temp_dir("equivalence");
    let mut direct_core = timem_in_process::agent_api::AgentCore::new(
        "STATIC {{ response_protocol }} {{ capability_catalog }}",
        CoreProfile {
            model: "test-model".to_string(),
        },
        &root,
    );
    let mut bridge_core = timem_in_process::agent_api::AgentCore::new(
        "STATIC {{ response_protocol }} {{ capability_catalog }}",
        CoreProfile {
            model: "test-model".to_string(),
        },
        &root,
    );
    let mut direct_config = config();
    let mut bridge_config = config();
    let mut direct_model = FinalAnswerModel {
        prompts: Vec::new(),
    };
    let mut bridge_model = FinalAnswerModel {
        prompts: Vec::new(),
    };
    let mut direct_ui = RecordingUi::default();
    let mut bridge_ui = RecordingUi::default();

    let direct_outcome = timem_in_process::agent_api::run_session_turn_with_model_client(
        &mut direct_core,
        &mut direct_config,
        TurnInput {
            input: "equivalent task",
            session: "session_equivalence",
            audit_file: &root.join("direct-audit.json"),
            runtime: "test-interface",
            run_bash_target: "test-host",
            additional_context: Some("same context"),
        },
        &mut direct_ui,
        None,
        &mut direct_model,
    );
    let bridge_outcome = timem_in_process::run_turn_with_model_client(
        &mut bridge_core,
        &mut bridge_config,
        TurnInput {
            input: "equivalent task",
            session: "session_equivalence",
            audit_file: &root.join("bridge-audit.json"),
            runtime: "test-interface",
            run_bash_target: "test-host",
            additional_context: Some("same context"),
        },
        &mut bridge_ui,
        None,
        &mut bridge_model,
    );

    assert_eq!(bridge_model.prompts.len(), direct_model.prompts.len());
    for (bridge_prompt, direct_prompt) in bridge_model.prompts.iter().zip(&direct_model.prompts) {
        assert_eq!(
            stable_prompt_semantics(bridge_prompt),
            stable_prompt_semantics(direct_prompt)
        );
    }
    assert_eq!(bridge_outcome.text, direct_outcome.text);
    assert_eq!(bridge_outcome.stats, direct_outcome.stats);
    assert_eq!(bridge_outcome.latest_usage, direct_outcome.latest_usage);
    assert_eq!(bridge_outcome.repair_issue, direct_outcome.repair_issue);
    assert_eq!(bridge_outcome.stop_reason, direct_outcome.stop_reason);
    assert_eq!(bridge_ui.model_responses, direct_ui.model_responses);
    assert_eq!(bridge_ui.topic_names, direct_ui.topic_names);
    assert_single_turn_identity(&direct_ui.projections);
    assert_single_turn_identity(&bridge_ui.projections);
    assert_eq!(
        projection_semantics(&bridge_ui.projections),
        projection_semantics(&direct_ui.projections)
    );

    std::fs::remove_dir_all(root).unwrap();
}

use agent_core::{
    ApiProtocol, CoreProfile, CoreTopicEvent, LlmResponse, ModelClient, ModelServiceConfig,
    NoopTurnUi, OpenAiCompatibleOptions, ResponseProtocolKind, TurnInput, TurnProjection, TurnUi,
    UsageStats,
};
use std::path::{Path, PathBuf};

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
    }
}

fn run_test_turn(
    root: &Path,
    input: &str,
    session: &str,
    ui: &mut dyn TurnUi,
    model: &mut dyn ModelClient,
) -> agent_core::TurnOutcome {
    let mut core = agent_core::AgentCore::new(
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

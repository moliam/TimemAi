use agent_core::model_catalog::{models, parse, validate, ReasoningAdapter};

#[test]
fn catalog_defaults_and_protocol_constraints_are_admissible() {
    assert_eq!(models().len(), 24);
    for m in models() {
        for p in &m.protocols {
            let effort = p.fixed_effort.as_deref().unwrap_or(&m.default_effort);
            let result = validate(&m.id, &m.model, &p.protocol, Some(effort), 200_000, 20_000);
            assert_eq!(
                result.is_ok(),
                p.disabled_reason.is_none(),
                "{} {}",
                m.id,
                p.protocol
            );
            if p.fixed_effort.is_some() {
                assert!(
                    validate(&m.id, &m.model, &p.protocol, Some("high"), 200_000, 20_000).is_err()
                );
            }
        }
    }
}

#[test]
fn catalog_budget_overrides_stay_within_limits() {
    for m in models() {
        let p = m
            .protocols
            .iter()
            .find(|p| p.disabled_reason.is_none())
            .unwrap();
        let effort = Some(p.fixed_effort.as_deref().unwrap_or(&m.default_effort));
        for output in [512, 8000, 30000, m.max_output] {
            assert!(validate(&m.id, &m.model, &p.protocol, effort, 3000, output).is_ok());
        }
        for (input, output) in [
            (2999, 10000),
            (3000, 511),
            (3000, m.max_output + 1),
            (m.max_input + 1, 10000),
            (m.context_window, 10000),
        ] {
            assert!(validate(&m.id, &m.model, &p.protocol, effort, input, output).is_err());
        }
        assert!(validate(&m.id, "wrong", &p.protocol, effort, 200000, 20000).is_err());
        assert!(validate(&m.id, &m.model, &p.protocol, Some("unknown"), 200000, 20000).is_err());
    }
}

#[test]
fn catalog_rejects_unimplemented_wire_binding() {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../resources/openai_model_catalog.json")).unwrap();
    value["models"][0]["profiles"][0]["bindings"][0]["path"] = serde_json::json!("/invented");
    assert!(parse(&value.to_string()).is_err());
}

fn extension_fixture(provider: &str) -> serde_json::Value {
    let source = if provider == "zhipu" {
        include_str!("../../../resources/zhipu_model_catalog.json")
    } else {
        include_str!("../../../resources/openai_model_catalog.json")
    };
    let mut value: serde_json::Value = serde_json::from_str(source).unwrap();
    let mut model = value["models"][if provider == "zhipu" { 1 } else { 0 }].clone();
    model["id"] = serde_json::json!(format!("fixture/{provider}-future"));
    model["model_id"] = serde_json::json!(format!("{provider}-future-fixture"));
    model["label"] = serde_json::json!("Future model fixture (not a real model)");
    value["models"] = serde_json::json!([model]);
    value
}

#[test]
fn external_catalog_rejects_unknown_fields_handlers_and_duplicate_identity() {
    use agent_core::model_catalog::load_catalog;
    let fixture = extension_fixture("zhipu");
    assert_eq!(parse(&fixture.to_string()).unwrap().len(), 1);
    for pointer in [
        "/models/0/profiles/0/bindings/0/handler",
        "/models/0/dimensions/1/handler",
    ] {
        let mut bad = fixture.clone();
        *bad.pointer_mut(pointer).unwrap() = serde_json::json!("unimplemented");
        assert!(parse(&bad.to_string()).is_err());
    }
    let mut bad = fixture.clone();
    bad["models"][0]["surprise_behavior"] = serde_json::json!(true);
    assert!(parse(&bad.to_string()).is_err());
    let dir = FixtureDir::new();
    std::fs::write(dir.path().join("future.json"), fixture.to_string()).unwrap();
    assert_eq!(load_catalog(Some(dir.path())).unwrap().len(), 25);
    let mut duplicate = fixture;
    duplicate["models"][0]["id"] = serde_json::json!("fixture/different-id");
    std::fs::write(dir.path().join("duplicate.json"), duplicate.to_string()).unwrap();
    assert_eq!(
        load_catalog(Some(dir.path())).unwrap_err(),
        "catalog_duplicate_identity"
    );
}

#[test]
fn external_catalog_json_only_extension_reaches_payload_in_fresh_process() {
    if std::env::var_os("TIMEM_CATALOG_FIXTURE_CHILD").is_some() {
        use agent_core::model_api::{build_model_request_with_reasoning, StructuredOutputHint};
        use agent_core::{ApiProtocol, ModelServiceConfig};
        agent_core::model_catalog::ensure_loaded().unwrap();
        assert_eq!(models().len(), 26);
        for provider in ["openai", "zhipu"] {
            let model = models()
                .iter()
                .find(|m| m.id == format!("fixture/{provider}-future"))
                .unwrap();
            for profile in model
                .protocols
                .iter()
                .filter(|p| p.disabled_reason.is_none())
            {
                let mut config = ModelServiceConfig {
                    model: String::new(),
                    base_url: "https://example.invalid/v1".into(),
                    api_key: "fixture".into(),
                    http_headers: Default::default(),
                    request_fields: Default::default(),
                    timeout_secs: 1,
                    max_llm_input_tokens: 100000,
                    max_llm_output_tokens: 10000,
                    api_protocol: ApiProtocol::OpenAiCompatible,
                    response_protocol: Default::default(),
                    interaction: Default::default(),
                    openai_compatible: Default::default(),
                    http_transport: Default::default(),
                };
                config.model = model.model.clone();
                config.api_protocol = if profile.protocol == "openai-responses" {
                    ApiProtocol::OpenAiResponses
                } else {
                    ApiProtocol::OpenAiCompatible
                };
                config.openai_compatible.requirements = serde_json::from_value(serde_json::json!({
                    "version":1,"provider":provider,"allowed_reasoning":model.efforts,"adaptive_reasoning":false
                })).unwrap();
                config.openai_compatible.reasoning_effort = Some(
                    profile
                        .fixed_effort
                        .as_ref()
                        .unwrap_or(&model.default_effort)
                        .clone(),
                );
                agent_core::model_requirements::validate_config(&config).unwrap();
                let body = build_model_request_with_reasoning(
                    &config,
                    &[],
                    StructuredOutputHint::None,
                    false,
                );
                agent_core::model_catalog::validate_request(&config, &body).unwrap();
                assert_eq!(body["model"], model.model);
                let path = if profile.protocol == "openai-responses" {
                    "/reasoning/effort"
                } else {
                    "/reasoning_effort"
                };
                assert_eq!(
                    body.pointer(path).unwrap(),
                    config
                        .openai_compatible
                        .reasoning_effort
                        .as_deref()
                        .unwrap()
                );
                if provider == "zhipu" {
                    if profile.protocol == "openai-compatible" {
                        assert_eq!(body["thinking"]["clear_thinking"], true);
                    } else {
                        assert!(body.get("thinking").is_none());
                        assert!(body.get("reasoning_effort").is_none());
                    }
                }
            }
        }
        return;
    }
    let dir = FixtureDir::new();
    for provider in ["openai", "zhipu"] {
        std::fs::write(
            dir.path().join(format!("{provider}.json")),
            extension_fixture(provider).to_string(),
        )
        .unwrap();
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "external_catalog_json_only_extension_reaches_payload_in_fresh_process",
            "--nocapture",
        ])
        .env("TIMEM_CATALOG_FIXTURE_CHILD", "1")
        .env("TIMEM_MODEL_CATALOG_DIR", dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct FixtureDir(std::path::PathBuf);
impl FixtureDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "timem-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn protocol_urls_override_connection_defaults_and_are_projected() {
    let mut fixture = extension_fixture("openai");
    let fallback = fixture["models"][0]["connection"]["default_base_url"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = parse(&fixture.to_string()).unwrap();
    assert!(original[0].protocols.iter().all(|p| p.base_url == fallback));
    fixture["models"][0]["profiles"][0]["default_base_url"] =
        serde_json::json!("https://responses.example.test/v1");
    let parsed = parse(&fixture.to_string()).unwrap();
    assert_eq!(parsed[0].base_url, fallback);
    assert_eq!(
        parsed[0].protocols[0].base_url,
        "https://responses.example.test/v1"
    );
    assert_eq!(parsed[0].protocols[1].base_url, fallback);
    assert_eq!(
        serde_json::to_value(&parsed[0]).unwrap()["protocols"][0]["base_url"],
        "https://responses.example.test/v1"
    );
    for bad in [
        "",
        "relative/path",
        "ftp://example.test",
        "https://user:secret@example.test",
        "https://example.test?key=secret",
        "https://example.test#fragment",
    ] {
        fixture["models"][0]["profiles"][0]["default_base_url"] = serde_json::json!(bad);
        assert!(parse(&fixture.to_string()).is_err(), "accepted {bad}");
    }
}

#[test]
fn every_registered_provider_is_admitted_by_endpoint_requirements() {
    for provider in agent_core::model_catalog::PROVIDERS {
        let requirements = agent_core::model_requirements::EndpointRequirements {
            version: 1,
            provider: Some(provider.id.to_string()),
            ..Default::default()
        };
        assert_eq!(
            requirements.validate(),
            Ok(()),
            "registered provider {} must be accepted by endpoint requirements",
            provider.id
        );
    }
    let unknown = agent_core::model_requirements::EndpointRequirements {
        version: 1,
        provider: Some("unregistered-provider".to_string()),
        ..Default::default()
    };
    assert_eq!(
        unknown.validate().unwrap_err(),
        "model_provider_adapter_not_implemented"
    );
}

#[test]
fn provider_registry_declares_closed_admission() {
    let zhipu = agent_core::model_catalog::provider_spec("zhipu").expect("zhipu registered");
    assert!(zhipu.requires_catalog);
    assert!(!zhipu.allowed_protocols.contains(&"anthropic"));
    let kimi = agent_core::model_catalog::provider_spec("kimi").expect("kimi registered");
    assert!(kimi.requires_catalog);
    assert!(!kimi.allowed_protocols.contains(&"anthropic"));
    let kimi_code =
        agent_core::model_catalog::provider_spec("kimi_code").expect("kimi_code registered");
    assert!(kimi_code.requires_catalog);
    assert!(!kimi_code.allowed_protocols.contains(&"anthropic"));
    let openai = agent_core::model_catalog::provider_spec("openai").expect("openai registered");
    assert!(!openai.requires_catalog);
    assert!(!openai.allowed_protocols.contains(&"anthropic"));
    assert!(agent_core::model_catalog::provider_spec("unknown-provider").is_none());
}

#[test]
fn catalog_exposes_documented_input_modalities() {
    let text_only = models()
        .iter()
        .find(|m| m.id == "z-glm5.3")
        .expect("zhipu glm-5.3 catalog entry");
    assert_eq!(text_only.input_modalities, vec!["text".to_string()]);
    let multimodal = models()
        .iter()
        .find(|m| m.id == "z-glm5.3-flash")
        .expect("zhipu glm-5.3-flash catalog entry");
    assert!(multimodal.input_modalities.contains(&"image".to_string()));
    assert!(models()
        .iter()
        .find(|m| m.id == "openai/gpt-5.5")
        .expect("openai catalog entry")
        .input_modalities
        .contains(&"image".to_string()));
}

#[test]
fn catalog_rejects_invalid_input_modalities() {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../resources/zhipu_model_catalog.json")).unwrap();
    // The descriptor schema rejects unknown modality enums and empty arrays
    // first; the interpreter adds duplicate detection on top.
    value["models"][0]["documented_facts"]["input_modalities"] =
        serde_json::json!(["text", "hologram"]);
    assert_eq!(
        parse(&value.to_string()).unwrap_err(),
        "catalog_schema_enum"
    );
    value["models"][0]["documented_facts"]["input_modalities"] = serde_json::json!([]);
    assert!(parse(&value.to_string()).is_err());
    value["models"][0]["documented_facts"]["input_modalities"] =
        serde_json::json!(["text", "text"]);
    assert_eq!(
        parse(&value.to_string()).unwrap_err(),
        "catalog_invalid_input_modalities"
    );
}

#[test]
fn catalog_required_tool_choice_marks_documented_rejections_only() {
    for m in models() {
        for p in &m.protocols {
            let k2_chat = m.provider == "kimi"
                && matches!(m.model.as_str(), "kimi-k2.6" | "kimi-k2.7-code")
                && p.protocol == "openai-compatible";
            let coding_responses = m.provider == "kimi_code" && p.protocol == "openai-responses";
            let deepseek_chat = m.provider == "deepseek" && p.protocol == "openai-compatible";
            assert_eq!(
                p.required_tool_choice,
                !(k2_chat || coding_responses || deepseek_chat),
                "{} {}",
                m.id,
                p.protocol
            );
        }
    }
}

#[test]
fn deepseek_templates_expose_documented_thinking_wire() {
    let spec = agent_core::model_catalog::provider_spec("deepseek").expect("deepseek registered");
    assert_eq!(spec.allowed_protocols.len(), 1);
    assert_eq!(spec.allowed_protocols[0], "openai-compatible");
    for (id, model_id, image) in [
        ("ds-flash", "deepseek-flash", true),
        ("ds-v4-pro", "deepseek-v4-pro", false),
    ] {
        let m = models().iter().find(|m| m.id == id).unwrap();
        assert_eq!(m.provider, "deepseek");
        assert_eq!(m.model, model_id);
        assert_eq!(m.efforts, vec!["none", "low", "high", "max"]);
        assert_eq!(m.default_effort, "high");
        assert_eq!(m.context_window, 1_048_576);
        assert_eq!(m.max_output, 393_216);
        assert_eq!(m.input_modalities.contains(&"image".to_string()), image);
        let p = &m.protocols[0];
        assert_eq!(p.protocol, "openai-compatible");
        assert_eq!(p.base_url, "https://api.deepseek.com");
        assert_eq!(p.reasoning_adapter, ReasoningAdapter::DeepSeekChatThinking);
        // Live DeepSeek 400: "Thinking mode does not support this tool_choice"
        // (request id f398c5e4). Chat thinking mode rejects tool_choice=
        // required, so the send layer must downgrade it to auto.
        assert!(!p.required_tool_choice);
        assert!(p.disabled_reason.is_none());
    }
}

fn bundled_catalog(file: &str) -> serde_json::Value {
    let path = format!("{}/../../resources/{file}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path).expect("bundled catalog readable");
    serde_json::from_str(&raw).expect("bundled catalog json")
}

/// 新增厂商/模板的第一道闸门：线路事实不允许“默认支持”。
/// 目录里漏写 `required_tool_choice` 时，旧语义会把它读成“支持 required”，
/// DeepSeek 思考模式就是这样在压缩阶段吃到 400。这里从原始 JSON 侧再兜一道，
/// 因为 schema 只约束内置目录，手改 JSON 或外部目录都可能绕过它。
#[test]
fn bundled_profiles_declare_required_tool_choice() {
    for file in [
        "openai_model_catalog.json",
        "zhipu_model_catalog.json",
        "kimi_model_catalog.json",
        "kimi_code_model_catalog.json",
        "deepseek_model_catalog.json",
    ] {
        let catalog = bundled_catalog(file);
        for model in catalog["models"].as_array().unwrap() {
            for profile in model["profiles"].as_array().unwrap() {
                assert!(
                    profile
                        .get("required_tool_choice")
                        .is_some_and(serde_json::Value::is_boolean),
                    "{file} {} {} 未显式声明 required_tool_choice；新增厂商/模板请按 \
                     docs/provider-model-descriptor.md「新增厂商/模板的常规动作」提交证据",
                    model["id"],
                    profile["protocol"]
                );
            }
        }
    }
}

/// 第二道闸门：即使漏写，运行时也必须 fail-safe，而不是乐观放行。
#[test]
fn absent_required_tool_choice_reads_as_unsupported() {
    let mut catalog = bundled_catalog("deepseek_model_catalog.json");
    catalog["models"][0]["profiles"][0]
        .as_object_mut()
        .unwrap()
        .remove("required_tool_choice");
    let parsed = parse(&catalog.to_string()).unwrap();
    assert!(
        !parsed[0].protocols[0].required_tool_choice,
        "缺省必须读作不支持：新厂商模板不能靠默认值继承“支持 required”的假设"
    );
}

/// 第三道闸门：注册表与目录必须互相覆盖，否则“加了 provider 却没模板”只在
/// Host 保存配置时才暴露。新增 Provider 时这里会直接失败。
#[test]
fn catalog_providers_are_registered_and_each_ships_usable_profiles() {
    for model in models() {
        assert!(
            agent_core::model_catalog::provider_spec(&model.provider).is_some(),
            "{} 的 provider {} 不在 PROVIDERS 注册表",
            model.id,
            model.provider
        );
        assert!(
            model.protocols.iter().any(|p| p.disabled_reason.is_none()),
            "{} 没有任何可用 profile",
            model.id
        );
    }
    for provider in agent_core::model_catalog::PROVIDERS {
        assert!(
            models().iter().any(|m| m.provider == provider.id),
            "provider {} 已注册但目录里没有模型模板",
            provider.id
        );
    }
}

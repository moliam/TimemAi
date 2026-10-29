//! 接入点验收矩阵 —— 新增厂商/模型模板时的常规动作，必须全绿。
//!
//! “能聊天”不足以说明接入点配置正确：压缩阶段发的是 `tool_choice=required`
//! 的关键请求，原生工具要带 tools 与并行控制，工具续轮要把上一轮 assistant
//! 消息（含不透明 reasoning_content）原样回传，推理强度还要按档位落成各厂商
//! 自己的 wire 字段。本文件对目录里每个 profile 跑一遍这些场景，再把最终 body
//! 交给 fail-closed 的 `validate_request` 复核：配置错了在测试期就炸，而不是
//! 等线上第一次压缩返回 400（DeepSeek 思考模式 400 正是这样暴露的）。
//!
//! 维护约定（新增厂商/模板时逐条核对，说明见
//! docs/provider-model-descriptor.md 的「新增厂商/模板的常规动作」）：
//! 1. 场景必须数据驱动，禁止给单个厂商写豁免分支；厂商差异只允许落成目录数据
//!    或 ReasoningAdapter 分支。
//! 2. 新增一种 wire 风格时，同时补 `model_payload::apply_reasoning` 分支与本
//!    文件对应的字段断言，否则“字段没落地”不会被任何测试发现。
//! 3. 目录漏声明 `required_tool_choice` 由 `model_catalog_tests` 的契约测试拦
//!    截；请求形态发不出去由本文件拦截。两者合起来才是新增厂商的完整性闸门。

use super::*;
use crate::model_catalog::{
    chat_preserves_reasoning, models, supports_required_tool_choice, validate_request,
    ReasoningAdapter,
};
use crate::{
    NativeExchange, NativeToolCall, NativeToolChoice, NativeToolResult, ToolCallMode,
    ToolDefinition,
};
use serde_json::{json, Value};

const INPUT_TOKENS: u32 = 200_000;
const OUTPUT_TOKENS: u32 = 20_000;
/// 只出现在回传的 reasoning_content 里，用来断言“回传/不回传”两条线路。
const REASONING_MARKER: &str = "acceptance-reasoning-marker";
/// 只出现在上一轮工具结果里，避免被 tools 定义里的工具名误判为“已回放”。
const TOOL_RESULT_MARKER: &str = "acceptance-tool-result-marker";
/// 续轮交换挂在这个 delta 上，提示里必须有同 id 的边界，否则投影为空。
const RESUME_DELTA_ID: &str = "pd_acceptance_1";

/// 与生产结构一致的最小提示：系统段 + 一个 delta 边界 + 尾部 trailer。
/// 少了 delta 边界时 native_exchanges 不会被投影，续轮场景就成了空转。
fn acceptance_prompt() -> String {
    format!(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n\
         [BEGIN DELTA delta_id: {RESUME_DELTA_ID}, time_ms: 1]\n\n## USER\n跑一遍接入点验收\n\n{}",
        crate::prompt_render::NATIVE_RESPONSE_TRAILER
    )
}

fn acceptance_config(
    model: &str,
    catalog_id: &str,
    protocol: &str,
    base_url: &str,
    stream: bool,
    effort: Option<&str>,
) -> ModelServiceConfig {
    ModelServiceConfig {
        interaction: Default::default(),
        model: model.to_string(),
        base_url: base_url.to_string(),
        api_key: "acceptance-key".to_string(),
        http_headers: Default::default(),
        request_fields: Default::default(),
        timeout_secs: 30,
        max_llm_output_tokens: OUTPUT_TOKENS,
        max_llm_input_tokens: INPUT_TOKENS,
        api_protocol: crate::parse_api_protocol(protocol).expect("catalog protocol label parses"),
        response_protocol: crate::ResponseProtocolKind::Json,
        openai_compatible: crate::OpenAiCompatibleOptions {
            catalog_id: Some(catalog_id.to_string()),
            reasoning_effort: effort.map(str::to_string),
            stream,
            ..Default::default()
        },
        http_transport: Default::default(),
    }
}

fn acceptance_tool() -> ToolDefinition {
    ToolDefinition {
        name: "timem_acceptance_probe".to_string(),
        description: "Records one acceptance slot without side effects.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {"slot": {"type": "integer", "enum": [1, 2]}},
            "required": ["slot"],
            "additionalProperties": false,
        }),
    }
}

fn acceptance_request(
    mode: ToolCallMode,
    tools: Vec<ToolDefinition>,
    tool_choice: NativeToolChoice,
    exchanges: Vec<NativeExchange>,
    parallel: bool,
    critical: bool,
) -> crate::ModelInteractionRequest {
    crate::ModelInteractionRequest {
        rendered_prompt: acceptance_prompt(),
        images: Vec::new(),
        static_tool_count: tools.len(),
        tools,
        native_exchanges: exchanges,
        resolved_mode: mode,
        parallel_tool_calls: parallel,
        send_parallel_tool_calls: parallel,
        tool_choice,
        critical_reasoning: critical,
    }
}

fn body_for(config: &ModelServiceConfig, request: &crate::ModelInteractionRequest) -> Value {
    prepare_model_interaction_http_request(config, request)
        .model_request
        .body
}

fn assert_admissible(config: &ModelServiceConfig, body: &Value, label: &str) {
    if let Err(error) = validate_request(config, body) {
        panic!("{label}: 请求体未通过 fail-closed 校验 -> {error}\nbody={body}");
    }
}

fn required_choice(protocol: ApiProtocol) -> Value {
    match protocol {
        ApiProtocol::Anthropic => json!({"type": "any"}),
        _ => json!("required"),
    }
}

/// 上一轮 assistant 的工具调用 + 工具结果：reasoning 回传线路要带元数据，
/// 其它线路必须原样丢弃，不能把不透明推理泄漏给无关厂商。
fn resume_exchange(
    config: &ModelServiceConfig,
    model: &crate::model_catalog::CatalogModel,
) -> NativeExchange {
    let continuation =
        chat_preserves_reasoning(config).then(|| crate::interaction::AssistantContinuation {
            catalog_id: model.id.clone(),
            model: model.model.clone(),
            reasoning_content: REASONING_MARKER.to_string(),
        });
    NativeExchange {
        delta_id: RESUME_DELTA_ID.to_string(),
        assistant_text: String::new(),
        calls: vec![NativeToolCall {
            assistant_continuation: continuation,
            id: "call_acceptance_1".to_string(),
            name: "timem_acceptance_probe".to_string(),
            arguments: json!({"slot": 1}),
            raw_arguments: "{\"slot\":1}".to_string(),
        }],
        results: vec![NativeToolResult {
            call_id: "call_acceptance_1".to_string(),
            name: "timem_acceptance_probe".to_string(),
            content: format!("{{\"marker\":\"{TOOL_RESULT_MARKER}\"}}"),
            is_error: false,
        }],
    }
}

fn wrote_reasoning_wire(adapter: ReasoningAdapter, body: &Value) -> bool {
    match adapter {
        ReasoningAdapter::KimiChatReasoning => body.get("reasoning_effort").is_some(),
        ReasoningAdapter::KimiK2Thinking
        | ReasoningAdapter::DeepSeekChatThinking
        | ReasoningAdapter::ZhipuChatReasoning => body.get("thinking").is_some(),
        ReasoningAdapter::EnumBodyField => {
            body.get("reasoning").is_some() || body.get("reasoning_effort").is_some()
        }
    }
}

#[test]
fn every_catalog_profile_accepts_chat_compression_and_tool_scenarios() {
    let mut profiles = 0usize;
    let mut providers = std::collections::BTreeSet::new();
    for model in models() {
        for profile in &model.protocols {
            if profile.disabled_reason.is_some() {
                continue;
            }
            profiles += 1;
            providers.insert(model.provider.clone());
            let label = format!("{} / {}", model.id, profile.protocol);
            // 目录约束可能把档位钉死（fixed_effort），Web 默认模板也按它取值；
            // 不按这条规则构造就会得到“默认配置存不下去”的假失败。
            let baseline_effort = profile
                .fixed_effort
                .as_deref()
                .unwrap_or(model.default_effort.as_str());
            let config = acceptance_config(
                &model.model,
                &model.id,
                &profile.protocol,
                &profile.base_url,
                false,
                Some(baseline_effort),
            );

            // 场景 1：普通聊天。最小请求也必须能出网。
            let plain = acceptance_request(
                ToolCallMode::Inline,
                Vec::new(),
                NativeToolChoice::Auto,
                Vec::new(),
                false,
                false,
            );
            assert_admissible(
                &config,
                &body_for(&config, &plain),
                &format!("{label} 普通聊天"),
            );

            // 场景 2：流式聊天。开关必须真的落到 wire 上。
            let streaming = acceptance_config(
                &model.model,
                &model.id,
                &profile.protocol,
                &profile.base_url,
                true,
                Some(baseline_effort),
            );
            let stream_body = body_for(&streaming, &plain);
            if config.api_protocol != ApiProtocol::Anthropic {
                assert_eq!(
                    stream_body["stream"],
                    json!(true),
                    "{label} 流式开关未落到 body"
                );
            }
            assert_admissible(&streaming, &stream_body, &format!("{label} 流式聊天"));

            // 场景 3：原生工具（auto）。工具定义必须真的进 body。
            let native = acceptance_request(
                ToolCallMode::Native,
                vec![acceptance_tool()],
                NativeToolChoice::Auto,
                Vec::new(),
                false,
                false,
            );
            let native_body = body_for(&config, &native);
            assert_admissible(&config, &native_body, &format!("{label} 原生工具 auto"));
            assert!(
                native_body
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(|tools| !tools.is_empty()),
                "{label} 原生工具请求没有携带 tools"
            );
            let auto_choice = native_body["tool_choice"].clone();

            // 场景 3b：并行原生工具。并行控制字段必须可出网。
            let parallel = acceptance_request(
                ToolCallMode::Native,
                vec![acceptance_tool()],
                NativeToolChoice::Auto,
                Vec::new(),
                true,
                false,
            );
            assert_admissible(
                &config,
                &body_for(&config, &parallel),
                &format!("{label} 并行原生工具"),
            );

            // 场景 4：压缩关键请求。声明支持的线路必须真的发出 required；
            // 声明不支持的线路必须降级为 auto，靠文本 trailer 保证必调工具。
            let compression = acceptance_request(
                ToolCallMode::Native,
                vec![acceptance_tool()],
                NativeToolChoice::Required,
                Vec::new(),
                false,
                true,
            );
            let compression_body = body_for(&config, &compression);
            assert_admissible(&config, &compression_body, &format!("{label} 压缩关键请求"));
            if supports_required_tool_choice(&config) {
                assert_eq!(
                    compression_body["tool_choice"],
                    required_choice(config.api_protocol),
                    "{label} 声明支持 required 却没发出该档位"
                );
            } else {
                assert_eq!(
                    compression_body["tool_choice"], auto_choice,
                    "{label} 声明不支持 required 却仍把该档位发上线路"
                );
            }

            // 场景 5：工具续轮。上一轮 assistant/tool 消息必须能回放，且
            // reasoning_content 只在文档要求回传的线路上出现。
            let exchange = resume_exchange(&config, model);
            let resume = acceptance_request(
                ToolCallMode::Native,
                vec![acceptance_tool()],
                NativeToolChoice::Auto,
                vec![exchange],
                false,
                false,
            );
            let resume_body = body_for(&config, &resume);
            assert_admissible(&config, &resume_body, &format!("{label} 工具续轮"));
            let serialized = resume_body.to_string();
            assert!(
                serialized.contains(TOOL_RESULT_MARKER),
                "{label} 工具续轮没有回放上一轮工具结果（delta 锚点或投影丢失）\nbody={resume_body}"
            );
            assert!(
                serialized.contains("call_acceptance_1"),
                "{label} 工具续轮丢失 call_id 对应关系"
            );
            assert_eq!(
                serialized.contains(REASONING_MARKER),
                chat_preserves_reasoning(&config),
                "{label} reasoning_content 回传策略与目录声明不一致"
            );

            // 场景 6：逐个推理档位。每个允许的档位都要写出对应 wire 字段。
            for effort in &model.efforts {
                if profile
                    .fixed_effort
                    .as_deref()
                    .is_some_and(|fixed| fixed != effort)
                {
                    continue; // 固定档位的模板不接受其它取值
                }
                let mut tuned = acceptance_config(
                    &model.model,
                    &model.id,
                    &profile.protocol,
                    &profile.base_url,
                    false,
                    Some(effort),
                );
                tuned.openai_compatible.reasoning_effort = Some(effort.clone());
                let tuned_body = body_for(&tuned, &plain);
                assert_admissible(&tuned, &tuned_body, &format!("{label} 档位 {effort}"));
                // kimi-k2.7-code 恒思考，官方要求省略 thinking 字段。
                let expects_wire = !(profile.reasoning_adapter == ReasoningAdapter::KimiK2Thinking
                    && !model.efforts.iter().any(|value| value == "none"));
                if expects_wire {
                    assert!(
                        wrote_reasoning_wire(profile.reasoning_adapter, &tuned_body),
                        "{label} 档位 {effort} 没有写出任何推理 wire 字段"
                    );
                } else {
                    assert!(
                        tuned_body.get("thinking").is_none(),
                        "{label} 档位 {effort} 不应发送 thinking 字段"
                    );
                }
            }
        }
    }
    assert!(profiles > 0, "目录里没有可用 profile，验收矩阵没有覆盖对象");
    assert!(
        providers.len() >= 5,
        "目录只覆盖了 {} 个 provider，新增厂商可能漏了验收",
        providers.len()
    );
}

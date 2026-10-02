import { describe, expect, it } from "vitest";
import { effectiveAllowedReasoning, endpointCapabilityIssue, endpointCapabilityIssueMessage, endpointDraftChanged, endpointImportCommandErrorMessage, endpointImportIssueMessage, endpointLabelForProfile, endpointDraftValid, endpointMatchesProfile, endpointNameForProfile, endpointSaveErrorMessage, formatContextWindowTokens, isValidMaxLlmInputTokens, MODEL_CONTEXT_WINDOW_OPTIONS, toggleAllowedReasoning } from "../src/model_endpoints";
import { setLocale } from "../src/i18n";

const endpoint = { id: "one", name: "Production", model: "gpt-4.1", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "https://api.example/v1", max_llm_input_tokens: 100_000, max_llm_output_tokens: 10_000, stream: false, api_key_configured: true };

describe("shared model endpoints", () => {
  it("matches an endpoint to the complete active Session route", () => {
    expect(endpointMatchesProfile(endpoint, { ...endpoint })).toBe(true);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, base_url: "https://other" })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, api_key_configured: false })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, max_llm_input_tokens: 200_000 })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, max_llm_output_tokens: 20_000 })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, stream: true })).toBe(false);
  });

  it("resolves only the shared endpoint name for a Session profile", () => {
    expect(endpointNameForProfile([endpoint], { ...endpoint })).toBe("Production");
    expect(endpointNameForProfile([endpoint], { ...endpoint, model: "other-model" })).toBeUndefined();
    expect(endpointNameForProfile([], { ...endpoint })).toBeUndefined();
  });

  it("requires every route field while allowing an empty key", () => {
    expect(endpointDraftValid({ name: "Local", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost:8000/v1", max_llm_input_tokens: 1_000_000, max_llm_output_tokens: 50_000, stream: true, api_key: "" })).toBe(true);
    expect(endpointDraftValid({ name: "", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 100_000, max_llm_output_tokens: 10_000, stream: false })).toBe(false);
    expect(endpointDraftValid({ name: "Custom input window", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 128_000, max_llm_output_tokens: 10_000, stream: false })).toBe(true);
    expect(endpointDraftValid({ name: "Invalid", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 0, max_llm_output_tokens: 8_000, stream: false })).toBe(false);
    expect(endpointDraftValid({ name: "Invalid", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 1.5, max_llm_output_tokens: 8_000, stream: false })).toBe(false);
  });
});

describe("context window options", () => {
  it("keeps preset options and adds 300K", () => {
    expect([...MODEL_CONTEXT_WINDOW_OPTIONS]).toEqual([100_000, 200_000, 300_000, 1_000_000]);
  });

  it("accepts any positive integer token count up to the u32 ceiling", () => {
    expect(isValidMaxLlmInputTokens(1)).toBe(true);
    expect(isValidMaxLlmInputTokens(128_000)).toBe(true);
    expect(isValidMaxLlmInputTokens(4_294_967_295)).toBe(true);
    expect(isValidMaxLlmInputTokens(0)).toBe(false);
    expect(isValidMaxLlmInputTokens(-100)).toBe(false);
    expect(isValidMaxLlmInputTokens(4_294_967_296)).toBe(false);
    expect(isValidMaxLlmInputTokens(Number.NaN)).toBe(false);
  });

  it("formats presets and custom values compactly", () => {
    expect(formatContextWindowTokens(100_000)).toBe("100K");
    expect(formatContextWindowTokens(300_000)).toBe("300K");
    expect(formatContextWindowTokens(1_000_000)).toBe("1M");
    expect(formatContextWindowTokens(2_000_000)).toBe("2M");
    expect(formatContextWindowTokens(131_072)).toBe("131.072K");
    expect(formatContextWindowTokens(500)).toBe("500");
  });
});

describe("Session endpoint labels", () => {
  it("keeps the saved name for a matching profile", () => {
    expect(endpointLabelForProfile([endpoint], endpoint)).toBe("Production");
  });

  it("shows a custom route when the Session retains a different URL", () => {
    const profile = { ...endpoint, base_url: "https://retained.example/v1" };
    expect(endpointLabelForProfile([endpoint], profile)).toBe("自定义配置 · gpt-4.1");
    expect(endpointMatchesProfile(endpoint, profile)).toBe(false);
  });

  it("does not lose the Session configuration when a preset changes or disappears", () => {
    expect(endpointLabelForProfile([], endpoint)).toBe("自定义配置 · gpt-4.1");
    expect(endpointLabelForProfile([endpoint], { ...endpoint, stream: true }))
      .toBe("自定义配置 · gpt-4.1");
  });

  it("reserves unconfigured for missing or empty model profiles", () => {
    expect(endpointLabelForProfile([endpoint], undefined)).toBe("未配置");
    expect(endpointLabelForProfile([], { ...endpoint, model: "  " })).toBe("未配置");
  });
});

describe("stable endpoint binding", () => {
  it("uses the bound ID despite route edits and renames", () => {
    const profile = { ...endpoint, model_endpoint_id: endpoint.id, base_url: "https://old.example/v1" };
    expect(endpointMatchesProfile(endpoint, profile)).toBe(true);
    expect(endpointLabelForProfile([{ ...endpoint, name: "Renamed" }], profile)).toBe("Renamed");
    expect(endpointMatchesProfile({ ...endpoint, id: "other" }, profile)).toBe(false);
  });
});

it("does not silently fall back when the bound endpoint is deleted", () => {
  const profile = { ...endpoint, model_endpoint_id: "deleted" };
  expect(endpointLabelForProfile([endpoint], profile)).toBe("接入点已删除");
  expect(endpointMatchesProfile(endpoint, profile)).toBe(false);
});

import { endpointProtocolOptions, canRestoreEndpointTemplateUrl, changeEndpointProtocol, restoreEndpointTemplateReasoning, restoreEndpointTemplateUrl, templateBaseUrl, applyEndpointTemplate, editEndpoint, editEndpointRequirements, initialEndpointRequirements, type CatalogModel, type ModelEndpointDraft } from "../src/model_endpoints";
const template: CatalogModel = { id:"fixture/model", revision:1, provider:"openai", model:"fixture-model", label:"Fixture", base_url:"https://example.test/v1", efforts:["none","low","high","max"], default_effort:"low", middle_default:false, min_input:3000, max_input:200000, min_output:512, max_output:30000, context_window:230000, protocols:[{protocol:"openai-responses",disabled_reason:null,fixed_effort:null,fixed_reason:null}] };
const blank = (): ModelEndpointDraft => ({ ...endpoint, http_headers:{}, request_fields:{}, allow_cross_origin_redirects:false, requirements:initialEndpointRequirements() });
describe("editable endpoint templates", () => {
  it("defaults token budgets from the selected model limits", () => {
    const standard = applyEndpointTemplate(blank(), template);
    expect(standard.max_llm_input_tokens).toBe(200_000);
    expect(standard.max_llm_output_tokens).toBe(10_000);
    expect(standard.requirements?.field_sources.max_llm_input_tokens).toBe("template");
    expect(standard.requirements?.field_sources.max_llm_output_tokens).toBe("template");

    const large = applyEndpointTemplate(blank(), {
      ...template,
      id: "fixture/large",
      max_input: 922_000,
      max_output: 128_000,
      context_window: 1_050_000,
    });
    expect(large.max_llm_input_tokens).toBe(256_000);
    expect(large.max_llm_output_tokens).toBe(10_000);

    const small = applyEndpointTemplate(blank(), {
      ...template,
      id: "fixture/small",
      max_input: 64_000,
      max_output: 8_000,
      context_window: 72_000,
    });
    expect(small.max_llm_input_tokens).toBe(64_000);
    expect(small.max_llm_output_tokens).toBe(8_000);
  });

  it("recomputes template-owned budgets but preserves user overrides", () => {
    const first = applyEndpointTemplate(blank(), template);
    const nextModel = {
      ...template,
      id: "fixture/next",
      max_input: 400_000,
      max_output: 6_000,
      context_window: 500_000,
    };
    const switched = applyEndpointTemplate(first, nextModel);
    expect(switched.max_llm_input_tokens).toBe(256_000);
    expect(switched.max_llm_output_tokens).toBe(6_000);

    const manual = editEndpoint(first, {
      max_llm_input_tokens: 123_000,
      max_llm_output_tokens: 5_000,
    });
    const preserved = applyEndpointTemplate(manual, nextModel);
    expect(preserved.max_llm_input_tokens).toBe(123_000);
    expect(preserved.max_llm_output_tokens).toBe(5_000);
  });

  it("suggests fields, preserves user overrides across template switches and removal", () => {
    let draft = applyEndpointTemplate(blank(), template);
    expect(draft.model).toBe(template.model);
    expect(draft.requirements?.field_sources.model).toBe("template");
    draft = editEndpoint(draft, {model:"manual-model",base_url:"https://proxy.test/v1",max_llm_output_tokens:25000});
    draft = editEndpointRequirements(draft, {allowed_reasoning:["low"],adaptive_reasoning:false});
    draft = applyEndpointTemplate(draft, {...template,id:"other",model:"other-model",default_effort:"high"});
    expect(draft.model).toBe("manual-model");
    expect(draft.base_url).toBe("https://proxy.test/v1");
    expect(draft.max_llm_output_tokens).toBe(25000);
    expect(draft.requirements?.allowed_reasoning).toEqual(["low"]);
    expect(draft.requirements?.adaptive_reasoning).toBe(false);
    expect(draft.reasoning_effort).toBe("high"); // invalid combination stays visible, not silently repaired
    expect(applyEndpointTemplate(draft)).toEqual({...draft,catalog_id:null});
  });
  it("migrates legacy values as user-owned and resolves only explicit legacy template identity", () => {
    const legacy = {...blank(),id:"old",api_key_configured:false,private_ca_configured:false,catalog_id:template.id};
    delete legacy.requirements;
    const requirements = initialEndpointRequirements(legacy,[template]);
    expect(requirements.provider).toBe("openai");
    expect(requirements.field_sources.model).toBe("user");
    expect(initialEndpointRequirements({...legacy,catalog_id:"unknown"},[template]).provider).toBeUndefined();
    const migrated = {...legacy, requirements};
    expect(applyEndpointTemplate(migrated,template).model).toBe(legacy.model);
    const copy = initialEndpointRequirements({...legacy,requirements});
    copy.field_sources.model = "template";
    expect(requirements.field_sources.model).toBe("user");
  });
  it("does not infer provider from a URL or inherit missing capabilities", () => {
    const draft = editEndpoint(blank(),{base_url:"https://open.bigmodel.cn/api/paas/v4",model:"glm-future"});
    expect(draft.requirements?.provider).toBeUndefined();
    expect(draft.requirements?.allowed_reasoning).toBeUndefined();
  });
});

const protocolTemplate: CatalogModel = {...template, protocols: [
  {...template.protocols[0], base_url:"https://responses.example.test/v1"},
  {...template.protocols[0], protocol:"openai-compatible", base_url:"https://chat.example.test/v2"},
]};
describe("protocol-specific template URLs", () => {
  it("follows both directions while retaining template ownership", () => {
    const first = applyEndpointTemplate(blank(), protocolTemplate);
    expect(first.base_url).toBe(protocolTemplate.protocols[0].base_url);
    const chat = changeEndpointProtocol(first, "openai-compatible", protocolTemplate);
    expect(chat.base_url).toBe(protocolTemplate.protocols[1].base_url);
    expect(chat.requirements?.field_sources.base_url).toBe("template");
    expect(changeEndpointProtocol(chat,"openai-responses",protocolTemplate).base_url).toBe(first.base_url);
    expect(first.api_protocol).toBe("openai-responses");
  });
  it("preserves manual values even when equal to the previous default", () => {
    const first = applyEndpointTemplate(blank(), protocolTemplate);
    for (const base_url of ["https://proxy.test/v1", first.base_url, ""]) {
      const manual = editEndpoint(first,{base_url});
      expect(changeEndpointProtocol(manual,"openai-compatible",protocolTemplate).base_url).toBe(base_url);
    }
  });
  it("restores the current protocol URL and resumes following", () => {
    let draft = editEndpoint(applyEndpointTemplate(blank(),protocolTemplate),{base_url:"https://proxy.test/v1"});
    draft = changeEndpointProtocol(draft,"openai-compatible",protocolTemplate);
    draft = restoreEndpointTemplateUrl(draft,protocolTemplate);
    expect(draft.base_url).toBe(protocolTemplate.protocols[1].base_url);
    expect(draft.requirements?.field_sources.base_url).toBe("template");
    expect(changeEndpointProtocol(draft,"openai-responses",protocolTemplate).base_url).toBe(protocolTemplate.protocols[0].base_url);
  });
  it("uses an explicit protocol when applying a new template", () => {
    const draft = applyEndpointTemplate(editEndpoint(blank(),{api_protocol:"openai-compatible"}),protocolTemplate);
    expect(draft.base_url).toBe(protocolTemplate.protocols[1].base_url);
  });
  it("does not guess URLs for absent protocols or detached templates", () => {
    const draft = applyEndpointTemplate(blank(),protocolTemplate);
    expect(templateBaseUrl(protocolTemplate,"anthropic")).toBeUndefined();
    expect(changeEndpointProtocol(draft,"anthropic",protocolTemplate).base_url).toBe(draft.base_url);
    const detached = applyEndpointTemplate(draft);
    expect(changeEndpointProtocol(detached,"openai-compatible",protocolTemplate).base_url).toBe(draft.base_url);
    expect(changeEndpointProtocol(draft,"openai-compatible").base_url).toBe(draft.base_url);
  });
  it("falls back to model URL and conservatively preserves legacy endpoints", () => {
    expect(templateBaseUrl(template,"openai-responses")).toBe(template.base_url);
    const draft = {...applyEndpointTemplate(blank(),protocolTemplate),requirements:undefined};
    expect(changeEndpointProtocol(draft,"openai-compatible",protocolTemplate).base_url).toBe(draft.base_url);
  });
});

describe("default URL restore visibility", () => {
  it("hides restore for the current combination default, regardless of provenance", () => {
    const draft = applyEndpointTemplate(blank(),protocolTemplate);
    expect(canRestoreEndpointTemplateUrl(draft,protocolTemplate)).toBe(false);
    expect(canRestoreEndpointTemplateUrl(editEndpoint(draft,{base_url:draft.base_url}),protocolTemplate)).toBe(false);
    expect(canRestoreEndpointTemplateUrl(changeEndpointProtocol(draft,"openai-compatible",protocolTemplate),protocolTemplate)).toBe(false);
    expect(canRestoreEndpointTemplateUrl(applyEndpointTemplate(blank(),template),template)).toBe(false);
  });
  it("shows only for a different value with a known default and hides again after restore", () => {
    const draft = editEndpoint(applyEndpointTemplate(blank(),protocolTemplate),{base_url:"https://proxy.test/v1"});
    expect(canRestoreEndpointTemplateUrl(draft,protocolTemplate)).toBe(true);
    expect(canRestoreEndpointTemplateUrl({...draft,base_url:""},protocolTemplate)).toBe(true);
    expect(canRestoreEndpointTemplateUrl(restoreEndpointTemplateUrl(draft,protocolTemplate),protocolTemplate)).toBe(false);
    expect(canRestoreEndpointTemplateUrl(draft)).toBe(false);
    expect(canRestoreEndpointTemplateUrl({...draft,api_protocol:"anthropic"},protocolTemplate)).toBe(false);
    const manual = editEndpoint(draft,{base_url:protocolTemplate.protocols[0].base_url!});
    expect(canRestoreEndpointTemplateUrl(changeEndpointProtocol(manual,"openai-compatible",protocolTemplate),protocolTemplate)).toBe(true);
  });
});

describe("endpoint protocol choices", () => {
  it("offers only implemented model protocols, not the generic list", () => {
    const draft = applyEndpointTemplate(blank(),protocolTemplate);
    expect(endpointProtocolOptions(draft,protocolTemplate)).toEqual([
      {protocol:"openai-responses",disabled:false},{protocol:"openai-compatible",disabled:false},
    ]);
    expect(endpointProtocolOptions(draft,template)).toEqual([{protocol:"openai-responses",disabled:false}]);
  });
  it("retains unsupported current values as disabled rather than changing them", () => {
    const draft = editEndpoint(applyEndpointTemplate(blank(),template),{api_protocol:"anthropic"});
    expect(endpointProtocolOptions(draft,template)).toEqual([
      {protocol:"openai-responses",disabled:false},{protocol:"anthropic",disabled:true},
    ]);
    expect(draft.api_protocol).toBe("anthropic");
    const constrained = {...protocolTemplate, protocols:protocolTemplate.protocols.map(p=>({...p,disabled_reason:p.protocol==="openai-compatible"?"Unavailable":null}))};
    expect(endpointProtocolOptions({...draft,api_protocol:"openai-compatible"},constrained).at(-1)).toEqual({protocol:"openai-compatible",disabled:true});
  });
  it("keeps generic choices for custom services but respects declared providers", () => {
    expect(endpointProtocolOptions(blank()).filter(p=>!p.disabled)).toHaveLength(3);
    const draft = applyEndpointTemplate(blank(),template);
    expect(endpointProtocolOptions(draft).some(p=>p.protocol==="anthropic")).toBe(false);
    const zhipu = {...draft,api_protocol:"openai-compatible",requirements:{...draft.requirements!,provider:"zhipu"}};
    expect(endpointProtocolOptions(zhipu)).toEqual([{protocol:"openai-compatible",disabled:false}]);
  });
});


describe("reasoning availability editing", () => {
  it("shows inherited known-model levels before the first edit", () => {
    expect(effectiveAllowedReasoning(null, ["none", "low", "medium", "high"], "medium"))
      .toEqual(["none", "low", "medium", "high"]);
    expect(effectiveAllowedReasoning(undefined, undefined, "medium"))
      .toEqual(["medium"]);
  });

  it("starts the first toggle from inherited model capabilities", () => {
    expect(toggleAllowedReasoning(
      null,
      ["none", "low", "medium", "high"],
      "medium",
      "medium",
      "low",
      false,
    )).toEqual({
      allowedReasoning: ["none", "medium", "high"],
      dailyReasoning: "medium",
    });
  });

  it("moves the daily level when its chip is removed", () => {
    expect(toggleAllowedReasoning(
      ["none", "low", "medium"],
      ["none", "low", "medium", "high"],
      "medium",
      "medium",
      "medium",
      false,
    )).toEqual({
      allowedReasoning: ["none", "low"],
      dailyReasoning: "none",
    });
  });

  it("does not allow the last available level to become an empty invalid set", () => {
    expect(toggleAllowedReasoning(
      ["low"],
      undefined,
      "low",
      undefined,
      "low",
      false,
    )).toEqual({ allowedReasoning: ["low"], dailyReasoning: "low" });
  });
  it("keeps recognized custom levels in semantic strength order regardless of click order", () => {
    expect(toggleAllowedReasoning(
      ["medium", "high"],
      undefined,
      "medium",
      undefined,
      "low",
      true,
    )).toEqual({ allowedReasoning: ["low", "medium", "high"], dailyReasoning: "medium" });
  });

  it("preserves user-declared order when custom levels have unknown semantics", () => {
    expect(toggleAllowedReasoning(
      ["balanced", "deep"],
      undefined,
      "balanced",
      undefined,
      "fast",
      true,
    )).toEqual({ allowedReasoning: ["balanced", "deep", "fast"], dailyReasoning: "balanced" });
  });
});


describe("endpoint editor change tracking", () => {
  const savedDraft = (): ModelEndpointDraft => ({
    ...blank(),
    id: "saved",
    name: "Saved endpoint",
    http_headers: { Authorization: "Bearer token", "X-Mode": "fast" },
    request_fields: { metadata: { enabled: true }, priority: 1 },
    api_key: "secret",
    private_ca_pem: "certificate",
  });

  it("keeps an unchanged draft disabled despite object key order", () => {
    const initial = savedDraft();
    const current = structuredClone(initial);
    current.http_headers = { "X-Mode": "fast", Authorization: "Bearer token" };
    current.request_fields = { priority: 1, metadata: { enabled: true } };
    expect(endpointDraftChanged(initial, current)).toBe(false);
  });

  it("treats template ownership as a persistent behavior change", () => {
    const initial = savedDraft();
    const current = structuredClone(initial);
    current.requirements!.field_sources.name = "template";
    expect(endpointDraftChanged(initial, current)).toBe(true);
  });

  it("enables save for a real edit and disables it again after restoring the initial value", () => {
    const initial = savedDraft();
    expect(endpointDraftChanged(initial, { ...initial, name: "Changed" })).toBe(true);
    expect(endpointDraftChanged(initial, { ...initial, name: initial.name })).toBe(false);
  });

  it("restores template reasoning levels and template ownership together", () => {
    const initial = applyEndpointTemplate(blank(), template);
    const manual = editEndpointRequirements(initial, { allowed_reasoning: ["low"] });
    const restored = restoreEndpointTemplateReasoning(manual, template);
    expect(restored.requirements?.allowed_reasoning).toEqual(template.efforts);
    expect(restored.requirements?.field_sources.allowed_reasoning).toBe("template");
    expect(restoreEndpointTemplateReasoning(manual, { ...template, id: "other" })).toBe(manual);
  });
});


describe("endpoint capability feedback", () => {
  it("classifies model constraints without exposing catalog reason text", () => {
    const draft = applyEndpointTemplate(blank(), template);
    const fixed = { ...template, protocols: [{ ...template.protocols[0], fixed_effort: "none", fixed_reason: "内部目录说明" }] };
    const issue = endpointCapabilityIssue(draft, fixed);
    expect(issue).toEqual({ code: "fixed_reasoning", level: "none" });
    setLocale("zh");
    expect(endpointCapabilityIssueMessage(issue!)).toBe("此协议要求日常推理档位为 none。");
    expect(endpointCapabilityIssueMessage(issue!)).not.toContain("内部目录说明");
  });

  it("provides precise localized budget feedback", () => {
    const draft = { ...applyEndpointTemplate(blank(), template), max_llm_input_tokens: 220_000, max_llm_output_tokens: 20_000 };
    const issue = endpointCapabilityIssue(draft, template);
    expect(issue).toEqual({ code: "input_budget" });
    setLocale("en");
    expect(endpointCapabilityIssueMessage(issue!)).toBe("The input token budget is outside this model’s supported range.");
    setLocale("zh");
  });

  it("localizes import diagnostics and safely hides unknown internal details", () => {
    setLocale("zh");
    expect(endpointImportIssueMessage("claude_thinking_tokens_not_imported"))
      .toContain("固定思考 token 预算");
    expect(endpointImportIssueMessage("codex_provider_wire_api_unsupported:zai:future-wire"))
      .toContain("API 协议");
    expect(endpointImportIssueMessage("unexpected_internal_detail:/private/path"))
      .toBe("部分配置无法完整导入，已安全跳过。");
    expect(endpointImportIssueMessage("unexpected_internal_detail:/private/path"))
      .not.toContain("private/path");
    setLocale("en");
    expect(endpointImportIssueMessage("claude_effort_level_not_imported"))
      .toContain("cannot be converted directly");
    setLocale("zh");
  });

  it("maps import command failures without exposing internal codes or paths", () => {
    setLocale("zh");
    expect(endpointImportCommandErrorMessage("scan", "model_endpoint_import_directory_required"))
      .toBe("请至少填写一个配置目录。");
    expect(endpointImportCommandErrorMessage("scan", "model_endpoint_import_home_user_unsupported:~other"))
      .not.toContain("~other");
    expect(endpointImportCommandErrorMessage("scan", "unexpected:/private/path"))
      .toBe("请确认配置目录存在且可读取，然后重试。");
    expect(endpointImportCommandErrorMessage("apply", "model_endpoint_import_candidate_not_found:private-id"))
      .toBe("扫描结果已失效。请重新扫描后再导入。");
    expect(endpointImportCommandErrorMessage("apply", "unexpected:secret"))
      .toBe("暂时无法导入所选模型。请重新扫描后重试。");
    setLocale("en");
    expect(endpointImportCommandErrorMessage("apply", "model_endpoint_import_selection_empty"))
      .toBe("Select at least one model to import.");
    setLocale("zh");
  });

  it("maps known save failures and safely hides unknown internal details", () => {
    setLocale("zh");
    expect(endpointSaveErrorMessage("model_endpoint_name_conflict")).toBe("已有同名接入点。请使用其他名称。");
    expect(endpointSaveErrorMessage("daily_reasoning_not_in_allowed_set")).toContain("推理设置");
    expect(endpointSaveErrorMessage("model_endpoint_store_write_failed:private/path")).not.toContain("private/path");
    expect(endpointSaveErrorMessage("unexpected_internal_detail:secret")).toBe("请检查配置后重试。");
    setLocale("en");
    expect(endpointSaveErrorMessage("catalog_token_budget_out_of_range")).toContain("token budget");
    setLocale("zh");
  });
});

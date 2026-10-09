import { describe, expect, it } from "vitest";
import { capabilityModelForDraft, effectiveAllowedReasoning, endpointCapabilityIssue, endpointCapabilityIssueMessage, endpointDraftChanged, endpointImportCommandErrorMessage, endpointImportIssueMessage, endpointLabelForProfile, endpointDraftValid, endpointMatchesProfile, endpointNameForProfile, endpointSaveErrorMessage, formatContextWindowTokens, isValidMaxLlmInputTokens, MODEL_CONTEXT_WINDOW_OPTIONS, REASONING_EFFORT_OPTIONS, toggleAllowedReasoning } from "../src/model_endpoints";
import { setLocale } from "../src/i18n";

const endpoint = { id: "one", name: "Production", model: "gpt-4.1", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "https://api.example/v1", max_llm_input_tokens: 100_000, max_llm_output_tokens: 10_000, stream: false, function_calling: true, api_key_configured: true };

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

import { endpointProtocolOptions, canRestoreEndpointTemplateUrl, changeEndpointProtocol, restoreEndpointTemplateReasoning, restoreEndpointTemplateUrl, templateBaseUrl, applyEndpointTemplate, editEndpoint, editEndpointRequirements, functionCallingDefault, initialEndpointRequirements, type CatalogModel, type ModelEndpointDraft, type ProviderSpec } from "../src/model_endpoints";
const template: CatalogModel = { id:"fixture/model", revision:1, provider:"openai", model:"fixture-model", label:"Fixture", base_url:"https://example.test/v1", efforts:["none","low","high","max"], default_effort:"low", middle_default:false, min_input:3000, max_input:200000, min_output:512, max_output:30000, context_window:230000, protocols:[{protocol:"openai-responses",disabled_reason:null,function_calling:"supported",fixed_effort:null,fixed_reason:null}] };
const blank = (): ModelEndpointDraft => ({ ...endpoint, http_headers:{}, request_fields:{}, allow_cross_origin_redirects:false, requirements:initialEndpointRequirements() });
const providerSpecs: ProviderSpec[] = [
  { id: "openai", allowed_protocols: ["openai-responses", "openai-compatible"], requires_catalog: false },
  { id: "zhipu", allowed_protocols: ["openai-responses", "openai-compatible"], requires_catalog: true },
];
describe("function calling defaults and ownership", () => {
  const modelWith = (support: "supported" | "conditional" | "unsupported" | "unknown"): CatalogModel => ({
    ...template,
    protocols: [{
      protocol: "openai-compatible",
      disabled_reason: null,
      function_calling: support,
      fixed_effort: null,
      fixed_reason: null,
    }],
  });

  it("defaults custom and catalog routes according to the protocol declaration", () => {
    expect(functionCallingDefault(undefined, "openai-compatible")).toBe(true);
    expect(functionCallingDefault(modelWith("supported"), "openai-compatible")).toBe(true);
    expect(functionCallingDefault(modelWith("conditional"), "openai-compatible")).toBe(true);
    expect(functionCallingDefault(modelWith("unknown"), "openai-compatible")).toBe(true);
    expect(functionCallingDefault(modelWith("unsupported"), "openai-compatible")).toBe(false);
    expect(functionCallingDefault(modelWith("unsupported"), "openai-responses")).toBe(true);
  });

  it("initializes template-owned values and preserves user overrides on ordinary routes", () => {
    const unsupported = modelWith("unsupported");
    const supported = { ...modelWith("supported"), id: "fixture/supported" };
    const initialized = applyEndpointTemplate(blank(), unsupported);
    expect(initialized.function_calling).toBe(false);
    expect(initialized.requirements?.field_sources.function_calling).toBe("template");

    const overridden = editEndpoint(initialized, { function_calling: true });
    expect(overridden.requirements?.field_sources.function_calling).toBe("user");
    const switchedTemplate = applyEndpointTemplate(overridden, supported);
    expect(switchedTemplate.function_calling).toBe(true);
    expect(switchedTemplate.requirements?.field_sources.function_calling).toBe("user");
    expect(changeEndpointProtocol(switchedTemplate, "anthropic", supported).function_calling).toBe(true);
  });

  it("forces Responses on even after a user disabled function calling", () => {
    const ordinary = modelWith("supported");
    const disabled = editEndpoint(applyEndpointTemplate(blank(), ordinary), { function_calling: false });
    const responses = changeEndpointProtocol(disabled, "openai-responses", ordinary);
    expect(responses.function_calling).toBe(true);
    expect(responses.requirements?.field_sources.function_calling).toBe("template");
  });
});

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

  it("suggests fields, preserves user overrides while a template switch adopts the address", () => {
    let draft = applyEndpointTemplate(blank(), template);
    expect(draft.model).toBe(template.model);
    expect(draft.requirements?.field_sources.model).toBe("template");
    draft = editEndpoint(draft, {model:"manual-model",base_url:"https://proxy.test/v1",max_llm_output_tokens:25000});
    draft = editEndpointRequirements(draft, {allowed_reasoning:["low"],adaptive_reasoning:false});
    draft = applyEndpointTemplate(draft, {...template,id:"other",model:"other-model",default_effort:"high"});
    expect(draft.model).toBe("manual-model");
    // Selecting a different template adopts its address; the proxy must be
    // re-entered afterwards if it should survive future switches.
    expect(draft.base_url).toBe(template.base_url);
    expect(draft.requirements?.field_sources.base_url).toBe("template");
    expect(draft.max_llm_output_tokens).toBe(25000);
    expect(draft.requirements?.allowed_reasoning).toEqual(["low"]);
    expect(draft.requirements?.adaptive_reasoning).toBe(false);
    expect(draft.reasoning_effort).toBe("high"); // invalid combination stays visible, not silently repaired
    expect(applyEndpointTemplate(draft)).toEqual({...draft,catalog_id:null});
  });
  it("migrates legacy values as user-owned without treating absent capability fields as overrides", () => {
    const legacy = {...blank(),id:"old",api_key_configured:false,private_ca_configured:false,catalog_id:template.id};
    delete legacy.requirements;
    const requirements = initialEndpointRequirements(legacy,[template]);
    expect(requirements.provider).toBe("openai");
    expect(requirements.field_sources.model).toBe("user");
    expect(requirements.field_sources.provider).toBeUndefined();
    expect(requirements.field_sources.allowed_reasoning).toBeUndefined();
    expect(requirements.field_sources.adaptive_reasoning).toBeUndefined();
    expect(initialEndpointRequirements({...legacy,catalog_id:"unknown"},[template]).provider).toBeUndefined();
    const migrated = {...legacy, requirements};
    const reapplied = applyEndpointTemplate(migrated,template);
    expect(reapplied.model).toBe(legacy.model);
    expect(reapplied.requirements?.provider).toBe(template.provider);
    expect(reapplied.requirements?.allowed_reasoning).toEqual(template.efforts);
    expect(reapplied.requirements?.adaptive_reasoning).toBe(true);
    const copy = initialEndpointRequirements({...legacy,requirements});
    copy.field_sources.model = "template";
    expect(requirements.field_sources.model).toBe("user");
  });

  it("repairs an unsupported legacy daily level as soon as a template is selected", () => {
    const glm53 = {
      ...template,
      id: "z-glm5.3",
      provider: "zhipu",
      model: "glm-5.3",
      efforts: ["low", "high", "max"],
      default_effort: "max",
      protocols: [{ ...template.protocols[0], protocol: "openai-compatible" }],
    };
    const legacy = {
      ...blank(),
      model: "glm-5.3",
      reasoning_effort: "medium",
      requirements: {
        version: 1,
        field_sources: { model: "user" as const, reasoning_effort: "user" as const },
      },
    };
    const applied = applyEndpointTemplate(legacy, glm53);
    expect(applied.reasoning_effort).toBe("max");
    expect(applied.requirements?.allowed_reasoning).toEqual(["low", "high", "max"]);
    expect(applied.requirements?.field_sources.reasoning_effort).toBe("template");
    expect(endpointCapabilityIssue(applied, glm53)).toBeNull();
  });

  it("preserves a supported user daily level when selecting a template", () => {
    const glm53 = {
      ...template,
      id: "z-glm5.3",
      provider: "zhipu",
      model: "glm-5.3",
      efforts: ["low", "high", "max"],
      default_effort: "max",
      protocols: [{ ...template.protocols[0], protocol: "openai-compatible" }],
    };
    const manual = editEndpoint(blank(), { model: "glm-5.3", reasoning_effort: "high" });
    const applied = applyEndpointTemplate(manual, glm53);
    expect(applied.reasoning_effort).toBe("high");
    expect(applied.requirements?.field_sources.reasoning_effort).toBe("user");
  });

  it("uses the explicitly selected template capability while a legacy endpoint is being migrated", () => {
    const legacyDraft = {
      ...blank(),
      catalog_id: template.id,
      model: template.model,
      requirements: { version: 1, field_sources: { model: "user" as const } },
    };
    expect(capabilityModelForDraft(legacyDraft, [template])).toBe(template);
    expect(capabilityModelForDraft({
      ...legacyDraft,
      requirements: { ...legacyDraft.requirements, provider: "other" },
    }, [template])).toBe(template);
    expect(capabilityModelForDraft({ ...legacyDraft, model: "manual-model" }, [template])).toBeUndefined();
  });

  it("keeps max available in the generic reasoning fallback", () => {
    expect(REASONING_EFFORT_OPTIONS).toContain("max");
  });

  it("admits Zhipu Responses through the provider gate while catalog profiles decide", () => {
    const dual: CatalogModel = {
      ...template,
      id: "z-glm-dual",
      provider: "zhipu",
      model: "glm-dual",
      label: "Dual",
      base_url: "https://open.bigmodel.cn/api/paas/v4",
      efforts: ["low", "high", "max"],
      default_effort: "max",
      protocols: [
        { protocol: "openai-compatible", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null },
        { protocol: "openai-responses", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null, base_url: "https://open.bigmodel.cn/api/v1" },
      ],
    };
    const draft = editEndpointRequirements(editEndpoint(blank(), {
      catalog_id: dual.id,
      model: dual.model,
      api_protocol: "openai-responses",
      base_url: "https://open.bigmodel.cn/api/v1",
      reasoning_effort: "max",
    }), { provider: "zhipu", allowed_reasoning: ["low", "high", "max"] });
    expect(endpointCapabilityIssue(draft, dual, providerSpecs)).toBeNull();
    const chatOnly = { ...dual, protocols: [dual.protocols[0]] };
    expect(endpointCapabilityIssue(draft, chatOnly, providerSpecs)?.code).toBe("protocol_unsupported");
    expect(endpointCapabilityIssue({ ...draft, api_protocol: "anthropic" }, dual, providerSpecs)?.code).toBe("provider_protocol_unsupported");
    expect(endpointCapabilityIssue({ ...draft, catalog_id: undefined, model: "glm-air" }, undefined, providerSpecs)?.code).toBe("model_capabilities_missing");
    expect(endpointCapabilityIssue({ ...draft, requirements: { ...draft.requirements!, provider: undefined } }, dual, providerSpecs)).toBeNull();
  });

  it("switches the template base URL by value even when the address was entered manually", () => {
    const dual: CatalogModel = {
      ...template,
      id: "z-glm-dual",
      provider: "zhipu",
      model: "glm-dual",
      label: "Dual",
      base_url: "https://open.bigmodel.cn/api/paas/v4",
      protocols: [
        { protocol: "openai-compatible", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null },
        { protocol: "openai-responses", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null, base_url: "https://open.bigmodel.cn/api/v1" },
      ],
    };
    const manual = editEndpoint(blank(), {
      catalog_id: dual.id,
      model: dual.model,
      api_protocol: "openai-compatible",
      base_url: "https://open.bigmodel.cn/api/paas/v4",
    });
    const switched = changeEndpointProtocol(manual, "openai-responses", dual);
    expect(switched.base_url).toBe("https://open.bigmodel.cn/api/v1");
    expect(switched.requirements?.field_sources.base_url).toBe("template");
    const proxy = editEndpoint(manual, { base_url: "https://proxy.example.test/v1" });
    expect(changeEndpointProtocol(proxy, "openai-responses", dual).base_url).toBe("https://proxy.example.test/v1");
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
  it("follows per-protocol template addresses while preserving custom values", () => {
    const first = applyEndpointTemplate(blank(), protocolTemplate);
    const custom = editEndpoint(first,{base_url:"https://proxy.test/v1"});
    expect(changeEndpointProtocol(custom,"openai-compatible",protocolTemplate).base_url).toBe("https://proxy.test/v1");
    const official = editEndpoint(first,{base_url:first.base_url});
    const switched = changeEndpointProtocol(official,"openai-compatible",protocolTemplate);
    expect(switched.base_url).toBe(protocolTemplate.protocols[1].base_url);
    expect(switched.requirements?.field_sources.base_url).toBe("template");
    expect(changeEndpointProtocol(editEndpoint(first,{base_url:""}),"openai-compatible",protocolTemplate).base_url).toBe("");
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
  it("adopts the template address when switching to a different template", () => {
    const legacy = editEndpoint(blank(), { model: "legacy-model", base_url: "https://open.bigmodel.cn/api/coding/paas/v4", api_protocol: "openai-compatible" });
    const applied = applyEndpointTemplate(legacy, protocolTemplate);
    expect(applied.base_url).toBe(protocolTemplate.protocols[1].base_url);
    expect(applied.requirements?.field_sources.base_url).toBe("template");
    expect(changeEndpointProtocol(applied, "openai-responses", protocolTemplate).base_url).toBe(protocolTemplate.protocols[0].base_url);
  });

  it("keeps a custom address when re-applying the same template", () => {
    const first = applyEndpointTemplate(blank(), protocolTemplate);
    const proxied = editEndpoint(first, { base_url: "https://proxy.test/v1" });
    const again = applyEndpointTemplate(proxied, protocolTemplate);
    expect(again.base_url).toBe("https://proxy.test/v1");
    expect(again.requirements?.field_sources.base_url).toBe("user");
  });

  it("keeps a shared model address across protocol switches", () => {
    const shared: CatalogModel = { ...template, protocols: [
      { protocol: "openai-responses", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null },
      { protocol: "openai-compatible", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null },
    ] };
    const draft = applyEndpointTemplate(blank(), shared);
    expect(draft.base_url).toBe(template.base_url);
    const chat = changeEndpointProtocol(draft, "openai-compatible", shared);
    expect(chat.base_url).toBe(template.base_url);
    expect(chat.requirements?.field_sources.base_url).toBe("template");
  });

  it("uses per-protocol entries and falls back to the model address per protocol", () => {
    const mixed: CatalogModel = { ...template, protocols: [
      { protocol: "openai-responses", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null, base_url: "https://responses.mixed.test/v1" },
      { protocol: "openai-compatible", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null },
    ] };
    const draft = applyEndpointTemplate(blank(), mixed);
    expect(draft.base_url).toBe("https://responses.mixed.test/v1");
    const chat = changeEndpointProtocol(draft, "openai-compatible", mixed);
    expect(chat.base_url).toBe(template.base_url);
    expect(changeEndpointProtocol(chat, "openai-responses", mixed).base_url).toBe("https://responses.mixed.test/v1");
  });

  it("trims whitespace when matching template addresses during a protocol switch", () => {
    const spaced = editEndpoint(applyEndpointTemplate(blank(), protocolTemplate), { base_url: " https://responses.example.test/v1 " });
    const switched = changeEndpointProtocol(spaced, "openai-compatible", protocolTemplate);
    expect(switched.base_url).toBe(protocolTemplate.protocols[1].base_url);
    expect(switched.requirements?.field_sources.base_url).toBe("template");
  });

  it("keeps the address when the target template has none for the user protocol", () => {
    const chatOnly: CatalogModel = { ...template, protocols: [protocolTemplate.protocols[1]] };
    const legacy = editEndpoint(blank(), { api_protocol: "openai-responses", base_url: "https://proxy.test/v1" });
    const applied = applyEndpointTemplate(legacy, chatOnly);
    expect(applied.base_url).toBe("https://proxy.test/v1");
    expect(applied.requirements?.field_sources.base_url).toBe("user");
  });

  it("falls back to the model URL and still follows known template values for legacy endpoints", () => {
    expect(templateBaseUrl(template,"openai-responses")).toBe(template.base_url);
    const draft = {...applyEndpointTemplate(blank(),protocolTemplate),requirements:undefined};
    expect(changeEndpointProtocol(draft,"openai-compatible",protocolTemplate).base_url).toBe(protocolTemplate.protocols[1].base_url);
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
    const followed = changeEndpointProtocol(manual,"openai-compatible",protocolTemplate);
    expect(followed.base_url).toBe(protocolTemplate.protocols[1].base_url);
    expect(canRestoreEndpointTemplateUrl(followed,protocolTemplate)).toBe(false);
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
    expect(endpointProtocolOptions(draft,undefined,providerSpecs).some(p=>p.protocol==="anthropic")).toBe(false);
    const zhipu = {...draft,api_protocol:"openai-compatible",requirements:{...draft.requirements!,provider:"zhipu"}};
    expect(endpointProtocolOptions(zhipu,undefined,providerSpecs)).toEqual([{protocol:"openai-compatible",disabled:false},{protocol:"openai-responses",disabled:false}]);
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
    expect(restored.reasoning_effort).toBe("low");
    expect(restoreEndpointTemplateReasoning(manual, { ...template, id: "other" })).toBe(manual);
  });

  it("repairs an unsupported legacy daily level when restoring template reasoning", () => {
    const glm53 = {
      ...template,
      id: "z-glm5.3",
      model: "glm-5.3",
      provider: "zhipu",
      efforts: ["low", "high", "max"],
      default_effort: "max",
    };
    const legacy = {
      ...blank(),
      catalog_id: glm53.id,
      model: glm53.model,
      reasoning_effort: "medium",
      requirements: {
        version: 1,
        field_sources: { model: "user" as const, reasoning_effort: "user" as const },
      },
    };
    const restored = restoreEndpointTemplateReasoning(legacy, glm53);
    expect(restored.reasoning_effort).toBe("max");
    expect(restored.requirements?.allowed_reasoning).toEqual(["low", "high", "max"]);
    expect(restored.requirements?.field_sources.allowed_reasoning).toBe("template");
    expect(restored.requirements?.field_sources.reasoning_effort).toBe("template");
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

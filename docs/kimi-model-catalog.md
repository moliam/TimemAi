# Kimi 模板

## 接入范围

Core 内置目录提供以下 Kimi 普通API 模板。使用 Bearer API Key，Chat 与 Responses
协议默认地址均为 `https://api.moonshot.cn/v1`，请求路径分别为 `/chat/completions`、`/responses`。
可覆盖代理地址；适配器根据显式供应商、模型能力与协议选择，不根据地址或模型名前缀猜测。
用户在模板中选择模型后最多只需填写 API Key。

| 模板 | 模型 ID | 协议 | 推理选项 | 默认 |
| --- | --- | --- | --- | --- |
| k-k3.1-pro | kimi-k3.1-pro | Chat + Responses | low / high / max | max |
| k-k3.1-pro-highspeed | kimi-k3.1-pro-highspeed | Chat + Responses | low / high / max | max |
| k-k3 | kimi-k3 | Chat + Responses | low / high / max | max |
| k-k2.6 | kimi-k2.6 | Chat | none / high | high |
| k-k2.7-code | kimi-k2.7-code | Chat | high（恒思考） | high |

三个模型均原生支持视觉理解，`documented_facts.input_modalities` 为 `text/image`，输出为 `text`。
上下文窗口按文档为 1,000,000；输出上限默认 131,072（可设置至 1,048,576）。
独立最大输入上限未明确公布，因此资源中为 null；配置检查仍要求输入预算与输出预算之和不超过窗口。
这不是最终请求的精确 tokenizer 校验，也不代表百万 token 输入已实测。
K2 系列使用 K2.x 专属 `thinking` 参数（`kimi-k2.6` 支持 enabled/disabled/keep:all，`kimi-k2.7-code` 恒思考不应传入），
上下文窗口 256K；此前因无法映射而 fail-closed，现以 `kimi_k2_thinking` 适配器入目录。
模型可用性因账户而异：部分 key 仅开通 K2 系列（此时 K3 模板请求会得到 404 resource_not_found），
`GET /v1/models` 可查询当前 key 的可用模型。

## 请求与工具往返

- `kimi_chat_reasoning`（K3）与 `kimi_k2_thinking`（K2）是 Core 中的有限、显式适配器；不是任意配置执行器。
- K2 Chat 请求按档位写入 `thinking`：kimi-k2.6 的 `none` 档发送 `{"type":"disabled"}`、`high` 档发送 `{"type":"enabled","keep":"all"}`（保留式思考，配合续轮回传）；kimi-k2.7-code 不发送 `thinking`（文档明确不应传入，服务端恒开）。K2 不支持 `reasoning_effort`。
- K2 的多轮/工具续轮同样要求原样回传 assistant message（含 `reasoning_content`）；不支持 `tool_choice:"required"`（Timem 仅在上下文压缩等关键轮使用 required，该场景下 K2 走 Auto）。
- Chat 请求在顶层写入 `reasoning_effort`（low/high/max，默认 max）；不发送 `thinking`、`enable_thinking`。
- Responses 协议使用标准 `reasoning.effort` 体字段绑定（`enum_body_field` → `/reasoning/effort`），请求不携带 `reasoning_effort` 顶层字段。
- K3 系列多轮对话与工具调用要求把 API 返回的完整 assistant message（含 `reasoning_content` 和 `tool_calls`）原样回传；该语义与智谱 Chat 相同，Core 在同一能力身份下保存并回传一次，不展示为正文，切换模型身份后不回传。
- `stream_options`（`include_usage=true`）按官方文档允许，随流式请求发送。
- 推理续接内容上限为 4 MiB，超限明确报错，不静默截断；上下文估算包含续接内容。
- Core 在网络发送前检查模型、协议、强度字段一致性（effort 必须为 low/high/max，且无 thinking/enable_thinking）和配置预算。

## Web

模板数据由宿主投影，Web 不硬编码 Kimi 规则。模板厂商标签 zh/en 均为 "Kimi"，
与 Zhipu/OpenAI 统一在下拉模板入口。协议级 Base URL、预算直接输入等行为与智谱模板一致（见 provider-model-descriptor.md）。

## Kimi Code（订阅制 Coding Plan）

订阅制 Kimi Code 是独立接入体系（与开放平台 API key 不通用，控制台签发的 key 以 `sk-kimi-` 开头）：
Base URL 国内 `https://api.kimi.com/coding/v1`、海外 `https://api.kimi.ai/coding/v1`，兼容 OpenAI Responses / Chat Completions 与 Anthropic 协议。
模板默认走 Responses（官方推荐原生协议以获得最佳效果稳定性）。用户只需选择模板并填 API Key。

| 模板 | 模型 ID | 协议 | 推理选项 | 默认 | 上下文 | 输入模态 |
| --- | --- | --- | --- | --- | --- | --- |
| kimicode_k3.1-pro | k3.1-pro | Responses + Chat | low / high / max | high | 1M* | 文本/图片/视频 |
| kimicode_k3.1-pro-highspeed | k3.1-pro-highspeed | Responses + Chat | low / high / max | high | 1M* | 文本/图片/视频 |
| kimicode_k3 | k3 | Responses + Chat | low / high / max | high | 1M* | 文本/图片/视频 |
| kimicode_k3-256k | k3-256k | Responses + Chat | low / high / max | high | 256K | 文本/图片 |
| kimicode_kimi-for-coding | kimi-for-coding | Responses + Chat | low / high / max | max | 1M | 文本/图片/视频 |

模板 ID 统一以 `kimicode_` 前缀区分订阅制体系；开放平台模板保持 `k-` 前缀（provider `kimi`），两套体系互不通用 key。
K3.1 Pro 系列默认档为端点 `/v1/models` 实测的 `high`（文档规格表写 max，以端点为准，见 api-models 来源）。

*1M 上下文需 Pro（新套餐）/Allegretto（老套餐）及以上；Plus/Moderato 档位下 k3.1-pro/k3/k3-256k 为 256K。
两套套餐之外的细节：K2.8 Preview（kimi-for-coding）消耗约为 K3 的两倍内；高速版速度 2-3 倍、消耗约两倍。
目录 fail-closed 项：none 档一律不入（关闭思考或被拒、或静默路由到 K2.8 Preview 无思考版）；`kimi-for-coding-highspeed` 思考程度不可调（仅 Thinking:ON），无法用现行维度绑定表达。
模型 ID 必须精确填写（如 `k3.1-pro`），官方文档警告填错会静默兜底到 K2.8 Preview 无思考版。

## 证据与验证

`resources/kimi_model_catalog.sources.json` 保存官方页面（platform.kimi.com/docs）URL、抓取时间、页面 SHA-256 与相关摘录。
SHA-256 标识当次完整页面；仓库只保留摘录，离线校验不冒充重新验证远端全文。
运行时集成状态为 `runtime_supported`；官方文档审核状态独立保留（`binding_status=api_reference_reviewed`）。

- `python3 scripts/validate_kimi_model_catalog.py`：schema、五个模板、Chat/Responses profile 及来源引用检查。
- `python3 scripts/validate_kimi_code_model_catalog.py`：Kimi Code（Coding Plan）schema、五个模板、Responses 优先 profile、来源引用与负例检查。
- `cargo test -p agent_core --lib kimi_code`：Responses/Chat 双协议 wire 与 none 档拒绝。
- `cargo test -p timem kimi_code_catalog`：Coding Plan 模板宿主准入与磁盘往返。
- `cargo test -p agent_core --lib kimi`：请求约束、wire guard、流式工具续轮 `reasoning_content` 保存回传。
- `cargo test -p agent_core --test model_catalog_tests`：kimi 目录准入与契约（含外部 JSON 扩展）。
- `cargo test -p timem kimi_catalog`：宿主准入与磁盘保存/重读。

这些测试不使用真实用户配置或收费 API，不声明 Kimi 远端模型已实测验收。

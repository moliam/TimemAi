# 智谱 GLM 模板

## 接入范围

Core 内置目录提供以下普通 API 模板。使用 Bearer API Key，Chat 协议默认地址为
`https://open.bigmodel.cn/api/paas/v4`，请求路径 `/chat/completions`。
可覆盖代理地址；适配器根据显式供应商、模型能力与协议选择，不根据地址或模型名前缀猜测。
z-glm5.3 另提供官方 Responses 协议入口：独立基址 `https://open.bigmodel.cn/api/v1`，
请求路径 `/responses`，推理绑定 `reasoning.effort`（默认 `max`）。
官方 Responses 侧将 `low`/`medium` 归并为 `high`、`xhigh` 归并为 `max`、`none`/`minimal` 放弃思考；
模板沿用模型档位 `low/high/max`，不新增兼容别名，也不提供关闭思考档（Chat 协议下 5.3 禁止关闭思考）。

| 模板 | 模型 ID | 协议 | 推理选项 | 默认 |
| --- | --- | --- | --- | --- |
| z-glm5.2 | glm-5.2 | Chat | none / high / max | max |
| z-glm5.3 | glm-5.3 | Chat + Responses | low / high / max | max |
| z-glm5.3-flash | glm-5.3-flash | Chat | low / high / max | max |

模板只列原生强度档位，不列 5.2 的兼容别名。5.3 系列不允许关闭思考。
上下文窗口按文档为 1,000,000；输出上限按 API schema 为 131,072。
独立最大输入上限未明确公布，因此资源中为 null；配置检查仍要求输入预算与输出预算之和不超过窗口。
这不是最终请求的精确 tokenizer 校验，也不代表百万 token 输入已实测。

本实现不声明 Anthropic 接口兼容；Responses 协议仅对目录声明的 z-glm5.3 开放，5.2 与 Flash 未提供该入口。
不声明 Timem 获得 Coding Plan 使用资格。
Coding Plan 的工具白名单、账户与计费限制独立于普通 API 协议适配。
Flash 的文档能力包含图片、视频与文件，但目录中的文档事实不等于 Timem 已实现全部输入格式。

## 请求与工具往返

- `zhipu_chat_reasoning` 是 Core 中的有限、显式适配器；不是任意配置执行器。
- Chat 请求发送 `thinking.type`、`reasoning_effort`；移除通用兼容层的 `enable_thinking` 和 `stream_options`。
- Responses（z-glm5.3）使用标准 `reasoning.effort` 体字段绑定，请求不携带 `thinking`、`reasoning_effort` 或 `enable_thinking`；`reasoning_content` 的原样保存与回传是 Chat 协议行为，Responses 不适用。
- 使用 `thinking.clear_thinking: true`。不启用跨 turn Preserved Thinking，因为上下文压缩可能删除历史。
- 当前工具往返中的 `reasoning_content` 原样保存，挂在首个 NativeToolCall 的可选 assistant continuation 元数据中，重建 assistant message 时回传一次。它不属于工具参数、可见正文或流式正文预览。
- 元数据记录能力描述 ID 与模型 ID，切换模型身份后不回传；仅修改模板建议来源不改变模型身份。旧记录缺少该字段时按 None 读取，不改写原记录。
- 推理续接内容上限为 4 MiB，超限明确报错，不静默截断；上下文估算包含续接内容。压缩后的 scratch 摘录保留公共工具语义，不把推理内容变成公共正文。
- Core 在网络发送前检查模型、协议、强度、thinking 字段一致性和配置预算。

## Web

模板数据由宿主投影，Web 不硬编码智谱规则。模板来源的 Base URL 随模板和协议切换（协议未指定入口时继承模型默认地址）；手动填写的地址保留，即使与模板值相同。显式恢复默认地址会填入当前协议模板地址，并恢复自动联动。保存后保留字段来源；缺少来源信息的旧配置按手动值处理。
输入与输出预算保留直接数字输入和范围校验，隐藏浏览器微调按钮，并阻止 ArrowUp/ArrowDown 的单 token 加减。

## 证据与验证

`resources/zhipu_model_catalog.sources.json` 保存官方页面 URL、抓取时间、页面 SHA-256 与相关摘录。
SHA-256 标识当次完整页面；仓库只保留摘录，离线校验不冒充重新验证远端全文。
运行时集成状态为 `runtime_supported`；官方文档审核状态独立保留。外部新模型可按 `docs/provider-model-descriptor.md` 部署，但仅能引用已实现适配器。

- `python3 scripts/validate_zhipu_model_catalog.py`：schema、三个模板、Chat/Responses profile 及来源引用检查。
- `cargo test -p agent_core --lib zhipu`：请求约束、JSON/SSE 解析、旧字段兼容、跨模型隔离、容量拒绝、真实本地 HTTP 双请求工具往返。
- `cargo test -p timem zhipu_catalog`：宿主准入与磁盘保存/重读。
- `TIMEM_CATALOG_TEST_URL=<独立临时宿主> node interfaces/web/tests/browser/catalog-endpoint-e2e.mjs`：真实浏览器模板列表、地址、预算直接输入、无微调、保存重读、窄屏。

这些测试不使用真实用户配置或收费 API。浏览器配置测试与模拟 HTTP 协议测试分层运行，不能合称完整的智谱远端模型 E2E。

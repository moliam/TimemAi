# OpenAI 模型白名单

规范见 [Provider–Model 描述协议 v2](provider-model-descriptor.md)。

- `resources/provider_model_catalog.schema.json`：严格 JSON Schema，Draft 2020-12。
- `resources/openai_model_catalog.json`：v2 目录、revision 2，9 个模型。
- `resources/openai_model_catalog.sources.json`：此前逐页读取的原始证据、URL、抓取时间、SHA-256；此次迁移未重写证据。
- `scripts/validate_openai_model_catalog.py`：离线结构、语义、证据校验与负例。

本批已把协议纳入 dimensions，记录推理初始化策略、结构化跨维度约束和协议绑定。
默认 URL 可由用户覆盖。描述仍待 runtime 接入；未改变当前接入点 UI 或请求行为。

| 模型 | 推理选项 | 初始值 | 例外 |
|---|---|---|---|
| GPT-5.6 Sol/Terra/Luna | none, low, medium, high, xhigh, max | medium | — |
| GPT-6 Sol/Luna | none, low, medium, high, xhigh, max | medium | Chat 原生函数调用固定 none |
| GPT-6 Astra | low, medium, high, xhigh, max | high | 官方未注明，取中间档 |
| GPT-6.1 Sol | low, medium, high, xhigh, max | medium | 原生函数调用禁用 Chat |
| GPT-5.5 | none, low, medium, high, xhigh | medium | 无 max |
| GPT-5.5 Pro | medium, high, xhigh | high | 仅 Responses |

GPT-5.5/Pro 未注明最大输入，保留 null；Pro 未列出 streaming，记 unknown，不能推断不支持。
temperature/top_p 等未核实维度不加入配置。网页平台工具不等于 Timem Function calling。

验证：`python3 scripts/validate_openai_model_catalog.py --self-test`。

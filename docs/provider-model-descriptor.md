# Provider–Model 描述协议 v2

## 1. 范围与权威文件

机器格式：`resources/provider_model_catalog.schema.json`（JSON Schema Draft 2020-12）。
首批目录：`resources/openai_model_catalog.json`、`resources/zhipu_model_catalog.json`。证据快照：同目录 `.sources.json`。
`activation=runtime_supported` 表示 Core 已实现本文限定的加载、约束及映射，不代表远端 API 实测。
profile 的 `binding_status` 独立记录官方 API 文档审核状态，不能因本地回归通过而改成已审核。
接入点分层与来源优先级见 [模型接入点架构](model-endpoint-architecture.md)。

根 schema_version=2 是格式版本；目录和条目的 revision 是数据版本。v1 到 v2 是不兼容的
维护期迁移，旧应用不能直接消费；没有修改用户已有 endpoint store。
未知行为字段、未知类型、未知处理器必须拒绝激活。元数据不会自动变成业务行为。

## 2. 结构与字段消费者

| 字段 | 含义 | 预期消费者 |
|---|---|---|
| provider_id / model_id | 精确服务商–模型身份，不做前缀猜测 | 目录选择、请求模型 ID |
| connection.default_base_url | pair 的默认入口，实例可覆盖 | URL 解析器 |
| connection.auth_handler | 已注册鉴权处理器，不含 Key | 发送前的凭据注入 |
| dimensions | 该模型允许配置的维度白名单 | 表单投影、选择校验 |
| profiles[].default_base_url | 可选的协议入口，缺省继承 connection.default_base_url；Core 投影解析后的 base_url | URL 解析器 / 模板建议 |
| profiles | 协议 route、绑定、Function calling 能力 | 请求适配器 |
| limits | 模型硬边界；null 是未知而非无限 | Core 预算与上下文组合校验 |
| constraints | 维度间及上下文条件约束 | Core 约束处理器 |
| documented_facts | 页面注明的能力事实、快照信息 | 说明展示，不直接写请求 |
| source_refs | 来源快照键 | 审查和证据校验 |

有效地址是实例覆盖值优先，否则 pair 默认值。去掉 base 的末尾斜杠后追加 route；不得丢失
自定义路径前缀。改写地址不意味着代理已验证兼容；更换目标域名需确认凭据是否继续使用。
保存配置不需要 Key、联网或额度；调用前另行检查。

## 3. 维度与选项

v2 可执行契约的类型目前严格限定 enum，ID 限定 api.protocol 与 reasoning.effort，
handler 分别为 api_protocol 与 reasoning_effort。其他维度属于后续版本扩展，不能在 v2
随意塞入 JSON。

每个维度含 id、label、type、ordered、allow_unset、options、default_policy、handler、ui、
source_refs。options 是允许值列表，id 用于持久化，label 只展示。ID、协议、选项、规则 ID
必须唯一。协议选项必须与 profiles 一一对应，profile 的 dimension_ids 必须存在。

不支持或未核实的维度不进入 dimensions，不显示灰色占位项。被条件限制的已支持维度则按
规则禁用或固定。未知不等于不支持。

api.protocol 是用户可选维度，不再只作为隐含外部状态；tool_mode 是运行上下文：
- native_function_calling：API 原生函数调用。
- text_protocol：Timem 文本/XML 工具路径。
- none：不使用工具。
上下文由 Core 提供，客户端不能通过伪造上下文绕过实际调用检查。

## 4. 默认值算法与标记

用户显式值优先于模板建议，但不能突破能力约束；冲突拒绝保存/发送，不静默覆盖。
初始化时：官方默认优先；未注明则取 fallback.candidates 中间档，偶数取靠低的一项
（索引 floor((n-1)/2)）。候选顺序来自显式列表，关闭档 none 和服务默认状态不参与排序。
候选列表必须是 options 的有序子集，不得重复。官方默认必须在 options 内。

协议没有高低顺序，使用 explicit/product 策略，首批产品默认 Responses。
Astra 的 official_value=null，中间候选 low/medium/high/xhigh/max，故初始选择 high。
其他模型使用页面注明的默认值。

表单输出 is_default，由 UI 给相应 label 加全角后缀 `（默认）`：例如 medium（默认）。
标记不随用户当前选择变化，不写进 option ID；来源同时输出为 official、middle 或 product。
使用中间档时必须显示原文：

> 注：官方未注明默认值，已选择中间档

实际选中的初始化值显式发送。新需求版本中，“默认”由 Core 解析为已知模型日常默认值并参与允许集合校验；未知模型保留未指定语义，不猜测能力。
编辑不重新初始化；默认值更新不得静默覆盖旧选择。固定值不冒充默认值。

若后续规则限制选项，初始化必须在最终合法候选内计算；无合法候选即报错，不回填非法默认。
官方默认受限时不可仍标为当前可用默认。此类规则尚未包含在本批 v2 操作集合中。

## 5. 约束与确定性

本版仅接纳首批所需的两个效果，不假称已实现任意规则语言：

### fixed_selection

条件成立后，合法值限定为 value；草稿仍可编辑，冲突必须显式报错；
reason 是禁用原因；send_policy=explicit 要求最终请求显式写入，不能省略。
固定值必须属于目标 options。多个有效固定值必须一致，禁止最后写入者覆盖。

GPT-6 Sol、GPT-6 Luna：api.protocol=openai-compatible 且 tool_mode=native_function_calling
时，reasoning.effort 固定 none。限制不适用于 text_protocol/none。

### disable_option

条件成立后，维度指定选项不可选，并提供 reason；当前值命中禁用项则配置不合法，
保存/调用拒绝，不能静默切协议或工具模式。
GPT-6.1 Sol 在 native_function_calling 时禁用 Chat Completions。

v2 的 when.all 是有界条件合取；叶子只能比较已注册的协议维度或 tool_mode 上下文。
禁止未知引用、空条件、矛盾固定值、禁用后无剩余选项。规则按集合解释，不依赖文件顺序。
本版固定效果仅修改推理维度，条件只读协议/上下文，不存在自动修改循环。

UI 切换协议时立即展示限制原因，不重置显式值；冲突草稿不能保存或发送。
若提交仍有与固定值冲突的显式值，Core 报错，不保存 high 却发送 none。

## 6. 绑定与工具语义

profiles.bindings 由严格注册处理器执行，目前仅允许：
- OpenAI 的 `enum_body_field`，按以下协议路径写入。
- openai-responses：reasoning.effort → /reasoning/effort。
- openai-compatible：reasoning.effort → /reasoning_effort。

路径是 JSON Pointer，必须与所选协议的准许路径匹配；不能覆盖 model/messages/input/tools
或凭据。最终请求须再次检查，用户扩展字段不得覆盖受管值。
智谱使用 `zhipu_chat_reasoning`，仅支持 Chat：同时写入 `reasoning_effort` 和 `thinking.type`、`thinking.clear_thinking=true`；不发送 `enable_thinking`／`stream_options`。工具续轮中的不透明 `reasoning_content` 仅在相同能力身份下回传，不展示为正文。
上述映射已接入本地请求链路；文档审核状态与远端可用性仍独立。

Timem 原生工具（如 run_bash）对应 Function calling；网页内置 web_search、code_interpreter
等平台工具不等于 Timem 工具。本目录不据此开放本地工具。
function_calling=conditional 必须有对应规则；unsupported 在原生工具模式下必须有禁用规则。

## 7. 输入输出与扩展边界

context_window_tokens、max_input_tokens、max_output_tokens 当前保存在 limits，有来源；
接入点输入/输出预算为独立字段，不把硬上限当默认预算。最大输入为 null 时仅使用上下文总窗口作为上界，而非声称已知官方独立输入上限。
Core 同时要求输入≥3000、输出≥512、各自不超上限、输入加输出不超上下文窗口。

若将预算进一步纳入任意动态维度协议，必须同时扩展：
1. Schema 的数值类型、范围与初始化规则。
2. Core 的输入预算处理器/输出请求映射。
3. 上下文与输出预留组合约束。
4. UI 数值投影和边界测试。
输入预算是本地行为，不能当作所有 API 都支持的请求字段。
温度、top-p、推理预算等也遵循相同流程；没有实现的 handler 不允许激活。
restrict_options、restrict_range、require_value、forbid_value 是后续候选效果，本版不接受。

## 8. 已实现的有限投影

parse_descriptor → 验证结构/引用/处理器 → resolve_constraints → 表单投影/有效配置。
新建、编辑、保存、发送共用 Core 语义；Web 不识别模型名，不另写厂商规则。
当前投影包含模型身份、协议、推理选项/默认、限制原因和预算范围；不声称支持任意动态表单语言。
数据合法不等于当前程序有能力执行：版本/处理器兼容检查是独立激活门禁。

## 9. 校验

```sh
python3 scripts/validate_openai_model_catalog.py --self-test
```

依赖 Python 3 与 jsonschema（Draft 2020-12）。schema 自校验、关闭未知字段、语义引用与映射、
原文哈希、精确选项/默认/限制及负例一并验证。该脚本针对本批 9 模型，不是通用 Rust runtime。
独立执行 JSON Schema 只保证结构，不保证文档证据或业务语义。

## 10. 新模型配置部署

1. 依据官方资料生成 v2 JSON：供应商、精确 model_id、协议/route、推理选项与有序关系、默认值、工具约束、token 上限及证据。
2. 选择已实现的适配器。不要只改旧模型名称而照搬其能力；未知事实不能伪装成已验证。
3. 将描述文件放入独立目录，通过 `TIMEM_MODEL_CATALOG_DIR=/absolute/path/to/catalog` 启动新实例；目录仅放模型描述 `.json`，schema 和 `.sources.json` 不放入该加载目录。
4. 启动时合并内置与外部描述；新增模板自动投影到接入点编辑器，无需修改 Rust/TS 或重新编译。当前实例不热加载，需在合适时机重新启动以读取新快照。

每文件≤1 MiB、最多128个外部文件、合计≤512模型；按文件名排序。拒绝非普通文件、重复 ID/供应商–模型身份、未知行为字段、未实现 handler、无效默认或约束。启动失败须修正文件，不能静默忽略。

若厂商引入新 wire 字段、鉴权机制、响应/工具协议，必须先扩展适配器并测试；JSON 不执行代码，不开放任意路径变换。`core/agent/tests/model_catalog_tests.rs` 用两个虚构未来模型证明：独立进程只加载新增 JSON，便可产生对应 OpenAI/智谱 payload。

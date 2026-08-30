# TimemAi 测试人员手册

本文面向 TimemAi 的测试人员、开发者和发布负责人，说明如何系统验证功能正确性、边界完整性、异常安全性、UI/Core 一致性以及发布质量。

本手册回答“怎样测试”；项目当前有哪些能力及测试保护，以以下文档为准：

- `docs/test-strategy.md`：整体测试策略与 CI 分层。
- `docs/feature-test-management.md`：功能—测试覆盖总台账。
- `docs/web-ui-feature-test-matrix.md`：Web UI 逐项需求矩阵。
- `docs/web_reliability_test_matrix.md`：浏览器、Web Host 与 Core 之间的可靠性交付契约。
- `docs/manual-release-smoke.md`：浏览器、终端、真实模型和干净机器上的人工发布冒烟。
- `docs/architecture.md`：系统边界与数据流。

## 1. 测试目标

测试不能只证明“正常点击一次能工作”，而要证明以下性质：

1. **功能正确**：用户目标能完成，状态与结果准确。
2. **边界完整**：空值、最小值、最大值、阈值前后、超长值等都有明确行为。
3. **异常安全**：无效输入、服务故障、断网、取消、拒绝、进程退出不会造成误执行或静默丢失。
4. **状态一致**：Core、Shell、Web Host、浏览器和持久化状态最终一致。
5. **会话隔离**：Session、Context、Worker、MEM、附件、配置和事件不会串线。
6. **可恢复**：刷新、重连、重启或瞬时失败后，用户工作可恢复且不会无故重复。
7. **显示可信**：UI 不虚构状态，不把“已发送”误写成“已提交”，不把旧结果挂到新任务。
8. **安全与隐私**：API Key、Header、Token、私有路径和提示词不会进入不应出现的 UI、日志或仓库。
9. **性能有界**：长会话、高频事件、大输入和多 Session 不导致明显卡顿、无限增长或死锁。
10. **可发布**：自动测试、人工冒烟、构建、文档和证据共同满足发布门禁。

## 2. 先理解系统边界

TimemAi 的主链路是：

```text
Shell UI / Web UI
        ↓
Host（timem_shell / timem_web）
        ↓
agent_core
        ↓
模型传输层 / LLM
        ↓
工具执行、存储、审计与结构化事件
```

测试归属必须遵循以下边界：

| 层 | 负责内容 | 重点验证 |
|---|---|---|
| `agent_core` | turn 状态机、模型协议、工具、上下文、内存、审计、重试、取消 | 语义和状态是否正确；坏输入是否安全；动作是否只执行一次或按契约重放 |
| `timem_shell` | 终端输入、菜单、渲染、Shell 专属命令 | CJK、粘贴、多行、窄终端、重绘、取消、第二次输入是否可继续 |
| `timem_web` | Web Host、认证、Session Worker、命令路由、快照、事件与持久化 | Session 隔离、命令确认、重连恢复、顺序、并发、Host 生命周期 |
| `web_ui` | 浏览器交互、渲染、草稿、队列、响应式布局 | 用户操作、可访问性、状态展示、断线体验、长页面、跨标签页一致性 |
| 安装与资源 | 安装脚本、配置、内嵌 Web bundle、能力清单 | 干净安装、版本一致、资源完整、升级兼容、无敏感信息 |

### 2.1 Core 与 UI 必须双向验证

跨层功能至少需要两类断言：

- **Core 断言**：实际状态和语义正确，例如 action 被执行、取消已生效、final answer 属于正确 turn。
- **UI 断言**：同一状态被准确、清晰地显示，例如 action 行从 running 更新为 finished，取消后按钮不再误显示可发送。

只验证 UI 文案不代表 Core 正确；只验证 Core 状态也不代表用户看到的内容正确。

## 3. 完备覆盖模型

每个功能至少从下列五个维度审查。若某维度不适用，应在功能测试台账中说明原因和剩余风险。

| 维度 | 核心问题 | 示例 |
|---|---|---|
| 正常路径 | 标准用户流程能否端到端完成？ | 创建 Session、发送任务、显示 final answer |
| 边界路径 | 临界值前后是否一致且有界？ | 0/1/N/N+1、空字符串、长行、窄屏、上下文 90% 阈值 |
| 错误路径 | 输入或依赖失败时是否安全、可理解？ | 401、超时、畸形模型输出、无效路径、拒绝审批 |
| 并发/重复 | 重复、竞态和高频操作是否稳定？ | 双击发送、两个标签页、四 Session 恢复、取消与补充竞态 |
| 恢复/持久化 | 刷新、重连、重启后是否收敛？ | ack 丢失、WebSocket 重连、Host 重启、Session 恢复 |

再附加四个横切检查：

- **安全**：鉴权、权限、注入、秘密脱敏。
- **兼容**：OS、浏览器、终端、旧数据、协议。
- **性能**：延迟、内存、DOM/事件窗口、输出上限。
- **可观测性**：错误、审计、状态和复现证据足够，但不泄密。

### 3.1 边界值设计法

对任何数量、长度、阈值或枚举，优先采用：

```text
非法负值 / 缺失
0
1
最小有效值
典型值
阈值 - 1
阈值
阈值 + 1
最大有效值
最大值 + 1
极大值
```

字符串额外覆盖：

- `null`、字段缺失、空字符串、仅空格、制表符、换行。
- ASCII、中文、Emoji、组合字符、RTL 文本。
- CRLF/LF、前后空白、NUL/控制字符。
- 引号、反斜杠、Markdown、HTML、JSON、XML、Shell 特殊字符。
- 超长单词、超长 URL、无空格长行、重复内容。
- 大小写差异、Unicode 归一化、相似字符。

集合额外覆盖：

- 空集合、单元素、重复元素、乱序元素。
- 最大允许数量及超限。
- 部分元素合法、部分非法。
- 删除首项、中间项、末项；删除不存在项；重复删除。

### 3.2 状态机设计法

对 Session、Turn、Action、请求决策、Web 命令等状态机，不只测页面结果，还要列出：

1. 合法状态。
2. 合法转换。
3. 禁止转换。
4. 每个转换的触发者。
5. 超时、取消和重启后的状态。
6. 重复事件是否幂等。
7. 乱序事件如何处理。

示例：Web 命令需区分 `pending → accepted → committed/rejected`。`WebSocket.send()` 成功不等于 Host 已接受，更不等于结果已持久化。

### 3.3 组合测试法

以下维度组合很多时，不必穷举笛卡尔积，但高风险组合不能遗漏：

- UI：浏览器 × 屏宽 × 主题 × 字体大小 × 连接状态。
- 模型：协议 × 正常/截断/畸形响应 × 重试 × action 数量。
- 会话：Session 数量 × Worker 数量 × 工作/空闲状态 × 重连。
- 输入：文本类型 × 附件 × 工作中/空闲 × 普通发送/立即补充。
- 工具：审批结果 × 前台/后台 × 超时 × 取消 × 输出大小。

采用 pairwise 组合覆盖一般交互，再为安全、持久化、取消、并发和不可逆副作用增加定向用例。

## 4. 测试环境与数据

### 4.1 基本依赖

完整本地验证通常需要：

- Rust/Cargo，版本与项目工具链兼容。
- Node.js 与 `pnpm`，用于 Web 前端测试和构建。
- `expect`，用于真实 pseudo-TTY 冒烟。
- Python 3，运行测试脚本和 fake model server。
- 支持的浏览器与终端，用于人工发布检查。

### 4.2 测试隔离

每次人工或自动场景应使用独立临时目录：

- 独立 `TIMEM_SPACE` / `--space`。
- 独立端口。
- 独立测试 API Key；优先不用真实凭证。
- 测试结束清理临时进程、端口和数据。

禁止：

- 用真实用户聊天、私有路径、密钥或内部 URL 制作 fixture。
- 在共享 MEM 上做破坏性测试。
- 把真实模型审计、token URL、Cookie 或 Header 提交到 Git。
- 让故障注入脚本误杀非测试进程。

### 4.3 三类模型环境

| 环境 | 用途 | 结论边界 |
|---|---|---|
| Fake model | 确定性功能、协议、重试、action、长输出、故障注入 | CI 主证据；不能证明真实供应商行为 |
| Fixture/replay | 历史协议样本、corner case、回归复现 | 稳定且快速；需持续补充真实失败形状 |
| Live model | 供应商兼容、真实流式输出、usage/cache、自然 action | 仅作发布补充；结果不可作为唯一自动门禁 |

测试 LLM 时不要断言自然语言逐字相等。应断言可观察契约，例如：

- 响应满足当前 JSON/XML 协议。
- final answer 存在且绑定正确 turn。
- action 名称、参数、审批和结果正确。
- 畸形输出触发修复而不是误执行。
- 重试次数和失败上限有界。
- usage、缓存和完成状态归属正确。

## 5. 测试执行分级

### 5.1 改动中：快速检查

根据改动范围执行最小相关集：

```bash
cargo fmt --all -- --check
cargo test -p agent_core <相关测试名>
cargo test -p timem_shell <相关测试名>
cargo test -p timem_web <相关测试名>
pnpm --dir interfaces/web test -- <相关前端测试文件>
git diff --check
```

最低要求：新增或修改功能必须有最低实用层级的自动测试；若跨状态、存储、模型、工具或 UI 边界，还必须有 integration/E2E。

### 5.2 合并前：完整自动门禁

```bash
scripts/ci.sh
```

该命令覆盖脚本语法、版本和模块边界、安装逻辑、契约检查、敏感扫描、Rust fmt/clippy/test/doc、Web 依赖与许可证、前端测试和构建、性能、重复边界回归、release build、Web 生命周期、跨 Host 恢复和真实 TTY。

若只想提高高风险状态机重复压力：

```bash
TIMEM_EDGE_ITERATIONS=5 scripts/edge_regression.sh
```

### 5.3 发布前：人工认证

按 `docs/manual-release-smoke.md` 执行与改动面相关的浏览器、终端、SSH、干净安装和 live model 场景。人工验证不能替代失败的自动测试。

## 6. Agent Core 专项测试

### 6.1 模型协议与响应解析

必须覆盖：

- JSON/XML 的正常 final answer、free talk、单 action、多 action、并行 action。
- 字符串内容中包含看似协议的 JSON/XML/Markdown 时，不被二次解释。
- 根节点错误、字段缺失、类型错误、未知 action、参数非法、嵌套过深、批次过大。
- 截断响应、半个 Unicode 字符、半个 XML 标签、代码围栏包裹。
- 修复成功、连续修复失败、达到修复上限。
- 畸形批次原子拒绝：不能执行其中“看起来合法”的部分。
- JSON/XML 语义对等；切换协议后不得沿用旧协议解析。

判定重点：**错误模型输出只能成为修复证据，不能被静默当作 final answer，更不能误触发工具。**

### 6.2 模型服务与传输

覆盖：

- 200 正常、400、401、403、404、408、413、429、500、502、503。
- DNS 失败、拒绝连接、TLS 错误、连接超时、流式停滞、传输中断。
- 持续有进展的长流不能被普通 inactivity timeout 错杀。
- 超大 request body 不受 argv 限制；大 stdout/stderr 不死锁。
- 瞬时错误按策略重试；永久错误不浪费重试。
- 错误理由可读且完成脱敏。
- OpenAI-compatible、OpenAI Responses、Anthropic 的 endpoint 拼接、请求字段和 usage 解析。

### 6.3 Turn、Session 与 Worker

覆盖：

- 单轮、多轮、round limit 后继续、最终完成。
- 工作中补充、普通下一问排队、final 与 supplement 竞态。
- 主 Worker 和子 Worker；子 Worker 完成不能错误结束主 Worker。
- 取消发生在模型调用前、等待中、action 中、等待人工决策时。
- stale reply、重复 reply、错误 `request_id`、错误 Session scope。
- Worker shutdown 能解除等待，不留下僵尸状态。
- 四个 Session 并发工作时事件、配置、token、final answer 完全隔离。

### 6.4 上下文、压缩与缓存

覆盖：

- Prompt delta/slice 顺序、隐藏、discard、offload。
- 空上下文、单 delta、多 slice、原生 exchange 和文本混合。
- 强制压缩阈值前后，尤其阈值 - 1、阈值、阈值 + 1。
- 静态提示占主导、动态内容无法继续缩小时，不无限请求压缩。
- compact 引用不存在、引用重复、同时 discard/offload、只 offload。
- 压缩后当前问题、关键 action 证据和活跃能力仍保留。
- KV-cache 标记稳定；当前 continuation trailer 不被误缓存。
- 审计中只保留必要摘要/hash，不泄露完整 prompt。

### 6.5 Memory、Scratch 与 Chat History

覆盖：

- Durable insert/read/update/delete；版本匹配与冲突。
- 两个进程或线程用同一版本更新，只允许契约定义的结果。
- SQL 只读限制、表白名单、placeholder 数量、注入尝试。
- Scratch write/search/read/delete；空查询、删除不存在项。
- Chat history 写入、分页、时间窗、损坏记录跳过、恢复顺序。
- 重启后 Session 和历史恢复；旧数据迁移不丢有效记录。
- MEM 切换期间拒绝旧空间操作；旧 epoch 命令不能落入新空间。

### 6.6 工具执行

对每个工具至少验证：

- manifest 中必填、枚举、条件字段和 any-of 校验。
- 正常执行、参数缺失、类型错误、未知字段、超长输入。
- 输出为空、仅 stderr、Unicode、大输出、截断边界。
- 超时、取消、权限失败、目标不存在。
- 并行动作保持正确顺序和 action 身份。
- UI 只显示脱敏后的结构化信息，Core 保留足够执行证据。

`run_bash` 额外覆盖：

- 前台完成、后台启动、timeout 后仍运行、稍后退出。
- PID 身份验证和 PID 重用，不能误杀其他进程。
- 子进程/进程组清理，不留下孤儿进程。
- 命令审批同意、拒绝、重复点击、取消等待。
- stdout/stderr 单流与双流、信号退出、非零状态。
- 凭证型参数、Header 和 URL 在显示与审计中脱敏。

### 6.7 审计与错误语义

- 每个 turn、模型调用、修复、重试、action 有可关联记录。
- 失败原因区分模型服务、协议、工具、Host、用户取消。
- 用户取消不应被记录为系统崩溃。
- API Key、Authorization、Cookie、token URL、敏感 Header 不出现。
- 大量失败时日志仍有界，不因写审计失败掩盖主要结果。

## 7. Shell UI 专项测试

### 7.1 输入与编辑

必须在函数测试之外运行 pseudo-TTY：

- 空输入、仅空格、普通 ASCII、中文、Emoji、组合字符。
- 单行、真实多行、Shift+Enter、CRLF 粘贴、超长粘贴。
- bracketed paste、被编辑的 paste placeholder、异常 paste 标签。
- 光标移动、行首/行尾、删除、退格、宽字符前后编辑。
- 控制字符清理，不能破坏终端或注入伪 UI。
- 提交后返回 prompt，再输入第二条命令，确认输入状态已恢复。

### 7.2 渲染与终端尺寸

覆盖：

- 宽屏、80 列、极窄宽度；终端运行中 resize。
- 超长 Thought/Action、长 URL、无空格命令、CJK 与 Emoji 宽度。
- final answer、状态栏、token、重试、修复、工具结果不互相覆盖。
- 高频 action 更新不产生重复行、残留 running 行或闪烁失控。
- ANSI/换行内容不能逃逸出预期区域。

### 7.3 控制与菜单

- `/help`、`/config`、`/workspace`、`/prof`、`/exit`。
- 菜单确认、取消、非法输入、返回编辑。
- `Ctrl+C`/`Esc` 在空闲、编辑、菜单、模型思考、action 中的含义。
- 一次误按 `Ctrl+C` 不应直接退出整个程序。
- 工作中输入下一问时，Q1 final answer 先显示，Q2 再开始。

## 8. Web Host 与 Web UI 专项测试

### 8.1 认证与网络

覆盖：

- 本地模式仅 loopback；public 模式显式绑定并仍要求 token。
- 本地 loopback 模式无需 token；public 模式覆盖无 token、错误 token、正确 token、Cookie reopen。
- public token 从可见 URL 移除；Referrer 和响应头不泄漏凭证。
- public 模式的 HTTP、静态资源、上传、API、WebSocket 使用一致认证策略。
- public Host 重启 token 轮换；旧 token 失效。
- 端口占用、自动 fallback、固定端口优先、非法端口。
- public 模式真实跨机器访问；生产暴露需 HTTPS/网络控制。

### 8.2 Session 与配置隔离

- 创建、重命名、删除、分组、拖动和恢复 Session。
- 不同 Session 使用不同模型、协议、URL、Header、API Key。
- 一个 Session 更新配置不改变全局默认或其他 Session。
- API Key 始终掩码；只在认证且请求专属的通道揭示。
- Session A 的 action、final、cwd、token、错误不得出现在 B。
- 多标签页对同一 Session 的操作按 Host 权威顺序收敛。

### 8.3 发送、队列与取消

- 空闲发送正常任务。
- 工作中普通 Send 创建下一 turn，不得偷偷变 supplement。
- 显式“立即”或快捷键才进入当前 turn。
- 双击/连点 Send 不重复提交同一草稿。
- 提交进行中继续输入，新文本不能被旧提交完成回调清空。
- 空草稿显示 Stop；输入非空切换 Send；点击 Stop 后立即退出运行态，并允许把新文本排为下一任务。
- 重复 Stop 幂等；取消覆盖当前 Session 的主/子 worker、模型请求、前台工具和已注册后台任务，不影响其他 Session。
- Stop 后的新任务只在权威取消完成后按 FIFO 启动；模型/系统错误不触发自动续发，也不以 `ready` 状态代替完成。

### 8.4 可靠性交付与重连

严格按 `docs/web_reliability_test_matrix.md` 注入故障：

- 写 socket 后、Host accept 前断开。
- accept 后、commit 前断开。
- commit 后丢最终 ack。
- 重复 `command_id`、相同 payload 不同 ID、队列满。
- snapshot 构建期间发生 mutation。
- live broadcast lag 后按 cursor replay。
- 等待人工决策时断线重连。
- Host 在 history 写入与 Core handoff 之间崩溃。
- 四 Session 同时恢复，task 与 ordered supplements 原子交付。

核心判定：

- 同一 ID 不产生重复领域效果。
- 不同 ID 即使 payload 相同，也代表两次用户意图。
- `accepted` 之前不能从浏览器 outbox 删除。
- ack A 不能改变命令 B。
- 刷新后 pending/accepted 命令从持久化浏览器存储恢复。

### 8.5 附件、MCP、Role 与决策

附件：

- 0 字节、小文件、大文件、长文件名、中文名、同名文件。
- 上传中取消、删除失败、重复删除、Session 切换。
- task/supplement 提交归属正确；失败不能静默丢附件。
- 路径穿越、非法文件名、错误 MIME 和超限输入安全拒绝。

MCP：

- stdio、Streamable HTTP、legacy SSE。
- 连接成功、失败、超时、重连、编辑、删除。
- 新 Session 默认不继承启用项；每 Session 独立选择。
- Secret 在 snapshot 中脱敏；不可用 MCP 不阻塞 Host 恢复。

Role：

- 创建、编辑、分组、排序、删除和跨 Session 即时可见。
- 乐观更新被拒绝后正确回滚，不留下幽灵 Role。
- 同时编辑和乱序响应按稳定 ID 收敛。

人工决策：

- 多 Session、同 Session 多 Worker 并发请求。
- stale、重复、错误 request ID、断线重连。
- 只解除匹配 Worker 的等待。

### 8.6 Markdown、布局与可访问性

覆盖 final answer 与过程区：

- 标题、列表、任务列表、表格、引用、链接、代码块、超长代码。
- HTML/XSS、危险 URL、图片/链接属性安全。
- CJK、Emoji、RTL、数学公式、重复标题、代码内伪标题。
- 代码复制；复制内容与显示内容一致。
- Light/Dark、字体、字号、降低动画偏好。
- 1280、768、390、320 px；无 body 横向溢出，composer 始终可用。
- 长内容的滚动锚定、历史 prepend、TOC 跳转。
- Modal focus trap、Escape 只关闭顶层、键盘可达、焦点恢复。
- 重要状态不能只依赖颜色表达；文本与背景对比满足 WCAG AA。

### 8.7 长历史和高频事件

- 超过 200 turns、每 turn 超过 500 events 的恢复快照。
- 五个 Session 各 1,500 events 的 burst。
- 分批挂载历史、prepend 后保持视口。
- DOM 窗口轮转后，新 user task 仍可见。
- 事件去重靠稳定 ID/sequence，不靠文本相等。
- bounded render 可以裁剪显示窗口，但不能跳过尚未归约的权威事件。

## 9. 安全与隐私测试

### 9.1 必查秘密

- API Key、Authorization、Cookie。
- public URL token、WebSocket token。
- 自定义模型 Header、MCP env/Header。
- 用户私有路径、内部 URL、聊天内容。

检查位置：

- 页面与 DOM、浏览器 console/network。
- Core topic、Web snapshot、错误消息。
- audit、diagnostics、普通日志。
- Git diff、构建产物和 fixture。

运行：

```bash
scripts/sensitive_scan.sh --current
```

### 9.2 输入安全

定向测试：

- Markdown/HTML XSS、`javascript:` 链接。
- Header 名非法 token 字符，Header 值 CRLF 注入。
- 文件名和路径穿越。
- SQL 注入与非只读 SQL。
- Shell 参数、URL userinfo/query/fragment 脱敏。
- XML entity、CDATA 边界、JSON 深层对象和资源耗尽输入。
- 超大 body、超长错误、重复嵌套，确保解析和显示有上限。

## 10. 性能、压力与稳定性

性能测试要记录基线、阈值、机器和提交，不只记录“感觉不卡”。

重点指标：

- 首屏与 Session 切换耗时。
- 输入响应与发送按钮反馈时间。
- 大 prompt render、topic fan-out、过程区渲染耗时。
- 长会话内存、DOM 节点、事件队列和日志大小。
- 并发 Session 数、重连恢复时间、CPU 峰值。
- 取消生效时间、后台进程清理时间。

项目已有守卫：

```bash
scripts/performance_guard.sh
scripts/edge_regression.sh
```

额外 soak 建议：

- 运行 1～8 小时，多 Session 周期性发送、取消、刷新和重连。
- 周期性制造 429/500/断流。
- 后台工具反复启动和退出，确认无 PID/文件描述符泄漏。
- 采集进程内存、CPU、打开文件和子进程数量。
- 结束后确认端口、MEM 锁和所有测试子进程均释放。

## 11. Corner Case 总清单

每次大功能或发布至少快速过一遍：

### 输入

- [ ] 空、空格、换行、超长文本。
- [ ] 中文、Emoji、组合字符、RTL。
- [ ] Markdown/HTML/XML/JSON/Shell 特殊内容。
- [ ] 重复提交、快速连点、提交过程中继续编辑。

### 状态与顺序

- [ ] 事件重复、乱序、延迟、丢失。
- [ ] stale Session/turn/request ID。
- [ ] cancel 与 final、supplement、action exit 同时发生。
- [ ] refresh/reconnect/restart 位于每个关键状态之间。

### 会话隔离

- [ ] 两个 Session 同时工作。
- [ ] 两个浏览器标签页操作同一 Session。
- [ ] 主/子 Worker 同时产生事件。
- [ ] MEM 切换时旧命令仍在路上。

### 依赖故障

- [ ] 模型 401/429/500、断流、截断、畸形协议。
- [ ] 文件不存在、权限拒绝、磁盘写失败。
- [ ] 端口占用、WebSocket 断线、Host 崩溃。
- [ ] MCP 不可用、工具超时、后台进程晚退出。

### UI

- [ ] 320 px、390 px、桌面、超宽屏。
- [ ] Light/Dark、大字号、减少动画。
- [ ] 超长 URL/代码/路径/文件名。
- [ ] 键盘操作、焦点、Escape、屏幕阅读语义。

### 安全

- [ ] 各类 secret 不进入 snapshot/topic/log/UI/Git。
- [ ] XSS、Header 注入、路径穿越、SQL 非只读。
- [ ] 显示脱敏不破坏必要的命令结构和排障信息。

## 12. 用例编写模板

每个重要用例建议使用以下格式：

```markdown
### TC-<模块>-<编号>：标题

- 需求/风险：
- 测试层级：unit / integration / E2E / manual
- 优先级：P0 / P1 / P2 / P3
- 前置条件：
- 数据与环境：
- 故障注入点（如有）：
- 步骤：
  1.
  2.
- 预期 Core 状态：
- 预期 UI 表现：
- 持久化/恢复预期：
- 安全检查：
- 清理步骤：
- 自动化位置：
- 实际结果与证据：
```

好的预期结果应可观察、可判定，避免“正常”“显示正确”之类含糊表达。例如：

- 差：点击两次 Send，系统正常。
- 好：同一草稿快速点击 Send 20 次，只生成一个稳定 `command_id`、一个 user entry 和一个 Core turn；按钮在请求未决时不可重复触发；随后输入的新草稿仍保留。

## 13. 缺陷报告与严重级别

### 13.1 缺陷报告最少内容

- 标题：`[模块][场景] 可观察错误`。
- 提交/版本、OS、浏览器或终端、启动参数。
- 是否使用 fake/live model，协议与模型名；不得附密钥。
- 独立 MEM 和必要 fixture。
- 最小复现步骤、实际结果、预期结果。
- 发生频率和首次出现版本。
- 截图/录屏、脱敏日志、相关 turn/session 时间点。
- 是否造成数据丢失、重复副作用、秘密泄漏或无法恢复。
- 临时规避方案。

### 13.2 建议严重级别

| 级别 | 定义 | 示例 |
|---|---|---|
| P0 阻断 | 安全泄漏、广泛数据破坏、不可逆误执行 | API Key 泄漏；错误 Session 执行危险 action |
| P1 严重 | 核心流程不可用、静默丢任务、重复副作用、稳定崩溃 | accepted 消息重连后丢失；取消误杀无关进程 |
| P2 一般 | 有明显功能错误但可绕过，不造成不可逆损失 | 某协议配置不能保存；附件删除需刷新恢复 |
| P3 轻微 | 文案、样式、小范围兼容问题 | 窄屏次要对齐问题，无操作阻塞 |

## 14. 发布判定

### 14.1 必须阻断发布

出现以下任一情况，应停止发布：

- `scripts/ci.sh` 失败。
- 存在未解释的 P0/P1。
- 有静默任务丢失、跨 Session 串线、重复不可逆动作。
- API Key/token/Header 等秘密泄漏。
- 取消、重连、重启后状态无法收敛。
- 发布构建或内嵌 Web bundle 不可复现。
- 改动涉及浏览器/终端/真实供应商，却没有对应人工证据。

### 14.2 有条件接受的剩余风险

仅当风险范围明确、无数据/安全影响、有可行规避方案，并记录以下内容时才可接受：

- 未覆盖平台或组合。
- 不自动化的原因。
- 已执行的替代验证。
- 影响用户与触发条件。
- 下一步补测负责人和时机。

## 15. 新功能测试工作流

1. **读需求和架构边界**：确认功能属于 Core、Host 还是 UI。
2. **画状态与数据流**：标记持久化点、异步边界和不可逆副作用。
3. **列风险**：正常、边界、错误、并发、恢复、安全、性能。
4. **先写验收标准**：同时包含 Core 状态与 UI 表现。
5. **选择最低实用测试层**：纯函数用 unit；跨状态用 integration；用户链路用 E2E。
6. **准备确定性 fixture/fake**：不要依赖 live model 才能复现。
7. **实现四维覆盖**：正常、边界、错误、重复/压力。
8. **增加历史缺陷回归**：缺陷修复必须有先失败后通过的自动用例。
9. **更新台账**：同步修改 `docs/feature-test-management.md`；Web 可见行为同步更新 Web UI 矩阵。
10. **运行相关测试和完整 CI**。
11. **执行必要人工冒烟**并记录日期、提交、环境和结果。
12. **清理测试环境**：进程、端口、MEM、临时凭证和生成物。

## 16. 常用命令速查

```bash
# 工作区 Rust 测试
cargo test --workspace --locked -- --test-threads=1

# 单 crate
cargo test -p agent_core
cargo test -p timem_shell
cargo test -p timem_web

# 前端依赖、测试、构建
pnpm --dir interfaces/web install --frozen-lockfile
pnpm --dir interfaces/web test
pnpm --dir interfaces/web build

# 高风险重复回归
TIMEM_EDGE_ITERATIONS=5 scripts/edge_regression.sh

# 性能守卫
scripts/performance_guard.sh

# Release 构建
cargo build --locked -p timem_shell -p timem_web --release

# 敏感信息与格式
scripts/sensitive_scan.sh --current
cargo fmt --all -- --check
git diff --check

# 全部门禁
scripts/ci.sh
```

## 17. 测试完成定义（Definition of Done）

一个功能只有同时满足以下条件，才算测试完成：

- [ ] 需求和边界清晰，可观察验收标准已写明。
- [ ] Core 与 UI 责任分别得到验证。
- [ ] 正常、边界、错误、并发/重复、恢复路径已覆盖或明确不适用。
- [ ] 安全、隐私、Session 隔离和取消行为已审查。
- [ ] 至少有一个真实用户链路的 integration/E2E；纯函数功能除外。
- [ ] 关键 corner case 已自动化，历史缺陷有回归测试。
- [ ] 相关测试及 `scripts/ci.sh` 通过。
- [ ] 涉及终端、浏览器、安装或真实模型时，人工冒烟有记录。
- [ ] 功能测试台账和相关文档已更新。
- [ ] 没有遗留测试进程、凭证、私有数据或未说明风险。

测试的最终目标不是追求用例数量，而是对关键产品不变量提供独立、可重复、可排障的证据。

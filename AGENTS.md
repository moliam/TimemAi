# TimemAi Development Contract

本文件是仓库级开发契约。修改代码前必须阅读；进入具体模块时，还必须阅读该目录的
`module_boundary.md`。详细设计、迁移记录和测试矩阵放在 `docs/`，不要堆入本文件。

## 1. 目录结构与归属

```text
core/
  agent/              # 模型循环、能力执行、Prompt/协议
  session/            # Session/Context/Worker/Turn 生命周期与用例
  ui_contract/        # UI 中立的命令、事件、投影和语义类型
  platform/           # 平台契约及 macOS/Windows/Linux 实现
bridges/
  in_process/         # 同进程类型化调用、回调和通道
  http_websocket/     # HTTP/WebSocket、序列、重放和重连
interfaces/
  shell/              # 终端交互与渲染
  web/                # 浏览器交互与渲染；dist 为受版本控制的构建产物
applications/
  timem/              # 统一产品组合根与唯一真实可执行程序
resources/            # 能力清单、工具实现及共享资源
tests/                # 跨模块/产品级测试
docs/                 # 架构、协议、测试、安装和发布文档
scripts/              # 架构守卫、质量门禁和交付脚本
```

Cargo 包名 `timem_web`、`timem_shell`、`agent_core` 是兼容身份，不代表物理目录归属。
不要恢复旧根目录 `timem_web/`、`timem_shell/`、`web_ui/`、`agent_core/` 或
`core/agent/src/os/`。完整布局见 `docs/semantic-project-layout.md`。

## 2. 架构设计

唯一语义方向是：

```text
Interface ↔ Bridge ↔ Core
```

- **Interface** 只负责人机交互、布局、渲染、可访问性和 UI 本地便利功能。
- **Bridge** 只负责通信、路由、序列化、排序、重放、重连和背压。
- **Core** 负责可复用的 Agent/Session 语义、权威状态、能力执行、持久化规则和平台策略。
- **Application** 只负责组装具体产品、选择运行模式和注入依赖。

依赖必须向内：

```text
interfaces/* -> bridges/* -> core/{session,ui_contract}
core/session -> core/{agent,ui_contract,platform}
core/agent   -> core/{ui_contract,platform}
```

`core/platform` 不得依赖 Agent、Session、Bridge 或 Interface；Core 不得依赖 Interface；
Interface 之间不得相互依赖。同进程 Rust Interface 使用 `bridges/in_process`，不得为了抽象
强加网络或序列化。浏览器通过 HTTP/WebSocket Bridge 通信。

Core 对 Session、Context、Worker、Turn、输入准入、取消和终态拥有最终解释权。Bridge
不得发明领域状态机；Interface 不得根据字符串、事件时序、Worker 数量或可见输出反推生命周期。
保留语义类型，不要把最终答案、进度、意图、证据、诊断和状态压成通用 `text`。

## 3. 扩展原则

- 新目录必须对应真实消费者、明确契约、已实现行为和可执行测试；禁止空占位和名称式支持声明。
- 同进程 Rust 客户端复用 `bridges/in_process`。
- 跨语言同进程客户端确有需要时，才添加 `bridges/native_ffi`，并适配到 in-process Bridge。
- 独立进程客户端优先复用 HTTP/WebSocket；只有通信语义确需时才添加 `bridges/ipc`。
- 新交互形态放入 `interfaces/<kind>`；新产品组合根放入 `applications/<product>`。
- 扩展不能复制 Agent、Session、Turn、取消、审批、重试或生命周期语义。
- 物理迁移默认保持包名、二进制、CLI、数据格式和线协议兼容；例外必须单独说明并测试。

## 4. 模块原则

- 行为放入拥有该语义的最内层模块；跨层便利不是越界理由。
- 模块保持内聚、命名表达语义、依赖显式、公开 API 最小化；优先组合与窄接口。
- Prompt 和模型响应格式属于协议。修改时必须同步生产端、解析、校验、修复、样例和测试。
- 内置工具保持 `resources/capabilities/tools/{tool}.yaml` 与 `{tool}.rs` 成对；清单定义接口，
  实现负责解析与执行，顶层 Turn 循环不得吸收工具细节。
- Core topic 回调在调用期间同步且由 Core 所有；异步保留前必须复制所需数据。
- 测试优先通过真实公开边界；测试主体放在各 crate 的 `tests/`，生产 `src` 仅允许最小测试钩子。
- 模块规则与本文件冲突时，以本文件为准，并在同一变更中修正文档冲突。

## 5. 硬性约束

- 选择修复根因的最小完整改动，不混入无关重构或投机抽象。
- 禁止硬编码自然语言关键词来判断用户意图；提供结构化证据和能力，由模型完成语义判断。
- 权限、所有权、破坏性操作、进程身份和平台支持必须 fail closed。
- 不得静默吞错；区分无效输入、能力不可用、瞬时失败、取消和内部缺陷，并提供已脱敏上下文。
- 避免无界集合/队列/历史/重试、异步路径阻塞、无界轮询、重复重建和不必要的全文件读取或克隆。
- 不得提交密钥、真实用户路径、私有事实、内部 URL、对话、临时文件、依赖目录或调试残留。
- Web 源码变化必须重新生成并提交 `interfaces/web/dist`，且解释所有产物差异。
- 行为变化必须同时更新受影响的源码、测试、架构守卫、文档和生成资产。
- 不得通过削弱、删除或绕过失败测试来制造成功。
- 未经用户明确要求，不得 push、发布、打 tag 或改写远端历史。

## 6. 交付与验证

修改前：检查现状、调用方、相关历史、模块边界和风险；明确兼容约束及测试计划。
修改中：先跑窄测试，保持语义归属，及时清理临时产物。提交前：格式化并运行适用门禁，
至少执行改动模块测试和 `git diff --check`；架构、热路径、Web 或发布相关改动还需运行对应守卫。

权威完整门禁是 `scripts/ci.sh`。常用专项检查：

```bash
python3 scripts/architecture_guard.py --self-test
scripts/module_boundary_check.sh
scripts/test_contract_check.sh
cargo fmt --all -- --check
scripts/clippy_check.sh
cargo test --workspace --locked -- --test-threads=1
pnpm --dir interfaces/web test
pnpm --dir interfaces/web build
git diff --exit-code -- interfaces/web/dist
scripts/performance_guard.sh
```

测试覆盖原则和功能登记见 `docs/test-strategy.md`、`docs/feature-test-management.md`。
任何适用门禁未运行或失败时，必须准确报告原因与剩余风险；不得仅凭代码审阅宣称完成。

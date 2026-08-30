# Turn 状态、Bridge 与 UI 投影架构

状态：**Proposed，待确认后实施**
范围：`agent_core`、所有 Bridge 与 UI shell；`timem_web`/`web_ui` 是首个迁移实现
目标：由 Core 提供极小、UI-neutral、权威的 Turn 语义，使 Shell、Web、iOS、桌面或未来任意 UI 壳只需适配命令、投影与表现层，不再各自实现 Agent 生命周期。

## 1. 决策摘要

Timem 的业务状态只有一个权威来源：**Agent Core 的 Turn 聚合器**。

```text
UI command
    ↓
Bridge（同步直连或异步/远程）
    ↓
Agent Core: Turn Gate → Turn Reducer → authoritative Core projection
    ↓
optional reconnectable Bridge（snapshot/revision/reliable delivery）
    ↓
UI shell: render and interact only
```

具体决策：

1. Agent Core 内部拆成两个很小的逻辑层：
   - **Turn Gate**：管理 Turn 身份、epoch 和消息归属；拒绝旧 Turn、旧 epoch、重复终结和迟到消息。
   - **Turn Reducer**：管理当前 Turn 的最小生命周期事实。
2. Core Turn projection 是所有 Bridge/Interface 的共同权威生命周期输入。同步 Shell 可以直接消费；异步、可重连或远程 UI 可通过 reconnectable Bridge 增加 snapshot、revision 和可靠投递，但不能改变 Core 语义。
3. 任何 UI shell 都不拥有 Agent 生命周期状态机。它只拥有：
   - Core projection 或 Bridge projection 的只读副本；
   - 输入框、弹窗、选中项、滚动位置等纯 UI 状态；
   - 命令发送/重试等传输状态。传输状态不能改变业务状态。
4. `timem_web` Pod 是 reconnectable Bridge 的首个实现，不是 Core 的必经层。`timem_shell` 等同步/in-process Interface 不需要为了接入权威 Turn 语义而引入 WebSocket、event sequence 或 reconnect outbox。
5. Worker、Tool、Topic activity 是 Active Turn 下的事实，不能创建 Turn、恢复 Turn 或覆盖终态。
6. Session/Bridge/Interface 不再维护独立生命周期状态机。`working` 等显示值由 Core Turn 投影直接给出。

## 2. UI 壳可移植性与当前适配状态

### 2.1 三层通用边界

```text
Core semantic contract
  TurnToken / input admission / PromptCut / activity / outcome / request-reply

Bridge contract
  command mapping / host policy / optional projection + reliable delivery

UI shell contract
  render / local interaction / accessibility / transport feedback
```

Core semantic contract 对所有 UI 完全一致。Bridge 可以有不同部署形态：

- 同步、单 Session 的 Shell 可直接调用 Core 并渲染 Core projection；
- 多 Session、异步、可重连的 Web/iOS/桌面 Interface 通常增加 reconnectable Bridge；
- 跨进程或跨语言 binding 只改变编码和传输，不改变 Turn 身份、输入接纳、终态与 request/reply 语义。

接入一个新 UI 壳时，允许新增 binding、Interface policy、projection transport 和视觉组件；不允许复制一套 `working/cancelling/finished` reducer 或根据事件顺序猜测 Turn 状态。

### 2.2 当前实现状态

本文是 Proposed 架构，不代表现有 UI 代码已经完成适配：

| 模块/壳 | 当前状态 | 目标适配 |
|---|---|---|
| `timem_shell` | 已是 Core 的直接 Bridge，但仍基于既有 topic/控制流展示生命周期 | 直接消费权威 Core Turn projection；删除 terminal-local 生命周期推断，不引入 Pod/WebSocket 复杂度 |
| `timem_web` Pod/Bridge | 仍有 `active_turn_id`、`pending_turn_id`、`cancelling_turn_id` 和 worker/topic 聚合等旧逻辑 | 成为通用 reconnectable Bridge 的首个实现，只增加 revision、snapshot、可靠命令投递、FIFO 与 MEM barrier |
| `web_ui` | 仍有既有 lifecycle reducer、cancel guard 和事件驱动推断 | 只消费 Host projection；topic/activity 仅追加 timeline |
| iOS/桌面/其他 UI | 尚无本架构适配实现 | 直接使用相同 Core semantic contract；根据部署需要选择直连或 reconnectable Bridge |

因此完成标准必须包含现有 Shell、Web Bridge 和 WebUI 的实际代码迁移与测试，不能只更新边界文档。

## 3. 为什么必须重构

### 3.1 当前存在三套状态判断

当前实现中，同一业务事实分散在三层：

- `agent_core`
  - `CoreSessionState::{Running, WaitingModel, WaitingUser, ...}` 随 topic 发出；
  - worker runtime 自己管理取消和完成。
- `timem_web`
  - `WebSession.state`；
  - `active_turn_id`、`pending_turn_id`、`cancelling_turn_id`；
  - `WebTurn.state`；
  - `WebWorker.state`；
  - 根据 worker 数量和 topic 再次聚合 Session 状态。
- `web_ui`
  - 消费 `turn_started`、`turn_finished`、`worker_activity`、`core_topic`；
  - 再次推断 Session/Turn/Worker 是否 working、cancelling 或 finished；
  - 重连时还需要把 snapshot 与浏览器持久化取消意图合并。

这不是简单的数据复制，而是多个可写状态机。每层都可能根据不同到达顺序得出不同结论。

### 3.2 已出现的 Stop 竞态

已验证的故障路径：

```text
TurnFinished(CancelledByUser)
    ↓
WebUI 显示 Cancelled
    ↓
同一 Turn 的迟到 TurnStarted / worker_activity / core.model.response
    ↓
某一层重新推导 working
    ↓
Session spinner 复活
```

重连时也可能收到取消前生成的 working snapshot。若浏览器仍需结合本地 cancel target 才能纠正显示，说明 snapshot 本身不是完整权威投影。

### 3.3 该根因还能预测的故障

如果不收敛状态所有权，后续会重复出现：

- child worker 晚结束导致 primary Session 状态反转；
- ToolGen 与普通 Turn 的事件串线；
- Stop 后 action finish 或 model response 恢复 working；
- reconnect、broadcast lag 或 event gap 后 UI 与 Host 状态不一致；
- Session restore 把历史终态误当成 live work；
- 同一 command 重试创建两个 pending/active Turn；
- MEM switch 前接受的命令污染新 MEM；
- UI 为修一个竞态不断增加新的本地 guard，最终形成第二套业务状态机。

因此修复不能只增加 WebUI 条件判断；必须收敛业务状态的写入点。

## 4. 设计原则

### 4.1 大道至简

运行时生命周期只保留两个槽位状态：

```text
TurnSlot = Empty | Active
```

终态是不可变 Turn 记录，不是继续运行的状态槽位。`starting`、`working`、`waiting_model`、`waiting_user`、`cancelling` 等不应成为互相竞争的顶层生命周期状态。

### 4.2 单一写入者

只有 Core Turn Reducer 可以：

- 建立 Active Turn；
- 接受 Stop 意图；
- 结束 Active Turn；
- 写入终态 outcome。

Bridge、reconnectable Bridge 和 UI shell 都不能执行这些状态转换。

### 4.3 身份先于内容

所有可能影响当前 Turn 的内部消息都必须携带一个 `TurnToken`。先由 Turn Gate 校验身份，再允许 reducer 或 activity store 接收内容。

### 4.4 业务状态与传输状态分离

- `command accepted / retrying / awaiting ack` 是传输状态；
- `active / stop_requested / finished outcome` 是业务状态；
- `waiting_model / waiting_user / running_tool` 是 Active Turn 的活动描述；
- 三者不能混成一个枚举。

### 4.5 Snapshot 必须自足

任意 UI shell 仅凭最新 Core projection 或 Bridge projection 就能正确渲染。可重连 UI 不得依赖旧 reducer 状态或本地取消目标纠正业务状态。

## 5. 核心概念

### 5.1 TurnId

`TurnId` 是持久、可审计、跨 snapshot 可引用的业务 ID。

用途：

- 聊天历史与 final outcome；
- favorite、message deletion、ToolGen source turn；
- command correlation 和诊断。

TurnId 必须由 Core 接受新 Turn 时生成或最终确认。Pod 可以在提交前持有 `command_id`，但不得自行创建一个被视为 Active 的业务 Turn。

### 5.2 TurnEpoch

`TurnEpoch` 是每个 Session 内单调递增的运行代次：

```rust
struct TurnToken {
    session_id: SessionId,
    turn_id: TurnId,
    epoch: u64,
}
```

用途只有一个：**阻止旧执行产生的迟到消息改变新执行**。

规则：

- 每次 Core 建立新 Active Turn，epoch 增加一次；
- worker、model、tool、decision、cancel callback 都持有创建时的 token；
- 只有 token 与当前 Active Turn 完全匹配时，消息才能进入当前 Turn；
- Turn 完成后，旧 token 永久失效；
- epoch 不负责 UI 排序，不替代 TurnId，不替代 MEM command barrier。

TurnEpoch 应由 Core 管理。其持久化要求只需保证恢复后不会接受旧进程消息；实现可采用持久高水位，或在 runtime incarnation 中加入随机/单调 generation。对外协议只承诺 token 不会在仍可能存在旧生产者时重用。

### 5.3 ProjectionRevision

`ProjectionRevision` 是 Pod 投影版本：

- 每当对 UI 可见的权威 projection 改变，revision 单调增加；
- WebUI 只接受更高 revision；
- event sequence 仍负责 WebSocket 传输顺序，projection revision 负责投影新旧判断；
- revision 不参与 Core 生命周期决策。

### 5.4 MemEpoch

现有 `mem_epoch` 继续作为 Host command barrier：防止旧 MEM 接受的命令在新 MEM 执行。

它不是 Turn 生命周期状态，也不决定 UI working 状态。不要把 `mem_epoch`、`turn_epoch` 和 `projection_revision` 合成一个含义模糊的全局计数器。

| 标识 | 所有者 | 作用域 | 唯一职责 |
|---|---|---|---|
| `turn_id` | Core | durable Turn | 业务身份与历史引用 |
| `turn_epoch` | Core Turn Gate | Session runtime | 拒绝迟到/旧执行消息 |
| `projection_revision` | Pod | projected Session/MEM | UI 投影新旧判断 |
| `event_seq` | Pod delivery | WebSocket baseline | 传输线性顺序与 gap 检测 |
| `mem_epoch` | Pod command barrier | MEM | 拒绝跨 MEM 的旧命令 |

## 6. Agent Core 内部两小层

这“两层”是逻辑边界，可以先在一个模块实现，不要求为了形式拆成复杂框架。

### 6.1 Turn Gate

职责：

- 分配/确认 `TurnId`；
- 增加 `TurnEpoch`；
- 保存当前 `Option<ActiveTurn>`；
- 校验每个内部消息的 `TurnToken`；
- 保证 finish 幂等；
- 拒绝旧 token、未知 token 和终态后的迟到消息；
- 输出明确的 accepted/ignored reason 供诊断和测试使用。

建议接口：

```rust
struct TurnGate {
    next_epoch: u64,
    active: Option<ActiveTurn>,
}

struct ActiveTurn {
    token: TurnToken,
    stop_requested: bool,
    input_admission: InputAdmission,
    next_input_seq: u64,
    pending_inputs: Vec<PendingTurnInput>,
    last_prompt_cut: Option<PromptCut>,
    activity: TurnActivity,
}

enum GateDecision {
    Apply,
    IgnoreStale,
    IgnoreFinished,
    IgnoreUnknown,
}
```

Gate 不解析模型语义，不渲染 UI，不聚合 CSS 所需字符串。

### 6.2 Turn Reducer

最小业务状态：

```rust
enum TurnSlot {
    Empty,
    Active {
        token: TurnToken,
        stop_requested: bool,
        input_admission: InputAdmission,
        activity: TurnActivity,
    },
}

enum InputAdmission {
    Open,
    Closed { terminal_commit_seq: u64 },
}

struct PromptCut {
    turn: TurnToken,
    model_round: u32,
    consumed_input_seq: u64,
}

enum TurnOutcome {
    Completed,
    Cancelled,
    Failed { code: String },
    Interrupted,
}
```

允许的转换只有：

```text
Empty  --start-->  Active
Active --stop request--> Active(stop_requested = true)
Active(input_open) --terminal commit--> Active(input_closed)
Active(input_closed) --finish--> Empty + immutable TurnOutcome record
```

`input_admission` 不是新的顶层生命周期。它只是 Active Turn 内部的单向接纳门：一旦 Core 接受 terminal/final response，当前 Turn 不再接纳新的 prompt 输入，之后不可重开。输入是否已经影响当前 Turn，必须由实际模型请求的 `PromptCut` 证明，不能由消息到达时间推断。

禁止的转换：

- activity 创建 Active Turn；
- worker working 恢复已结束 Turn；
- finish 后再 start 同一个 token；
- 旧 epoch 修改当前 activity；
- Bridge、Projection Adapter 或 UI shell 直接写 reducer。

### 6.3 Activity 不是第二套生命周期

`waiting_model`、`waiting_user`、`running_tool`、`compacting` 等保留为 Active Turn 的展示事实：

```rust
enum TurnActivity {
    Running,
    WaitingModel,
    WaitingUser { request_id: String, timeout_ms: Option<u64> },
    RunningTools,
    Paused,
}
```

Activity 可以改变文案和控件，但不能决定 Turn 是否存在。`stop_requested` 也只是 Active Turn 上的事实，不需要增加 `Cancelling` 顶层状态。

### 6.4 用户输入接纳边界与 PromptCut

一个 Turn 不能仅用“浏览器是否已经看到 final answer”或“输入是否比 final answer 先到”判断输入归属。Core 必须同时维护：

```text
InputAdmission = Open | Closed
PromptCut = 本次 model request 实际消费到的输入序列高水位
```

`PromptCut` 在每次模型请求组装完成、发送前由 Core 封存，至少包含 `TurnToken`、`model_round` 和 `consumed_input_seq`。它是“哪些输入实际影响了这次模型响应”的证据，不是 UI revision，也不是普通 wall-clock timestamp。

唯一封口点是：**Core 接受 terminal/final response，并决定该响应将结束当前 Turn 的时刻**。不是模型开始生成 final answer、Pod 收到 topic、WebUI 开始渲染或动画结束的时刻。

规则：

1. 普通 `turn_submit` 从来不进入当前 Active Turn，只形成独立的 `NextTurnIntent`。
2. 只有显式 `turn_supplement` 才能请求进入当前 Turn，并必须携带目标 `TurnToken`。Core 接受它时分配单调 `input_seq`，先进入 pending input buffer；“accepted”不等于“已被模型消费”。
3. 每次 model request 只消费其 `PromptCut` 覆盖的输入。一个 supplement 只有在某个已发送请求的 `PromptCut` 中，才能记为当前 Turn 已消费输入。
4. terminal response 必须关联产生它的 `PromptCut`。Core commit terminal response 时原子执行：
   - 将该 cut 已消费的输入留在当前 Turn；
   - 永久关闭 input admission；
   - 将所有已接受但未被任何已发送 `PromptCut` 消费的用户 task 输入交还 Pod，转换为独立 `NextTurnIntent`。
5. 因此，supplement 即使比 terminal response 更早到达，只要 final response 对应的模型请求已经发出且没有消费它，就不能倒算进旧 Turn。禁止根据时间先后伪造因果关系。
6. 转换保留同一稳定 `command_id`、文本、附件和原始顺序。新 intent 启动时由 Core 分配新的 `TurnToken`，并作为**新 Turn 的第 1 个 model round**输入；不能继承旧 Turn 的 round 计数、stop 标记、pending decision 或附件消费状态。
7. terminal commit 后，即使 final answer 尚未投影到 WebUI，输入边界也已经关闭；反之，仅看到模型流式输出像 final answer，不能提前封口。
8. `Closed` 是单向状态。Stop、迟到 worker event、reconnect、旧 ACK 都不能将其重新打开。
9. 每个用户 task command 必须恰好处于一种 ownership：pending input、某个 PromptCut 已消费、NextTurnIntent、Active Turn 首轮输入或明确 rejected。不得同时属于两处，也不得无归属。

这给竞态一个可审计答案：输入归属由稳定 command identity 与 PromptCut 消费关系决定，而不是由观察到的事件先后猜测。

### 6.5 Worker 状态从属于 Turn

Worker projection 必须带 `TurnToken`。Worker activity 只允许更新匹配 Active Turn 的 worker 明细。

```text
worker activity cannot:
- allocate a Turn
- set Session working
- clear a Turn
- override an outcome
```

Session 是否 working 的唯一判定是：

```text
working = core_projection.active_turn != null
```

不是“是否存在 working worker”，也不是“最后一个 topic 的 state 是 Running”。

## 7. reconnectable Bridge：可选的权威投影出口

reconnectable Bridge 指连接 Agent Core 与异步/可重连 UI 的投影与可靠投递层。它是可选部署层：同步 Shell 可以直接消费 Core projection；当前首个实现位于 `timem_web`，称为 Pod。未来 iOS、桌面或远程 Interface 可复用相同模式，但不要求复用 Web 进程或协议。

### 7.1 Bridge 输入

- Core authoritative Turn projection；
- Core timeline facts：topic、worker activity、sub-answer、final outcome；
- Host command delivery state；
- Session metadata、history、attachments、settings；
- MEM barrier 和 semantic delivery sequence。

### 7.2 Bridge 输出

reconnectable Bridge 输出完整、可直接渲染的 Session projection；当前 Web 实现由 Pod 输出：

```rust
struct SessionProjection {
    session_id: String,
    revision: u64,
    active_turn: Option<ActiveTurnProjection>,
    last_outcome: Option<TurnOutcomeProjection>,
    workers: Vec<WorkerProjection>,
    turns: Vec<TurnProjection>,
    command_delivery: Vec<CommandDeliveryProjection>,
}

struct ActiveTurnProjection {
    turn_id: String,
    epoch: u64,
    stop_requested: bool,
    input_admission: InputAdmissionProjection,
    activity: TurnActivityProjection,
}
```

兼容期可继续输出 `state: "working" | "ready"`，但该字段必须由 `active_turn.is_some()` 单向派生，不能再由 worker/topic/WebUI 写入。

### 7.3 Bridge 不拥有第二套状态机

reconnectable Bridge 可以：

- 校验 projection revision；
- 合并 history 与实时 timeline 数据；
- 生成 redacted snapshot；
- 管理 command ack、MEM barrier 和 WebSocket sequence；
- 将 Core outcome 映射成稳定 wire enum。

reconnectable Bridge 不可以：

- 根据 worker 数量决定 Turn 是否结束；
- 根据 topic 的 `CoreSessionState` 恢复 working；
- 在 Core 未确认时把 pending command 变成 Active Turn；
- 将迟到 event 归到“当前看起来最像”的 Turn；
- 修改 Core outcome。

### 7.4 Timeline 与生命周期分离

Core topic、worker activity 仍可作为 timeline 数据展示，但必须满足：

- 每条 live timeline fact 带 `TurnToken`；
- Adapter 只把它追加到对应 Turn；
- 对已结束 Turn 的迟到 fact：默认丢弃并记录诊断；若未来需要审计，可进入独立 late-event diagnostics，不能进入普通 UI projection；
- 任意 UI shell 收到 timeline event 时不能据此改变 Session/Turn lifecycle。

## 8. UI shell 通用边界

### 8.1 UI shell 可以拥有的状态

- theme、layout、sidebar、modal；
- composer draft、file picker、selection；
- scroll、focus、expanded/collapsed；
- command outbox、sending/retrying/ack status；
- projection cache，按 `revision` 替换。

### 8.2 UI shell 禁止拥有的状态

- 从事件推断 `active_turn_id`；
- 从 worker/core topic 推断 `session.state`；
- 自行执行 `working → cancelled → working` 等生命周期转换；
- 用 local cancel target 修正服务端 snapshot；
- 把 `turn_started`、`turn_finished`、`worker_activity` 当作独立业务状态机输入。

### 8.3 Stop 的 UI 规则

用户点击 Stop：

1. WebUI 只发送带 `command_id` 和精确目标 Turn token 的 Stop command；点击本身不修改 Session/Turn 业务状态；
2. Host 校验命令后原子记录并持久化完整 Session projection，其中目标 Turn 为 `finished + CancelledByUser`、worker/Session 不再显示 working，同时保留私有执行屏障；
3. Host 返回或广播完整权威 Session；WebUI 收到后替换 projection，移除 spinner 和 Stop 控件并呈现 `Cancelled`；
4. Pod/Core 的 `stop_requested`、worker join 与 terminal barrier 继续作为后台执行事实，但不映射成 `Stopping…`；
5. Core finish outcome 到达后只补全权威统计和 continuation grant，不把 Turn 从 `Cancelled` 改回其他可见运行态；任意旧 worker/topic 消息也不能改变该结果。

用户感知到的快速反馈来自 WebUI 与同进程 Host 之间的低延迟通信，而不是浏览器乐观推进生命周期。`Cancelled` 表示 Host 已确认用户与该 Turn 的交互终结，不代表外部调用或子进程已经物理退出；该清理由 runtime 在后台完成。若 Stop command 无法可靠保存或被 Host 拒绝，WebUI 继续呈现上一份 Host projection 并显示明确错误。

## 9. 快速 Stop / Start 用户体验

该架构不仅减少竞态，也允许比当前实现更流畅的交互。设计必须区分“用户意图已接收”“Core 已请求停止”和“旧 Turn 已终结”。

### 9.1 三种状态分层

```text
WebUI transport state:
  sending_stop | sending_start | retrying

Pod command intent state:
  accepted | queued | committed | rejected

Core business state:
  Active(stop_requested=false)
  Active(stop_requested=true)
  Empty + immutable outcome
```

WebUI 可以立即显示 `Sending…` 或 `Waiting…`，但这些文案只说明命令/意图状态，不能冒充 Core 业务状态。

### 9.2 Stop 的即时反馈

点击 Stop 后：

1. WebUI 发送 Stop command，并在 Host 回复前继续渲染上一份权威 projection；
2. command 携带精确 `{ turn_id, epoch }`，重复点击由本地传输 guard 和 Host 幂等处理；
3. Host 接受后持久化完整取消后 Session projection，并立即返回给 WebUI；WebUI 仅据此停止可见 running 动画并显示 `Cancelled`；
4. Core 将匹配 Active Turn 的 `stop_requested` 设为 true，并在模型等待、工具 join、后台进程和 host decision 等取消点持续检查同一个 token；
5. 对可终止的子进程触发安全终止；无法强制中断的外部调用继续在后台收敛；
6. UI 不显示 `Stopping…`、清理超时或 worker join 进度，也不因迟到事件恢复 working；Core 写入权威 outcome 后只补全 elapsed/stats 等事实。

这里的 `Cancelled` 表示用户与该 Turn 的交互已经终结，不是对底层资源瞬时释放的虚假声明。产品必须隐藏不影响用户决策的清理延迟，同时保留真实错误：如果取消命令没有可靠提交，必须明确回滚或报错。

### 9.3 Stop 期间立即开始下一轮

同一 Context 仍保持最多一个 Active Turn。用户在旧 Turn stopping 时发送新消息，Pod 将它保存为**下一 Turn 意图**：

```rust
struct QueuedTurnIntent {
    command_id: CommandId,
    session_id: SessionId,
    enqueue_seq: u64,
    input: TurnInput,
    attachments: Vec<AttachmentRef>,
    origin: OrdinarySubmit | UnconsumedSupplement { target: TurnToken },
}
```

流程：

```text
Turn A Active
  → user clicks Stop; WebUI sends the targeted command without changing business state
  → Host records and returns A Cancelled; WebUI renders that projection
  → ordinary Start(B) is accepted and immediately shown as a new task
  → Pod privately retains B behind A's terminal barrier
  → Core finishes A cleanup
  → Pod submits B to Core
  → Core allocates Turn B token and publishes activity
```

这不是第二个 Core Active Turn，也不是 WebUI 自行执行 B。B 已经是用户可见、可关联、可重放的独立 Turn；`queued intent` 只是 Pod 内部的执行所有权与串行屏障实现，不能渲染成“待发送队列”“等待旧任务停止”，也不能出现仅对 active supplement 有意义的“立即”控件。

### 9.4 连续 Stop / Start

该模型天然支持：

```text
A active
Stop(A)
Start(B) queued
B active
Stop(B)
Start(C) queued
C active
...
```

安全性的来源：

- `Stop(A)` 只匹配 A 的 token，永远不能停止 B；
- A 的迟到 worker/model/tool 消息因 epoch 不匹配而被 Gate 丢弃；
- 每个 start 使用稳定 `command_id`，重试不会创建重复 Turn；
- Pod 使用按 `enqueue_seq` 排序、按 `command_id` 去重的有界 FIFO；不允许无界队列；
- queued intent 可以被用户显式编辑、重排或取消，但这些都是 Pod command-intent 操作，不能修改当前 Core Turn；
- 每次最多只允许队首进入 Core；权威 terminal outcome 只解除生命周期 barrier，是否自动派发仍由显式自动发送偏好与该 outcome 的 continuation grant 决定。

### 9.5 推荐产品策略：有界 Next Intent FIFO

每个 Session 保持：

```text
0 or 1 Active Turn
0..N queued NextTurnIntent（N 为明确配置上限）
```

必须允许多于一个 intent，因为 terminal commit 时可能同时存在普通 Send 和一个“已接受但未被当前 PromptCut 消费”的 supplement。单个可替换槽会迫使系统覆盖、丢失或错误合并用户输入。

有界 FIFO 只是 Pod 的命令数据结构，不是第二套 Turn 生命周期状态机：它没有 Active/Stopping/Finished 转换，不分配 `TurnEpoch`，也不能让两项并发进入 Core。队列可以原子写盘，以便同一进程异常期间避免半写并在启动时识别哪些可见历史曾处于 waiting；但**持久化不代表跨进程恢复执行权**。达到上限时必须在接纳前明确拒绝并保留浏览器草稿，不能先 ACK 后丢弃。

用户可在 intent 尚未 dispatch 时编辑、重排或取消；操作必须引用稳定 `command_id` 并产生新的队列 revision。已经 dispatch/owned 的项不可被旧编辑或旧 ACK 复活。

#### Runtime 重启是硬 Stop 边界

浏览器刷新、WebSocket 断线和重连不改变 Host/Core 进程，因此同一 runtime incarnation 内仍可从 Pod snapshot 恢复有序 queued intents 并继续等待 terminal barrier。

Bridge/Core 进程重启则完全不同：旧 runtime incarnation 的 Active Turn 与所有未派发 NextTurnIntent 都失去执行权。启动恢复必须：

1. 读取残留队列只用于识别对应的可见历史 Turn；
2. 将尚无 final outcome/completion 的对应 Turn 标记为 `interrupted`；
3. 清空内存队列，并尽力原子写回空队列；即使写回失败，本次新进程也不得装载执行权；
4. 旧 `command_id` 重投只能幂等返回该 interrupted Turn，不调用 Core；
5. 用户要继续必须提交新的 `command_id`，由 Core 创建全新的 Turn。

因此，“可以重连继续等待”只适用于同一进程；“进程重启后一律 Stop”不依赖旧队列是否完整、是否已有 `CoreAccepted` 记录或某个 terminal event 先后到达。

### 9.6 与其他操作的耦合规则

所有操作先回答两个问题：它是否修改当前 Turn 的 prompt/timeline；它是否必须等到下一 Turn 边界。不得让每种操作各自发明 final-answer race 处理。

| 操作 | `input_open` 时 | terminal commit / `input_closed` 后 | 隔离要求 |
|---|---|---|---|
| 普通 Send | 始终创建/更新 Next intent，不进入当前 Turn | Next intent | 不需要与 final answer 竞争 |
| 显式 supplement | 进入 pending input；仅被 `PromptCut` 覆盖后才算当前 Turn 已消费 | 未消费项原子转换为 Next intent | 保留同一 `command_id`，恰好消费一次 |
| Stop | 关闭执行，不替代输入封口规则 | 对已终结 token 幂等成功 | 旧 Stop 不能作用于新 Turn |
| final answer | 必须关联产生它的 `PromptCut` | terminal commit 关闭输入门并迁移未消费输入 | final answer 永远留在原 Turn |
| queued 自动发送 | 只排队 | 仅在允许 continuation 的 outcome 后派发队首 | error/reject 不得误触发或跳过顺序 |
| 附件 | 随 command 进入 pending；随 PromptCut 消费 | 未消费时与 command 整体迁移 | 不能旧 Turn 和新 Turn 各消费一次 |
| inline decision / approval | 只回复精确 `{TurnToken, request_id}` | stale，忽略或明确过期 | 不能转换成新 Turn 用户输入 |
| ToolGen | 仅绑定已完成 source turn，作为独立受控 Turn | 不追加 source turn | source final answer 不变 |
| Role/MCP/runtime 设置 | 记录 desired revision | 在下一新 Turn 边界应用 | 不改变已封口 prompt |
| child worker/tool callback | 只更新匹配 token 的当前 activity/timeline | Gate 丢弃迟到回调 | 不得触发 Next intent |
| 浏览器 reconnect/retry（同一进程） | 从 Pod 恢复 Active + admission + intent | 重放同一 command | 不能依据浏览器旧状态重新判归属 |
| Bridge/Core 进程重启 | 所有 live ownership 失效 | 队列历史标记 interrupted 后清空；旧 ID 只回显 | 必须以新 command ID 创建新 Turn，绝不 redrive |
| MEM switch | 受 `mem_epoch` barrier 阻断 | 旧 MEM intent 不进入新 MEM | 输入、附件、ACK 同 epoch 隔离 |
| Session delete/shutdown | 拒绝新 intent并取消当前 Turn | 丢弃/显式失败未启动 intent | 不在后台偷偷启动下一 Turn |

特别地，decision reply 不是用户 task 输入；ToolGen guidance 也不能借用 supplement 通道。二者必须保留自己的命令类型和目标身份，避免“任何晚到文字都转下一 Turn”这种错误泛化。

### 9.7 UX 文案与控件建议

| 阶段 | 权威来源 | 用户可见呈现 | 输入行为 |
|---|---|---|---|
| 用户点击 Stop、Host 尚未回复 | WebUI transport | 继续呈现上一份 Host projection；不本地改写生命周期 | 只发送精确 Stop command；本地 guard 仅防重复传输 |
| Host 接受 Stop | Host Session projection | 旧 Turn 显示 `Cancelled`，working/spinner/Stop 消失 | 可立即输入并提交普通新 Turn |
| Core 后台清理 | Core execution + Host private barrier | 不显示 `Stopping…` 或清理进度 | 新输入按普通 `turn_submit` 可靠发送 |
| 新 Turn 已被 Host 接受、尚在 terminal barrier 后 | Pod intent ownership | 立即显示独立的新用户任务，不显示“待发送”或“等待停止” | 可继续编辑 composer；不可把新任务补充进旧 Turn |
| 旧 Turn 权威 terminal | Core outcome | 旧 Turn 保持 `Cancelled`，可补全统计 | Pod 自动越过内部屏障派发新 Turn |
| 新 Turn Active | Core projection | 正常 working/activity | Stop 精确绑定新 token |
| command 失败/断线 | Pod delivery | 明确可恢复错误；必要时恢复草稿 | 不伪造已接受或已执行 |

### 9.8 额外不变量

除第 11 节通用不变量外，快速交互还必须满足：

1. 同一 Context 永远不同时运行两个 Active Turn。
2. queued intent 不是 Active Turn，不分配业务 epoch。
3. queued intent 只有在 Core 确认旧 Turn terminal 后才具备派发资格；自动派发还必须满足用户偏好与 outcome continuation grant。
4. Stop command 必须绑定精确 token；无目标 Stop 不得影响未来 Turn。
5. start command 的重复投递只产生一个 queued intent 或一个 Turn。
6. 同一 Host 进程内 UI 断线重连后，Pod snapshot 必须同时包含 Active Turn projection 与有序 queued intents projection。
7. Bridge/Core 进程重启后，所有 queued intent 执行权必须清空；其旧 `command_id` 只能回显 interrupted 历史，不能 redrive。
8. 用户取消 queued intent 后，它不能因重试或旧 ack 再次启动。
9. 队列达到上限时必须在 command acceptance 前明确拒绝，不得静默覆盖。
10. 同一 `command_id` 在 pending input、PromptCut、队列和 Active Turn 间始终只有一个 owner。

## 10. 建议协议

### 10.1 Core → Bridge

优先输出权威 projection change，而不是让 Bridge 根据细粒度事件重建生命周期：

```text
CoreTurnProjectionChanged {
  session_id,
  revision,
  active_turn?: {
    turn_id,
    epoch,
    stop_requested,
    input_admission,
    activity
  },
  finished_turn?: {
    turn_id,
    epoch,
    outcome
  }
}
```

Timeline topic 继续存在，但 envelope 增加：

```text
turn:
  turn_id: string
  epoch: integer
```

不再使用无 Turn 身份的 activity 改变生命周期。

### 10.2 reconnectable Bridge → UI shell

过渡期新增：

```text
session_projection_updated {
  session_id,
  revision,
  projection
}
```

最终：

- `hello.snapshot.sessions[]` 携带相同 projection；
- 在线增量只发送 revision 更高的新 projection 或明确 patch；
- `turn_started`、`turn_finished` 可在兼容期双发，WebUI 新路径不再消费它们来改变生命周期；
- `core_topic`、`worker_activity` 仅追加 timeline。

### 10.3 Command

```text
turn_submit {
  command_id,
  session_id,
  input,
  ...
}

turn_supplement {
  command_id,
  session_id,
  target: { turn_id, epoch },
  input,
  attachments
}

turn_stop {
  command_id,
  session_id,
  target: { turn_id, epoch }
}
```

Core/Pod 对 supplement 必须返回明确的接纳结果：

```text
AcceptedPending | ConsumedByPromptCut { model_round }
ConvertedToNextTurnIntent { enqueue_seq } | Duplicate | Rejected
```

`AcceptedPending` 只表示 Core 已拥有输入，不表示模型已经看到它。`ConsumedByPromptCut` 才证明其进入当前 Turn 的某次模型请求。转换不是浏览器重发一个新 command，而是 Core/Pod 对同一 command ownership 的原子交接，避免断线窗口中丢失或重复输入。

若 target 已结束，Stop 应幂等返回 committed/current projection，而不是作用于后来的 Turn。

## 11. 必须满足的不变量

1. 每个 Session 最多一个 Active Turn。
2. 只有 Core 可以创建或结束 Active Turn。
3. 终态不可逆。
4. 非当前 `TurnToken` 的消息不能改变当前状态。
5. Worker/Tool/Topic activity 不能创建、恢复或结束 Turn。
6. Session working 当且仅当 Core projection 存在 Active Turn。
7. Stop 只作用于精确 Turn token，不能误停后续 Turn。
8. 重复 submit/stop/finish 是幂等的。
9. Snapshot 单独即可决定正确 UI，不依赖浏览器旧业务状态。
10. Pod projection revision 单调；WebUI 不接受旧 revision。
11. MEM switch 后，旧 `mem_epoch` command 不能执行。
12. WebUI 所有业务状态显示均来自 Pod projection。
13. terminal commit 是 input admission 的唯一封口线性化点，但不是输入影响 final answer 的证据。
14. 只有关联模型请求的 `PromptCut` 能证明输入已被该请求消费；禁止由到达先后推断因果。
15. `input_admission` 只能从 Open 变为 Closed，不能重新打开。
16. terminal commit 时，未被任何已发送 PromptCut 消费的 task 输入只能进入 Next intent，不能追加到旧 Turn。
17. supplement 与 Next intent 的转换必须保留 command ownership、顺序，并且输入与附件恰好消费一次。
18. Next intent FIFO 必须有界、去重，且一次只派发队首；Session 变为 Empty 本身不构成自动派发授权。队列写盘只用于同进程可靠性与重启后的历史中断识别，绝不授予跨进程 redrive。
19. Bridge/Core 进程重启是硬 Stop 边界：Active 与 queued ownership 全部失效，旧 command ID 不得再次调用 Core。
20. decision、ToolGen guidance、设置变更不能被泛化成 late supplement。
21. final answer、outcome、completion stats 永远绑定原 Turn，不因后续输入迁移。

## 12. 迁移计划

禁止一次性删除旧协议。采用可验证的分阶段迁移。

### Phase 0：文档、并发 harness 与基线压力

- 本文评审通过；
- 先实现第 13 节统一 stress harness：独立执行方、关键 barrier/test hook、seeded jitter、deadline、ownership ledger、阶段时延与资源收敛检查；
- 新增聚焦入口 `scripts/turn_concurrency_stress.sh`，明确 PR/release/soak profile；在脚本和 CI 真正接入前不得把它记录为已有测试证据；
- 用当前实现跑出 Stop、reconnect、late event、FIFO、MEM epoch 的可复现基线，保留失败 seed 和延迟分布；
- 为上述 21 条不变量建立必要的纯逻辑单元测试，但不得用它们替代 4 个真实并发压测。

验收：压力入口实际执行非零轮数，失败可按 seed 重放；基线报告包含轮数、运行时长、p50/p95/p99/max、最慢样本和资源前后差异。

### Phase 1：Core Turn Gate 与 PromptCut（不改 Web 协议）

- 在 `agent_core` 增加 `TurnToken`、`TurnGate`、最小 reducer；
- 增加 input admission、pending input ownership 与单调 `input_seq`；
- 每次 model request 发送前封存 `PromptCut`，terminal response 必须关联原 request cut；
- worker start/stop/finish 和内部 producer 全部持有 token；
- 对 stale/finished token 做显式拒绝；
- 保持现有 Host 输出，先建立单一 Core 真相。

验收：Core 单元测试覆盖乱序、重复 finish、Stop 后晚消息、下一 Turn 不受旧 Stop 影响；压测一在 PR profile 下证明请求已发出后才到达的 supplement 不被错误归因给该响应。

### Phase 2：Core authoritative projection 与 ownership handoff

- Core worker event 增加完整 Turn token；
- Core 输出 `CoreTurnProjectionChanged`；
- terminal commit 原子返回 accepted-but-unconsumed task commands，保留 command identity、输入、附件和顺序；
- `CoreSessionState` 降级为 activity/wait hint，不再是 Session lifecycle 真相。

验收：Host 不需要结合 worker count 判断 Active Turn，并能区分 `AcceptedPending`、`ConsumedByPromptCut` 与 `ConvertedToNextTurnIntent`；压测一、二在 PR profile 下无 ownership 或生命周期失败。

### Phase 3：Pod projection 与有界 intent FIFO

- 从 `timem_web/src/server.rs` 提取小型 projection 模块；
- 用 Core projection 替换 `active_turn_id/pending_turn_id/cancelling_turn_id` 的分散写入；
- 建立按 `enqueue_seq` 排序、按 `command_id` 去重、容量明确且持久化的 Next intent FIFO；
- lifecycle barrier、自动发送偏好和 outcome continuation grant 分别判断，不能把 `Empty` 当自动派发授权；
- `state` 改为只读派生字段；
- 增加 projection revision；
- 保留旧 wire event 双发并做 shadow comparison。

验收：旧投影与新投影在正常流程一致；故意注入迟到事件时只有新投影保持正确；压测二、三证明普通 Send 与未消费 supplement 并存时不丢失、不覆盖、不重复派发。

### Phase 4：WebUI 改为纯 projection consumer

- snapshot 和 `session_projection_updated` 成为唯一生命周期输入；
- `core_topic`、`worker_activity` 仅影响 timeline；
- 删除 WebUI 的 lifecycle reducer 分支；
- 本地 cancel intent 只保留 command retry，不再修正业务 projection。

验收：浏览器 Stop、reload、sequence gap、late event、下一 Turn 全通过；压测四使用 release browser path 报告时延分位数并满足对应 gate 预算。

### Phase 5：删除兼容状态机

- 删除 Host 的重复 lifecycle 字段和 worker-count 聚合；
- 删除旧 `turn_started/turn_finished/turn_cancelling` 生命周期消费路径；
- 删除 WebUI `sessionCancellationApplies` 等迁移期业务 guard；
- 更新 topic protocol 和 module boundaries。

### Phase 6：持久化与恢复收口

- 明确 epoch 高水位/runtime incarnation 策略；
- restore 只恢复历史 Turn outcome，不恢复 Active Turn；
- 进程异常退出时最后 Active Turn 投影为 `Interrupted`；
- Snapshot 由 Core/Pod 当前投影重新生成。

## 13. Shadow comparison 与可观测性

迁移期同时计算旧、新投影，但只有一个对外生效：

```text
old_state != new_projection.derived_state
→ structured diagnostic
  session_id
  old active/pending/cancelling fields
  new TurnToken
  triggering event kind
  event_seq
```

禁止用静默 fallback 掩盖差异。差异应成为测试失败或 debug diagnostic，以便删除旧路径前证明覆盖完整。

## 14. 并发压测与用户体验验收

本架构不能依靠几个同步 reducer case 宣称竞态已解决。测试策略采用**少而重**：保留必要的纯不变量单元测试，但发布门槛集中在以下 4 个真实并发压测。每个压测都必须运行真实 `CoreSessionWorker`、Turn Gate/Reducer、Pod command/projection 路径和持久 command ownership；只允许用可控 fake model 替代外部模型服务。

### 14.1 统一压测 harness

压测 harness 至少有两个独立执行方：

```text
model/core side                  user/host side
-----------------------------    -----------------------------
real Core worker thread/task     independent command producer
fake model request + delay       Send/Supplement/Stop/Reconnect
PromptCut + terminal commit      command ack/outbox/FIFO changes
Core projection publication      projection/browser observation
```

约束：

- 不允许在同一测试线程中按预设顺序直接调用两边函数来冒充并发。
- `Barrier`、`Condvar` 或显式 test hook 用来保证命中 PromptCut sealed、request in flight、terminal parsed、terminal commit 等关键窗口。
- fake model 的短 `sleep` 用来模拟真实服务耗时和扩大调度窗口，**不能**用 `sleep` 的先后判断输入归属或测试是否成功。
- 第二种模式在关键 barrier 之间加入可复现 jitter。jitter 由 seed 驱动，覆盖 0–20 ms 等小延迟以及少量较慢响应；失败必须打印 seed、iteration、command ID、TurnToken、PromptCut 和阶段记录。
- 所有等待都有 deadline。超时必须报告卡住阶段，不能无限等待或用更长 sleep 掩盖死锁。
- 每轮结束检查完整 ownership ledger：每个 command/附件恰好在 pending、PromptCut-consumed、FIFO、Active Turn 首轮或 rejected 中的一处。
- 使用单调时钟测同一进程/浏览器时延。跨浏览器与 Host 时钟不得相减；timestamp 顺序不能当成因果证据。

运行规模不是“重复两次”：

| 模式 | 每个核心场景最低迭代 | 用途 |
|---|---:|---|
| PR/普通 CI | 300 | 每次变更都真实施压，同时控制总时长 |
| Linux/macOS release gate | 1,000 | 跨平台调度差异 |
| 手动/定时 soak | 10,000 或持续 10 分钟，先到者为准 | 长尾、资源泄漏、极低概率交错 |

通过 `TIMEM_TURN_STRESS_ITERATIONS`、`TIMEM_TURN_STRESS_SEED` 和 `TIMEM_TURN_STRESS_DURATION_SECS` 控制；默认值不能低于上表对应 gate。测试输出总轮数和实际运行时长，防止配置错误导致“零轮压测”。

### 14.2 压测一：PromptCut / terminal ownership race

一个真实 Core worker 线程执行 fake model；另一个独立用户线程持续提交显式 supplement。每轮由 barrier 将输入分别放在：

- request PromptCut 封存前；
- PromptCut 封存后、模型响应返回前；
- terminal response 解析后、commit 前；
- terminal commit 后、Pod projection 到达前。

每个固定窗口先做确定性命中，再用 seed jitter 混合运行。fake model 每轮 sleep 2–20 ms，周期性加入 50–100 ms 慢响应，验证短慢模型都不会改变 ownership 规则。

验收：

- 只有 PromptCut 覆盖的 supplement 能留在原 Turn；
- 已接受但未消费的 supplement 全部按稳定 command identity 进入 FIFO；
- 新 Turn 使用新 token 并从 model round 1 开始；
- final answer、outcome、stats 留在原 Turn；
- 无输入/附件丢失、重复消费或错误倒算；
- 无 deadlock、panic、未回收线程和持续增长的 pending ownership。

### 14.3 压测二：Stop / Start 生命周期风暴

Core/model 线程持续执行可取消的短模型等待与工具等待；用户线程连续执行：

```text
Stop(A) → ordinary Send(B) → Stop(B) → Send(C) → ...
```

同时周期性注入 child-worker late finish、旧 tool callback、重复 Stop、重复 submit 和 delayed projection。至少 300/1,000/10,000 个 Turn cycle，而不是只点击几次。

验收：

- 任意时刻每个 Context 最多一个 Active Turn；
- 旧 Stop 和旧 callback 永远不能修改新 Turn；
- terminal outcome 不可逆，spinner/working 不复活；
- FIFO 顺序、容量和 continuation policy 正确；
- 每个 cycle 最终收敛为 terminal 或明确 queued/rejected，不留下幽灵 pending 状态；
- worker、线程、channel、队列和持久记录在压测结束后回到有界基线。

### 14.4 压测三：真实 WebSocket 恢复与 FIFO ownership

启动真实 `timem_web` Host、真实 Core worker 和 fake model endpoint；至少两个独立 WebSocket client 并发操作同一 Session。在 accepted、Core handoff、terminal commit、projection publish 和 ACK 前后主动断线/重连，并注入重复 command、普通 Send 与未消费 supplement、附件、队列编辑/重排/取消、MEM switch barrier。

验收：

- 同一 `command_id` 的 domain effect 恰好一次；不同 ID 的相同文本不能被误去重；
- snapshot 或 sequenced projection 必须覆盖每个 committed effect，不能两者都没有；
- 普通 Send 与迁移 supplement 同时存在时不覆盖、不丢失，并按权威 `enqueue_seq` 收敛；
- 附件与 command 一起移动且恰好消费一次；
- reconnect/duplicate ACK 不启动第二个 Turn；
- 旧 `mem_epoch` command 不进入新 MEM；
- 断线风暴后 outbox、Host ownership ledger、FIFO 和 Core projection 一致。

### 14.5 压测四：真实 Chrome 用户体验时延

使用 release Web binary、真实 WebSocket、真实 Host/Core worker 和 fake model，在 headless/可见 Chrome 中循环执行 Stop、立即输入下一任务、显式 supplement、断线重连和 Session A/B 切换。前端 reducer fixture 不能代替该测试。

按 `command_id` 记录以下同域阶段：

```text
browser_input → local_feedback → websocket_send
server_received → accepted → core_projection
terminal_commit → next_dispatch
browser_projection_applied → browser_painted
```

报告 p50/p95/p99/max 和最慢 10 个样本。模型故意 sleep 的时间单独记录；体验验收看系统附加时延，不能把 fake-model sleep 算成 Timem 回归，也不能从跨时钟 timestamp 相减。

首版 release-build loopback 预算：

| 用户可感知路径 | p95 | p99 | 说明 |
|---|---:|---:|---|
| 点击/输入 → 本地 Sending/Waiting/Stopping 反馈 | 50 ms | 100 ms | 浏览器单调时钟 |
| WebSocket send → Host accepted | 100 ms | 250 ms | 同机 loopback，不含断线重试 |
| Host accepted → 对应 Core/Pod projection 可用 | 200 ms | 500 ms | 不含 fake-model 固定 sleep |
| terminal commit → 已授权的下一队首 dispatch | 100 ms | 250 ms | 自动发送开启且有 continuation grant |
| projection 到达浏览器 → 首次 paint | 100 ms | 250 ms | Chrome 同一时钟域 |

这些是产品预算，不是通过把测试 sleep 调短就能满足的数字。普通共享 CI 若机器噪声导致绝对时延不稳定，仍必须执行完整正确性压测，并使用同机空载基线后的附加时延与宽松上限防止数量级回退；Linux/macOS release stress runner 必须执行上表绝对预算。任何单样本超过 2 秒且不能由故意断线、重试 backoff 或 fake-model delay 解释，都作为 stall 失败并打印完整阶段链。

### 14.6 失败证据与 CI 落点

实施 Phase 0 时必须新增一个集中入口：

```text
scripts/turn_concurrency_stress.sh
```

它只运行上述 4 个重压场景，不把整个 workspace 重复 1,000 次。Rust 并发场景进入 `agent_core`/`timem_web` integration test binary；Chrome 场景使用现有浏览器工具链启动 release binary。实现后由 `scripts/edge_regression.sh` 或 CI 的独立 Turn-stress step 调用 300 轮 PR profile，release gate 调用 1,000 轮，定时任务调用 soak profile。普通 edge loop 不得通过外层重复 300 次整个 workspace 来冒充该压力入口。

失败输出至少包含：

- seed、iteration、平台、build profile；
- command ID、TurnToken、PromptCut、projection revision、event sequence、mem epoch；
- 两边线程的命名阶段，不只是一串 wall-clock timestamp；
- ownership ledger diff 和最终 snapshot；
- p50/p95/p99/max、最慢样本及其是否包含模型 sleep/重试。

测试不得通过提高 timeout、减少迭代、关闭 jitter 或只断言最终 `ready` 来“修复”。任何 flaky 失败都先保留 seed 并形成可复现 regression。

## 15. 非目标

本次重构不负责：

- 改变模型循环、prompt、tool semantics；
- 把 Pod 变成远程服务；
- 引入通用 event-sourcing 框架；
- 持久化运行中的 worker/process；
- 重写所有 timeline UI；
- 用一个巨大枚举描述所有细节。

目标只是收敛 Turn 真相和 UI 投影，不创造新的基础设施复杂度。

## 16. 代码落点建议

确认设计后，优先采用小模块：

```text
core/agent/src/turn_state.rs
  TurnId / TurnEpoch / TurnToken
  TurnGate
  TurnSlot / TurnOutcome / TurnActivity
  InputAdmission / PendingTurnInput / PromptCut
  invariant tests

timem_web/src/turn_projection.rs
  SessionProjection
  ProjectionRevision
  bounded NextTurnIntent FIFO (restart discards execution ownership)
  Core projection → Web projection
  compatibility/shadow comparison

interfaces/web/src/projection.ts
  revision acceptance
  immutable projection replacement
  no lifecycle inference
```

避免创建大型“状态框架”。每个模块应能在一次屏幕内看清核心状态转换。

## 17. 完成定义

只有同时满足以下条件才算重构完成：

- Core 是 Active Turn、input admission、PromptCut 和 outcome 的唯一写入者；
- Core projection 是所有 Bridge/Interface 的共同生命周期真相；
- reconnectable Bridge 只增加交付语义；`timem_web` Pod 只按明确 continuation policy 派发有界队列；
- `timem_shell` 直接消费 Core projection，不维护 terminal lifecycle；
- `web_ui` 删除生命周期推断；
- Web Bridge 删除 worker-count/topic-driven lifecycle 聚合；
- 21 条不变量均有自动化测试；
- 4 个真实并发压测在 PR、macOS/Linux release 和 soak profile 达到规定轮数，正确性、资源收敛和体验时延预算通过；
- 旧兼容协议和迁移 guard 已删除，而不是永久保留两套路径。

# 实例资源上限（Instance Resource Caps）

## 1. 背景与语义

Timem 实例运行时派生的 shell job（构建、测试、索引等）可能把整机 CPU 与内存吃满，
导致系统失去响应。实例资源上限为**当前 Timem 实例**提供一层内核强制的资源围栏：

- 上限是**比例值**，以机器总量为基准：CPU 相对在线逻辑核总容量、内存相对系统总内存。
- 默认均为 **85%**，即本实例的受管 job 树最多使用整机 CPU 容量的 85% 与系统总内存的 85%。
- 配置只作用于当前实例；不做任何系统级全局配置，不触碰其他进程。
- 被约束的是**受管 job 树**（`timem.jobs` cgroup 子树，全部派生 job 进程）——它是资源
  大户；Timem 主进程本体不在其中（模型 IO 循环占用很小），保持不受限以免拖慢自身响应。

## 2. 配置

| 环境变量 | 默认 | 含义 | 非法值行为 |
|---|---|---|---|
| `TIMEM_SYSTEM_CPU_CAP_PERCENT` | `85` | 实例 job 树可用的系统 CPU 容量比例，`(0, 100]` | stderr `[config_error]` 警告并回退默认 |
| `TIMEM_SYSTEM_MEM_CAP_PERCENT` | `85` | 实例 job 树可占的系统总内存比例，`(0, 100]` | 同上 |

- 读取与应用发生在 host 启动早期（`applications/timem` 的 Web host 装配），先于任何
  shell job 运行。
- 每个维度带独立**开关**：关闭即把该维度内核值写回无界（`cpu.max = "max 100000"` /
  `memory.high = "max"` / `memory.max = "max"`），状态记录为 `Disabled`；百分比在校验中恒须合法（作为重新
  开启时的保留档位）。env 面只提供百分比种子，开关默认为开。
- WebUI 的数值输入是**固定档位**下拉（10 / 25 / 50 / 70 / 85 / 100），自由输入已移除。
- 每进程仅应用一次（幂等）；后续调用返回首次记录的结果。运行时修改走热更新路径。
- 非法值绝不以错误配额生效：一律警告 + 回退默认 85。

## 3. 平台技术矩阵

| 平台 | 机制 | 状态 |
|---|---|---|
| Linux | cgroup v2：在 `timem.jobs` 子树根写 `cpu.max`（quota = cap% × 在线核数 × 100ms 周期，内核最小 quota 1000µs）、`memory.max`（cap% × `MemTotal`，硬上限）与 `memory.high`（`memory.max` × 85%，软节流点） | **已实现并验证**（`core/platform/src/linux.rs`） |
| Windows | 计划：runtime-wide Job Object 上启用 `JOBOBJECT_CPU_RATE_CONTROL` 硬上限 + `JobMemoryLimit`（`windows/job.rs` 已有 kill-on-close Job 骨架可挂载） | 未实现；caps 返回 `Unsupported` 并如实呈现 |
| macOS | 无内核级实例配额公开机制（仅有软优先级，如 `nice`/taskpolicy），不伪装支持 | 未实现；caps 返回 `Unsupported` 并如实呈现 |

平台扩展点：新平台只在 `core/platform/src/api.rs` 增加 `platform_*` 分发实现；未实现前
统一返回 `Unsupported`/空操作，`agent`/`session`/`bridge`/`interface`/WebUI 不需要任何
平台条件分支。公共 facade 的名称与通用诊断文案保持平台中立（不绑定 cgroup 等单一机制），
后端词汇只留在 `linux.rs`/`macos.rs`/`windows/` 内部。

### Linux 生效细节

- `timem.jobs/runtime-*/session-*/job-*` 层级中，进程只进入叶子；cgroup v2 规定子组创建
  时从父组拷贝 controller 参数，因此在 `timem.jobs` 根写一次 `cpu.max`/`memory.max`/
  `memory.high` 即可覆盖本实例当前与未来全部 job 叶子。
- `cpu.max` 是硬上限：超过 quota 的 CPU 时间被内核节流，任务仍运行但变慢。
- `memory.max` 是硬上限：整棵 job 树的可回收内存用尽时触发 cgroup OOM，杀掉越限子树内
  的进程；Timem 主进程在 `timem.self` 兄弟组，不参与该限制。
- `memory.high = memory.max × 85%` 是软节流点：超过后内核回收 page cache/换出 anon 并
  节流分配，通常把用量钉在 high 附近而不杀进程；只有回收也无法满足时才走到 max 的 OOM。
  有 swap 时内核优先换出（实测 64MiB max、512MiB 分配存活，`max` 事件增长而 `oom_kill=0`）。
- `memory.high` 与 `memory.max` 作为一个维度成对应用：任一文件写入失败时，平台把两者都
  回滚到写入前的值，再报告 `Unavailable`，避免 UI 显示“未生效”而内核仍残留半套限制。
- 需要 systemd delegated cgroup v2 子树；权限不足时 `cpu`/`mem` 分别报告 `Unavailable`
  及原因，`probe_instance_resource_caps` 会进一步分类：无需 root 即可用（`Ready`）、
  root 可修复（`NeedsEscalation`，附最小动作计划）或 root 也修不了（`Unavailable`）。
- Timem 与 `timem.jobs` 同属委托子树下的兄弟路径，写入不影响兄弟 scope。

### Linux 提权修复（一次性 sudo，仅当前实例）

当探测判定 `NeedsEscalation`（典型：`cpu` 控制器未下放到用户树），Timem 提供一次性
root 修复，动作只有两类白名单操作：

- `MigrateSelf`：把 Timem 主进程迁入实例根下自管的 `timem.self` 组（不写任何限额，
  主进程永不受限），使实例根保持无进程。populated 的实例根一旦启用控制器会被内核
  强制转为 threaded 子树，而 threaded 只承载 cpu——memory 将永久不可用（实验证实该
  转换不可逆，threaded 状态无法原地回落）。探测阶段先尽力无特权迁移；失败时由本
  动作以 root 修复（`mkdir -p` + 写 `cgroup.procs`，幂等）。排序上迁移永远先于
  一切控制器启用。
- 共享实例根(根下除 Timem 外还有兄弟进程,如同一 scope 里的终端与构建工具)不做自迁移:
  迁移后根仍 populated 且子组 populated,cpu 控制器启用会被内核拒绝(EBUSY),反而使 cpu
  失效。共享根的探测结果固定为 cpu 照常走提权(populated 根上启用,内核可能自动转
  threaded)、memory 恒 `Unavailable`(固定文案,fail closed)——cpu 行为与无自迁移架构
  完全一致,零退化。
- 无特权自修复(`unprivileged_self_repair`,探测与直接应用路径共用):实例根属主是当前
  用户时,委托子树允许 Timem 自己完成修复——先把主进程迁入 `timem.self`(写自己的
  `cgroup.procs`),再沿祖先链(浅 → 深)对缺失控制器写 `+controller`,遇到第一个用户不
  拥有的层即止(该层留给提权计划)。空根判定以 `cgroup.procs` 为准(`cgroup.events` 的
  populated 标志在迁移后会滞后)。root 授权因此只服务于用户不拥有的层(root-owned 的
  session scope)与共享根两种情形。
- `EnableController`：沿祖先链（浅 → 深）向缺失层级的 `cgroup.subtree_control` 写
  `+<controller>`；
- `MakeWritable`：root 将实例根目录属主归还当前用户，使 `timem.jobs`/`timem.self`
  可无特权创建。

安全模型（fail closed）：

- 执行载体是 `sudo -S`，root 密码只从 stdin 流给 sudo，内存中用完即按字节清零
  （`ZeroizingPassword` drop 保证所有早退路径），绝不落盘、不进日志、不进错误消息；
  错误归因固定分流：sudo 自身退出码 1（密码错误/sudoers 拒绝，脚本未执行）映射为
  固定文案 `root authorization rejected`；白名单动作失败使脚本 `exit 77`（例如内核
  拒绝控制器写入——threaded 子树仅支持 cpu，不支持 memory），此时仅附最多 400 字符的
  内核 `sh` stderr 尾行（路径与 errno，绝不含密码）；15 秒超时即 kill。
- 脚本由固定模板渲染，cgroup 路径与控制器名严格白名单校验（字符集、前缀、长度、
  `..` 拒绝），任何用户输入都无法注入。
- 修复范围**仅当前实例**：不写 `/etc`、不改 systemd 单元，重启/重新登录后失效，
  需要时再次授权；Timem 自身全程以普通用户运行。
- 交互形态：命令行密码提示**只存在于启动阶段**——启动时若存在维度需要修复且
  `isatty(stdin)` 判定为真（后台服务/测试环境 stdin 非 tty，永不阻塞），提示 `y/N`
  （30 秒无输入视为拒绝），拒绝或无终端则降级启动并在 WebUI 保持可见。**host 运行后
  不再出现任何命令行密码提示**：需要 root 的修改一律由 WebUI 弹密码窗走
  `system_resource_caps_escalate` 命令。

### 零 root 启动与多实例共存（推荐路径）

启动方式按"少用 root"优先排序：

1. **首选 `systemd-run --user --scope`**（systemd 用户管理器可用时，真零 root）：

   ```bash
   XDG_RUNTIME_DIR=/run/user/$(id -u) \
     systemd-run --user --scope --unit=timem-myinstance \
     /path/to/timem --space /path/to/space --public --port 22540 --no-open
   ```

   实测（2026-11-05）：scope 落在 `user@<uid>.service/app.slice/<unit>.scope`，属主即当前用户；
   Timem 启动后自行完成 `timem.self` 迁移、`subtree_control=[cpu memory]` 启用与
   `timem.jobs` 三限额（默认 85% 档 `cpu.max=2380000 100000`、`memory.max=28310351872`、`memory.high=24063799091`），
   零密码、零 `[config_error]`；SIGINT 退出后 scope 目录由 systemd 自动删除，无残留。
2. **次选：已在用户可写委托子树内直接启动**（无 systemd 或偏好前台进程）：进程本身位于
   用户属主的子树（如 `user@<uid>.service/` 下自建目录）时，`unprivileged_self_repair`
   自行完成迁移 + 控制器启用 + 限额写入，全程零 root。
3. **裸终端（root 属主 session scope）**：首次把进程放进 `user@<uid>.service` 子树需要 root
   一次——cgroup v2 委派要求写入者对源/目标共同祖先的 `cgroup.procs` 有写权限，实测普通
   用户写 scope 根报 `EACCES`（root 权限通过后被 no-internal-process 拦，报 `EBUSY`）。
   该次 root 修复（属主归还、控制器启用）是内核持久态，同会话重启零 root；新登录会话再来一次。

实现本身不依赖 systemd：资源管控代码中是纯 cgroupfs 操作（零 systemd 调用），`systemd-run` 只是获得用户属主 scope 的便捷启动器；没有 systemd 时直接 cgroup 路径等价工作。命令执行同理——cgroup v2 job 作用域创建或配置失败即回退进程组（cgroup 在 `pre_exec` 前后失效时再重试一次），不会因可选机制缺失而阻塞命令。

没有 cgroup v2 设施（例如容器内 cgroupfs 不可写/已卸载）时 **fail-open**：实例正常启动
（实测 HTTP 200、进程存活），`cpu`/`mem` 维度报 `Unavailable` 及原因、输出
`[config_error]` 诊断，命令执行降级到进程组回退链，功能不中断。

多实例共存的语义是**一实例一 scope**：

- 不同 `--unit` 的实例各自拥有 `<scope>/timem.jobs` 与 `<scope>/timem.self`，限额互不影响。
  实测两实例同启：各自 `subtree_control=[cpu memory]`、实例根 `cgroup.procs` 为空、
  `timem.self` 只含本实例主进程、`cpu.max`/`memory.max`/`memory.high` 三写成功、`config_error=0`。
- **同一个 scope 内不能共存两个实例**：内核 no-internal-process 规则要求启用控制器的
  中间节点不得有成员进程，实测第二进程写入 scope 根 `cgroup.procs`——普通用户 `EACCES`、
  root `EBUSY`。第二实例会退到原 session scope（root 属主），创建 `timem.jobs` 失败并
  `Unavailable`，属 fail closed 的正确报错；需要并行实例就各自开 scope。
- 观测提示：jobmanager 对每个存活任务注入 `/proc/<pid>/cgroup`（observation only，不代表
  任务所有权），因此能看到当前 session 全部任务的 cgroup 落点——裸终端任务在 session
  scope，systemd-run 任务在其 scope。该信息仅用于观察，不参与调度或限额判定。

### 启动时自动进入独立 scope（默认行为，2026-11-06）

- 启动早期（`run_web` 的第一件事，早于进程安全网、workspace 锁与 cgroup 探测）检查当前实例根是否具备承载内存控制器的结构。以下任一成立即自动迁移：
  - 实例根与其它进程共享（内核会把 populated 根强制转为只支持 cpu 的 threaded 子树）；
  - `cgroup.type` 不是 `domain`（已是 `domain threaded` / `domain invalid` / `threaded`）；
  - 实例根 `cgroup.controllers` 不含 `memory`（例如运行期 per-job 围栏 cgroup：单进程、`domain`，但整条链没有控制器委托）。
- 迁移命令为 `systemd-run --user --scope --collect --unit=timem-scope-<pid>-<nanos> -- <exe> <args...>`，并通过 `exec` 原地替换当前进程。实测 systemd 249 下 `--scope` 会把调用进程移入新 unit 后直接 exec，PID、stdio、工作目录、终端与会话关系不变；scope 位于 `user@<uid>.service/app.slice/`，只含 Timem 一个进程。
- 新 scope 的根是空 `domain`，`cgroup.controllers=[cpu memory pids]`；随后既有的 `timem.self` 自迁移、控制器启用与三限额写入链路零 root 生效（实测 85% 档写入 `cpu.max=2380000 100000`、`memory.high≈22.41GB`、`memory.max≈26.37GB`，启动日志 `CPU limit: 85% applied` / `MEM limit: 85% applied`）。
- 防循环：迁移时设置 `TIMEM_INDEPENDENT_SCOPE=1`；子进程即使仍看到共享/threaded 根也只打印告警，不再嵌套迁移。
- 显式关闭：`TIMEM_NO_INDEPENDENT_SCOPE=1`，供自行管理 scope 的操作者使用。
- 失败回退：`systemd-run` 不存在、用户 systemd manager socket 不可用或迁移后仍不满足条件时，Timem 继续在当前 scope 启动并打印 `[timem_scope_isolation] ...` 告警；资源管控仍按原有 fail-closed 语义显示 `Unavailable`，不阻塞启动。
- `-h`/`--help`/`-V`/`--version` 不触发迁移，避免帮助/版本输出产生进程副作用。
- 已运行的实例不会热迁移：升级二进制后需重启一次，新进程启动时自动完成。

## 4. 可观测性

- `## RUNTIME` 新增 `resource` reporter（`core/agent/src/runtime_info.rs`）：
  - 完整生效：报告 caps、换算值（核数/GiB）与 cgroup scope，每次 host 启动后只呈现一次。
  - 未完整生效（部分失败、平台不支持、配置非法）：每次请求持续呈现，并提示模型在此期间
    少派、派轻量 shell job——这是内核围栏缺失时唯一的软补偿。
- 降级同时在启动时打到 stderr（`[config_error]` 风格），不会静默。
- WebUI 的 Session Insights 看板通过 `session_resource_usage_query`（只读、连接内
  直连应答、不进语义重放）每 2s 采样当前 session 的 CPU/mem，曲线只保留最近 20s；
  内存主值取 `memory.stat` 的 `anon + file_mapped`（真实占用），`file` 中未映射的
  可回收页缓存单列显示，避免把 page cache 误当成真实占用；平台无 cgroup 记账时返回
  `usage: null`，前端显示“不可用”而不是伪造 0。
- Insights 采样合流（2026-11-07）：`session_resource_usage_query` 优先复用监控循环
  最新一轮完整 sweep（`ResourceSampleCache` 单槽、每轮整体原子替换、只保留最新一轮，
  天然有界）：多个浏览器标签（Insights 看板、头部芯片）共享同一份内核级采样，不再
  各自直读 cgroupfs 与 /proc；缓存未命中（监控恢复中、两轮 sweep 之间、无记账
  scope、瞬时读取失败或系统快照缺失）时回退现场直采，语义不变；失败样本不入缓存。
- 统计页刷新探针（2026-11-07）：统计页每次重渲染时版本号 +1 并以 `data-stats-version`
  嵌入 `<main class="shell">`；页面轮询先请求 `GET /api/debug/stats-version`
  （只读版本号，认证与现有 debug API 相同），版本一致则跳过整页 fetch，减少无谓
  传输；版本不一致或探针失败才拉取全页并做 innerHTML 对比替换。

## 5. WebUI 系统设置接口

系统设置（Timem Web → 设置 → 系统）提供「系统资源管控」区块，与 `TIMEM_*` 环境变量
共同构成配置面：

- **查看**：快照 `server.mem.system_cpu_cap_percent` / `system_mem_cap_percent` 呈现当前
  保存值（首次无保存值时以环境变量为种子，缺省 85）。
- **修改**：前端提交 `system_resource_caps_update`（`ClientCommand::SystemResourceCapsUpdate`），
  处理顺序为「内核先热应用、成功才保存」：
  1. `agent_core::resource_caps::update_system_resource_caps` 校验并调用平台后端重写
     cgroup `cpu.max` / `memory.max` / `memory.high`（运行时热生效）；
  2. 应用成功才写 `mem_settings.json`（`save_web_mem_settings`）并更新 `state.mem`；
  3. 广播 `WireEvent::MemSettingsUpdated`（携带两个新字段）刷新所有客户端。
  内核应用失败（权限不足、平台不支持、数值非法）时**不保存、不静默**，命令返回
  `invalid_system_resource_caps:<原因>`，UI 报错保留原值——存储值与内核实际执行值永不漂移。
- **持久化与启动**：设置保存在当前 MEM 空间的 `mem_settings.json`（含 serde 默认值：
  缺字段时用环境变量/85 种子）。host 启动加载 MEM 设置后、以及 MEM 空间切换后，都会
  以当前设置重新应用一次 caps（`apply_instance_resource_caps_from_mem`），因此重启后
  保存值继续生效；环境变量仅作为首次种子。
- **可观测性联动**：`## RUNTIME` 的 `resource` 段在 caps 值变化后会再次呈现一次新状态。
- **UI 细节**：每个维度一行 = 开关（`role="switch"`，on=内核管控）+ 档位下拉
  （10/25/50/70/85/100），不再渲染内核状态徽标或技术文案，只按能力四态呈现
  （`resourceCapability`，`interfaces/web/src/runtime_settings.ts`）：
  - **可管控**（`applied`/`disabled`）：普通 on/off 开关，保存走
    `system_resource_caps_update`。
  - **缺权限**（`unavailable` 且 `escalation_required`）：开关保持可交互且一律显示
    OFF（开关反映内核实际可执行状态，而非存储的期望标志）；打开开关立即弹管理员密码
    对话框走 `system_resource_caps_escalate`（目标维度强制 enabled=true 提交，内核先
    热应用、成功才保存），取消或点击遮罩则保持 OFF。面向非技术用户，密码框文案不使用
    root / 内核 / 提权等术语，只说明"需要管理员授权、密码仅用于本次操作"。
  - **暂时不可用**（`unavailable` 且 `escalation_required=false`）：能力存在但当前无法
    应用，开关禁用、强制 OFF，行灰化并附 ⚠️ 图标。常见根因是**登录会话 scope 与其它
    进程共享**（直接终端启动时，同一 cgroup 里还有 shell、构建工具等）：内核已把该树
    转为只承载 cpu 的 threaded 子树，memory 在该结构中永远无法启用。服务端为此下发
    结构化 `blocker="shared-instance-root"`，面板同时显示行内可见说明，明确告知用独立
    scope 启动（如 `systemd-run --user --scope timem`）后重试；启动摘要也优先展示这一
    结构性原因，而不是底层 `EACCES` 写入错误。其余情况回退到通用文案「当前系统暂时
    无法使用 CPU/内存 管控」。这与平台不支持是两种不同事实，不能混为一谈。
  - **不支持**（`unsupported`）：平台不存在该能力，开关禁用、强制 OFF，行灰化并附 ⚠️
    图标，原生 title 提示「当前系统不支持 CPU/内存 管控」，不提供任何操作入口。
  两种不可操作状态共用 `is-inert` 样式与 `resourceCapabilityIsInert` 判定，仅提示文案
  不同。提示只使用原生 `title`：设置面板内容区会滚动/裁剪，绝对定位的自定义 tooltip
  会在面板边缘被截断；⚠️ 图标保留 `tabIndex=0` 与 `aria-label`，键盘/读屏用户仍可获取
  提示。密码对话框打开后焦点落在密码框，Tab 由应用级对话框焦点陷阱限制在卡片内，
  文档级 Escape 可从任意控件关闭对话框（不再依赖密码框自身的键盘处理）。
  未修改/非法时禁用保存，保存中与失败状态均有 aria-live 反馈；密码输入不持久化、
  提交后即清空；中英文文案齐全（`system.resourceCaps*`）。

## 6. 验证状态

- Linux：`core/platform/tests/unit/platform_tests.rs` 覆盖配额计算（核数换算、内核最小
  quota、内存比例）、`/proc/meminfo` 解析、caps 校验边界，以及真实 cgroup 写入 + 幂等
  （无 delegated 子树的环境自动跳过真实写断言）。
- Agent：`core/agent/tests/unit/resource_caps_tests.rs` 覆盖 env 解析（默认/合法/非法/
  独立维度告警）；`runtime_info.rs` 内部测试覆盖渲染（生效一次呈现、降级持续呈现、
  平台不支持文案）。
- Windows/macOS：`Unsupported` 分支在本轮未做运行时验证（与仓库既有 Windows 后端一致，
  以 `docs/windows-support-matrix.md` 为准；编译通过不代表声明支持）。

### 6.1 Linux 内核机制模拟压测（memory.high，user 委托树）

`timem.jobs` 子树根写入的 `memory.high` 与 systemd 在委托树内设置的 `MemoryHigh=` 作用于
同一 cgroup v2 内核开关。为在本机（Ubuntu 22.04，kernel 5.15，systemd 249，user 树仅委托
`memory pids`）验证真实执行效果，用 `systemd-run --user` 瞬态服务做了等价压测：python
逐页触碰分配 400MB，随后第二遍重触全部页面（强制把被回收换出的页再换回，放大节流效应）：

| 场景 | `memory.high` 回读 | `memory.current` 峰值 | `memory.events` high | 二次触页耗时 | 进程结局 |
|---|---|---|---|---|---|
| `MemoryHigh=256M` + 400MB | `268435456`（256 MiB） | `268640256`（钳制在限值旁） | **1753 次** | **2.57s** | rc=0，存活 |
| 无限制对照 | `max` | `424091648`（≈400MB 全量驻留） | 全程 0 | **0.01s** | rc=0，存活 |

- 软限按预期钳制：峰值内存停在限值附近而非继续上涨；超限期间内核累计节流 1753 次，
  进程耗时被拉长约 250 倍——实例 job 变慢，但系统其余部分不受影响。
- 软限不杀进程：两组进程均正常退出（rc=0）；`memory.high` 语义正是"节流而非 OOM"。
- 对照组确认无限制时 400MB 全量驻留、零节流，排除环境噪声。

CPU/memory 维度内核级压测均已完成(2026-11-05;root 先将 `cpu` 加入
user@1000.service 委托,此前 `cgroup.controllers` 仅为 `memory pids`)。CPU:
`timem-diag-t7` 写入 `cpu.max=200000 100000`(2 核),4 进程满载 python 进驻后 10s 窗口
实测用量 **2.02 核**、`cpu.stat` `nr_throttled` +100、`throttled_usec` +19.8s——内核真实
节流;限值恢复 `max` 后同负载用量回到约 4 核。memory:`memory.high=1073741824`(1 GiB)
下 1.6GB python 逐页分配,`memory.current` 被钳制在限值旁(1073.2~1073.5MB),
`memory.events` high 累计 **5289 次**,逐页触摸耗时 14.7s(正常量级 <2s),进程存活、
`oom_kill` 保持 0——节流而非 OOM,与 `timem.jobs` 写入路径同一内核开关,行为一致。
未委托环境行为不变:真实 cgroup 写入测试自动跳过(cpu 与 mem 两维度都 `Applied` 才
执行内核断言,其余环境打印跳过原因后返回)。生产部署因此要求 timem 运行于
委托子树(systemd user service 启动);session scope 手动启动时 caps 降级为持续告警,
无内核强制。

### 6.2 第三阶段验证（能力探测 / 一次性提权 / WebUI 实时状态）

- `core/platform` 51 项测试全绿：escalation 动作白名单脚本渲染（路径/控制器校验、
  `..` 与越界拒绝）、探测分层（Ready / NeedsEscalation / Unavailable / Unsupported）、
  `sudo -S` 流透传（PATH 注入假 sudo 记录 argv 与 stdin，断言密码只经 stdin 流入且
  不进日志）、`enabled=false` 写回 `max`（写失败但已为 max 时判 `Disabled`）。
- `agent_core` 925 项全绿：系统 caps 环境变量/设置解析、`Disabled` 的 `## RUNTIME`
  渲染（intentionally unbounded 文案）。
- `timem` lib 471 项 + embedded_assets 2 项全绿；前端 882 项全绿，
  `pnpm build` 重建并提交 `interfaces/web/dist`。
- `lifecycle_process_tests` 8/8（3.6s）：初版启动 prompt 以 `/dev/tty` 判定交互性，
  在后台/测试环境误判为可交互而阻塞 stdin 造成 4/8 失败（两轮复现确认）；修复为
  `isatty(stdin)` 判定并给 spawn 点显式 `stdin(Stdio::null())`（`.output()` 形式按
  std 语义天然 null stdin），非 tty 启动永不阻塞，降级提示走 stderr。同轮修正
  `resource_caps_prompt` 的符号归属：经 `agent_core::resource_caps` 转发引用平台
  能力（timem 包本地 `os` 模块不承载平台转发）。
- 门禁：`cargo clippy --workspace -D warnings` 绿、`cargo fmt --all --check` 绿、
  `git diff --check` 绿。
- 真实运行验证：独立空间与端口启动 `timem --no-open`，无 tty stdin 时立即完成启动
  （stderr 打印 `[resource_caps]` 降级提示与 `[config_error]` 持续告警，不阻塞、
  不等待）；`GET /api/snapshot` 呈现
  `resource_caps.cpu/mem = {enabled, percent, kernel: "unavailable", limit: null,
  reason: "timem.jobs subtree unavailable: Permission denied", escalation_required:
  true}` 与 `scope = session scope 路径`，同时快照含
  `system_cpu_cap_percent/system_mem_cap_percent/system_*_cap_enabled` 字段——
  会话 scope 无委托子树时 fail closed 降级，WebUI 与内核实况不漂移。

### 6.3 mem 可用架构验证(timem.self 自迁移 + 共享根守卫,2026-11-05)

真实 sudo、真实二进制、WS 驱动端到端(全程密码仅 stdin/内存,不进 argv/日志):

- 无特权迁移:主进程自动迁入 `timem.self`,实例根保持空 domain;root 迁移路径(root-owned scope)同样真实生效。
- 错密码:`sudo -k` 后真实提权返回固定文案 `root authorization rejected`,内核零改动。
- 白名单脚本失败(内核 EBUSY):`exit 77` 归因 + stderr 尾行(≤400 字符)如实呈现。
- 专属终局:投影双 applied(`cpu.max=2380000 100000` + `memory.max=28310355354` + `memory.high=24063802050`)、`subtree_control=cpu memory`、实例根 `type=domain`、`timem.self` 不受限、动态 cap 文件属主修复生效。
- 共享守卫:实例根含兄弟进程时端到端复验——cpu applied 2380000(与旧版逐位一致,零退化)、memory `Unavailable`(固定文案)、无任何进程迁移、cpu-only 降级提示。
- 门禁:platform 55 / agent_core 926 / timem lib 471 / web 889 全绿,clippy/fmt/diff-check 绿。
- 委托子树内零 root 端到端(用户属主 scope,默认 85% 档):第一次启动无特权完成迁移 +
  控制器启用 + 限额写入,投影双 `applied`(`cpu.max=2380000 100000` +
  `memory.max=28310351872` + `memory.high=24063799091`,回读差值为内核页取整)、零 `[config_error]`;Ctrl+C(SIGINT)
  优雅退出即清理自身 cgroup 设施——`timem.jobs`/`timem.self` 全部删除、实例根
  `subtree_control` 恢复为空;第二次启动零 root 重建全链并重放限额,退出再次清零。清理为
  尽力而为:仍有活孤儿(目录 populated)或用户不拥有的层会保留,下次启动幂等重建。
- 用户获得 memory 管控的操作:让实例根内只有 timem 一个直接进程(专属 scope,如构建完成后再单独启动 timem),WebUI 输 root 密码一次,cpu+mem 双生效;共享 scope 下 cpu 可用、memory 按内核规则不可用,fail closed 如实呈现。


### 6.4 零 root 部署与多实例共存验证(2026-11-05)

- 两实例各自 `systemd-run --user --scope`（`timem-mi-a`/`timem-mi-b`，端口 22550/22551）：
  双 scope 各自 `subtree_control=[cpu memory]`、实例根空、`timem.self` 各含本实例 pid、
  `cpu.max=2380000 100000` + `memory.max=28310351872` + `memory.high=24063799091`、`config_error=0`、HTTP 200/200；
  SIGINT 双停后进程退出、scope 目录由 systemd 自动删除、`app.slice` 无残留、全程零 root。
- 同 scope 双实例对照：A 启用控制器后，B 无法进入 scope 根（用户 `EACCES`，root `EBUSY`），
  落到 root 属主 session scope 后 `timem.jobs` 创建失败并如实报 `Unavailable`；B 退出时
  该 session scope 零改动（`subtree_control` 保持空、无 `timem.*` 目录），清理守卫生效。
- 无设施 fail-open：cgroupfs 不可写环境下实例照常启动并给出 `Unavailable` 原因；执行链
  三层回退（结果解析 → 进程组 → kill 进程组）由单测锁定。

## 7. 边界与不做什么

- 不监控系统整体负载做动态退避：caps 是静态实例配额，语义是"本实例最多占用多少系统资源"。
- 不做系统级全局配置（不写 `/etc`、不改 systemd 全局单元、不杀其他进程）。
- 不强制接管启动方式：默认在启动早期把自身迁移到独立 `systemd-run --user --scope` scope（可用
  `TIMEM_NO_INDEPENDENT_SCOPE=1` 关闭）；无 systemd 用户管理器时留在当前 scope 并告警，
  cgroup v2 设施缺失时 caps fail-open 降级、命令执行回退进程组，均不阻塞实例启动。
- 不约束 Timem 主进程本体；如未来需要，应另立显式契约评估（自我 cgroup 迁移会与
  process-Job 所有权模型交互，不能顺手做）。


### 6.7 启动自动隔离验证（2026-11-06）

从现有共享登录会话的 per-job cgroup（`type=domain`、单进程、`cgroup.controllers` 为空）启动新二进制，验证：

- 进程自动 exec 到 `user@1000.service/app.slice/timem-scope-<pid>-<nanos>.scope`；scope 根 `cgroup.procs` 为空、`type=domain`、`controllers=[cpu memory pids]`、`subtree_control=[cpu memory]`，`timem.self` 仅含主进程。
- `timem.jobs` 三限额真实写入：`cpu.max=2380000 100000`、`memory.high=24063819776`、`memory.max=28310372352`（31.0GiB 的 85%），启动日志明确输出 `CPU limit: 85% applied` 与 `MEM limit: 85% applied`。
- SIGTERM 优雅退出后，瞬态 unit、scope 目录与 `timem.self`/`timem.jobs` 全部被 systemd 回收，无残留；临时 space 已清理，现役实例未受影响。

## 6.5 memory.max 行为与 OOM 归因实测（2026-11-05）

在 user 委托 cgroup 树内用独立 scope 做三组对照（不影响现役实例）：

| 场景 | 内核设置 | 结果 |
|---|---|---|
| max + swap 可用 | `memory.max=64MiB`，子进程分配 512MiB | 进程存活 rc=0；`memory.current` 钉在 max，`memory.events.max +1624`，`oom_kill=0`（swap 吸收） |
| max + 禁用 swap | 同上，另设 `memory.swap.max=0` | 子进程被 SIGKILL（rc=137）；根 `oom 1`、`oom_kill 1` |
| OOM 计数传播 | 同上 | 被杀进程所在 `session-*` 与 `job-*` 的 `oom_kill` 各 +1；session 层是整棵子树的汇总，job 层用于命名具体作业。两者不能求和（会重复计数） |

结论：

- `memory.max` 是真正的硬上限；有 swap 时它是最后手段，只有回收/换出都失败才杀。
- cgroup OOM 会传播 `oom_kill` 事件到被杀任务所在的 session/job 层，因此运行时观测可以
  准确归因到具体 session，不需要“猜最大占用者”。
- 主进程位于 `timem.self`，不在 `timem.jobs` 的 memcg OOM 候选集内；外部给实例根设置
  `MemoryMax` 或系统级全局 OOM 才会影响主进程。

## 6.6 运行时资源观测提示（session resource monitor）

Host 每 2s 读取每个存活 session 的 `session-*` 聚合用量；阈值与投递语义：

| 观测 | 触发条件 | 归因 | 投递 |
|---|---|---|---|
| CPU 持续高负载 | 该 session `cpu.stat usage_usec` 增量折算 ≥ 有效 quota 的 90%，连续 ≥40s | 该 session 自身 | 运行中：system supplement 搭下一次模型请求；idle：启动一个 `resource_observation` runtime turn |
| OOM kill（详细） | 该 session 某个 `job-*` 叶子的 `memory.events.oom_kill` 增加 | 被杀任务所在 session + job scope 名 | 立即一条；使用 OOM 前最近一次采样/`memory.peak` 作为内存数值 |
| OOM kill（主导份额） | 同一采样 tick 发生 OOM kill 时，取当前 `memory.current` 最大的 session | 该 session | 若它不是被杀 session，再发一条 dominant-share 提示（“an OOM kill just happened; this session now has the largest memory usage of all sessions”）；若它同时是被杀 session，则把占比句并入详细提示，避免重复 |

- CPU 告警发出后锁存，不重复提示；连续 ≤60% quota 持续 6s 后才复位，允许下一次
  40s 高负载提醒。

- `memory.high` **只作为内核软节流点**，不产生任何模型提示：内存用量可能在 high 附近
  来回浮动，逐次提示会形成报警风暴。只有真正的 OOM kill 才注入 prompt。

- 共享配额被多个 session 合计打满、但没有任何单个 session 越线时，不把提示塞给某一个
  session（归因不唯一，避免误导）；仅记录 Host 诊断。CPU 配额共享但 `usage_usec` 是每
  session 的，因此仍按“该 session 自身持续占用”归因。
- 注入文案为英文（`<RUNTIME>` 角色，与用户/模型语言无关）：CPU 以
  `Runtime observation:` 开头并写明 “has stayed high for more than 40s” 与内存
  GiB/占全部内存百分比；OOM 写明 “OOM-killed” 与 “total task memory usage”，仅当峰值
  确实达到硬上限时才追加 “above the quota threshold”；dominant-share 提示写明
  “largest memory usage of all sessions”。无可用内存统计时写 “memory stats unavailable”。
- 采样失败/无 cgroup 时静默跳过；状态 map 随 session 消失清理，不持久化历史。
- 非 Linux：`session_resource_usage` 返回 `Ok(None)`，monitor 自然降级为不提示。

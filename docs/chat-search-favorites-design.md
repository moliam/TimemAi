# 聊天记录搜索与收藏设计

状态：设计草案
目标分支：`feat/chat-search-favorites`

## 1. 目标

为当前 MEM 中的聊天记录增加两个相互配合、但语义独立的能力：

1. **搜索聊天记录**：按关键词在当前 Session 或全部 Session 中查找用户消息和模型最终回复，查看上下文并跳回原对话。
2. **收藏模型回复**：收藏模型的最终回复，添加标题、备注、标签与收藏夹，之后可以独立浏览和整理。

第一版收藏对象只包含模型的 **final answer**。中途答案、思考/动作事件、用户消息暂不允许收藏，以保持入口和数据语义明确。

## 2. 产品原则

- **本地优先**：搜索与收藏都属于当前 MEM，不上传外部服务。
- **跨 Session 可用**：用户不应先记住答案在哪个 Session，才能找到它。
- **搜索与收藏分层**：搜索命中原始聊天记录；收藏是用户主动建立的资料库。
- **收藏不能只是脆弱链接**：收藏时保存回复正文快照。原消息或 Session 删除后，收藏仍可阅读，但标记为“来源已删除”。
- **原文不可悄悄漂移**：收藏的正文快照不可编辑；标题、备注、标签、收藏夹可以编辑。
- **Core 拥有数据语义**：索引、持久化、查询、分类规则放在 `agent_core`；Web 只负责命令编排与展示。未来 Shell/iOS 可复用同一能力。
- **不把 UI 搜索伪装成模型记忆**：该功能不自动改变 Prompt，也不自动把收藏注入模型上下文。

## 3. 用户流程

### 3.1 左侧导航入口

Search、Favorite、Settings 固定在 Web 左侧栏底部，位于 Session 列表下方，组成一个竖排导航组，顺序不可变化：

```text
[搜索图标]  Search
[收藏图标]  Favorite
[设置图标]  Settings
```

- 左侧栏展开时，每行显示“图标 + 英文标签”。
- 左侧栏收起时，三项仍在相同位置竖排，只隐藏文字并保留居中的图标。
- 收起状态下通过 `title`、`aria-label` 和 Tooltip 保持可理解性。
- Search 与 Favorite 是两个独立入口，不合并成一个“资料库”入口，也不放进 Settings。
- 点击 Search 打开搜索面板；点击 Favorite 打开收藏面板；点击 Settings 沿用现有设置中心。
- Search/Favorite 面板打开时，对应入口显示选中状态；再次点击当前入口关闭面板。
- Search 与 Favorite 互斥，同一时刻只显示其中一个资料面板；打开 Settings 时关闭二者，避免多层遮挡。
- 桌面端面板从右侧打开并可调整宽度；移动端使用全屏覆盖，但入口顺序保持一致。

### 3.2 搜索

点击左下角 **Search** 入口打开搜索面板：

1. 输入关键词，停顿约 200–300 ms 后发起查询；Enter 可立即查询。
2. 默认范围为“全部 Session”，可切换“当前 Session”。
3. 默认搜索用户消息与模型最终回复，可按角色过滤。
4. 结果显示：
   - 命中摘要与关键词高亮；
   - Session 名称；
   - 时间；
   - 角色；
   - 若是模型回复，显示是否已收藏。
5. 点击结果：
   - 切换到来源 Session；
   - 若目标不在当前已加载的历史页，按定位接口加载目标 Turn 附近窗口；
   - 滚动到目标 Turn，并短暂高亮。

空查询不扫描全部正文，只显示最近记录或最近搜索历史（搜索历史第一版仅保存在浏览器，不进 MEM）。

### 3.3 收藏

模型最终回复的操作区在“复制”旁增加星标按钮：

1. 第一次点击直接收藏到“未分类”，立即反馈成功。
2. 再次点击可取消收藏；取消前不弹确认，资料库中的删除操作则使用确认或可撤销 Toast。
3. 星标旁的下拉/长按入口可直接选择收藏夹。
4. 收藏后可在资料面板编辑：
   - 自定义标题；
   - 备注；
   - 多个标签；
   - 所属收藏夹。

默认标题从答案首个非空文本行提取，去除 Markdown 标记并限制长度；提取失败时使用“来自 {Session} 的回复”。不调用模型生成标题。

### 3.4 收藏归类与收纳

点击左下角 **Favorite** 入口直接进入收藏面板，不需要先经过搜索页签。收藏面板采用：

- 左栏：全部收藏、未分类、收藏夹列表、标签列表；
- 中栏：收藏卡片列表，可按最近收藏、原回复时间、标题排序；
- 详情：完整 Markdown 正文、备注、来源信息、打开原对话、移动收藏夹、标签编辑、取消收藏。

第一版规则：

- 一条收藏属于零个或一个收藏夹；
- 一条收藏可有多个标签；
- 收藏夹可创建、重命名、排序、删除；
- 删除非空收藏夹时，收藏回到“未分类”，不删除收藏；
- 标签按使用产生，不单独维护空标签；移除最后一次使用后自然消失。

暂不做嵌套收藏夹。扁平收藏夹 + 标签已经能覆盖“归类”和“交叉收纳”，同时避免树结构移动、循环和移动端交互复杂度。

## 4. 关键架构判断

### 4.1 数据归属

搜索和收藏均为 **MEM 级数据**，不是浏览器 localStorage，也不是单 Session 状态。

推导链：

1. 同一 MEM 的 Web/Shell Session 共享聊天历史；
2. 用户要求在需要时统一回看和整理；
3. 若收藏存在浏览器中，则换浏览器、重启 Host 或使用另一 Host 时不可见；
4. 因此权威数据必须由 `agent_core` 在 MEM 下持久化，并受 `MemGuard` 保护。

建议布局：

```text
<MEM>/
├─ sessions/<session_id>/raw_chat_history.jsonl
└─ chat_library/
   ├─ favorites.jsonl
   └─ collections.jsonl
```

采用 JSONL，与现有本地持久化、恢复和原子替换模式一致。查询时可构建进程内索引；第一版无需引入常驻 SQLite 数据库。现有 `rusqlite` 是读取时建立的受限快照，不应误当成权威数据库。

### 4.2 稳定消息身份

现有 `ChatHistoryRecord::Message` 没有持久化消息 ID，Web 恢复历史时以 `turn_id + created_at_ms + role` 合成 UI ID；删除则使用 `(turn_id, role, role_index)`。这不足以支撑长期收藏和精确定位。

在 `ChatHistoryRecord::Message` 增加可选字段：

```rust
message_id: Option<String>
```

规则：

- 新写入的 user/assistant message 必须生成稳定、MEM 内唯一的 `message_id`；
- 旧记录读取时允许缺失，以保持向后兼容；
- 对旧 assistant final answer，派生 legacy source key：
  `legacy:{session_id}:{turn_id}:assistant:{role_index}`；
- 一旦旧消息被收藏，可在收藏记录中保存该 legacy key，不必为了补 ID 重写整份历史；
- 搜索结果和历史页都返回 `message_id/source_key`，Web 不再自行猜测身份。

`turn_id` 仍是跳转和上下文加载的主要定位键；`message_id` 用于判定具体消息、去重和收藏状态。

### 4.3 收藏保存正文快照

收藏记录必须同时保存来源引用和正文快照：

```rust
pub struct ChatFavorite {
    pub id: String,
    pub source: FavoriteSource,
    pub content_snapshot: String,
    pub title: String,
    pub note: String,
    pub collection_id: Option<String>,
    pub tags: Vec<String>,
    pub source_created_at_ms: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub version: u64,
}

pub struct FavoriteSource {
    pub session_id: String,
    pub turn_id: String,
    pub message_id: Option<String>,
    pub legacy_source_key: Option<String>,
    pub session_display_name_snapshot: String,
}
```

正文快照解决两类问题：

- 删除原消息或 Session 后，收藏仍然有内容可读；
- 以后历史格式迁移或保留策略变化，不会破坏用户主动保存的资料。

来源状态在读取时动态计算为 `available | message_deleted | session_deleted`，不把易过期状态永久写死。

### 4.4 删除语义

- 删除原聊天消息：不级联删除收藏；收藏显示“来源消息已删除”。
- 删除 Session：不级联删除收藏；收藏显示“来源 Session 已删除”。
- 取消收藏：只删除收藏记录，不影响聊天消息。
- 删除收藏夹：收藏转入未分类。
- MEM 切换：加载目标 MEM 的独立收藏库，绝不跨 MEM 混合。

## 5. Core API 设计

建议新增 `agent_core::chat_library` 模块，负责：

- 聊天记录词法搜索；
- 精确定位目标 Turn 附近的历史窗口；
- 收藏、更新、取消收藏；
- 收藏夹 CRUD 与排序；
- 数据校验、恢复、权限和锁。

核心接口草案：

```rust
pub struct ChatSearchQuery {
    pub text: String,
    pub session_id: Option<String>,
    pub roles: BTreeSet<ChatHistoryRole>,
    pub before_created_at_ms: Option<i64>,
    pub limit: usize,
}

pub struct ChatSearchHit {
    pub source_key: String,
    pub message_id: Option<String>,
    pub session_id: String,
    pub session_display_name: String,
    pub turn_id: String,
    pub role: ChatHistoryRole,
    pub content: String,
    pub created_at_ms: i64,
    pub favorite_id: Option<String>,
}

pub struct ChatHistoryWindow {
    pub records: Vec<ChatHistoryRecord>,
    pub target_source_key: String,
    pub before_cursor: Option<String>,
    pub has_more_before: bool,
}
```

词法搜索规则第一版：

- Unicode 小写归一化后做不区分大小写的子串匹配；
- 查询按 Unicode 空白切词，所有词均需命中同一条消息；
- CJK 不要求分词，连续文本可直接子串命中；
- 结果按 `created_at_ms DESC` 排序；
- 最大查询长度 256 个字符，默认 50 条、上限 200 条；
- 只搜索 `Message`，不搜索 action/event；
- 搜索失败或遇到损坏 JSONL 行时跳过损坏行并报告可诊断计数，不阻断其他 Session。

规模演进：第一版为各 Session 建立轻量行偏移/消息元数据缓存，文件长度或 mtime 变化时失效。只有性能测试证明大 MEM 不达标时，再增加持久倒排索引或 SQLite FTS；不要提前维护第二份难以一致的数据源。

## 6. Web 协议

新增命令：

```ts
{ type: "chat_search"; query: string; session_id?: string; roles?: ("user" | "assistant")[]; before_created_at_ms?: number; limit?: number }
{ type: "chat_history_locate"; session_id: string; turn_id: string; source_key: string }
{ type: "favorite_create"; source: { session_id: string; turn_id: string; source_key: string }; collection_id?: string }
{ type: "favorite_update"; favorite_id: string; expected_version: number; title?: string; note?: string; collection_id?: string | null; tags?: string[] }
{ type: "favorite_delete"; favorite_id: string; expected_version: number }
{ type: "favorites_list"; collection_id?: string | null; tag?: string; query?: string; before_updated_at_ms?: number; limit?: number }
{ type: "favorite_collection_create"; name: string }
{ type: "favorite_collection_update"; collection_id: string; expected_version: number; name?: string; ordinal?: number }
{ type: "favorite_collection_delete"; collection_id: string; expected_version: number }
```

对应事件：

```ts
chat_search_result
chat_history_window
favorite_created
favorite_updated
favorite_deleted
favorites_page
favorite_collections_updated
```

所有变更命令继续使用现有 `command_id`、accepted/committed/rejected 机制，支持重连去重。`expected_version` 防止两个浏览器标签页覆盖彼此的收藏编辑。

Snapshot 只携带：

- 收藏夹列表；
- 收藏总数和每收藏夹计数；
- 当前已加载消息对应的收藏 source key 集合（或映射）。

不把全部收藏正文塞进 Snapshot。收藏列表按需分页获取，避免启动负担随资料库线性增长。

## 7. Web UI 组件与状态

建议拆分文件，避免继续膨胀 `main.tsx`：

```text
web_ui/timem-web/src/chat_library/
├─ protocol.ts
├─ reducer.ts
├─ SearchPanel.tsx
├─ FavoritesPanel.tsx
├─ FavoriteEditor.tsx
└─ chat_library.css
```

交互细节：

- 左侧栏底部新增统一的 `.sidebar-primary-actions` 竖排容器，内部依次放置 Search、Favorite、Settings；不使用横排，也不因折叠改变顺序。
- 展开态按钮统一为图标加文字的左对齐行；折叠态统一为固定尺寸的居中图标按钮，三个按钮保持相同点击区域和垂直间距。
- 现有 `.sidebar-settings-button` 应抽象为可复用的侧栏动作样式，而不是复制三套近似 CSS。
- 搜索图标使用 Lucide `Search`；收藏入口使用语义清晰的星标/书签图标，最终实现时三者在视觉重量、尺寸和 active 状态上保持一致。
- 搜索请求带递增 request key；迟到结果若 query/scope 不匹配则丢弃。
- 收藏按钮使用 `aria-pressed`，收藏/取消收藏进行中禁用，失败回滚并显示明确错误。
- 搜索高亮必须在纯文本摘要上切片渲染，不用 `dangerouslySetInnerHTML`。
- 跳转历史时先保留当前 Session 滚动位置，再切换并定位；返回原 Session 时恢复位置。
- 移动端资料面板全屏覆盖；桌面端沿用现有可调整宽度侧面板模式。
- 键盘：`Cmd/Ctrl+K` 打开搜索；`Escape` 清空搜索或关闭面板；结果列表支持上下键与 Enter。
- 收藏详情正文复用现有 Markdown 安全渲染器和代码复制能力。

## 8. 一致性与恢复

`favorites.jsonl`、`collections.jsonl` 采用与 Session index 类似的恢复原则：

- 单条记录有 `id/version/updated_at_ms/deleted`；
- 写入追加新版本；读取时同 ID 取最高 version，版本相同时取最新位置；
- 定期压实通过临时文件 + `sync_all` + 原子 rename；
- malformed、超大、截断记录隔离或跳过，有界读取；
- Unix 目录 `0700`、文件 `0600`；
- 同一 MEM 的写入经 `MemGuard` 串行化；
- 更新/删除校验 `expected_version`，冲突返回 `favorite_conflict` 或 `collection_conflict`。

标签校验：trim、去重、单标签最长 40 字符、最多 20 个。收藏夹名称 trim 后非空、最长 80 字符，同一 MEM 下大小写不敏感唯一。

## 9. 分阶段实施

### Phase 1：基础身份与 Core 存储

- 为新聊天消息持久化稳定 `message_id`；
- 实现 `chat_library` 数据结构、JSONL store、恢复、版本冲突；
- 收藏夹和收藏 CRUD；
- Core 单元测试。

### Phase 2：搜索与定位

- 跨 Session 词法搜索；
- 文件索引缓存与失效；
- `chat_history_locate` 返回目标 Turn 窗口；
- 大历史、损坏行、旧记录兼容测试。

### Phase 3：Web 协议与收藏入口

- Host 命令/事件；
- final answer 星标；
- 收藏资料面板、收藏夹、标签、备注；
- 重连、重复命令、并发编辑测试。

### Phase 4：搜索 UI 与端到端验收

- 全局/当前 Session 搜索；
- 高亮、筛选、分页和跳转；
- 桌面/移动端、键盘与无障碍；
- 文档、测试矩阵和 release checks。

## 10. 验收矩阵

必须覆盖：

1. 新旧历史记录都能被搜索；CJK、英文大小写、多关键词正常。
2. 只搜索消息，不把 action/result 等过程事件混入结果。
3. 全部 Session 与当前 Session 范围隔离正确。
4. 搜索命中未加载的旧 Turn，能够加载、切换、滚动并高亮。
5. 收藏 final answer 后重启 Web、换浏览器连接同一 MEM，收藏仍存在。
6. 同一来源重复收藏幂等，不生成两条记录。
7. 原消息删除、Session 删除后收藏正文仍可读，来源状态正确。
8. 取消收藏不删除原消息；删除收藏夹不删除收藏。
9. 两个浏览器并发编辑同一收藏，旧 version 被拒绝且不会静默覆盖。
10. MEM 切换后收藏、收藏夹和搜索范围完全切换。
11. malformed/truncated/oversized JSONL 不导致 Host 启动失败或无界内存。
12. 1 万、10 万消息规模下搜索延迟和 UI 响应达到约定门槛；性能门槛在实现前写入测试矩阵。
13. WebSocket 断线、命令重发、迟到搜索结果、快速反复星标均保持一致。
14. Markdown、超长答案、代码块、CJK、空白标题、重复标签均正常显示与保存。
15. 键盘、屏幕阅读器和窄屏流程可用。
16. 左侧栏展开时严格按 Search、Favorite、Settings 竖排显示图标和文字；收起时严格按同一顺序只显示图标，点击区、Tooltip、焦点和 active 状态正确。
17. Search、Favorite、Settings 互斥打开，不出现搜索/收藏面板与 Settings 中心叠层冲突。

## 11. 暂不纳入第一版

- 语义/向量搜索；
- 自动摘要、自动打标签；
- 收藏中途答案或任意选区；
- 嵌套收藏夹；
- 云同步、导出分享；
- 自动将收藏注入模型上下文；
- 修改收藏正文快照。

这些能力以后可以建立在稳定消息身份和独立收藏库之上，不应阻塞第一版。

## 12. 待确认的产品选择

推荐默认值如下，可直接进入实现：

- 搜索默认跨全部 Session；
- 只允许收藏 final answer；
- 收藏单选收藏夹、多选标签；
- 删除原聊天不删除收藏；
- 收藏正文不可编辑；
- 使用关键词词法搜索，不上语义搜索；
- Web 先交付，Core 能力保持跨 Host 可复用。

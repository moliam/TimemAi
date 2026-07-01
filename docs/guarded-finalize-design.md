# Guarded Finalize (status:finished + action + expect)

## 背景
当前 runtime 处理 `status:finished` 时立即 Final，忽略 `next_actions`；处理 `status:working` + actions 时执行完必回模型一轮。用户希望增加一种"乐观终止"：模型自信任务已完成，同时让 runtime 帮忙跑一个动作 + 一个断言命令，断言通过就直接展示 `final_answer`，跳过一次模型调用。

## Schema 增量
每个 next_actions 元素可选字段：

| 字段 | 类型 | 必填 | 语义 |
|---|---|---|---|
| expect | string | 否 | 断言用的 bash 命令；exit==0 视为 pass |
| expect_timeout_ms | int | 有 expect 时必填 | 断言命令超时（毫秒） |

约束：
- `expect` 只在 `status:finished` 下且是 next_actions 的**最后一个** action 时生效。
- 其他 action 带 expect → 忽略并置 repair_issue = `next_actions[i].expect_only_allowed_on_last_action_with_status_finished`。
- `status:working` + 任何 action 带 expect → 忽略并 repair_issue = `expect_requires_status_finished`。
- 有 `expect` 但缺 `expect_timeout_ms` → repair_issue = `expect_timeout_ms_required`。

## Runtime 执行流程
1. 模型返回 `status:finished` + 有 expect 的最后一个 action → runtime 走 guarded 分支。
2. 顺序执行所有 next_actions（复用 execute_action）。
3. 期间若任一 action 触发 approval → 走既有 NeedsUserApproval 流程，approval 恢复后回 NeedModel（放弃本轮 guard，保守）。
4. 全部 action 执行完（无 approval 中断）后：
   - runtime 通过受控 Bash 路径跑 `expect` 命令（timeout = `expect_timeout_ms`，受普通 bash 上限约束）。
   - `expect` 继承 `TIMEM_BASH_APPROVAL`：`ask` 会先进入用户确认，`approve` 才直接执行。
   - exit==0 → 把 `final_answer` 作为 `TurnFinal.response_to_user` 展示；审计记录 `guard_pass`。
   - exit!=0 或超时或命令不存在 → 打包 action 结果 + expect 结果为 prompt_delta，回到 NeedModel。

## 失败回传给模型的 prompt slice 格式
```
Action result: run_bash
command: <原命令>
status: 0
output: ...

Expect check:
command: <expect 命令>
status: 1
stdout: ...
stderr: ...
verdict: FAIL

Note: 你上轮用 status:finished + expect 声明任务完成，但 expect 命令 exit!=0。请根据以上证据修正后再回复。
```

## 与 acceptance_check 的关系
expect 是客观 runtime 验证，优先级高于 `acceptance_check.is_satisfied`。带 expect 时 acceptance_check 不影响判定。

## 审计事件
- `guarded_finalize_start`：记录 action 数、expect 命令、expect_timeout_ms
- `guarded_finalize_expect_result`：记录 expect 受控 Bash 结果摘要
- `guarded_finalize_pass` 或 `guarded_finalize_fail`

## 改动清单

| 文件 | 改动 |
|---|---|
| agent_core/src/lib.rs | ParsedAction 加 expect/expect_timeout_ms；parse_envelope 抽取；schema 校验新规则；apply_model_response 的 status:finished 分支加 guarded 路径；expect 无 approval 执行 helper |
| timem_shell/src/session_runtime.rs | 2 个 e2e：guarded_finalize_pass_skips_model / guarded_finalize_fail_returns_to_model |
| static prefix (prompt_0) | Response_rule 补 guarded finalize 语义 |
| docs/architecture.md | 补简短说明 + 链回本文档 |
| CHANGELOG.md | 记录新特性 |

## 测试用例
1. **guarded_pass**：模型 status:finished + run_bash 写文件 + expect grep 匹配 → runtime 执行 action 后 expect exit==0 → 只调用模型 1 次，Final 文本 = final_answer。
2. **guarded_fail**：模型 status:finished + run_bash + expect grep 不匹配 → 调用模型 2 次，第 2 次 prompt 含 `Expect check:` 与 `verdict: FAIL`。
3. **guarded_missing_timeout**：模型带 expect 但缺 expect_timeout_ms → repair 一次。
4. **guarded_on_non_last_ignored**：中间 action 带 expect → 提示 repair_issue，最后一个若也无 expect 则退化为普通 status:finished。

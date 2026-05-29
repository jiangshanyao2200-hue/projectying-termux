# ProjectYing 流式请求与思考降噪修复计划

## 背景

本轮测试里，`Coding` 角色反复出现三类系统性问题：

1. `SSE stream semantic idle timeout after 60s without model progress`，并且日志里只看到“第1次”，没有真正进入后续重试。
2. `context_summary` / `context_compact` 看起来在失败，但 `status` 和上下文体积又在下降，说明“工具结果”和“存储结果”可能不同步。
3. 长思考会把请求、输出、UI 刷新一起拖慢，最终演化成卡顿、超时、失败。

相关证据：

- `Aidebug/events.jsonl:19072 / 19135 / 19185`，都是同一类 SSE 语义空闲超时。
- `Aidebug/events.jsonl:18304 / 18806 / 18813 / 15087`，都出现过 `context_summary failed: context_summary entry_id 不存在`，但上下文流程仍在继续变化。
- 代码里 Codex 路径的重试入口在 `src/main.rs:45981`，但 `chat_completion_codex_with_tools()` 只在“没有任何 tool activity”时才会重试。

## 现象拆解

### 1. 为什么只看到“第1次”，没有真正重试

当前 Codex 专用路径是：

- 先走 `chat_completion_codex_with_tools_single_attempt()`
- 失败后只有在 `!progress.saw_tool_activity` 时才重试

这意味着：

- 只要这一轮里已经跑过工具，后续就不会再重试
- 语义上这不是“重试次数不够”，而是“重试条件过窄”

### 2. 为什么长思考会明显变慢

当前实现把 reasoning / thinking 当成实时流处理：

- 每个 chunk 都会进 UI
- 每个 chunk 都会更新活跃消息
- 只要 reasoning 还在动，整个请求就持续占用前台节奏

这会放大两件事：

- 模型本身思考慢
- UI / 状态 / 日志也跟着慢

### 3. 为什么 fold 看起来失败，但上下文在减少

从日志看，`context_summary` 有时会对不存在的 `entry_id` 报错，但同时另一路上下文存储/折叠可能已经发生。

这说明当前需要区分：

- 工具调用结果是否成功
- 存储层是否已经发生局部写入
- 状态栏是否应该把“部分成功”表现成“失败”

现在这三者混在一起，容易误判。

## 修复目标

1. SSE / Responses 请求出现 transient failure 时，能可靠重试。
2. 重试上限从 3 调整到可配置 5，默认 5。
3. 不让长 reasoning 直接把主线程和 UI 卡死。
4. 把“失败 / 部分成功 / 已写入但回执异常”分开表达。
5. 保证思考压缩不会把下一轮逼进无限补齐状态。

## 修复方案

### A. 重新划分重试策略

把错误分成 3 类：

1. 纯传输类
   - DNS / connect / TLS / read timeout / SSE 空闲超时
   - 允许重试，默认 5 次

2. 已接受但未产生副作用
   - 已收到首包，但未执行任何 tool call
   - 允许重试，同一轮复用当前输入

3. 已产生副作用
   - 已执行过工具
   - 不允许盲目整轮重放
   - 需要“可恢复的 round checkpoint”

### B. Codex 流式路径改成“可恢复重试”

重点修 `chat_completion_codex_with_tools()` 这条路径：

- 失败后不要直接终结整轮
- 保留 `current_input`
- 保留 `round_index`
- 如果还没执行工具，只重试同一轮
- 如果工具已经执行过，只发“恢复提示”，不要重复执行破坏性工具

### C. 降低过度思考带来的滞后

当前默认对 gpt-5.x 使用 `reasoning_effort=high`，这对长任务太激进。

建议：

- Coding / 长施工任务默认改成可配置档位
- 让 `reasoning_summary` 保持简短，不要把完整思维链长期塞进前台
- reasoning chunk 不要每条都触发 UI 重绘
- 按时间窗或字符窗做批量刷新

### D. 做“思考快照”，不要做“思考复写”

要保留的是：

- 当前结论
- 未完成步骤
- 下一步计划
- 仍然成立的假设

不要保留的是：

- 全量细碎 reasoning
- 重复自证过程
- 已经过时的中间补齐

这样能避免下一轮为了“补齐上轮未显示的思考”而无限循环。

### E. 让上下文折叠结果更可解释

需要把 `context_summary` / `context_compact` 的结果拆开：

- `tool call failed`
- `store updated`
- `ui status reduced`

如果只是回执异常但存储成功，状态不该显示成纯失败。

### F. 让失败日志更像调试数据

建议补齐这些字段：

- `attempt_index`
- `max_attempts`
- `error_class`
- `first_chunk_ms`
- `last_meaningful_ms`
- `tool_activity_count`
- `stream_chunk_count`
- `reasoning_chars`
- `text_chars`

## 实施顺序

1. 先修重试分类和 Codex 可恢复重试。
2. 再修流式 chunk 的节流与 UI 刷新频率。
3. 再修 `context_summary` / `context_compact` 的结果表达。
4. 最后补全回归测试与日志字段。

## 测试清单

1. 语义空闲超时前无工具副作用时，应自动重试到上限。
2. 已执行 tool 后的失败，不应盲目整轮重放。
3. `REQUEST_MAX_RECONNECTS` 可配置到 5。
4. 长 reasoning chunk 不应导致明显 UI 卡顿。
5. `context_summary` 失败但存储成功时，要能区分部分成功。
6. 失败后 status 不应显示成“已成功整理”，也不应误导成“没发生变化”。

## 验收标准

- transient SSE 中断能自动恢复，不再只停在“第1次”。
- 长思考不再把输出、状态、主流程一起拖慢。
- fold / compact 的结果可解释、可追踪、可复查。
- context 与 toolmemory 的职责仍然分离，没有把原始大回执重新塞回活跃上下文。

## 2026-05-23 执行进展

已完成第一批“降卡顿”修复：

1. `REQUEST_MAX_RECONNECTS` 从 3 提升到 5，失败提示同步显示 `5/5`。
2. Codex/gpt-5.x 默认推理档位从 `high` 下调为 `medium`；运行时配置清洗会把旧的 `high/xhigh` 降为 `medium`。
3. 当前 `config/advisor.json`、`config/coding.json`、`config/server.json` 已同步把 `reasoning_effort` 改为 `medium`，避免下次启动继续走旧 high 配置。
4. 前台流式处理改为：只有 `text_delta` / `plan_delta` 才触发聊天渲染缓存失效；纯 `thinking_delta` 只累计内容，不强制整屏重绘。
5. Aidebug 观察器不再为每个 thinking chunk 写 `flow.ui.chunk_applied`，改为可见文本/计划增量或每 64 个 chunk 采样，降低 JSONL I/O 压力。

已验证：

- `cargo test thinking_only_stream_chunk_updates_content_without_forcing_repaint -- --nocapture`
- `cargo test retrying_clears_partial_assistant_draft_before_next_attempt -- --nocapture`
- `cargo test request_reconnect_budget_defaults_to_five_retries -- --nocapture`
- `cargo test sanitize_provider_clears_codex_sampling_controls -- --nocapture`
- `cargo check --all-targets`

## 2026-05-23 第二轮执行进展

这次把根因进一步收窄到两条：

1. `context_summary` 对已过期 `entry_id` 直接硬失败，导致“工具回执失败”但上下文其实已经被别的路径折叠过。
2. Codex 的 SSE 断流需要按 round 处理，而不是只按整次请求处理；否则一旦前面 round 已经执行过工具，后面的空闲超时就会直接把整次请求判死。

已完成的代码改动：

1. `context_summary` 现在对 stale `entry_id` 走部分成功语义：能折叠的继续折叠，过期的记录为 `skipped_stale_entry_ids`，不再因为单个旧 id 直接失败。
2. Codex round 流式读取现在支持“中断但可恢复”：
   - 没有 tool call 的空闲超时会按 round 重试；
   - 已经拿到 function calls 的中断会继续推进，不再把整轮直接锁死；
   - 重试上限保持 5。
3. retry 时前台 assistant 草稿会被清掉，但已完成的 tool runs 保留，避免重复显示上一轮的残留 thinking/text。

已补测试并通过：

- `cargo test retrying_clears_partial_assistant_draft_before_next_attempt -- --nocapture`
- `cargo test retrying_clears_draft_but_keeps_tool_runs -- --nocapture`
- `cargo test context_summary_fold_tolerates_stale_entry_ids_and_reports_partial_success -- --nocapture`
- `cargo test request_reconnect_budget_defaults_to_five_retries -- --nocapture`
- `cargo test thinking_only_stream_chunk_updates_content_without_forcing_repaint -- --nocapture`
- `cargo test sanitize_provider_clears_codex_sampling_controls -- --nocapture`
- `cargo check --all-targets`

结论：

- 这批问题主要是错误处理与状态同步，不是推理强度本身。
- `context_summary` 的失败告警和“上下文减少”可以同时成立，原因是它之前把 stale id 当成了硬失败。
- SSE 断流之前没有被正确拆成“可重试的 round 问题”和“不可重放的工具副作用问题”。

尚未完成：

- 需要继续观察真实运行日志，确认 round 级重试是否足够覆盖“先出工具、后空闲超时”的场景。
- 还要继续盯 `thinking` 长链路是否仍会在 UI 上制造不必要的高频刷新。

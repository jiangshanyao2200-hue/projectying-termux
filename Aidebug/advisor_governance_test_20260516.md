# Advisor Governance Test

日期：2026-05-16

## 目标

本轮从 Matrix 工具治理切换到 Advisor / 司的后台治理能力。

重点验证：
- 司是否能维护自己的 context，而不是只有管理别人 context 的能力。
- 管理别人 context 时，系统是否使用隔离的 focus-mode maintenance payload，并带入目标 persona snapshot。
- datememory 写入是否保留 source_persona、日期、详细事件脉络和必要时间线。
- 同一天 datememory 是否续写而不是覆盖或重复建同日主记录。
- 司是否能把可复用治理经验写入 fastmemory experience。
- 司是否能以低损耗方式向 Matrix 汇报全局治理状态。

## 源码回归

- `coding_context_maintenance_routes_to_advisor_system_request`：PASS。
- `matrix_context_maintenance_routes_to_advisor_system_request`：PASS。
- `self_compact_context_maintenance_routes_to_current_persona`：PASS。
- `inactive_dynamic_role_self_compact_is_scanned_and_queued_for_role_target`：PASS。
- `persona_context_maintenance_payload_is_advisor_context_free`：PASS。
- `self_compact_maintenance_payload_is_context_free`：PASS。

## 初始观察

- 当前只保留用户启动的 ProjectYing 进程，已关闭上一轮 Codex 留下的后台实例，避免双进程同时写 Aidebug / context。
- `context/Advisor/fastmemory.json` 初始为空：`surface=[]`，`experience=[]`。
- 源码设计上，Advisor 固定展开 `context_manage / focus_mode / persona_manage / memory_add / memory_check / memory_read / context_summary`。
- Advisor 不具备 `tool_manage` 管理权；工具开关仍只归 Matrix。
- persona context maintenance provider 输入不会注入司自己的标准 context / focuscontext / fastmemory，只带 toolbox、system prompt、focus-mode payload 和目标 snapshot。

## Live 测试计划

1. 直接投递给 Advisor 一条治理任务，让它：
   - 写入 datememory，要求日记带 `2026-05-16 HH:MM UTC` 时间感；
   - 写入 fastmemory experience，沉淀“司作为全局副脑”的治理经验；
   - 用 `persona_manage.send` 向 Matrix 汇报。
2. 观察 `memory/Advisor/datememory.db`：
   - 新条目是否写入；
   - 是否同日续写；
   - 内容是否足够详细、含时间线、含 source_persona。
3. 观察 `context/Advisor/fastmemory.json`：
   - 是否新增 experience；
   - 是否是可复用策略，不是流水账。
4. 观察 `events.jsonl` 和 `latest_reply.txt`：
   - 工具调用是否成功；
   - 是否向 Matrix 汇报；
   - 是否出现冗长回执或上下文污染。

## Live T5 结果：司自检链路

- Advisor 成功调用 `memory_add persona=advisor target=datememory clear_context=false`，长期日记写入全局 `memory/Matrix/datememory.db`，`source_persona=Advisor`，内容包含 `2026-05-16 18:16 UTC`、Aidebug live T5、fastmemory、context_manage、persona_manage 等线索。
- Advisor 成功调用 `context_manage write target=fastmemory section=experience`，写入“司作为全局副脑”的可复用治理经验：datememory 记录时间线，fastmemory 只写稳定策略，管理他人 context 依赖隔离 snapshot，低损耗汇报 Matrix。
- Advisor 成功用 `persona_manage.send` 向 Matrix 汇报。
- 暴露问题：虽然任务要求“只收口本轮新增测试条目”，Advisor 实际调用了 `context_manage summary target=context` 且没有 `entry_ids/range`，导致系统按整区收口，把 Advisor 旧上下文 e31..e97 压成 e98，丢失旧锚点。这是工具边界问题，不应只靠模型自觉。

## Live T6 结果：Matrix 压力维护

- 大 payload 触发 Matrix 标准 context 硬阈值维护，系统将 `System Batch · 2` 路由给 Advisor；provider 输入只包含工具箱、司提示和维护 payload，未注入 Advisor 标准上下文，隔离链路通过。
- Advisor 对 Matrix 调用了 `context_manage persona=matrix target=context summary`，Matrix 标准 context 收口为 1 条摘要，文件约 7.7KB。
- Advisor 对 Matrix 调用了 `memory_add persona=matrix target=datememory clear_context=true`，Matrix datememorycontext 被写入长期日记并清空。
- 暴露问题：随后 Advisor 自身 datememorycontext 进入维护，但 `MEMORIE` 系统维护提示被完整写回了同一个 datememorycontext；第一次维护 payload 约 477k chars，第二次变成约 951k chars，形成递归膨胀。request 6 没有工具调用却回复“已完成”，request 7 空输出，健康面板显示 Advisor 缓冲约 3495KB 且 pending=false。

## 本轮修复

- `context_manage summary` 的模型工具入口新增 `scope` 参数；未给 `entry_ids/range` 或 `item_ids/range` 时，必须显式传 `scope="all"`，否则拒绝执行。这样保留内部整区收口能力，但切断模型误触发整区压缩。
- Advisor / Matrix prompt 和系统维护 ticket 文案已同步：整区收口必须显式 `scope="all"`。
- `MEMORIE` / `[sys:memory_maintenance]` 系统提示写入 memory 前会被压缩成短摘要；渲染旧 datememorycontext 时也会对旧的大型 `SYSTEM · MEMORIE` 条目做短摘要，避免历史噪声再次进入维护 payload。
- 调度层新增保险：如果永久记忆维护轮结束后 datememorycontext 仍然超限，会移除本轮 recent 门禁并重新挂起 memory maintenance，而不是留下 `pending=false` 的假完成状态。
- 已清空当前 `memory/Advisor/datememorycontext.json` 的过渡 entries，保留 `next_entry_id=21`。这是对已入库维护后的缓冲清理，不是删除长期记忆。

## 回归验证

已通过：
- `context_manage_input_preview_marks_summary_all_scope`
- `execute_context_manage_summary_without_ids_requires_explicit_all_scope`
- `memory_maintenance_notice_is_rendered_as_short_summary`
- `memory_maintenance_completion_requeues_when_buffer_is_still_over_limit`
- `maintenance_sync_generates_context_summary_ticket_when_threshold_exceeded`
- `maintenance_sync_generates_fastmemory_summary_ticket_without_item_ids`

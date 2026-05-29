# 2026-05-26 Matrix Coordination Upgrade Plan

## 目标
- 用真实用户语气测试 Matrix 是否仍然像“门户 / 调度层 / 母体”，而不是只做一个普通聊天角色。
- 验证 Matrix 发起调度后，Coding / Server 的结果能否及时回流 Matrix，并由 Matrix 合并为用户可用的结论。
- 验证 Matrix 在等待部门结果时不会失去主任务方向；收到反馈后应调整计划，而不是被反馈打断成另一条任务。
- 若发现链路缺口，先记录证据，再小范围修复提示词、调度提示或运行态投递逻辑。

## 当前已知链路
- Matrix 当前有 `persona_manage`、`spawn_agent`、`wait_agent`、`command`、`memory_*`、`focus_mode`、`tool_manage` 等主控工具。
- `persona_manage.send` 会把任务排入目标 persona 的系统请求队列；其他 persona 也可以用 `persona_manage.send` 主动向 Matrix 汇报。
- `persona_manage.observe` 只允许 Matrix / 司读取折叠活动，是 Matrix 等待部门结果时的低成本观察入口。
- `wait_agent` 属于子代理等待工具，不等同于 persona 之间的等待；本轮要观察模型是否会误把两者混用。

## 实机测试设计
1. 投递给 Matrix 一条用户口吻任务，要求它同时协调 Server 与 Coding：
   - Server 负责观察当前 ProjectYing 运行态、Aidebug health/status、网络/本机进程的只读事实。
   - Coding 负责检查源码里 Matrix/persona 调度和反馈注入链路，给出可能的缺口。
   - Matrix 需要在自己继续梳理方案的同时等待/观察两边反馈，最终给用户一份合并判断。
2. Aidebug 观察点：
   - `Aidebug/status.json`：`active_request*`、`pending_system_requests`、`system_request_dispatch_state`。
   - `Aidebug/events.jsonl`：`request.sent/done`、`tool.start/done`、`persona_manage` 工具记录、目标 persona 请求是否启动。
   - `Aidebug/latest_reply.txt`：Matrix 最终回执是否合并了 Server/Coding 结果。
   - `Aidebug/performance.json`：是否有失败、超时、慢请求、慢 UI。
3. 通过标准：
   - Matrix 至少调度两个目标 persona，且不是只给用户空泛计划。
   - Coding / Server 至少一方主动用 `persona_manage.send` 回报 Matrix，或 Matrix 用 `observe` 明确取回折叠结果。
   - Matrix 最终回复能说明调度结果、证据、风险和下一步，不把部门结果丢在后台。
   - 无 `request.failed` / `tool.failed` / 长期卡在 `pending_system_requests`。

## 预期设计方向
- 如果只是 prompt 问题：加强 Matrix 的调度闭环指令，要求“发出任务后必须等待/观察回执并合并”，并要求部门“完成后主动向 Matrix 汇报”。
- 如果是运行态问题：增加部门完成消息对 Matrix 的可见通知，或把 persona_manage 回执聚合成 Matrix 可观察的调度收件箱。
- 如果是 UI/Aidebug 问题：在 Aidebug 状态中暴露调度批次、发送方、目标、是否已被 Matrix 消化，方便后续自动测试。

## 备份
- 本轮源码快照：`/data/data/com.termux/files/home/AItermux/projectying-backups/projectying-source-20260526-132637.tar.gz`
- 已排除：`target/`、`target-codex/`、`.git/`、运行日志、processed/failed inbox、截图和临时目录。

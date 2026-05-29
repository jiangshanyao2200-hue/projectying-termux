# Aidebug

ProjectYing AI 调试口。

- `events.jsonl`：唯一 AI 调试事件流，按 `stream` 区分 interface/request/tool/department/alert；超过 4 MiB 自动保留尾部约 1 MiB。UI 慢帧明细主要写入 `performance.json`，避免观测日志本身拖慢 TUI。
- `status.json`：当前运行状态快照，由运行时生成，包含协议版本、persona、当前 provider/model、context / memory 布局、动态角色协议与治理摘要、工具输出协议、配置治理协议、调度协议摘要与活跃请求观测字段。
- `health.json`：链路健康侦测快照，由 `status.json` 同源派生，按 persona、动态角色治理、communication、memory、context、token、tool、config、UI、scheduler、tool_projection 链路给出状态、分数和证据。
- `performance.json`：AI 调试性能快照，记录请求/工具耗时、网络阶段、UI 慢帧、上下文体积、重试、失败与阈值告警；`recent_network` 可区分线程启动、HTTP POST、headers、首个 SSE event 与 stream 结束，`recent_ui` 记录慢 draw/tick。
- `tool_projection_snapshot.json`：只读工具投影对账快照，按 persona/role 列出 default_tools、governance 自动工具、observe/toolbox 状态、provider 暴露、可调用集合与每个工具的原因。
- `persona_dispatch.jsonl`：`persona_manage` 派单生命周期事件流，记录 queued/requeued/delivered/running/completed/failed/skipped。
- `latest_reply.txt`：最近一次 AI 正文回执；`latest_reply/<Persona>.txt` 保留每个 persona 的最近回执，避免互相覆盖。
- `inbox/`：外部调试投递入口，放入 `.txt` / `.md` / `.json` 即可让程序按当前或指定 persona 发送。
- `processed/`：已消费的调试投递。
- `failed/`：无法消费或发送的调试投递。

TXT 格式可选首行：`persona: matrix|advisor|coding|server`。
JSON 格式：`{"persona":"server","debug_session_id":"dbg-demo","text":"任务内容"}`。
Aidebug inbox 投递会作为目标 persona 的模型输入发送，但聊天 UI 统一显示为 `Aidebug / 开发者AI调试` 调试来源，不混入普通用户身份。
当前 persona 清单：Matrix · 萤、司、Coding · 绫、Server · 御。

AI 排障入口：先读 `status.json` 判断请求状态、司队列、动态角色 governance、活跃角色 contract、活跃请求是否已有工具调用、thinking/text 规模与协议摘要，再读 `health.json` 查看各链路状态/分数和证据；persona 调度链路读 `persona_dispatch.jsonl` 或 `persona_manage.observe` 的 recent_persona_dispatch；再按 `stream` / `data.department` / `performance.json.alerts` / `performance.json.recent_network` / `performance.json.recent_ui` 过滤问题；工具大输出优先用 `tool.output.externalized` 与 `memory_read target=output` 查 `memory/output` 引用，动态任务按 `scheduler.task.*` 判断 started/progress/done/timeout/cancelled/skipped。

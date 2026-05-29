# ProjectYing INDEX

本文件记录 `AItermux/projectying` 在 `2026-05-13` 的当前工程地图、仓储边界与下一阶段城市化目标。

- `INDEX.md` 是当前运行时地图；目录、源文件、存储边界变化后必须同步。
- `项目记忆.md` 记录历史变更、测试轮次和旧结构演进。
- `统一地基整改计划.md` 是工具、外导、UI、轮询与 Matrix 外部工具能力的执行记录；完成后把运行真源回写本文件。
- `城市化模块化整改计划.md` 是本轮 persona/UI/MCP/热重载城市化重构蓝图，目标是把内建 persona 抽成 `src/persona.rs`，并为 Matrix 的扩城治理与模块化热重载打地基。
- 旧 `结构核对计划.md` 仅作为历史归档，不再作为当前待办或固定蓝图真源。

## 0. 2026-05-13 当前城市图

- Matrix · 萤：作为前台主体、用户聊天、任务判断和 persona 协调者。Matrix 持有最小 `tool_manage` 入口，可按需打开折叠工具，但默认不展开执行型 schema；后台治理结果只以结论和风险回到前台。
- 司：可见后台治理 persona，负责上下文整理、datememory 写入和大回执压缩。旧 `src/advisor.rs`、Advisor request/submission/failure、Aidebug advisor stream 和建议板口径仍退役；当前司是新 persona，不复活旧审计重连链。
- Context：每个 persona 使用 `context/<Persona>/{prompt.txt,fastmemory.json,state.json}`，运行态上下文携带 stable entry id。维护请求投给司，司通过 `context_manage.persona=...` 管理目标 persona。
- ContextManage：唯一上下文治理工具，已支持明确 `persona` 作用域。司是常规治理者；Matrix 保留权限但默认 closed，只在明确需要时经 `tool_manage` 临时打开。Coding/Server 等普通 persona 保持固定投影，避免误整理其它上下文。
- 维护模式：Matrix、司、Coding、Server 的上下文阈值票据统一进入司队列。一次性维护 provider 请求只发送目标 persona snapshot 和维护说明，不注入司的标准 context；维护包已作为 focus-mode payload 提供，司不再为每条维护重复 `focus_mode.enter/exit`。
- DateMemory：统一落在 `memory/Matrix/datememory.db`，每条日记带 `source_persona`。`datememorycontext` 压缩由司处理，是隔离一次性 provider 请求，只发送目标 persona 的 datememorycontext + 管理说明，不附带普通 context/fastmemory/schema 细节；维护写入必须是带日期、关键词、事件脉络、结果和后续线索的详细日记，不接受一两句空泛摘要。
- Persona 协作：所有 persona 可通过 `persona_manage.send` 做受控部门联络，并记录 `source_persona`；Matrix 和司额外拥有 `observe/interrupt`，观察只返回折叠聊天轨迹、执行命令摘要和最终结果，不回传完整工具 output。Matrix/司向执行部门发送方向纠偏默认按 urgent 注入，普通 persona 也可在阻塞或用户中断纠偏时显式 `priority=urgent`。跨 persona 投递在 provider/context 里仍保持 `user/input_text` 语义，但聊天 UI 和 metamemory 采用来源身份与 `source_channel`：Matrix 显示 `Matrix/萤`，Aidebug inbox 显示 `Aidebug/开发者AI调试`，避免部门沟通被误看成用户发言。完整工具内容仍通过 `memory_check/memory_read/toolmemory` 按需精读。
- Provider：统一由 Matrix 的 API provider catalog 管理。司/Coding/Server 不再维护独立 API 密钥和 BaseUrl；旧 overlay 默认继承 Matrix 当前 provider，只有在对应 persona 设置页显式保存后才允许覆盖 provider 选择，model/reasoning 仍可作为轻量 overlay。
- 工具治理：工具、schema、UI display spec、动态角色和外部热重载工具分发都由 Matrix 主导，不再有独立工具管理角色。`tool_manage` 负责工具箱投影、reload、open/close/pin、外部工具 manifest 的 create/update/remove，以及动态角色的 `role_list/role_create/role_update/role_remove/role_tool_add/role_tool_remove`；新增或修改工具时，Matrix 用 `tool_manage` 维护 `tools/external/` manifest，用 `command/apply_patch` 维护必要脚本，再 `open/pin` 到目标 persona 或分配给动态角色并实测。prompt 改动统一走 `context_manage`，角色创建时的 prompt 只作为初始化/绑定入口。
- 动态角色：注册表位于 `context/roles.json`；角色目录位于 `context/<RoleContextDir>/`，包含 `prompt.txt / fastmemory.json / state.json`。角色 id 是稳定 ASCII 标识，`display_name/glyph` 用于 UI 标签。当前动态角色先通过 `base_persona` 执行：`persona_manage.observe` 返回角色折叠状态，`send/interrupt` 会路由到 base persona，在投递前把角色默认工具投影到 base persona 的 toolbox，并注入角色 prompt；后续可把 provider runtime 从静态 `PersonaKind` ledger 中抽出，升级为真正独立 agent。
- 外部工具：`tools/external/` 是 Matrix 管理的热重载外部工具区。每个工具由 `<name>.tool.json` 或 `<name>/tool.json` 描述，运行时按需重读；manifest 已支持 `parameters / display / output_policy / scheduler`，用于统一 provider schema、工具前端文案、大输出外导和轮询策略。`scheduler.deadline_ms` 会覆盖执行 deadline；无 scheduler 的旧工具继续使用 `timeout_secs`。外部工具默认 closed，不进入 provider schema，只有 Matrix 通过 `tool_manage` 给目标 persona 临时 open/pin/close。
- 图片工具：`view_image` 已支持截图/相片自动识别、目录取最新图和本地压缩；`draw_image` 会调用真实图片模型生成/编辑位图，并在工具完成前把最终图片保存到 `media/camera/`，不再生成 request JSON sidecar。Matrix/Coding/Server 默认展开 `view_image/draw_image`，重执行型工具仍按需打开。
- Token 纪律：上下文维护和 persona 协作必须避免循环重试和重复组包。运行态上下文治理以 KB 字节阈值为主，条数只作开放兼容；硬阈值触发时目标 persona 的 launch 会暂停，等待司完成 compact 后重新读取最新 context 再恢复发送。忙碌可延迟；配置错误、不可恢复错误、用户 ESC 打断后必须跳过本次任务并记录结构事件。

## 1. 源码主城

- `src/main.rs`（50357 行）
  - 首都：主循环、Provider、上下文、记忆、设置、聊天编排、Persona 切换、系统队列与 Aidebug 接入。
  - 性能重点：文件体量最大，聊天渲染缓存、上下文拼接、记忆写入、请求回流与事件日志都集中在这里；后续优化优先保持城市化分区，不做随意拆文件。
- `src/mcp.rs`（19592 行）
  - 工具城：schema 投影、工具执行、普通 command、PTY/Terminal、子代理、浏览器桥、`apply_patch`、输出预算与 toolmemory 归档。
  - 当前边界：普通 command / ADB / Termux API 不再文件外导；完成态进 `toolmemory`。工具输出内联硬上限为 128KB，超出后只返回摘要与引用读取提示。Terminal 和 multiagent 仍保留实时输出文件。`tools/external/` manifest 在这里热加载为外部工具 schema 与执行入口。
- `src/ui.rs`（1946 行）
  - 渲染城：布局、顶栏、聊天区、输入区、Terminal/Agent/Command 面板和命中区。
  - 当前边界：只做渲染与命中，不写运行真源。
- `src/coding.rs`（15 行）
  - Coding 域元信息与系统提示词测试锚点。
- `src/server.rs`（16 行）
  - Server 域元信息与 SSH/服务器管理提示词测试锚点。
- `Aidebug/aidebug.rs`（1382 行）
  - AI 调试口：目录协议、inbox 投递、状态快照、最新回复、单事件流和旧 `log/` 退役归档。
- `Aidebug/departmentrs.rs`（998 行）
  - 观测路由：请求状态机、部门分类、告警规则和 `stream=department|alert` 事件写入；旧 Advisor 审计部门已退役，当前司按普通 persona 出现在 persona catalog。

## 2. Aidebug 调试链路

- `Aidebug/`
  - `events.jsonl`：唯一 AI 调试事件流，按 `stream` 区分 `interface/request/tool/department/alert`；超过 8 MiB 自动裁剪并保留尾部约 4 MiB。
  - `status.json`：运行状态快照，由运行时生成，包含 `protocol_version=2`、persona 清单、当前 provider/model、context 布局、memory 布局、工具输出协议和调度协议摘要。
  - `latest_reply.txt`：最近一次 AI 正文回执，由运行时生成。
  - `inbox/`：外部调试投递入口，支持 `.txt / .md / .json`。
  - `processed/`：已消费投递。
  - `failed/`：失败投递及错误原因。
- AI 排障顺序：先看 `status.json` 的运行状态和协议摘要，再按 `stream`、`event`、`data.department` 过滤 `events.jsonl`，必要时向 `inbox/` 投递复现或维护任务。
- 事件写入纪律：`events.jsonl` 必须保持一行一条合法 JSON；多线程写入和滚动裁剪只能走 `Aidebug/aidebug.rs` 的统一写入口。
- 退役边界：`Aidebug/interface/`、`Aidebug/logs/`、分部门 `*.log` 已删除；不再作为当前运行真源。

## 3. 配置索引

- `config/matrix/`
  - `api.json / system.json / context.json / theme.json`
- `config/advisor.json`（运行时按需生成）
- `config/coding.json`
- `config/server.json`
- `config/tokeninfo.json`
- `config/lastgood/`
  - `matrix/{api,system,context,theme}.json`
  - `advisor.json / coding.json / server.json`

退役命名：`config/Matrix/`、`config/coding/`、`config/companioning/`、`config/server-ssh.json`、`config/runtime/` 只作为历史迁移口径，不是当前真源。Server SSH 设置并入 `config/server.json.server_ssh`。

当前 Provider 边界：Matrix provider catalog 是 API 真源；司/Coding/Server 只从 Matrix catalog 读取 BaseUrl/Key/provider 列表，并保留轻量 provider/model overlay。

## 4. 上下文索引

- `context/Matrix/`
  - `prompt.txt / fastmemory.json / state.json`
  - `state.json` 承载 `context / focus / meta / toolbox` 运行态；旧分散 JSON 只作为迁移来源，不是当前真源。
  - `schema/codex_tools.rsinc`：Matrix 共享本地工具 schema 源。
- `context/Advisor/`
  - `prompt.txt / fastmemory.json / state.json`
  - `state.json` 承载 `context / focus / meta / toolbox` 运行态。
- `context/Coding/`
  - `prompt.txt / fastmemory.json / state.json`
  - `state.json` 承载 `context / focus / meta / toolbox` 运行态。
- `context/Server/`
  - `prompt.txt / fastmemory.json / state.json`
  - `state.json` 承载 `context / focus / meta / toolbox` 运行态。

退役命名：`context/codex/`、`context/deepseek/`、`context/codexcoding`、`context/codexcompanion`、旧小写 `context/advisor`、`context/codex/companion/games` 不再作为当前真源。

## 5. 记忆与输出

- `memory/Matrix/`
  - `datememory.db`：全局长期日记库，条目用 `source_persona` 区分来源。
  - `datememorycontext.json / metamemory.db / toolmemory.db`
- `memory/Advisor/`
  - `datememorycontext.json / metamemory.db / toolmemory.db` 按需生成。
- `memory/Coding/`
  - `datememorycontext.json / metamemory.db / toolmemory.db` 按需生成。
- `memory/Server/`
  - `datememorycontext.json / metamemory.db / toolmemory.db` 按需生成。
- `memory/output/`
  - `terminal/`：PTY/Terminal 实时输出。
  - `terminal/adb/`：ADB 终端实时输出。
  - `terminal/termux-api/`：Termux API 终端实时输出。
  - `multiagent/`：子代理实时输出。
  - `message/`：手动消息导出目录，由运行时按需创建。

当前边界：普通 `command`、ADB 普通命令、Termux API 普通命令完成态进入当前 persona 的 `toolmemory`；不再创建 `memory/output/command*`。

`memory_read target=output` 是当前统一外导读取入口：只允许读取 `memory/output` 下文件，支持 `latest / tail / range / since_cursor / summary / full`，默认返回尾部或增量，默认 32 KiB、硬上限 128 KiB。PTY、Terminal 和 multiagent 的实时输出应优先通过 `memory_read target=output` 按需读取，不再用 entries 拉全量日志。每次读取会写入 `tool.output.read` Aidebug 事件，记录 mode、返回大小、截断状态和 cursor。旧外导读取工具已从执行入口、schema/projection 和 toolbox 状态中移除。PTY/Terminal 启动、自动观察、完成、超时和取消会写入 `scheduler.task.started/progress/done/timeout/cancelled`，维护跳过写入 `scheduler.task.skipped`；这些协议摘要同步出现在 `status.json` 和 `aidebug.layout_ready`。

## 5.1 外部热重载工具

- `tools/external/README.md`：外部工具 manifest 协议。
- `tools/external/<name>.tool.json`：单文件 manifest 入口。
- `tools/external/<name>/tool.json`：单工具文件夹入口，可配套脚本。

运行边界：外部工具接收 JSON stdin，stdout/stderr 作为工具回执；`working_dir` 和带 `/` 的程序路径必须位于项目根内。工具名不能与内建工具冲突。所有外部工具在各 persona toolbox 中默认 folded/closed，只有 Matrix 用 `tool_manage` 打开后才进入 provider schema。Matrix 新增工具时应通过 `tool_manage create/update` 同时填写 `parameters / display / output_policy / scheduler`，让参数 schema、工具卡片、Input/Output 文案、折叠摘要、外导预算和 deadline/retry 策略统一走 manifest；如填写 `scheduler.deadline_ms`，运行时会按该 deadline 超时并终止外部工具进程组，避免派生子进程拖住本轮调度。

## 6. 日志与生成物

- `log/`
  - 退役旧入口；新运行时不再写入。若启动时发现旧内容，会归档到 `memory/output/legacy-log-*` 后移除旧目录。
- `target/`
  - Cargo 构建产物。
- `.git/`
  - Git 元数据。
- `.codex_backup/`
  - 外部工具备份目录，不纳入运行真源。

## 7. 本轮审查结论

- Persona 边界：Matrix · 萤负责前台对话、主控协调、工具/schema/prompt 治理和外部工具分发；司负责后台上下文压缩、datememory 和大回执治理；Coding/Server 聚焦各自执行域。
- 工具管理边界：`tool_manage` 只对 Matrix 生效；toolbox prompt 逐项展示可见工具，closed 工具只显示一行摘要且不进入 provider schema，expanded/pinned 工具才进入 provider schema，参数详情由 provider schema 承载，避免 toolbox prompt 复制大段 schema。Matrix 默认展开 `tool_manage / memory / persona_manage / web_search / view_image / draw_image / subagent`，但 `context_manage / command / apply_patch / pty` 默认 closed；司默认展开 `context_manage / focus_mode / persona_manage / memory_add/check/read`；Coding 默认保留 `command / apply_patch / update_plan / focus_mode / persona_manage.send / view_image / draw_image / memory_check/read / pty_run / pty_wait`，`ask_user` 不进入 Coding 投影，`pty_input/list/kill` 仅由 Matrix 按需打开；Server 默认展开服务器执行、PTY、图片和记忆读取工具。
- 工具协议边界：`src/mcp.rs` 已提供 `ToolOutputEnvelope / ToolDisplaySpec / ToolOutputRef / ToolOutputPolicy / SchedulerPolicy / ReadPolicy` 基础协议。当前执行完成态会生成 Aidebug `tool.envelope.created` 事件；发生大输出归档时会生成 `tool.output.externalized` 事件；`memory_read target=output` 会生成 `tool.output.read`；外部工具、PTY/Terminal、`wait_session`、`wait_agent` 和维护跳过会生成 scheduler 事件。Aidebug `status.json` 暴露 `tool_output_protocol / scheduler_protocol`，让 AI 排障先理解“引用读取、不叠包、超时跳过”的统一规则。UI 处于兼容迁移阶段：通用工具卡片已读取 DisplaySpec 的标题、动作和 Input/Output 文案，专用工具卡片后续逐步迁入 display registry。
- Matrix 边界：Matrix 不承担后台上下文维护，但独占工具治理主控权限；默认保留轻量治理、记忆检索和 persona 协作工具，必要时可临时打开 `command/apply_patch/context_manage` 做小范围治理或工具/prompt/schema 维护，完成后通过 `tool_manage reload/list/open/pin/close` 收拢投影。
- 司边界：司承担后台 context/datememory/大回执治理；维护票据不再污染 Matrix 前台聊天。司可跨 persona 调用 `context_manage.persona=...`，并可 `observe/interrupt` 做治理纠偏。
- Persona 协作边界：Coding/Server 的 `persona_manage` provider schema 只暴露 `send`；Matrix/司 schema 暴露 `observe/send/interrupt`，避免普通 persona 误调用治理动作并减少 schema token。
- `update_plan` 边界：工具已支持 `decision / plan / blueprint` 三种模式；`decision` 不强制 focus gate，`plan/blueprint` 用于普通任务和结构性大任务。
- 编译健康：`cargo fmt --manifest-path Cargo.toml`、`cargo check --all-targets`、`cargo test --all-targets -q`（453 passed）和 `cargo build --release -q` 已通过，本轮未发现新的编译告警。
- Aidebug 实机健康：当前 Aidebug persona 清单为 Matrix、司、Coding、Server；旧独立工具管理入口的实机投递不再作为当前健康项。近期 Matrix/Coding/Server 自检均完成，无 `request.failed` 和 retry；Coding command 归档到 `toolmemory_entry_id=171`，Server status 未执行远程命令。
- 性能债务：`main.rs` 与 `mcp.rs` 仍是主要性能风险集中区；已确认 Aidebug 事件流此前无上限，本轮已加 8 MiB/4 MiB 滚动保留。
- 残余代码：代码层已无 `Aidebug/interface`、`Aidebug/logs`、`memory/output/command` 运行依赖；历史文档仍会保留旧路径作为变更记录，不作为当前真源。
- 功能冲突：普通 command 与 Terminal 输出边界已分离，command 归 toolmemory，Terminal/multiagent 保留实时文件，避免重复外导。
- Aidebug 适配：`status.json` 与 `aidebug.layout_ready` 统一暴露 v2 布局摘要、工具输出协议和调度协议；旧 README 会在启动时刷新，保留当前 persona 清单，不恢复旧 advisor stream/建议板入口。
- 请求启动边界：Matrix 启动问候发送前会重新从磁盘刷新当前 Matrix settings，避免启动态沿用旧 provider 选择；Aidebug status 也暴露当前 provider/model。
- Persona provider 选择边界：司/Coding/Server 的旧 API-only 配置若没有 `provider_selection_override`，运行时会继承 Matrix 当前 provider，避免 stale 选择盖过 Matrix；显式 override 后仍可选不同 provider/model。
- HTTP 错误边界：`402/404` 等不可恢复 HTTP 错误不再继续尝试其它鉴权变体；第三方 relay 的 `401/403` 可有限尝试其它鉴权头，官方 provider 的鉴权错误立即失败。
- 观测路由：`persona_manage / spawn_agent / send_input / wait_agent / list_agent / close_agent` 统一归 `department=agent`，wait 超时告警证据也指向 agent，不再误导到 tool。
- 司重建边界：当前 `司` 是普通可见 persona 和后台治理队列承接者；不恢复旧后台审计重连、审计包、submission 或 Advisor stream。Matrix 和其它 persona 的上下文维护进入司的 context-free 维护队列。
- 请求级重试：启动问候和系统回传的临时 system payload 现在只在单次 provider 请求里注入，不再 append 到持久 context；Codex `/responses` 若系统-only 组包导致 `input=[]`，会自动加入轻量 user 占位，避免中转 502。
- Codex replay 治理：Responses 工具轮会在下一 round 前压缩超大 `function_call_output` 和超大 function arguments；`view_image` 的 `input_image` 只允许一次视觉回放，后续 round 改为文本摘要，避免同一活跃请求内图片 payload 叠雪球。
- 记忆阈值：Datememory context 默认阈值降为 `128 KB`，设置页仍可直接输入自定义正整数 KB；底部状态栏按当前 persona 独立显示 `Per current/request_count · N session ↑↓ · A total ↑↓ · C entries/limit · M used/limitK`。
- Datememory 隔离边界：维护 provider messages 已断言不夹带标准 context、focuscontext 或 fastmemory，只发送 datememorycontext 和维护说明，执行者为司。
- 记忆路由：`memory_check/read/add` 支持 `persona` 作用域；省略时走当前 persona。Matrix/司可显式读写司/Coding/Server 的 metamemory/toolmemory/datememorycontext，长期日记统一写入 Matrix datememory 并标记来源。
- PTY 命名边界：设置页、系统提示和 MCP Terminal Progress 使用“PTY 自动观察/快照”，不再用“审计”描述 Terminal 长运行回传，避免与退役 Advisor 审计混淆。
- 取消边界：主界面普通 `Esc` 只取消当前活跃 persona 的请求，并只冷却/跳过该 persona 的维护 key；跨 persona 运行中的 Coding/Server/司 请求不被全局误杀。需要跨部门纠偏时走 `persona_manage.interrupt` 指定目标 persona。
- Aidebug 观测：`status.json` 暴露 `active_request_observation / active_request_tool_runs / active_request_thinking_chars / active_request_text_chars`，用于区分 connecting、thinking_without_tool、tooling、responding，避免长 thinking 被误判为卡死。
- Codex 请求格式：`/responses` 投影只发送 model/input/stream/instructions/tools/reasoning/service_tier 等 Codex 支持字段；Temperature 和 Max tokens 只用于非 Codex `chat/completions`，Codex 设置页不再展示这两项。
- 城市化结构：`departmentrs` 已外置到 `Aidebug/`，`src/` 不再放调试观测模块；Aidebug 是调试中枢，mcp 是工具城，main 是首都，ui 是渲染城。

## 8. 维护纪律

- 改目录结构或运行真源，必须同步 `INDEX.md`。
- 改实现细节、验证结果、迁移历史，必须同步 `项目记忆.md`。
- 推进工具/输出/调度整改时，以 `统一地基整改计划.md` 为阶段计划；推进 persona/UI/MCP/热重载城市化整改时，以 `城市化模块化整改计划.md` 为阶段计划；阶段完成后把实际结果同步回本文件。
- 历史文档中的旧路径只作追溯，不得反向驱动当前代码。

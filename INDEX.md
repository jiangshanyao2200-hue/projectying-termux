# ProjectYing 城市规划地图（INDEX）

本文件是 `projectying` 的工程地图，不记录开发历史；历史变更、轮次回顾、测试纪要统一写入 `项目记忆.md`。

维护规则：
- 改结构前先看本图，改结构后同步更新本图。
- 本图只写“当前生效结构、连接关系、存储边界、维护规则”。
- 凡是跨城改动，至少同步更新“连接关系”和“存储边界”两节。
- 行号只是当前快照，后续以函数名和模块名搜索为准。

---

## 1. 城市总览

- 首都：`src/main.rs:29`
  - 承担主循环、状态机、事件分发、上下文/记忆/设置/Provider 六大内嵌卫星城。
  - 当前体量最大：`42400` 行，是系统中枢。
- 工具城：`src/mcp.rs:24`
  - 承担 MCP schema、工具执行、背景命令、子代理、ApplyPatch / RewriteSection、PTY 运行时。
  - 当前体量第二：`13329` 行。
- 渲染城：`src/ui.rs:22`
  - 承担布局、面板高度、顶栏、聊天区、输入区、Terminal/Agent 面板绘制。
- 观测城：`src/departmentrs.rs:19`
  - 承担启动日志、请求生命周期日志、信息流审计、观测状态机，以及 `log/department/` 分部门日志治理、回归基线索引、事件反查索引与运行时告警流。

辅助城区：
- 配置区：`config/`
- 上下文区：`context/`
- 永久记忆区：`memory/`
- 运行日志区：`log/`
- 视觉入口区：`media/`
- 启动器：`run.sh`

---

## 2. 目录索引

- `Cargo.toml`
  - Rust crate 入口清单；二进制入口固定为 `src/main.rs`。
- `run.sh`
  - 启动器；负责源码变更检测与 `release` 构建。
- `src/main.rs`
  - 首都；主循环 + 内嵌业务卫星城。
  - 系统队列现已按 persona 批次取件；`TerminalDone / BackgroundDone` 会把同类待发项统一顺延到最新窗口，并在放行后把后续新消息自然切到下一批次，避免逐条回传 AI。
  - `datememory` 证据链已贯通到 `add / replace / check / search / read`；`Advisor` 提交器已升级为批量写入 `AdviceBoard / FastMemory`，并在 `revision` 冲突时自动重排下一轮审计。
- `src/mcp.rs`
  - 工具城；工具 schema、执行站、Patch 工务局、Terminal 港区。
- `src/ui.rs`
  - 渲染城；布局规划局与所有纯渲染规则。
- `src/coding.rs`
  - `Coding` 域副将；承接编程域元信息与系统提示词。
- `src/advisor.rs`
  - `Advisor` 颅内系统模块；承接独立 prompt、`managecontext` 动态包、审计 JSON 协议、结果解析与独立目录路径助手。
- `src/companion.rs`
  - `Companion` 域副将；承接陪伴域元信息、游戏路口状态与上下文目录骨架。
- `src/departmentrs.rs`
  - 观测城；日志与请求观测，现按 `system / request / provider / context / advisor / tool / agent / terminal / flow / ui / misc` 十一部门落盘，并对每个部门日志做 `5 MB` 上限、超限裁掉最旧 `3 MB`；启动时同步重建 `_index.txt / _triage.txt / _regression.txt / _eventmap.txt / _watchpoints.txt`，运行时持续写入 `_alerts.log`。
- `config/Matrix/`
  - Matrix 主配置仓；包含 `api.json / system.json / context.json / theme.json` 与对应 `*.lastgood.json`。
- `config/coding/`
  - Coding 独立配置仓；结构与 Matrix 同构，共享同一套设置框架。
- `config/companioning/`
  - Companioning 独立配置仓；结构与 Matrix 同构，供陪伴域与后续游戏路口独立生效。
- `context/codex/`
  - `codexprompt.txt / fastmemory.json / fastcontext.json / context.json / toolcontext.json / focuscontext.json / contextmeta.json / schema/codex_tools.rsinc`；当前对外统一使用 `toolcontext / task mode`，旧 `focuscontext / focus_*` 仅保留兼容。
- `context/codexcoding/`
  - `Coding` 编程副将的独立上下文仓；结构与 `context/codex/` 同构，但 prompt 与 schema 收口为纯编程工具集。
- `context/advisor/`
  - 颅内系统独立上下文仓；当前已落地 `codexprompt.txt / fastmemory.json / fastcontext.json / context.json / managecontext.json / contextmeta.json / schema/codex_tools.rsinc`，其中 `managecontext.json` 现由主线程在每次审计前动态重建。
- `context/codexcompanion/`
  - `Companion` 陪伴域的独立上下文仓；结构与 `context/codex/` 同构，并新增 `games/lobby|roulette|zhajinhua|wolfkill/` 独立玩家 prompt/context/notebook 骨架。
- `context/codex/schema/codex_tools.rsinc`
  - Codex 工具 schema 外置定义；与 `codexprompt.txt` 同区维护，不拼进上下文，只作为结构入口；并行读查已从显式工具收口为“同轮多个独立调用 + 运行时原生并行批处理”，文件编辑入口已收口为单一 `apply_patch`；记忆工具已补齐 `contextmemory`，`memory_add / memory_replace` 现支持 `evidence_entry_ids`；任务模式现新增独立 `task_mode` 工具，`context_manage.task_enter/task_exit` 仅保留兼容。
- `context/codexcoding/schema/codex_tools.rsinc`
  - `Coding` 专属 schema；当前暴露 `exec_command / apply_patch / request_user_input / update_plan / task_mode / context_manage`，多个彼此独立的读代码 / 搜索 / 轻量验证命令由运行时自动原生并行收口。
- `context/codexcompanion/schema/codex_tools.rsinc`
  - `Companion` 当前先沿用 Matrix 的完整工具 schema，已与主 schema 同步 `task_mode / contextmemory / adviceboard` 语义，后续再按游戏域收口。
- `memory/`
  - `datememorycontext.json` 过渡层 + `datememory.db / metamemory.db / contextmemory.db / historytools.jsonl`；工具历史主读取面已完全转向 `memory/historytools.jsonl`，按 `entry_id` 精读回放。
- `log/`
  - `commandoutput / terminaloutput / multiagentoutput / department / codex_stdout.txt / codex_stderr.txt` 等运行日志。
- `media/README.txt`
  - 本地图片/截图接入说明。
- `笔记.txt`
  - 当前测试反馈与本轮执行结果。
- `项目蓝图.md`
  - 目标结构蓝图；定义 persona、工具、目录与治理边界。`INDEX.md` 记录现状，本文件记录目标分工。
- `回归基线.md`
  - 核心回归链路清单；定义当前优先保稳的 6 条主链路、关键事件、证据来源与后续扩展方向。
- `ApplyPatch专项收敛计划.md`
  - `apply_patch` 工具链专项施工计划；用于本轮对账、分轮实施与验收。
- `调查模式与ToolMemory压缩计划.md`
  - 调查模式、ToolMemory 外导统一化、软标签上下文压缩、颅内系统被动审计与 `fastmemory` 治理计划稿。

### 2.1 新增内炉审计链

- `src/main.rs`
  - `App` 现新增颅内系统运行态：审计频率、内部请求、提案挂起与提交器写回都在主线程安全点处理。
  - Provider 新增“无工具 Responses 审计链”，供 `Advisor` 只读审计上下文，不再暴露 Matrix 工具集。
  - Context 设置页新增 `AdviceBoard` 上限与 `颅内频率` 两项；保存后会同步进上下文运行阈值。
- `项目记忆.md`
  - 历史变更、长期开发记忆。

---

## 3. 启动与主干道路

### 3.1 启动链

- `run.sh`
  - 入口：`run.sh`
  - 职责：检测 `Cargo.toml` / `Cargo.lock` / `src/*.rs` 是否新于 `target/release/projectying`，必要时自动 `cargo build --release -q`。
- CLI 预检：`src/main.rs:653`
  - `evaluate_cli_preflight(...)` 决定是否允许进入 TUI。
  - 非 TTY 直接拒绝，只允许 `--help / --version`。
- 主循环入口：`src/main.rs:5425`
  - `run_loop(...)` 是所有键盘、鼠标、tick、Provider/Terminal 事件的总入口。

### 3.2 用户消息链

- 输入 -> 发送：
  - `handle_key(...)`：`src/main.rs:6082`
  - `handle_mouse(...)`：`src/main.rs:5652`
  - `launch_provider_request(...)`：`src/main.rs:3861`
- 发送前处理：
  - 用户消息写入标准上下文
  - 注入待发送的维护建议
  - 从当前 persona 对应的上下文仓重新拼接 Provider 输入
- Persona 分流：
  - 当前已存在 `Matrix / Coding / Companion` 三域 persona。
  - `Matrix` 继续承担总控与全工具主线；`Coding` 只处理编程任务；`Companion` 当前先承接陪伴聊天壳、独立设置与游戏路口，后续逐步接入 AIgames。
- 模型回执汇流：
  - `apply_model_task_output(...)`：`src/main.rs:2943`
  - 将 thinking / display / tool / system / terminal done 回填进聊天区、上下文、记忆和状态栏

### 3.3 工具链

- Schema 暴露：`src/mcp.rs:926`
  - `codex_tools()`
- 回执描述：`src/mcp.rs:1910`
  - `describe_function_call(...)`
- 调度入口：`src/mcp.rs:2407`
  - `execute_function_call(...)`
- 执行分发：`src/mcp.rs:2187`
  - `dispatch_function_call(...)`
- 并行探索：
  - `src/main.rs` 的 provider 层会把同轮相邻、可安全并行的读查类工具批次原生并行执行，并在聊天区折叠成单张 `Explore` 卡；`multi_tool_use.parallel` 仅保留隐藏兼容层，避免旧上下文回放失效。
- 任务模式：
  - `task_mode` 现成为独立工具入口，负责 `enter / exit`；底层仍复用同一套 `toolcontext` 写盘与回收逻辑，`context_manage.task_enter/task_exit` 只保留兼容。
- 工具回执进入首都后，再由 `src/main.rs:2943` 汇总到聊天区和上下文。

### 3.4 PTY / Terminal 链

- 主工具入口：`src/mcp.rs:2982`
- Terminal 港区：`src/mcp.rs:8586`
- 首都接入点：
  - `apply_terminal_event(...)` 在 `App` 内编排
  - UI 面板由 `src/ui.rs:393`、`src/ui.rs:744`、`src/ui.rs:990`、`src/ui.rs:1042` 负责绘制
  - 当前 PTY 面板已按终端屏幕尺寸同步 `rows/cols`，并基于 `vt100` 屏幕 cell 渲染前景色、背景色、粗体、下划线、反显与光标位置；交互键桥会跟随 `application cursor mode`，终端粘贴会跟随 `bracketed paste mode`，当 TUI 自己开启鼠标追踪后，点击 / 拖拽 / 滚轮会按当前 `mouse protocol mode + encoding` 直接转发给 PTY，而不是继续走本地滚动预览。
  - 终端启动仍即时写一张 `SYSTEM · TERMINAL` 启动卡；终端完成会先即时写本地结束卡，但 AI 回传仍先进系统队列，等 10 秒静默窗口收口后再按单条/多条批次统一回传。

---

## 4. 首都地图：`src/main.rs`

### 4.1 中央法典区

- crate 说明与边界口径：`src/main.rs:1`
- 首都总图：`src/main.rs:29`
- 状态机 `status`：`src/main.rs:48`
  - 底栏状态条的唯一来源。
- 城市章程局 `charter`：`src/main.rs:238`
  - `ProviderConfig / ThemePreset / apiurl / OfficialProviderKind` 的共享契约源。

### 4.2 市政账本与主状态机

- `App` 主账本与运行态：`src/main.rs:1095`
  - 聚合聊天、输入、设置、Topbar、Terminal、Agent、上下文模式、系统请求队列等。
  - 现已新增 `active_persona + matrix_session + coding_session`，通过会话切换保留两套聊天态。
- Provider 回流编排：`src/main.rs:2943`
- 系统维护消息发送：`src/main.rs:3999`
- 历史消息导出 / 剪贴板 / 标题抽取：`src/main.rs:5035`

### 4.3 交通枢纽区

- 主循环：`src/main.rs:5425`
- 鼠标事件：`src/main.rs:5652`
- 键盘事件：`src/main.rs:6082`
- 顶栏 persona 标签：
  - `src/ui.rs` attention bar 第一行固定为 `Matrix / Coding` 标签页。
  - `PgUp/PgDn` 可把焦点切到标签页，`← → / Tab` 可切 persona，触控点击同样生效。

### 4.4 卫星城（内嵌模块）

- `chat`：`src/main.rs:6708`
  - 聊天消息模型、Segment、工具块、可见窗口、选择/展开/链式整合。
  - 工具块现同时支持普通工具卡、旧同类链式收敛，以及原生并行批次的 `Explore` 卡片；并行批次折叠时显示 `○ Search/Read/Run · brief`，展开时逐项展示 `INPUT/Output`；旧 `multi_tool_use.parallel` 仅作为兼容回放入口。
  - 任务模式 UI 已兼容 `context_manage` 与独立 `task_mode` 两条入口，对外统一渲染为 `Mission Start / Mission Over`。
  - 当前已接入消息级渲染缓存：旧消息不再每帧全量重渲染，聊天绘制/命中/定位共用缓存入口。
  - 左右切换不再依赖静态消息顺序；`RenderCache` 现在按真实渲染后的 `selection_ranges` 做前后移动，`Explore` 组头、系统卡和普通工具卡的可选中顺序与屏幕所见保持一致。
- `context`：`src/main.rs:13464`
  - 外置上下文仓、focus 模式、维护票据、Provider 输入拼接。
  - 当前已按 persona 分流到 `context/codex` 与 `context/codexcoding`，并通过线程本地 profile 选仓。
- `memory`：`src/main.rs:17932`
  - 永久记忆系统：`datememory / metamemory / contextmemory + historytools`；当前对模型主暴露链路固定为 `memory_check -> memory_read`，`memory_search` 仅保留执行层兼容。
- `input`：`src/main.rs:20141`
  - 输入框、IME 粘贴、防抖、placeholder。
- `messageline`：`src/main.rs:21636`
  - Markdown 渲染、消息队列、文本排版。
- `provider`：`src/main.rs:22930`
  - SSE/非 SSE 请求、Codex 工具轮、上下文归一。
  - 流式请求当前已拆成“单次尝试 + 最多 3 次无进展重试”；仅在没有有效吐字/工具事件且错误可重试时才回退重连。
- `settings`：`src/main.rs:25809`
  - 设置状态机、命中测试、配置读写与 API Test。

### 4.5 首都内的重要公共路口

- `launch_provider_request(...)`：`src/main.rs:3861`
- `run_loop(...)`：`src/main.rs:5425`
- `context::manage_request(...)`：`src/main.rs:16414`
- `load_settings_bundle(...)`：`src/main.rs:28868`

---

## 5. 工具城地图：`src/mcp.rs`

### 5.1 工具法典区

- 工具总图：`src/mcp.rs:24`
- MCP 协议结构：`src/mcp.rs:116`
- Agent / Background / FunctionCall / ExecutedFunctionCall 等核心协议体都在这里定义。

### 5.2 调度中心

- schema 暴露：`src/mcp.rs:926`
  - 当前已按 persona 分流：`Matrix` 走完整工具集，`Coding` 走收口后的编程工具集。
- 提取函数调用：`src/mcp.rs:1887`
- 描述函数调用：`src/mcp.rs:1910`
- 分发工厂：`src/mcp.rs:2187`
- 总执行入口：`src/mcp.rs:2407`
  - `task_mode` 在这里先转换成 `context_manage` 兼容参数，再走统一执行站。

### 5.3 执行站

- 执行总入口：`src/mcp.rs:2982`
- 关键工具：
  - `execute_exec_command(...)`：`src/mcp.rs:2865`
  - `execute_view_image(...)`：`src/mcp.rs:3084`
  - `execute_apply_patch(...)`：`src/mcp.rs:3213`
  - `execute_update_plan(...)`：`src/mcp.rs:3253`
  - `execute_task_mode(...)`：`src/mcp.rs:3871`
  - `execute_context_manage(...)`：`src/mcp.rs:3427`
  - `execute_memory_*`：`src/mcp.rs:3693` 起，覆盖 `memory_add / memory_replace / memory_check / memory_read`，并保留 `memory_search` 隐藏兼容层
  - `execute_spawn_agent(...)`：`src/mcp.rs:5026`
  - `execute_wait_agent(...)`：`src/mcp.rs:5207`
  - `execute_list_agent(...)`：`src/mcp.rs:5012`
  - `execute_resume_agent(...)`：`src/mcp.rs:5053`
  - `execute_close_agent(...)`：`src/mcp.rs:5106`
  - `execute_spawn_agents_on_csv(...)`：`src/mcp.rs:5185`

### 5.4 Patch 工务局

- 语法解析与落盘：`src/mcp.rs:6473`
- 文本展开工厂：`src/mcp.rs:7121`

### 5.5 Terminal 港区

- 港区总图：`src/mcp.rs:8586`
- 运行时协议区：`src/mcp.rs:8640`
- 会话注册中心：`src/mcp.rs:8957`
- 会话创建站：`src/mcp.rs:9026`
- 控制塔：`src/mcp.rs:9441`
- 交互桥：`src/mcp.rs:9672`
- 港务内勤：`src/mcp.rs:9726`

### 5.6 工具城对外接口

- 给首都的接口：
  - `install_request_user_input_sink(...)`：`src/mcp.rs:352`
  - `install_background_command_sink(...)`：`src/mcp.rs:358`
  - `list_agent_snapshots(...)`：`src/mcp.rs:365`
  - `set_agent_retention_limit(...)`：`src/mcp.rs:405`
  - `list_background_commands(...)`：`src/mcp.rs:476`
  - `snapshot_background_command(...)`：`src/mcp.rs:492`
  - `kill_background_command(...)`：`src/mcp.rs:507`
  - `prepare_command_output_dir(...)`：`src/mcp.rs:2454`
  - `prepare_multiagent_output_dir(...)`：`src/mcp.rs:2472`

---

## 6. 渲染城地图：`src/ui.rs`

### 6.1 规划局

- 城区总图：`src/ui.rs:22`
- 主题与布局协议：`src/ui.rs:56`
- 总装配线 `draw(...)`：`src/ui.rs:138`
- 布局唯一来源 `layout(...)`：`src/ui.rs:194`

### 6.2 面板与主屏街区

- Terminal 面板条例：`src/ui.rs:332`
- 公共设施：`src/ui.rs:348`
- 顶栏街区 / 主屏绘制：`src/ui.rs:608`
- Topbar 命中：`src/ui.rs:634`
- Top Panel 绘制：`src/ui.rs:744`
- Agent 面板：`src/ui.rs:990`
- 后台命令面板：`src/ui.rs:1042`
- 聊天区：`src/ui.rs:1155`
- 设置页：`src/ui.rs:1204`
- 调色板：`src/ui.rs:1217`
- 队列面板：`src/ui.rs:1255`

### 6.3 输入与设置出站口

- 输入顶部与状态区：`src/ui.rs:1448`
- 输入框正文：`src/ui.rs:1508`
- 设置输入框：`src/ui.rs:1578`
- 主题出站口：`src/ui.rs:1623`

### 6.4 渲染城边界规则

- `ui.rs` 只消费 `App` 的公开状态，不负责改业务。
- 命中测试与渲染共用 `layout(...)`，禁止在 `main.rs` 另写布局算法。

---

## 7. 观测城地图：`src/departmentrs.rs`

- 行政边界约束：
  - `department` 的部门定义、路由规则、巡检顺序、告警注册、回归索引、日志保留策略全部只在 `src/departmentrs.rs` 内定义。
  - `main.rs / mcp.rs` 只允许调用 `departmentrs::prepare_startup_log(...)`、`departmentrs::log_event(...)`、`departmentrs::ObserveStateMachine` 这类公开入口，不在别处再造一套部门规则。
- 城区总图：`src/departmentrs.rs:19`
- 观测状态机：`src/departmentrs.rs:70`
- 部门路由：`src/departmentrs.rs:95`
- 启动日志准备：`src/departmentrs.rs:405`
- 统一事件落盘：`src/departmentrs.rs:424`
- 观测状态机职责：
  - 请求发送
  - 首包到达
  - thinking/text 统计
  - 完成 / 失败写盘
- 分部门机构：
  - `system.log`：启动清理、行政目录、治理事件
  - `request.log`：请求发送 / 首包 / 流式 / 完成 / 失败
  - `provider.log`：provider responses 差异与回放修正
  - `context.log`：上下文维护票据、任务模式、记忆治理
  - `advisor.log`：颅内审计、提案提交、revision 重排
  - `tool.log`：工具调用与执行总线
  - `agent.log`：子代理派发、等待、回执与协同状态
  - `terminal.log`：PTY/TTY 生命周期、Done 延迟与终端回传
  - `flow.log`：AI ↔ API ↔ UI 的关键信息流审计
  - `ui.log`：布局、尺寸、滚动、拖拽、触控
  - `misc.log`：未归类兜底
- `log/department/_index.txt` 会在启动时重建，记录部门职责、事件前缀与保留预算，主要供 AI 维护使用。
- `log/department/_triage.txt` 会在启动时重建，记录“先看哪个部门、再追哪些证据”的症状排查顺序。
- `log/department/_regression.txt` 会在启动时重建，记录核心回归基线、证据日志与锚点测试，主要供 AI 维护使用。
- `log/department/_watchpoints.txt` 与 `log/department/_alerts.log` 共同组成运行告警层；前者定义值得优先关注的异常模式，后者记录本轮实际命中的告警。
- 默认巡检顺序：
  - 运行期：`request -> provider -> flow -> tool -> context -> advisor -> agent -> terminal -> ui -> system -> misc`
  - 启动期：`system -> context -> request -> provider -> flow -> tool -> advisor -> agent -> terminal -> ui -> misc`

观测城只做日志，不参与业务状态决策。

---

## 8. 城市连接关系

### 8.1 首都 -> 渲染城

- 首都提供 `App` 与 `TopLaneKind / FocusArea / Screen` 等状态。
- 渲染城通过 `draw(...)` 与 `layout(...)` 只读消费这些状态。
- 鼠标命中坐标也必须走 `ui::layout(...)` 反推，不能另算。

### 8.2 首都 -> 工具城

- 首都调用 `mcp::codex_tools()` 暴露工具。
- Provider 收到工具调用后，首都转入 `mcp::execute_function_call(...)`。
- 工具执行结果返回后，首都再将结果写回聊天区、上下文、记忆与状态栏。

### 8.3 工具城 -> 首都

- 通过 sink 把后台命令、用户选择、PTY 事件推回首都。
- 首都统一在 `on_tick()` 中 drain 这些异步事件。

### 8.4 首都 -> 上下文城

- 用户消息、助手消息、系统维护、工具重放都写入 `context`。
- Provider 每轮请求前，从 `context` 重新拼出模型输入。
- task 模式会切换主上下文轨道：`context` ⇄ `toolcontext`；`focuscontext` 仅保留兼容镜像。

### 8.5 首都 -> 永久记忆城

- 用户/助手正文写入 `metamemory`。
- 工具调用与回执归档写入 `memory/historytools.jsonl`，`memory_read(target=toolmemory)` 直接按 `entry_id` 读取该归档。
- `datememorycontext.json` 是待压缩的人类视角缓冲层。

### 8.6 设置城 -> 章程局 / Provider / 配置区

- 设置城复用城市章程局的 `apiurl` 和 Provider 类型定义。
- 设置页落盘后，运行时立即通过首都同步到 context/meta 和 mcp/agent 保留策略。

---

## 9. 仓储边界

### 9.1 `config/`

- 配置已按 persona 分仓：
  - `config/Matrix/`
  - `config/coding/`
  - `config/companioning/`
- 每个 persona 仓都固定包含：
  - `api.json`
  - `system.json`
  - `context.json`
  - `theme.json`
  - 对应 `*.lastgood.json`
- 本轮已收口：
  - 空文件恢复不再生成 `*.invalid-*` 空壳
  - 缺失时会定点重建
  - 有效手改 JSON 不会因格式不同被初始化阶段强改

### 9.2 `context/codex/`

- `codexprompt.txt`
- `fastmemory.json`
- `fastcontext.json`
- `context.json`
- `contextmeta.json`
- `toolcontext.json`
- `focuscontext.json`（兼容镜像）
- 标准 `context` 内建最近 3 轮 compact 保护区，不再单独落盘 `notebook.json / last3round.json`。

### 9.3 `memory/`

- `datememorycontext.json`
- `datememory.db`
- `metamemory.db`
- `contextmemory.db`
- 工具历史不再进独立 sqlite；统一归档到 `memory/historytools.jsonl`，由 `entry_id` 做精读定位。

### 9.4 `log/`

- `commandoutput/`
- `terminaloutput/`
- `multiagentoutput/`
- `department/`
- `codex_stdout.txt`
- `codex_stderr.txt`
- `department/_index.txt`
- `department/_triage.txt`
- `department/_regression.txt`
- `department/_eventmap.txt`
- `department/_watchpoints.txt`
- `department/_alerts.log`
- `department/system.log / request.log / provider.log / context.log / advisor.log / tool.log / agent.log / terminal.log / flow.log / ui.log / misc.log`
- `commandoutput/` 与 `terminaloutput/` 现按文件夹做滚动保留：每个输出家族最多保留 `30` 组外导，触顶后清掉最旧 `15` 组。
- `commandoutput/adboutput`、`commandoutput/termuxapioutput`、`terminaloutput/adboutput`、`terminaloutput/termuxapioutput` 启动时会自动补齐。
- `department/` 启动时整目录清空，只保留本轮最新分部门观测日志与回归基线索引；每个部门日志单文件上限 `5 MB`，超出时裁掉最旧 `3 MB`；旧 `testrs.log` 已退役。

---

## 10. 当前结构性判断

本轮校准结果：
- 未发现新的模块错位或明显边界反转；当前主要问题是地图锚点漂移，而不是代码城市结构失控。
- `INDEX.md` 之前最主要的失真点是行号老化、目录清单漏掉 `contextmeta.json`，以及日志仓新增 `codex_stdout.txt / codex_stderr.txt` 后未同步。
- 当前工程主干约 `59374` 行，其中 `src/main.rs` `42399` 行、`src/mcp.rs` `13329` 行、`src/ui.rs` `1987` 行、`src/departmentrs.rs` `1659` 行。

当前仍存在、但这轮不做结构拆分的压力：
- `src/main.rs` 仍是超大首都文件，承担过多内嵌卫星城。
- `src/mcp.rs` 仍同时承载工具城和 Terminal 港区。
- 项目根目录仍有 `.projectying-noninteractive.err/.out` 这类非交互启动残留文件；来源明确，但尚未迁移到 `log/`。
- `cargo clippy --tests` 这条线预计仍会以风格级告警为主，需要后续分批收口。

---

## 11. 后续改动纪律

- 改 `src/main.rs` 内嵌卫星城时，必须同步更新本图的“首都地图”和“连接关系”。
- 改 `src/mcp.rs` 的工具或 PTY 边界时，必须同步更新“工具城地图”和“仓储边界”。
- 改 `src/ui.rs` 布局时，必须同步更新“渲染城地图”，尤其是 `layout(...)` 的职责说明。
- 改 `config/ context/ memory/ log/` 目录结构时，必须同步更新“仓储边界”。
- 开发历史、测试纪要、轮次回顾一律写入 `项目记忆.md`，不要继续堆进本图。

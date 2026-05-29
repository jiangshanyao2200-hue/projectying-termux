# Coding 对标 Codex 能力推进计划

日期：2026-05-23

## 当前判断

Coding 不能只靠 prompt 变成 Codex。Codex 强在“模型 + harness”：工具投影、上下文回放、补丁编辑、命令执行、测试验证、失败恢复、状态观测和长期任务纪律。ProjectYing 要让 Coding 对标甚至在本项目内超过 Codex，核心不是堆话术，而是把 Coding 做成可验证的工程执行系统。

今晚执行原则：只做低风险、可回滚、可记录的推进；不删文件，不做大架构迁移，不做需要用户清醒拍板的产品决策。

## 目标定义

### P0：不空壳

Coding 必须能独立完成一个工程闭环：

1. 读代码和项目记忆。
2. 拆计划。
3. 修改文件。
4. 构建/测试。
5. 安装或运行验证。
6. 观察 Aidebug/status/log。
7. 发现问题后继续修。
8. 写出可复查的结论。

### P1：对标 Codex

Coding 的每次任务都应该具备 Codex 类似的 agent harness 纪律：

- 文件读查优先 `rg` / 精读，不凭记忆改。
- 手工编辑统一走补丁，不用粗暴覆盖。
- 工具输出外导后能按需回读，不把大输出塞爆上下文。
- 对编译、安装、运行、UI 现象做真实验证。
- 失败时分类处理：编译错误、安装错误、权限错误、网络错误、provider 错误、UI 缓存错误分别走不同路径。

### P2：在 ProjectYing 内超过通用 Codex

Coding 应该利用 ProjectYing 专有资产获得优势：

- 主动读 `项目记忆.md`、`INDEX.md`、Aidebug 计划文档。
- 主动观测 `Aidebug/status.json`、`health.json`、`performance.json`、`events.jsonl`。
- 知道 Matrix / 司 / Coding / Server 的职责边界。
- 知道本项目历史雷区：active replay 不回落、status 与真实 provider body 不一致、tool preview 残影、Termux 私有目录安装失败、APK launcher icon 需要 mipmap/adaptive icon。

## 当前硬缺口

### 1. active replay 不受 focus/context manage 充分治理

证据：`live-coding-agentbrowser-tweb-20260522` 中 Coding 进入/退出 `focus_mode`，但同一条多轮 Responses 请求体从约 `124KB` 增到 `169.7KB`，未因 focus/context manage 明显回落。

风险：status 显示已治理，但 provider 实际输入面仍在膨胀，用户会误判上下文已经被压缩。

要求：

- 区分三类数值：持久 context、当前 active provider body、当前 Responses replay。
- `focus_mode.exit` 后如果继续同一任务，需要让后续 provider body 真实下降，或明确显示“active replay 未重组”。
- reasoning / tool call / tool output / function arguments 只要进入 provider replay，就必须能被统计和折叠。

### 2. status / health 偏乐观

证据：本轮 `datememory_context_kb=181/200`，pressure 约 90%，但 `health.json` 仍整体 PASS 100。

风险：接近阈值时没有 WARN，用户只看到“健康”，但系统已在高水位。

要求：

- 高于 80% 给 WARN。
- 超过阈值给 BLOCKED 或明确 maintenance pending。
- status、health、performance 使用同一份压力计算源。

### 3. Coding 完成标准不够硬

证据：APK 任务中 Coding 完成了构建和签名验证，但没有安装；图标只做了 drawable shape，实机 launcher 仍显示方块/空图标。

要求：

- APK/UI 任务默认要包含安装或明确说明未安装。
- launcher icon 必须检查 `application-icon-*` 是否指向 `mipmap` / adaptive icon。
- 能用 adb 时执行 `adb install -r`、`monkey` 启动、必要时截图/日志。

### 4. 工具和上下文纪律仍依赖模型自觉

风险：长任务中模型可能忘记 context manage，或只做表面 summary。

要求：

- tool projection / prompt 底部加入轻提示：当前上下文由轮询注入，请先判断是否需要整理上下文。
- 对长工具链提供 runtime 级提醒，不只靠 prompt。
- 工具输出超限必须自动 externalize，并给模型低成本索引。

## 执行路线

### Phase A：先把观测做准

1. 在 Aidebug/provider phase 中稳定记录：
   - `provider_body_chars`
   - `provider_body_tokens_estimated`
   - `responses_replay_chars`
   - `tool_schema_chars`
   - `persistent_context_chars`
2. status 增加或确认字段：
   - `active_provider_body_kb`
   - `active_replay_kb`
   - `context_kb`
   - `datememory_pressure_percent`
3. health 对 80% 以上压力给 WARN。

验收：live 长任务里，用户能看出到底是 context 文件大、schema 大、replay 大，还是 datememory 大。

### Phase B：修 active replay 真实回落

1. 找出 Responses 多轮 replay 的组包源。
2. 确认 reasoning、tool call output、function arguments 进入 replay 的路径。
3. 在 focus/context manage 成功后，对后续 round 触发更强 replay folding 或新请求重组。
4. 增加回归：构造大 reasoning + 大 tool output，fold 后下一轮 provider body 必须明显下降。

验收：一次 focus/compact 后，下一轮 `responses.round.start.body_chars` 至少下降到可解释范围；如果不能下降，status 必须明确显示 active replay 未重组。

### Phase C：强化 Coding 工程闭环

1. 更新 Coding prompt：
   - APK/UI 任务必须 build + install/launch 或说明阻塞。
   - 图标任务必须用 mipmap/adaptive icon，不只改 drawable shape。
   - 完成前必须跑最小验证命令。
2. 更新工具提示：
   - adb 可用时优先真实安装验证。
   - Termux `pm install` 不能直接读取私有目录，优先 adb install。
3. 增加 Aidebug live benchmark：
   - Rust warning 修复。
   - APK 图标/名称/安装验证。
   - 长工具输出外导和回读。
   - focus 后 provider body 回落。
   - 网络失败重试。

验收：Coding 不能只说“已构建”，需要给出运行验证证据。

### Phase D：建立 Coding 能力评分

每次 live Coding 任务记录：

- 是否完成代码修改。
- 是否构建/测试。
- 是否运行/安装。
- 是否发现实机问题。
- 是否主动修复二阶问题。
- provider body 峰值。
- focus/context manage 是否真实降体积。
- request/tool 是否失败。
- 是否写入项目记忆。

目标不是让 Coding 主观“像 Codex”，而是让它在这些指标上稳定达标。

## 下一步

清醒后优先执行 Phase A，不先大改 prompt。原因：如果观测不准，prompt 再强也会掩盖 active replay 和 status 错位问题。

建议第一刀：

1. 搜索 provider body / replay compact 的代码入口。
2. 把 `responses.round.start.body_chars`、status 的 `current_round_input_tokens`、`active_context_kb` 建立同源对账。
3. 加一个最小回归，证明 focus/compact 后 provider body 是否应该回落。

## 2026-05-23 只读源码定位

已确认几个关键入口：

- `src/main.rs::refresh_live_input_estimate_from_body_chars()`：当前 status 的 `current_round_input_tokens` / `active_context_kb` 会在 provider phase 携带 body 时，用整包 body chars 估算刷新。
- `src/main.rs::provider_phase_carries_request_body()`：目前只接受 `http.post.start` 和 `responses.round.start` 两类 phase 刷新 live input 估算。
- `src/main.rs::chat_completion_codex_with_tools_single_attempt()`：Responses 多轮工具链使用 `current_input` 递增 replay，每轮先 `compact_codex_replay_text_items()`，再组 `build_codex_tool_request_body()` 并 emit `responses.round.start`。
- `src/main.rs::compact_codex_replay_text_items()`：目前只压 `function_call_output`，而 `function_call.arguments` 只在 context replay item 构造时压；普通 `input_text`、累积 reasoning/assistant text、其它 replay 项还没有统一折叠策略。
- `Aidebug/aidebug.rs::build_health_chains()`：context 80% 会 Degraded，但 datememory 只有 over limit 才 Degraded；所以 `181/200KB` 这种 90% 高水位仍 PASS，和用户观察一致。

下一轮最小安全补丁建议：

1. 先给 datememory buffer 高水位加 WARN/Degraded 规则和测试，改动小、收益明确。已完成：`datememory_pressure_percent >= 80` 时 `memory.datememory.buffer` 和 `token.budget` 会进入 Degraded，并输出 `datememory_high_pressure=true` 证据。
2. 再给 provider phase 增加更细字段：`body_chars` 之外补 `replay_chars`、`tool_schema_chars`、`persistent_context_chars`。先观测，不改变行为。
3. 最后处理 active replay：不要直接粗暴清 `current_input`，先加回归证明 `focus_mode.exit` 后如果继续同任务，下一轮 body 应该下降到什么范围。

## 2026-05-23 新增 live 观察：Per 请求次数不累计

用户观察到 Coding 已经来回发送多轮上下文，但底部仍显示类似 `Per 4.1W/1`，其中 `/1` 没有随多轮 provider 请求增加。

初步定位：

- `src/main.rs::record_usage_input()` 只在高层模型请求开始时增加 `usage.request_count`。
- Codex Responses 工具链后续每个 `responses.round.start` / `http.post.start` 只通过 `refresh_live_input_estimate_from_body_chars()` 刷新当前 token 和 KB，不增加 request count。
- 因此 `/1` 目前更像“本 persona 的高层任务次数”，不是“本次启动本角色实际 provider POST / Responses round 次数”。

目标口径：

- status 的 `Per 4.1W/N` 中 `N` 应表示本次启动该 persona 实际发送给 provider 的输入轮次数。
- Codex Responses 多工具轮每个 round 只计一次，不能同时被 `responses.round.start` 和 `http.post.start` 双算。
- 普通非 Codex 请求仍保持一次请求计一次。

建议修复：

1. 增加 active request 内部已计数的 provider round key，按 `(request_id, phase attempt/round)` 去重。
2. `responses.round.start` 优先作为 Codex round 计数点；普通 provider 用 `http.post.start` 计数。
3. status / `persona_usage` / `current_round_input_count` 均读取同一 `usage.request_count`，避免 UI 和 Aidebug 分叉。

本轮验证：

- `cargo fmt --check --manifest-path Cargo.toml`
- `cargo test --quiet --manifest-path Cargo.toml datememory_health_warns_before_fixed_limit`
- `cargo test --quiet --manifest-path Cargo.toml datememory_health`
- `cargo test --quiet --manifest-path Cargo.toml derive_health_snapshot_scores_token_budget_by_pressure`
- `cargo check --all-targets --manifest-path Cargo.toml`

注意：中途并行跑两个 cargo 命令触发一次 target 构建目录竞争，随后已串行复测通过。

## 2026-05-23 status request_count 口径修正已落地

- `src/main.rs` 已把 `Per .../N` 的分母从“高层 launch 次数”推进为“launch 首轮 + 后续 provider attempt/round 次数”。首轮仍由 `record_usage_input()` 计一次，后续 `responses.round.start` / `http.post.start` 的 `attempt > 1` 才额外计数。
- 同一 active request 内新增已计数 attempt 集合，按 attempt 去重；Codex 的 `responses.round.start` 和内部 `http.post.start` 同一轮不会双算，非 Codex 同一 attempt 的多 auth/url 探测也不会双算。
- `status_resource_line()`、`persona_usage.request_count`、`current_round_input_count` 继续读同一份 usage ledger，避免 UI 和 Aidebug 分叉。
- 新增回归覆盖：首轮 body 只刷新 token 不额外增计数；attempt 2 的 `responses.round.start` 与 `http.post.start` 只计一次；attempt 3 后 status 可显示 `/3`。
- 验证通过：`cargo fmt --manifest-path Cargo.toml`、`cargo test --quiet --manifest-path Cargo.toml provider_attempt_round_count_dedupes_same_attempt_phases`、`cargo test --quiet --manifest-path Cargo.toml provider_body_phase_refreshes_live_per_without_session_accumulation`、`cargo test --quiet --manifest-path Cargo.toml bottom_status_resource_line_reports_usage_and_pressure`、`cargo test --quiet --manifest-path Cargo.toml aidebug_status_surfaces_background_persona_active_request`、`cargo check --all-targets --manifest-path Cargo.toml`。

## 2026-05-23 计数链路规范化收口

- 进一步把这次计数逻辑收拢为更稳定的 helper 结构：
  - `record_launch_usage_input()` 只负责首轮 launch 的 token/计数写入；
  - `record_provider_attempt_round()` 只负责后续 attempt 的去重计数；
  - `bump_current_round_request_count()` 统一两处的计数增量；
  - `clear_provider_round_tracking()` 只做 attempt 去重集清理。
- 这次不改口径，只做命名和职责收口，目的是让后续 status / Aidebug / 计数回归都只认一条清晰路径。

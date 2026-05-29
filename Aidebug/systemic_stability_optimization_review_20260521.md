# ProjectYing 复杂机制链路系统性审查与优化计划

日期：2026-05-21

## 审查目标

这轮只做系统性审查和可执行计划，不直接改机制代码。

目标是检查 ProjectYing 当前几个复杂链路是否足够稳定，是否能在多种可选治理路径里选到更优动作，尤其关注上下文治理、token 计量、datememory 入库、工具投影、动态角色、系统调度和 Aidebug 健康观测之间是否互相打架。

## 审查范围

- `src/main.rs`：状态栏、token 账本、provider 请求启动、系统维护队列、context/datememory 维护调度、provider reasoning 捕获。
- `src/mcp.rs`：工具 schema、工具执行、`context_manage / context_summary / context_vision / context_compact`、`tool_manage context_governance_set`、`persona_manage`、工具输出 envelope 和外部化。
- `src/roles.rs`：动态角色 contract、context governance normalize、治理模式到工具授权的同步。
- `Aidebug/aidebug.rs`：`status.json / health.json / performance.json` 的上下文、token、datememory、scheduler 健康判断。
- 既有计划：`Aidebug/context_token_governance_plan_20260520.md`、`Aidebug/health_monitor_plan.md`、`Aidebug/context_governance_tools_plan.md`。

## 代码证据索引

- `src/main.rs:2294`：`status_resource_line` 已显示 `Per / N / Ctx / Date`，`Ctx` 百分比按 soft，分母按 hard。
- `src/main.rs:2206`、`src/main.rs:8235`：请求 token 仍主要用 `chars / 4` 估算，schema chars 已计入。
- `src/main.rs:4698`：tick 内会读取 context stats 并更新 active context 与 token 估算。
- `src/main.rs:32626`：context 维护票生成，`advisor_managed` soft 触发，`summary_compact / vision_compact` 仅 hard 触发。
- `src/main.rs:37270`、`src/main.rs:37355`：datememory 维护提示和聚合触发逻辑。
- `src/main.rs:4436`、`src/main.rs:8057`：hard context maintenance 会阻断目标 persona 正常请求。
- `src/main.rs:31053`：provider 注入包含 toolbox、system prompt、context state、shared board、fastmemory、focus mode。
- `src/main.rs:46886`、`src/main.rs:47332`：reasoning / thinking 会被捕获并进入 `ChatCompletion.thinking`。
- `src/mcp.rs:1748`、`src/mcp.rs:1835`、`src/mcp.rs:1876`、`src/mcp.rs:1928`：四个上下文治理工具 schema 已统一追加主动治理提示。
- `src/mcp.rs:9954`：`tool_manage context_governance_set` 统一治理入口，Matrix-only。
- `src/mcp.rs:11919`：`persona_manage` 队列发送和 observe 权限。
- `src/roles.rs:86`、`src/roles.rs:90`：动态角色默认 `800KB / 1000KB`。
- `src/roles.rs:319`：动态角色治理阈值 normalize 仍允许 `32..4096KB`，和当前 1M hard cap 方向有潜在漂移。
- `Aidebug/aidebug.rs`：health token warn/hard 已收敛为 `16w / 20w`；session 累计只保留为证据，不再决定 token.budget 状态。

## 当前稳定点

1. 上下文治理工具链已经分层。

`context_manage` 是精细治理，`context_summary` 是低回执折叠，`context_vision` 是可见性控制，`context_compact` 是粗压缩。schema 和工具 preview 已有统一的主动治理提示，模型有机会在每次看到轮询上下文时主动判断是否整理。

2. 默认阈值已经接近目标方向。

运行设置默认 `context_max_size_kb = 800`，hard cap 为 `1000`；动态角色默认也是 `800 / 1000`。状态栏 `Ctx` 分母已经按 hard cap 展示，`Date` 固定 `0.2M`。

3. 自管路线和司托管路线已经分开。

`advisor_managed` 可在 soft 触发后交给司做精细治理；`summary_compact / vision_compact` 不会被 soft 阈值频繁打断，只在 hard 阈值进入兜底。这个方向符合“不到触底由角色自助管理，触底交给司”的目标。

4. 维护轮已经尽量隔离上下文。

context maintenance、self compact maintenance、datememory maintenance 都有独立 provider messages，避免把司自己的完整 context、focuscontext、fastmemory 一起塞进维护请求。

5. datememory 已有聚合兜底。

单 scope 超限会触发维护；如果总量超过 200KB 但单 scope 都没超，也会强制扫描非空 scope，避免“总量爆了但没有维护票”的假健康。

6. 工具输出已经有外部化协议。

tool envelope 会生成 `output_refs`，Aidebug 会记录 `tool.output.externalized`，persona observe 不暴露 raw tool receipts，长期精读走 `memory_read target=output` 或 toolmemory。

## 2026-05-21 实施进展

- `status` 的 `Per` 已切到当前活跃请求的实时估算，不再显示 `session_input_tokens` 作为主值。
- Codex 多轮 `responses.round.start` / `http.post.start` 会刷新当前 body 估算，避免一次请求里 body 变长但 status 不回落或不抬升。
- `Ctx` 改为按实际 request surface 估算，不再只看 `message.content`，并把 `codex_item_json` replay 和工具 schema 纳入统计口径。
- `Aidebug health` 的 token 链已改成只看当前轮和当前 context pressure，session 累计只保留为证据，不再直接决定 BLOCKED。

## 主要风险

### Live 补充：Server 17w session token 与 0.1M context 的错位

2026-05-21 live 排查到一组直接证据：

- `Aidebug/status.json` 当前 active persona 为 `Server`，`session_input_tokens=173693`，但 `current_round_input_tokens=15373`，`active_context_kb=70`。
- 这说明状态栏/health 里的 `17w` 是 Server 本次启动累计输入，不是单次请求 active context。
- Server `context/Server/state.json` 当前 15 条 context entry，文本约 `9966 chars`，thinking 约 `3918 chars`，但 `codex_item_json` 约 `42591 chars`。
- Codex provider 组包会优先把 `codex_item_json` 作为 Responses input replay，而当前 `message_chars` / `active_context_kb` 主要统计 `message.content`，所以真实 body 比状态栏估算更大。
- `Server#13` 的 `request.sent` 估算为 `18043 input tokens`，但 Responses 请求在同一轮内发了 11 次 POST，body chars 分别从 `168519` 增长到 `230393`；合计 `2161421 body chars`，按当前粗略 `chars/4` 就是约 `54w` token 级别的在途输入成本。
- `Server#15` 在司整理后单次 POST body 仍有 `88716 chars`，而状态估算只有 `15044 input tokens`，仍然存在 body 口径低估。
- 司确实在 `1779340052` 对 Server 执行了 `context_manage target=context persona=server scope=all`，把旧 e1..e91 收成 e92；但随后 Server r15/r16 又追加工具调用 replay、工具输出 replay、assistant thinking 和长正文，context 很快重新长回去。
- 当前 Server state 里的实际 context governance 是 `manage_threshold_kb=200 / compact_threshold_kb=200`，当时维护票也写了 `217KB / 200KB hard`；但 Aidebug health/status 显示的是 settings 口径 `800/1000KB`。这说明 status 展示阈值和实际角色上下文维护阈值可能不同源。

结论：

- 这不是单纯 “100KB context 文件怎么等于 17w token” 的问题；`17w` 是累计账本，单轮真实 body 又被低估。
- 真正的问题是三层叠加：session 累计值容易误读、Codex replay JSON 没进入预估、Responses tool round 多次 POST 的在途消耗没有进入账本。
- `context_manage` 不是完全没生效，但缺少后续复测和持续压缩策略；长工具链一轮就能把刚整理好的 Server 上下文重新堆起来。
- health 当前 PASS 不可靠，因为它没有同时看 `actual_request_body_chars`、`codex_item_json_chars`、`tool_round_post_count` 和实际 role governance 阈值。

### P0. Token 口径仍不是最终真源

当前请求前 token 主要来自 `chars / 4`，虽然已经把 message chars 和 local tool schema chars 算进去，但这不是模型感知 token。`Ctx` 的 KB/M 展示和 token 预算仍可能在中英文、JSON、schema、reasoning 混合时偏差很大。

风险：

- 20w input token 强制 compact 可能触发不准。
- 300KB JSON 到底是 2w token 还是更多，仍依赖估算。
- reasoning 已进入 `ChatCompletion.thinking`，但“压缩 thinking 后 token 是否真实下降”缺少专门验收。
- health 里 `PERF_INPUT_HARD_TOKENS = 250000`，和用户目标“20w 封顶”不完全一致。

计划：

- 新增统一 `TokenBudgetEstimator`，按 provider/model 选择 tokenizer；缺失 tokenizer 时显式标注 `estimate_method=chars_div_4`。
- 请求前生成 `TokenBudgetSnapshot`：`prompt_tokens_estimated`、`tool_schema_tokens_estimated`、`context_tokens_estimated`、`fastmemory_tokens_estimated`、`reasoning_tokens_estimated`、`preflight_input_tokens`。
- 请求后优先用 provider usage 回填真实 `input_tokens / output_tokens`，并记录 `usage_source=provider|estimator|fallback`。
- 把 `20w input token hard cap` 做成统一配置项，默认 `200000`，Aidebug、status、请求启动前检查共用同一常量。
- health 不再用 `session_input_tokens >= hard * 4` 直接当 BLOCKED，改成单独的 session pressure 证据，避免一次长会话误判当前轮。
- Codex Responses 预估必须按实际 request body 口径计算，至少包含 `instructions`、`input`、`tools`、`reasoning`、`codex_item_json` replay，而不是只看 `ApiMessage.content`。

### P0. Responses tool round 在途成本没有入账

当前 `record_usage_input` 在请求启动时只记一次预估；但 Codex Responses 里一次用户请求可能拆成多轮 POST：模型发工具调用，运行工具，再把 function_call/function_call_output replay 追加后重新 POST。`Server#13` 实测 11 次 POST 合计 `2161421 body chars`，但账本只显示启动时估算 `18043 token`。

风险：

- 模型在一次长工具链里真实消耗可能远超 status 的 `N ↑...`。
- `current_round_input_tokens` 低估导致 20w hard cap 不会触发。
- replay compact 只压了一部分 text item，function_call_output 的完整 output 仍可能通过 `codex_item_json` 回流。
- 大工具输出虽然 externalized 到 toolmemory，但 replay JSON 仍带较大的 output 文本，造成 “外导了但还是被发回模型”。

计划：

- 在每个 `http.post.start` / `responses.round.start` 记录 `actual_body_chars` 和 `actual_body_tokens_estimated`。
- `N ↑` 区分 `preflight` 与 `inflight_total`：例如 `N ↑1.5w/54w`，前者是启动前，后者是本次请求累计 POST body。
- request hard cap 要看 `max_single_post_tokens` 和 `sum_inflight_post_tokens` 两个值；任一超限都触发 compact/中断策略。
- function_call_output replay 进入下一轮前必须按工具输出策略裁剪，优先只保留 `toolmemory ref_id + 摘要 + exit_code`，不把完整 output 重放。
- `responses.replay.compacted` 要输出压缩前后各类项目的数量和字符：`message_text / function_call / function_call_output / reasoning`。

### P0. hard cap 语义还有漂移口

默认值已经是 `800 / 1000KB`，但角色 normalize 仍允许到 `4096KB`；context sync 对非默认 custom soft 在没有 compact threshold 时会回退 `soft * 2`。这和“hard 默认统一 1M，0.8M 触发并留缓冲”的目标不完全一致。

风险：

- Matrix 新建角色如果传了 `0.8M` soft 但漏传 hard，某些路径可能变成 `1.6M` hard。
- 角色 registry、settings、runtime threshold、health status 对同一概念使用不同字段名和边界。
- `context_governance_set` 现在仍输出 `manage_threshold_kb / compact_threshold_kb`，文档目标是向 M/token 兼容字段迁移。
- live 里 Server 实际 state 是 `200/200KB`，但 health/status 展示 `800/1000KB`，说明“实际维护阈值”和“状态展示阈值”不同源。

计划：

- 将 `ContextGovernanceSpec` 语义改成：`soft_trigger_kb` 默认 800，可调建议 200..800；`hard_cap_kb` 默认 1000，普通 UI 不暴露。
- `compact_threshold_kb` 兼容旧字段，但默认回填必须固定到 1000，不再对 custom soft 默认 `soft * 2`。
- `normalize_context_governance` 默认 clamp 收敛到 `soft 200..800`、`hard 800..1000`，除非显式开启高级/实验参数。
- `tool_manage context_governance_set` 增加 `context_soft_limit`、`context_hard_limit`、`input_token_hard_cap` 输出，旧 KB 字段保留兼容一轮。
- Aidebug status 同时输出 `context_soft_limit_kb`、`context_hard_cap_kb`、`context_soft_percent`、`context_hard_percent`，减少 “32% 0.1/0.2M” 这种误读。
- status/health 的 context 阈值必须从当前 active persona/role 的实际 `ContextMeta + ContextGovernanceSpec` 读取，settings 只能作为默认配置来源，不能覆盖实际 state 展示。

### P0. reasoning/thinking 必须能被上下文管理真实压缩

provider 层会捕获 `response.reasoning*`、`<think>`，并归一到 `ChatCompletion.thinking`。这说明 reasoning 不是纯 UI 概念，它可能进入上下文条目，成为 active context 的一部分。

风险：

- `context_summary / context_compact / context_manage` 如果只压 display/text，不处理 thinking，就会出现“前端折叠了，但 AI active context 没降”的问题。
- 维护 snapshot 有 thinking preview 测试，但还需要验证压缩动作会移除或替换 thinking 装载。
- UI 展开/折叠链路和 AI provider context 链路必须分开，不能因为管理 AI context 而破坏用户前端查看历史。

计划：

- 明确 `ContextEntry` 的 AI 装载字段：`text + thinking + codex_item_json` 哪些会进入 provider，哪些只用于 UI。
- `context_summary fold`、`context_compact fast/old/all`、`context_manage summary` 都必须能把目标条目的 thinking 处理掉或替换为摘要。
- 新增回归：构造含大 thinking 的 context，执行 fold/compact 后，`active_context_kb` 和 `preflight_input_tokens` 必须明显下降。
- UI 层保留原始消息展开能力时，单独存 UI history 或 historical evidence，不把原始 thinking 继续注入 provider。

### P0. 硬触底后的暂停与恢复需要单一真源

现在 hard context maintenance 会阻断目标 persona，系统请求也有 pending、active、recent cooldown、batch window。机制已经存在，但判断分散。

风险：

- 触底后维护完成但 context 未下降，可能进入 recent cooldown 导致短时间不再重试。
- datememory 聚合触发会把多个小 scope 都放进维护候选，可能制造批量噪音。
- 如果 active request、pending request、dynamic role runtime 之间 key 不一致，status 会出现 pending=false 但实际没恢复的假象。

计划：

- 增加统一 `MaintenancePressureState`：`none / soft / hard / blocked / recovering`。
- 所有硬触底只走一个入口：暂停目标请求，生成 Advisor/司 维护请求，完成后重新测量，未下降则短冷却后重排。
- datememory 聚合触发改为“按 bytes 倒序、每批最多 N 个 scope”，小 scope 用合并说明，避免一次维护轮过大。
- health 增加 `maintenance_recovery_state` 证据：`queued / active / completed_but_still_over / cooldown / cleared`。
- pending key 使用 `dedupe_scope` 后仍要保留 source scope 明细，便于对账哪个 persona/role 卡住。

## P1 优化项

### 1. 状态栏和 Aidebug 观测降 I/O

`status_resource_line` 每次渲染都会读取 datememory 总大小。这个值通常不需要逐帧实时。

计划：

- 建立 `ResourcePressureSnapshot`，在 tick 或维护扫描周期更新。
- 状态栏只读缓存，Aidebug 写快照时复用同一份数据。
- performance 增加 `resource_snapshot_ms`，观察是否消除 UI 慢帧。

### 2. 工具 schema 和工具投影择优

当前 schema 已投影，但 token 压力大时应自动选择更小工具面。

计划：

- 统计每个 persona/role 的 `local_tool_count`、`local_tool_schema_chars`、`tool_schema_tokens_estimated`。
- health 在 schema 超重时给出具体 top tools，而不是只提示“工具 schema 偏高”。
- 对临时维护轮使用更小 toolbox profile：context maintenance 只给 context 工具，datememory maintenance 只给 memory 工具。
- Matrix 的二级抽屉收纳低频参数，普通角色创建只显示 mode、soft trigger、hard cap 摘要。

### 3. dynamic role governance 幂等性

角色治理已有 `persona_manage` required tool 和治理模式工具同步，但角色创建、更新、reload、context_governance_set 之间仍需加强 idempotency。

计划：

- `role_create` 直接带 context governance 时，确保 default_tools 一次性补齐，不依赖后续 reload。
- `role_update` 切回 `advisor_managed` 时，自动关闭 `context_summary / context_vision / context_compact`，但保留用户显式 pin 的工具需要记录原因。
- role registry health 增加 `tool_projection_matches_governance=true/false`。
- dynamic role 通过 base persona 执行时继续防止向同一 role 递归派单。

### 4. scheduler 事件一致性

terminal、external tool、persona command、system maintenance 都在写 `scheduler.task.*`，但字段丰富度不完全一致。

计划：

- 所有 scheduler event 必带：`task`、`dedupe_key`、`payload_policy`、`deadline_ms`、`state`、`target_persona`、`target_role_id`。
- 对 `replace_not_stack` 做自动检查：同 dedupe scope 不允许 pending 队列堆多个旧 payload。
- health 增加 `scheduler.stuck_reason`：target busy、gate until、cooldown、unknown persona、capacity。
- 对 background terminal / wait / snapshot 保留现有低成本摘要，不把完整日志注入模型。

### 5. tool output 外部化闭环

工具输出 envelope 和 output refs 已有，但需要确保所有大输出路径都能被回收和精读。

计划：

- 大输出必须满足：模型回执短、UI 可看摘要、Aidebug 有 `tool.output.externalized`、`memory_read target=output` 可读完整内容。
- persona observe 永远不带 raw tool output，只带 folded recent activity。
- context_summary fast fold 应优先折叠工具回执 entry，保留 ref_id、路径、exit code、关键摘要。

## P2 收敛项

1. 文档去冲突。

旧文档里仍有 200KB context 阈值、0.5M datememory、25w hard token 等历史说法。需要统一引用本计划和 `context_token_governance_plan_20260520.md`，旧说法标注为历史。

2. 设置 UI 二级抽屉。

默认界面只保留模型、工具输出预算、基础角色治理；context hard cap、datememory 阈值、schema override、实验压缩参数进入高级区。datememory 普通设置不开放，固定 200KB。

3. 压力测试脚本化。

把人工 Aidebug live probe 固化成 smoke：

- focus mode 后 active context 是否换成 focuscontext。
- 300KB JSON context 经过 compact 是否降到预期。
- 大 thinking entry fold 后 preflight token 是否下降。
- datememory 聚合超限是否只发一批维护。
- role self_compact 是否 soft 不打断、hard 才升级。

## 择优策略

运行时应按“最少打断、最小损耗、硬兜底优先”的顺序选治理动作：

1. 未触底且角色有自管工具：优先 `context_summary fold` 或 `context_vision`，不打断任务。
2. 未触底且角色为 `advisor_managed`：soft 后交由司 `context_manage summary` 精细收口。
3. 工具回执造成污染：优先 `context_summary fold` 已知 entry ids，不做全区 compact。
4. 只是下一轮不需要旧上下文：优先 `context_vision` 降低返回比例，不物理删除。
5. active context 或 preflight input token 达 hard cap：立即暂停目标会话，交司全量 compact。
6. datememory 超限：只走 datememory 入库维护，不读、不压标准 context。
7. schema/token 压力来自工具面：先收工具投影，再考虑压上下文。

## 执行顺序

### 阶段 1：统一预算真源

- 实现 `TokenBudgetEstimator` 和 `TokenBudgetSnapshot`。
- Aidebug/status/performance 共用同一 token 和 context pressure 数据。
- 统一 20w input token hard cap，修正 `25w` 和 `session * 4` 的语义。
- 把 Codex `actual_body_chars`、`actual_body_tokens_estimated`、`tool_round_post_count`、`inflight_body_chars_total` 纳入 request/status/performance。

### 阶段 2：收紧阈值语义

- `soft 800KB / hard 1000KB` 成为默认硬规则。
- custom soft 不再隐式推导 `soft * 2` hard。
- 角色治理、settings、runtime threshold、health 字段统一命名。
- status/health 展示当前 persona/role 实际阈值，不再用 settings 默认值冒充 active 阈值。

### 阶段 3：让 thinking 可管理

- 审计 provider context 注入链，明确 thinking 是否作为 active context 装载。
- context fold/compact/manage 覆盖 thinking。
- 增加大 thinking 压缩后 token 下降回归。

### 阶段 4：维护队列恢复闭环

- `MaintenancePressureState` 单一化。
- hard maintenance 完成后强制复测，未下降短冷却重排。
- datememory 聚合维护批量策略收敛。

### 阶段 5：工具投影和 UI 收敛

- 工具 schema token top list。
- 二级抽屉放低频参数。
- schema override、role governance、tool projection 做 health 对账。

## 验证矩阵

| 链路 | 验证点 | 建议测试 |
| --- | --- | --- |
| token budget | preflight 估算和 provider usage 可对账 | 单测 + `performance.json` 对账 |
| responses inflight | 一次请求多轮 POST 的 body/token 总量可见 | `Server#13` 类多工具轮回归 |
| codex replay | `codex_item_json` replay 被纳入预算并可裁剪 | function_call_output replay 回归 |
| context hard cap | `Ctx 80% 0.8/1.0M` soft，`1.0M` hard block | 构造大 context 回归 |
| thinking 管理 | fold/compact 后 thinking 不再进入 active provider context | 大 thinking entry 回归 |
| focus mode | focus 后 active stats 指向 focuscontext，不误读 standard context | focus smoke |
| datememory | 总量 200KB 聚合触发，维护后 clear_context | 多 scope 压力回归 |
| scheduler | 同 dedupe 不堆旧 payload，busy/cooldown 可解释 | pending queue 单测 |
| role governance | mode 切换后工具授权与 registry 一致 | role_create/update/reload 回归 |
| tool output | 大输出只进外部化，不污染 observe/context | output ref smoke |
| Aidebug health | DEGRADED/BLOCKED 有准确证据和恢复状态 | health snapshot 回归 |

## 暂不做的边界

- 不立刻删除旧 KB 字段，先兼容一轮。
- 不把所有 rare knobs 都放到普通设置页。
- 不重写整个 `mcp.rs`，先在现有工具入口旁加统一预算和治理状态。
- 不让模型前端显示链路依赖 AI context 链路，用户 UI 历史和 provider active context 要保持分离。
- 不把 datememory 阈值开放给普通设置，默认固定 200KB。

## 本轮结论

当前架构方向是对的：外置 context、datememory、工具输出、角色治理和 Aidebug 观测已经有分层基础。真正需要优先补强的是“预算真源统一”和“硬触底恢复闭环”。只要把 token 计量、1M hard cap、thinking 可管理、维护队列复测这四件事打实，前面 300KB context 显示不降、focus 后 status 误读、context manage 动不到 thinking 这类问题就能被系统性定位和修复，而不是每次靠单点补丁。

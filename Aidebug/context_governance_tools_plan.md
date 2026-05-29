# ProjectYing 上下文治理工具计划

日期：2026-05-16

> 这份是旧版工具蓝图。当前默认口径与执行优先级以 `Aidebug/context_token_governance_plan_20260520.md` 为准，尤其是 token cap、`0.8M/1.0M` cap 和主动治理提示。

## 目标

在现有外置上下文格式上增加三层治理工具，让 ProjectYing 同时支持精密治理、粗暴自管和低损耗工具回执折叠。

核心原则：
- `context_manage` 保持精细化治理，由司优先使用。
- `context_compact` 是粗暴压缩工具，由 Matrix 分配和授权，通常给低成本角色自管上下文。
- `context_summary` 是轻量折叠工具，面向所有 agent 开放，主要用于并行折叠旧工具回执，避免额外轮询。
- 默认路由仍是司管；只有 Matrix 明确给某个角色打开并授权 `context_compact` 后，该角色才进入自管 compact 路线。

## 现有基础

当前上下文格式：
- `context/<Persona>/prompt.txt`
- `context/<Persona>/fastmemory.json`
- `context/<Persona>/state.json`
- `state.json` 内承载 `context / focus / meta / toolbox`

当前治理入口：
- `context_manage` 支持 `write / summary / compact(alias) / replace(alias)`
- Matrix 与司可显式指定 `persona`
- Coding 等执行 persona 默认不直接暴露 `context_manage`
- Aidebug 已观测 `active_context_entries`、`context_entry_limit`、token、datememory buffer 等状态

## 工具边界

### context_manage

定位：精密上下文治理。

用途：
- 按 `entry_ids`、连续区间或整区做高质量 summary。
- 保留用户意图、关键决策、代码位置、错误结论、验证结果、路径、坐标、命令和后续线索。
- 清扫重复、过期、无用的大段过程文本。
- 建立专注、有效、可追溯的全局视野。

约束：
- 不应为了整洁乱压仍有用的关键细节。
- 不应把代码坐标、错误证据、验证结果和用户明确偏好压丢。
- 司收到治理任务时要优先保守，不能确定就写明边界。

### context_compact

显示名：`CONTEXT COMPACT · ALL/OLD/FAST`

内部工具名建议：`context_compact`

定位：粗暴压缩 / 自主管理上下文。

模式：
- `ALL`：压缩目标 persona 的标准上下文整体，只保留一条粗摘要和必要续作线索。
- `OLD`：压缩旧半区，上半部分或旧 entries 替换成一条粗摘要，保留最近上下文原样继续执行。
- `FAST`：快速整理模式，优先折叠所有可折叠工具回执，保留当前任务最近正文；适合任务进行中降噪。

参数草案：
- `mode`: `all | old | fast`
- `persona`: 可选；省略时默认当前 persona
- `text`: 可选压缩后摘要。`all/old` 推荐提供；不提供时使用系统保底摘要模板。
- `reason`: 可选，说明触发原因，例如 `context_kb_over_threshold`
- `report_to_matrix`: 默认 `true`，自管角色完成 compact 后应汇报 Matrix

权限：
- Matrix 和司可以对任何 persona 调用。
- 普通 persona 只有在 Matrix 授权并打开该工具后，才能对自己调用。
- 普通 persona 不能用 `context_compact` 管理其他 persona。

语义：
- 这是暴力治理，不追求 `context_manage` 的高精度。
- 它适合 cheap role、临时 role、轻任务 role、非编程任务 role。
- 不适合作为 Coding / Matrix / 关键施工任务的默认治理方式，除非 Matrix 明确认为可以承受信息损耗。

### context_summary

显示名：`Context Summary`

内部工具名建议：`context_summary`

定位：低回执、低轮询、局部折叠工具。

动作：
- `fold`

参数草案：
- `persona`: 可选；省略时默认当前 persona
- `entries`: 必填数组
- `entries[].entry_id`: 必填，目标 entry id
- `entries[].text`: 可选，折叠后替换内容；为空时默认 `已折叠`
- `reason`: 可选

行为：
- 支持一次折叠多个 entry。
- 每个 entry 可单独指定折叠后内容。
- 不读取上下文，不返回上下文正文。
- 前端只显示成功/失败。
- 模型侧回执必须极小，只包含 `ok / failed_count / affected_entry_ids`。
- 单独调用时不会触发额外模型轮询；推荐与 `command`、`pty_wait`、`memory_read` 等工具并行调用。

典型用法：
- 模型刚读取了大输出，下一步执行命令前，顺手把上一个工具回执 entry 折叠成关键摘要。
- `context_compact FAST` 前后配合，先折叠工具回执，再继续任务。
- 避免模型持续携带完整工具输出。

## 路由机制

为每个 persona / dynamic role 增加上下文治理策略：
- `advisor_managed`：默认值，由司按阈值进行精密治理。
- `self_compact`：Matrix 授权角色自主管理，使用 `context_compact` 与 `context_summary`。

建议配置位置：
- 静态 persona：`context/<Persona>/state.json` 的 `meta.context_governance`
- 动态角色：role registry contract 中增加 `context_governance`

字段草案：
```json
{
  "mode": "advisor_managed",
  "manage_threshold_kb": 200,
  "compact_threshold_kb": 200,
  "report_to_matrix": true,
  "last_notice_entry_id": null,
  "last_compact_at": null,
  "over_threshold_notice_count": 0
}
```

默认：
- `mode=advisor_managed`
- `manage_threshold_kb=200`
- `compact_threshold_kb=200`
- 未配置 persona 继承全局默认 200KB

Matrix 可配置：
- 全局：`set context manage threshold 200kb`
- 单 persona：`set matrix context manage to 500kb`
- 单 role：`set 工 context compact to 300kb`

实现上不一定先做自然语言 parser；第一版可通过 `tool_manage role_update` 或设置页写入结构化字段。

## 阈值触发

### advisor_managed

当目标 persona 外置上下文达到 `manage_threshold_kb`：
- 系统在下一次用户消息或下一次该 persona 请求前注入维护提示。
- 维护提示交给司。
- 司使用 `context_manage` 精确压缩。
- 压缩完成后用简短结论回报 Matrix。

### self_compact

当目标 persona 外置上下文达到 `compact_threshold_kb`：
- 系统在该 persona 下一次请求前注入最新系统提示。
- 提示内容大意：当前上下文达到阈值，需要使用 `context_compact` 压缩；完成后汇报 Matrix。
- 如果连续超过阈值但未 compact，记录 `over_threshold_notice_count`。
- 超过允许次数后只汇报 Matrix，不频繁打扰用户。

建议默认：
- 第 1 次超限：提示当前 persona 自行 compact。
- 第 2 次超限：再次提示，并要求汇报 Matrix。
- 第 3 次仍未处理：Aidebug / Matrix 标记 `context.self_compact.ignored=DEGRADED`。

## 设置调整

要移除或降级旧的设置页 `context_manage` 固定阈值入口，改为模型可配置的治理策略：
- 全局默认阈值 200KB。
- 每个 persona / role 可覆盖。
- `context_manage` 和 `context_compact` 各自有阈值。
- 设置页只作为可视化/手动修改入口，不再是唯一来源。

## 工具投影

默认工具：
- Matrix：可管理 `context_manage`、`context_compact`、`context_summary` 的分配与授权。
- 司：默认有 `context_manage`，可有 `context_summary`，不负责分配 `context_compact` 路由。
- Coding：默认不直接暴露 `context_manage`；可开放 `context_summary`，用于折叠自身工具回执。
- 动态角色：默认 `advisor_managed`，只有 Matrix 授权后才打开 `context_compact`。

`context_summary` 应可开放给所有 agent，因为它不读取上下文、不返回大回执、只做明确 entry 替换。

## Aidebug 观测

新增健康链或扩展 `context.manage`：
- 当前 persona / role 的治理模式
- 当前上下文 KB
- manage threshold
- compact threshold
- 是否超限
- 是否已注入治理提示
- 自管角色是否忽略 compact 提示
- 最近一次 compact / summary 的 entry id 和结果

建议 health evidence：
- `context_governance_mode=advisor_managed|self_compact`
- `context_kb=...`
- `manage_threshold_kb=...`
- `compact_threshold_kb=...`
- `over_threshold_notice_count=...`

## 实施步骤

1. 文档与 prompt 对齐
- [x] 更新 Matrix prompt：说明 compact 路由归 Matrix 分配。
- [x] 更新 Advisor prompt：强调 `context_manage` 是精密治理，不乱压关键细节；司不主管 `context_compact` 路由。
- [x] 更新 Coding / role prompt：说明可使用 `context_summary` 折叠工具回执。

2. 状态结构
- [x] 在 context state 或 role contract 中加入 `context_governance`。
- [x] 默认迁移为 `advisor_managed / 200KB`。
- [x] 支持 per persona / per role override；第一版通过 state/role contract 结构化字段生效，自然语言设置解析后续扩展。

3. 工具 schema
- [x] 新增 `context_compact` schema。
- [x] 新增 `context_summary` schema。
- [x] 更新 `context/Matrix/schema/codex_tools.rsinc`。
- [x] 更新工具投影逻辑，让 `context_summary` 可安全开放给所有 persona。

4. 执行层
- [x] 实现 `context_compact all/old/fast`。
- [x] 实现 `context_summary fold`，只替换指定 entries 的展示/模型内容。
- [x] 保证 `context_summary` 模型回执极小，避免形成新的上下文负担。

5. 阈值注入
- [x] 在构建请求前检查目标 persona context KB。
- [x] 根据治理路由注入司维护提示或自 compact 提示。
- [x] 避免同一阈值状态重复注入，沿用现有 maintenance ticket dedupe。

6. Matrix 分配入口
- [x] `tool_manage role_create/role_update` 支持 `context_governance`。
- [x] `tool_manage list/role_list` 显示治理模式和阈值。
- [x] 新增 `tool_manage context_governance_set`，Matrix 可直接设置静态 persona 或动态角色的治理路由与阈值；设置为 `self_compact` 时自动开放 `context_compact/context_summary`。
- [ ] 普通设置页后续只做可视化读写入口；主管入口以 Matrix 的 `tool_manage context_governance_set` 为准。

7. Aidebug 与测试
- [x] 增加 health/contract 证据字段。
- [x] 增加单测：默认司管、授权自管、阈值提示、summary 低回执。
- [x] 实机 Aidebug 测试：Matrix 配置自管 worker，触发 compact 阈值，确认不会交给司，且 worker 能向 Matrix 汇报完成。

## 2026-05-16 实现记录

已完成第一版代码接入：
- `context_compact`：支持 `all / old / fast`，可按 persona/role scope 调用；普通 persona 只允许管理自己。
- `context_summary`：支持 `fold` 多 entry，替换 raw `function_call_output` 的 provider 回放内容，模型回执保持极小。
- `context_governance`：加入动态角色 contract 和 role state；默认 `advisor_managed / 200KB`，`self_compact` 时阈值维护会路由给当前 persona 而不是司。
- Aidebug：动态角色 contract 快照与 health context 链路增加 context_governance 证据。
- Prompt：Matrix/司/Coding 已同步三层治理边界。

已通过定向测试：
- `cargo test context_compact -- --nocapture`
- `cargo test context_summary -- --nocapture`
- `cargo test self_compact -- --nocapture`
- `cargo test aidebug -- --nocapture`
- `cargo test role_ -- --nocapture`
- `cargo test context_manage_schema -- --nocapture`

全量回归：
- 2026-05-16：`cargo test -- --nocapture --test-threads=1` 通过，495 passed / 0 failed。
- 2026-05-16：补齐 Matrix 结构化治理设置入口后，`cargo test -- --nocapture --test-threads=1` 通过，497 passed / 0 failed。
- 2026-05-16：补齐动态角色 provider/tool 身份后，`cargo test -- --nocapture --test-threads=1` 通过，500 passed / 0 failed；`cargo build --release` 通过。

补充实现：
- `tool_manage action=context_governance_set` 支持 `persona` 指向 `matrix/advisor/coding/server`，或用 `role_ids` / `role.id` 指向动态角色。
- 支持顶层 `context_governance` 对象，也支持快捷字段 `mode/manage_threshold_kb/compact_threshold_kb/report_to_matrix`。
- 静态 persona 进入 `self_compact` 时会自动 `open context_compact/context_summary`。
- 动态角色进入 `self_compact` 时会把 `context_compact/context_summary` 合并进 `default_tools`，保证后续 role send 时能实际拿到工具。
- 切回 `advisor_managed` 时会撤回 `context_compact`：静态 persona 自动 close，动态角色从 `default_tools` 移除；`context_summary` 保留为通用低回执折叠工具。

实机回归补充：
- `worker` 设置为 `self_compact`，`compact_threshold_kb=32`。
- 构造 `SELF_COMPACT_LIVE_FIX_20260516` 超阈值上下文后，运行时生成 `上下文自管压缩` 系统请求并路由到 `Role_worker`。
- `worker` 调用 `context_summary fold` 折叠 `[15,16,17,18,20]`，随后 `persona_manage.send persona=matrix` 成功 queued。
- Matrix 侧收到报告，正文含 `from: 工 (worker)`、`source_role: worker`、`source_context: context/Role_worker`；这验证动态角色向其 base persona 汇报不会再被当作同 persona 自发自收。

当前迭代收口：
- Matrix `context/Matrix/state.json` 由 `91` 条保守收口到 `45` 条，合并了 `e971-e1017` 这一段旧 smoke / filler / 重复正文噪声，保留 worker self_compact、datememory 维护、角色治理和用户问答的关键事实。
- 新增调试开关 `PROJECTYING_SKIP_STARTUP_GREETING=1`，可在实机短启动时跳过 Matrix 启动问候，避免为了刷新 `Aidebug/status.json` 而额外发模型请求。
- 已重新构建 release 并用无问候短启动验证：`active_context_entries=45`，`overall_state=PASS / overall_score=100`。
- 2026-05-16 新增角色治理同步修复：`role_create` 若直接创建 `self_compact` 角色，现在会在创建时自动补齐 `context_compact/context_summary/persona_manage`，而不是只等后续 `context_governance_set` 再补。对应回归 `tool_manage_can_create_self_compact_role_and_route_maintenance_to_itself` 已通过。
- 2026-05-16 新一轮回归通过：
  - `context_compact`
  - `context_summary`
  - `role_create`
  - `self_compact`
  - 全量 `cargo test -- --nocapture --test-threads=1` = 502 passed / 0 failed
- 当前真实工作区 roles registry 为空，短启动 health 里 `dynamic_role.governance` 会暂时 DEGRADED；后续若要做 live role 测试，需要先通过 Matrix/tool_manage 创建或恢复至少一个启用角色。

## 验收标准

- 默认行为不变：未授权角色仍由司精密治理。
- `context_manage` 与 `context_compact` 边界清晰，不互相污染职责。
- `context_summary` 可并行调用，且不会单独造成大轮询。
- Matrix 能看到并配置每个角色的上下文治理路由。
- 角色进入 `self_compact` 后，超限提示发给角色自己；持续不处理才汇报 Matrix。
- Aidebug 能明确显示是哪条 persona / role 的上下文超限、采用哪种治理模式、最近治理动作是否成功。

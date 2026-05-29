# Aidebug Round Log

日期：2026-05-16

## 本轮测试

- `cargo check`
- `cargo test aidebug -- --nocapture`
- `cargo test tool_manage_settings -- --nocapture`
- `cargo test tool_manage_context_governance -- --nocapture`
- `cargo test config_governance_health_tracks_settings_route_and_matrix_only_policy -- --nocapture`
- `cargo test scheduler_health_detects_connecting_without_progress -- --nocapture`

## 本轮结论

- `tool_manage.settings` 已接入统一 settings 路由，Matrix-only 约束生效。
- `config.governance` 已加入 Aidebug health，能单独反映 settings 路由是否完整。
- `scheduler.lifecycle` 的 `connecting` 无推进场景已能明确降级。
- `datememory`、`context`、`token`、`tool`、`UI`、`role` 基线仍为 PASS。

## 当前基线

- `overall_state=PASS`
- `overall_score=100`
- `config.governance=PASS`
- `memory.datememory.buffer=PASS`
- `token.budget=PASS`
- `scheduler.lifecycle=PASS`

## 下一步

- 先重启当前运行的 ProjectYing / 萤，再复查 `health.json` 是否载入 `config.governance`。
- 继续按 `memory.datememory.buffer -> context.manage -> token.budget -> scheduler.lifecycle -> dynamic_role.governance` 的顺序做压力测试。

## 2026-05-17 补充

- 已记录：`update_plan` 在 live 路径里会带来明显的全局卡顿，后续要作为独立体验问题追踪。
- 已确认：后端 `update_plan` 已支持 `decision`、`todo/plan`、`blueprint` 三种模式，当前工作重点是把这三种语义稳定用起来，而不是再扩散新的模式名。
- 下一步实战：清理测试角色后，创建 `~/snake.html` 作为一次真正的前端轻实战，顺带观察生成、打开和持续运行时的稳定性。
- `snake.html` 已完成语法检查与 headless Chromium 截图验证，初版版面有高度偏满的问题，已收紧为适配 900x1000 视口的布局，截图可正常渲染且控制区可见。
- 当前 `dynamic_role.governance` 在 health 里显示 `DEGRADED`，原因是本轮已经按测试要求清空了测试角色注册表；这是清理后的预期状态，不是新增故障。

## Live 测试计划入口

- 2026-05-16 已建立 `Aidebug/live_test_plan_20260516.md`。
- 本轮针对当前已启动的萤执行集中测试；测试期间只记录问题，除非出现 BLOCKED / BROKEN，否则最后统一修复。

## Live T1 · health smoke

- 投递：`Aidebug/inbox/live_t1_health_20260517.json`
- 结果：当前运行的萤成功消费 Aidebug 投递，Matrix 调用 `tool_manage health` 成功，工具回执正常写入 `events.jsonl`。
- 观察：工具内读取 health 时短暂回报 `overall_state=DEGRADED / overall_score=98`，链路为 `scheduler.lifecycle`；请求完成后的 `Aidebug/health.json` 已恢复 `overall_state=PASS / overall_score=100`。
- 问题记录：health 在“当前请求仍处于连接/工具调用中”的瞬间可能产生临时 DEGRADED，容易被 Matrix 当成真实故障。后续修复时考虑区分 in-flight 自观察与长期卡住。

## Live T2 · settings/governance smoke

- 投递：`Aidebug/inbox/live_t2_settings_20260517.json`
- 结果：Matrix 成功调用 `tool_manage settings`，仅写回当前默认输出等级 `Normal` 与主题 `Terminal Rose`；随后 `tool_manage health` 回报 `overall_state=PASS / overall_score=100`。
- 结果：请求完成后的 `Aidebug/health.json` 也保持 PASS，`memory.datememory.buffer`、`context.manage`、`token.budget`、`scheduler.lifecycle` 均 PASS。
- 问题记录：当前运行实例写出的 `status.json` 缺少 `config_governance_protocol`，`health.json` 缺少 `config.governance` 链；源码与回归测试已包含该字段/链路，说明 live 进程疑似旧构建或未重启到最新二进制。后续修复/验证需要重启萤后复查该链路。

## Live T3 · role governance smoke

- 投递：`Aidebug/inbox/live_t3_role_governance_20260517.json`
- 结果：Matrix 成功对 `compact_probe` 重放 `context_governance_set`，并通过 `role_list` 复查到 `default_tools=memory_check|persona_manage|context_compact|context_summary`，`context_governance=self_compact(200/32)`。
- 结论：`compact_probe` 的 self_compact 路由已同步，动态角色治理链路在 live 实例中可用。

## Live T4 · compact probe smoke

- 投递：`Aidebug/inbox/live_t4_compact_probe_20260517.json`
- 结果：`compact_probe` 在真实请求里调用了 `context_compact mode=fast`，输出 `ContextCompact succeeded`，并随后调用 `context_summary fold`，输出 `ContextSummary succeeded`。
- 结果：最终回报明确表示 `context_compact：已执行`、`context_summary：已执行`，且暂不需要继续折叠。
- 结果：请求后 `Aidebug/health.json` 保持 `overall_state=PASS / overall_score=100`。
- 结论：自管压缩链路在 live 实例中已跑通，不只是 role 列表层面的配置存在。

## 源码回归补测

- `CARGO_TARGET_DIR=target-codex cargo test config_governance_health_tracks_settings_route_and_matrix_only_policy -- --nocapture`：PASS。
- `CARGO_TARGET_DIR=target-codex cargo test tool_manage_context_governance_sets_role_and_adds_compact_tools -- --nocapture`：PASS。
- `CARGO_TARGET_DIR=target-codex cargo test inactive_dynamic_role_self_compact_is_scanned_and_queued_for_role_target -- --nocapture`：PASS。
- `CARGO_TARGET_DIR=target-codex cargo test matrix_schema_exposes_context_management_tools -- --nocapture`：PASS。
- 备注：曾并行触发一次 cargo cache/target lock 报错，随后顺序重跑通过；该报错不计为源码失败。

## Live T5 · 重启修复验证

- 处理：完成 `cargo build --release`，停掉旧的 `projectying` 进程后，重新拉起最新 release。
- 结果：新的 live `status.json` 已写入 `config_governance_protocol`，字段内容与源码一致。
- 结果：新的 live `health.json` 已写入 `config.governance`，链路状态为 `PASS`，`overall_state=PASS / overall_score=100`。
- 结论：本轮问题属于“旧构建未重启”而非源码缺失；集中修复已完成，当前 live 实例已恢复到最新链路。

## Live T7 · APK 链路探针派单阻断

- 投递：`Aidebug/inbox/live_t7_apk_probe_20260517.json`，已被移动到 `Aidebug/processed/1778979138668-live_t7_apk_probe_20260517.json`。
- 预期：Matrix 只做主控调度，指挥 Coding 复用 `android/agentbrowser` 做最小 `build -> adb install -> launch -> 回报`。
- 实际：Matrix 先调用 `update_plan` 后，`persona_manage.send` 被运行时门禁拒绝，提示“当前任务已列出计划，后续复杂执行必须先进入专注模式”。
- 实际：Matrix 随后尝试 `focus_mode.enter`，但 `focus_mode` 转入 `context_manage` 时 target 为空，触发 `focus_mode failed: 不支持的 context target：`。
- 结果：Matrix 反复尝试派单/进 focus，最终收口为“未执行 build / adb install / launch”。Coding 未收到实际任务。
- 问题记录：`focus_mode.enter/exit` 不应要求 context target；`persona_manage` 属于 Matrix 标准调度动作，不应被 update_plan 后的 focus 门禁阻断。

## Live T8 · APK 链路复测（修复后）

- 投递：`Aidebug/inbox/live_t8_apk_probe_after_fix_20260517.json`，已被移动到 `Aidebug/processed/1778979884239-live_t8_apk_probe_after_fix_20260517.json`。
- 修复点 1：`persona_manage` 不再被 `update_plan` 后的 focus 门禁误拦，Matrix 成功把任务派给 Coding。
- 修复点 2：`focus_mode.enter/exit` 不再依赖 `context target`。
- Coding 真实命令结果：
  - `cd android/agentbrowser && bash build.sh` 成功，APK 刷新为 `37404` bytes，mtime `2026-05-17 09:05:32 +0800`。
  - `adb devices -l` 看到在线设备 `emulator-5554`。
  - `adb install -r dist/projectying-browser-debug.apk` 成功。
  - `adb shell monkey -p com.projectying.browser -c android.intent.category.LAUNCHER 1` 失败，原因是派单包名与 Manifest 实际包名不一致。
  - 真实包名 `io.projectying.agentbrowser` 启动成功，`Events injected: 1`。
- 结论：build / install / launch 链路可用，残留问题是 T8 派单文本中的包名写错，不是 APK 本体问题。
- 额外修复：`focus_mode` 现在允许 Coding 自己进入/退出专注模式，已补单测通过。

## Live T9 · Coding focus smoke retry（兼容修复后）

- 处理：将 `run.sh` 对应的默认 `target/release/projectying` 重新构建并重启 live 实例，确保不是旧 binary。
- 修复点：`focus_mode` 参数解析新增行式兼容，`action: focus_enter` / `action: focus_exit` 这类 live 回执也能正确进入执行层。
- 结果：Matrix 重新派单给 Coding 后，Coding 成功执行 `focus_mode.enter` 与 `focus_mode.exit`。
- 结果：`persona_manage observe` 显示 `mode: task`、`focus: Coding focus smoke`，退出后回到标准态，未再出现 `当前 persona CODING 不支持本次任务/上下文管理调用`。
- 结论：T9 门禁问题已修复，live 链路通过。

## 源码整改 · 松散参数入口统一加固

- 目标：把 live provider 常见的行式 key/value 回执统一接入同一层解析网关，不再只修 `focus_mode` 单点。
- 改动：
  - 新增通用 `LooseArgumentFields` 取值辅助，统一支持 `action: ...` / `target: ...` / `text: ...` 这类行式字段。
  - `context_manage`、`context_compact`、`context_summary`、`focus_mode`、`tool_manage`、`persona_manage`、`memory_add`、`memory_check`、`memory_read` 都改为 JSON 优先、行式兼容兜底。
  - `context_summary` 增加 `e123` 这类稳定条目编号兼容，避免界面显示的稳定 ID 不能直接回填。
  - `persona_manage` 与记忆链不再依赖纯 JSON，行式派单与行式记忆回填都可执行。
- 回归测试：
  - `CARGO_TARGET_DIR=target-codex cargo test line_style -- --nocapture`
  - `CARGO_TARGET_DIR=target-codex cargo test memory_tools -- --nocapture`
  - `CARGO_TARGET_DIR=target-codex cargo test context_manage -- --nocapture`
  - `CARGO_TARGET_DIR=target-codex cargo test tool_manage -- --nocapture`
  - `CARGO_TARGET_DIR=target-codex cargo build --release`
- 结果：
  - `line_style` 组中 `context_summary`、`focus_mode`、`persona_manage`、`memory_add/read/check`、`tool_manage.settings` 全部通过。
  - `context_manage` 23 项回归通过，未破坏 summary / focus / fastmemory 既有行为。
  - `tool_manage` 17 项回归通过，未破坏 settings / role / health / governance 路由。
  - `cargo build --release` 成功，说明这轮不是仅测试态成立。
- 结论：
  - 当前问题不再是单点漏水，而是模型输出格式漂移对工具层造成的脆弱入口；已改成统一入口层，后续同类漂移更不容易再反复炸开。

## Live T10 · Context Vision / 三模式上下文迭代

- 改动：
  - 新增 `context_vision` 工具，支持 `context_percent`、`percent`、`mode` 三种入口，并在工具执行后写回下一轮可见度。
  - provider 侧上下文注入改为百分比可见度提示，默认保留最新轮次，旧上下文按预算节能隐藏。
  - 连续工具回合会自动抬升可见度预算，避免深度对账长期停在低可见度。
  - 只有真正执行的工具调用才推进连续轮次；被拒绝的调用不抬预算。
  - `advisor_managed / summary_compact / vision_compact` 已接入治理路由，`self_compact` 保留兼容别名。
  - `vision_compact` 达阈值后也进入自管维护票，提示 `context_vision + context_compact`。
  - `tool_manage` 相关 schema、角色默认工具投影、context_system 计划文档同步更新。
- 回归测试：
  - `cargo test tool_manage_context_governance_sets_static_persona_and_opens_compact -- --nocapture`
  - `cargo test tool_manage_context_governance_sets_role_and_adds_compact_tools -- --nocapture`
  - `cargo test tool_manage_can_create_summary_compact_role_and_route_maintenance_to_itself -- --nocapture`
  - `cargo test provider_messages_include_context_visibility_hint_and_keep_latest_round -- --nocapture`
  - `cargo test continuous_tool_rounds_raise_context_visibility_budget -- --nocapture`
  - `cargo test context_vision_sets_next_round_visibility_budget -- --nocapture`
  - `cargo test self_compact_context_maintenance_routes_to_current_persona -- --nocapture`
- 结果：
  - 上述回归通过。
  - `cargo test --no-run` 通过，说明全测试树仍可编译。
- 结论：
  - 三套上下文模式已落到代码层，接下来可以继续补 `context_system` 二层抽屉和更细的前端展示，但核心预算路由已经连通。

# ProjectYing Matrix 自检计划 · 2026-05-17

## 目标

- 不改代码，只让 Matrix 读取程序状态并做自检。
- 避免继续混测 `vision_probe` 上不应该开放的 `context_summary`。
- 补测 `summary_compact` 角色、`context_system` 二层抽屉和 SharedBoard 真实可见性。

## 依据

- 代码里 `context_system` 已进入 toolbox prompt。
- `vision_probe` 只应测 `context_vision + context_compact`。
- `context_summary` 需要切到 `summary_compact` 路线验证。
- 当前健康状态提示 `context.manage` / `token.budget` 已降为 DEGRADED，需要做最小收口而不是继续堆探针。

## 自检步骤

1. 读取 `Aidebug/status.json`、`Aidebug/health.json`、`Aidebug/latest_reply.txt`。
2. 用 `tool_manage list` 检查 Matrix 的工具投影，确认 `context_system` 仍是折叠抽屉语义。
3. 创建或重载一个 `summary_probe` 动态角色，治理模式为 `summary_compact`，默认工具至少包含 `context_summary` 与 `context_compact`。
4. 在 `summary_probe` 上实测：
   - `context_summary` 折叠最近一条真实工具回执
   - `context_compact fast` 收口旧上下文
5. 再做一条 `vision_probe` 只读检查，确认 SharedBoard 是否可见。
6. 最后只回传 `HEALTH / DRAWER / SUMMARY / COMPACT / BOARD` 五项 PASS/FAIL 与最短证据。

## 约束

- 不改源码。
- 不做代码修复。
- 不展开无关长聊。

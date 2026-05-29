# ProjectYing Context Vision 回归记录 · 2026-05-17

## 范围

- 只记录本轮对 `projectying` 的 `context vision / summary / compact / shared board` 回归。
- 不改代码，仅记录结果，避免后续重复测试。

## 已验证

- `context_vision_sets_next_round_visibility_budget`：PASS
- `context_summary_accepts_line_style_and_entry_refs_from_live_provider`：PASS
- `tool_manage_can_create_summary_compact_role_and_route_maintenance_to_itself`：PASS
- `dynamic_role_shared_board_reads_live_matrix_public_after_role_bootstrap`：PASS
- `self_compact_context_maintenance_routes_to_current_persona`：PASS
- `cargo build --release`：PASS

## 观察

- `context_vision` 的提示仍是“本轮上下文返回阈值”，并能正确把 0 / medium 映射到下一轮可见度预算。
- `summary_compact` 角色路由正常，`context_summary` 与 `context_compact` 会同时投影。
- 动态角色读取 `SharedBoard` 已按 Matrix 根目录最新 `fastmemory.public` 读取。

## 备注

- 当前运行中的旧 `projectying` 进程仍是 `PID 23419`，本轮未强制重启它，因此现网 live smoke 仍可能反映旧实例状态。

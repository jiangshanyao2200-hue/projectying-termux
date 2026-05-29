# ProjectYing 设置、记忆与工具回执收口计划

日期：2026-05-16

## 本轮目标

把人类设置页从“上下文参数面板”收回到“运行系统参数面板”，让上下文治理继续由 Matrix / 司 / 角色治理策略承担，用户只管理供应商、模型、系统权限和工具回执大小。

## 需求核对

- 新建动态角色的人类入口只需要 `/settings`，设置页只用于选择供应商和模型。
- 动态角色默认继承 Matrix 当前正在使用的 provider/model，之后人类可在 `/settings` 里手动覆盖。
- 用户设置页不再配置上下文档位；上下文治理阈值由 Matrix / 司通过治理策略维护。
- 默认上下文治理阈值按 200KB；Matrix / 司可对不同 persona / role 调整 `context_manage` 或 `context_compact` 的触发条件。
- `datememorycontext` 阈值固定 200KB，不提供设置页调节；超过后交由司写入 `datememory.db`。
- 同一天 `datememory` 写入必须续写同日记录，不能覆盖，也不能新建重复日期。
- 工具回执大小仍允许用户配置，但配置口径改为 KB 档位，不再显示或暴露 lines/chars。
- 工具回执大小并入 System 页；Context 页移除。
- 旧 `memory/output` 外导路径要退场，工具输出应优先进入 `toolmemory` 数据库链路；仍在使用的实时流路径需要单独迁移，不和设置页改动混做。

## 本轮执行切片

1. 设置页收口
- 移除 Matrix 设置页的 Context tab。
- System 页保留身份、子代理、权限、PTY 观察。
- System 页新增工具回执默认档位和 Small / Normal / Large KB 配置。
- 上下文治理相关条目不再作为用户可选字段出现。

2. 工具回执预算改为 KB
- `ToolOutputSettings` 改为 KB 字段。
- `CommandOutputBudget` 仍可保留内部 line cap 作为安全限流，但不作为用户配置项。
- 配置兼容旧字段：旧 `*_chars` 可迁移为 KB；旧 `*_lines` 不再参与用户配置。
- 回执标签改为 `small (48 KB)` 这类文案。

3. datememory 阈值固定化
- 对设置页隐藏 `memory_context_limit_kb`。
- 运行态默认保持 200KB。
- 回归确认同日写入为追加，不覆盖。

4. 动态角色设置入口确认
- 保持角色设置只走 API-only surface。
- 回归确认 API-only persona/role 默认继承 Matrix provider/model selection。

5. `memory/output` 迁移调查
- 本轮先记录现状：PTY / multiagent 实时流仍存在 `memory/output` 引用。
- 后续独立迁移到 toolmemory-backed stream/ref 协议，避免破坏实时终端日志。

## 验证计划

- `cargo test -q settings_`
- `cargo test -q tool_output`
- `cargo test -q memory_`
- `cargo test -q dynamic_role`
- `cargo check --all-targets`

## 当前结论

已完成切片 1-4：

- Matrix 设置页已移除 Context tab；工具回执预算并入 System 页，以 KB 档位显示和持久化。
- 新建 / 动态角色的 `/` 命令入口只暴露 `/settings`；动态角色仍继承 Matrix 当前 provider/model，用户可在 settings 里手动覆盖。
- `datememorycontext` 阈值固定为 200KB，设置页和配置文件不再开放该阈值。
- `memory_add target=datememory` 对同一 `source_persona + date` 改为续写已有日记行，不覆盖、不再新建同日重复日期行。
- 旧 `tool_output_*_lines/chars` 仅作为反序列化兼容入口，新保存的配置只写 `tool_output_*_kb`。

切片 5 当前结论：

- 普通 command 完成态已经归档到 `toolmemory`，不再依赖 `memory/output/command` 作为主链路。
- `memory/output/terminal` 与 `memory/output/multiagent` 仍是 PTY / 子代理实时流日志，`memory_read target=output` 依赖它们做运行中观察；不能在本轮直接删除，否则会破坏 AI 与用户协同看终端/代理日志。
- 下一轮如要彻底去掉 `memory/output`，需要先设计 `toolmemory-backed live stream/ref`：运行中写入可增量读取、完成后转为 toolmemory entry，并替换 `memory_read target=output` 的路径白名单。

## 本轮验证结果

- `cargo check --all-targets`：PASS
- `cargo test -q settings_`：17 passed
- `cargo test -q memory_`：53 passed
- `cargo test -q tool_output`：3 passed
- `cargo test -q dynamic_role_palette_keeps_only_settings_even_with_matrix_base`：1 passed
- `cargo test -q api_only_persona_inherits_matrix_provider_selection_without_override`：1 passed

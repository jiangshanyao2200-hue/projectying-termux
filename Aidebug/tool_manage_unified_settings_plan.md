# ProjectYing Matrix 统一治理工具计划

日期：2026-05-16

## 目标

把 Matrix 的治理入口继续收口到 `tool_manage`，让它同时能管：

- 工具回执预算
- 系统体验参数
- 其他 persona 的模型选择
- 司 / 角色的上下文治理阈值
- 动态角色的工具与治理路由

## 设计边界

- `tool_manage` 仍然是唯一统一入口，不再新增第二个“半套治理工具”。
- 上下文精细治理仍走 `context_manage` / `context_governance_set`。
- 工具预算、系统体验参数、persona 模型切换走新的 `settings` 动作。
- 默认展示保持折叠，前端只显示简洁摘要。
- 不暴露密钥写入、不开放安全上限、不把 runtime 内部路径做成可乱改的入口。

## 这轮要补的能力

1. 工具预算
- Matrix 可统一设置 tool output small / normal / large KB 档位。
- 默认档位可切换。

2. 系统体验参数
- Matrix 可调子代理保留数、权限模式、PTY 观察间隔、图片压缩质量、上传上限、名称和主题。

3. persona 模型切换
- Matrix 可直接切换其他 static persona 的 provider / model。
- 只改模型和当前 provider 选择，不碰密钥明文。

4. 现有上下文治理不回退
- `context_governance_set` 继续负责 advisor_managed / self_compact 路由。
- 角色级阈值仍由 Matrix 统一分配。

## 验收

- `tool_manage settings` 能正确写入 Matrix / coding / server / advisor 的可写项。
- `tool_manage context_governance_set` 仍正常工作。
- 运行时 side effect 立即生效，不需要重启。
- 相关测试通过。

## 落地记录

- [x] `tool_manage.settings` 接入统一 settings 分发，只允许 Matrix 调用。
- [x] Matrix 可统一写入工具回执 KB 预算、默认档位、系统体验参数和主题。
- [x] Matrix 可切换静态 persona 的 provider/model；动态角色不走该入口。
- [x] `context_governance_set` 继续独立负责 advisor_managed / self_compact 路由与阈值。
- [x] `SettingsState::save_config` 只在 Matrix 自身保存时同步运行时 side effect，避免其他 persona 的 API-only 设置误改全局运行时。
- [x] 补充回归测试：Matrix-only 权限、Matrix 工具预算/系统参数持久化、静态 persona 模型切换。

## 验证记录

- `cargo check`
- `cargo test tool_manage_settings -- --nocapture`
- `cargo test tool_manage_context_governance -- --nocapture`
- `cargo test matrix_settings_keeps_project_identity_and_hides_context_tab -- --nocapture`
- `cargo test context_settings_saved_to_context_json -- --nocapture`
- `cargo test settings_runtime_getters_keep_open_limit_and_runtime_values_effective -- --nocapture`

注意：工具回执 KB 预算仍会经过安全钳制，例如超过上限的 large 档会落到运行时允许的最大值；这属于预期边界，不向 Matrix 暴露无限上限。

# ProjectYing 上下文系统三模式计划

日期：2026-05-17

## 目标

把上下文治理从单一路径升级为三套系统级模式，并把相关工具收进二层抽屉，避免 Matrix 长期展开一堆上下文工具占用视觉和 token。

本计划的核心不是继续细抠单个工具，而是把“怎么返回上下文、谁来治理、什么时候提预算”做成一套路由系统。

## 实测补充

- 短任务 README 查询时，模型会在 `command` 里自动带 `context_percent=0 / tiny`。
- 跨文件对账时，预算会抬到 `medium`，`ctx` 提示也会同步刷新。
- 深度对账仍会多轮探路，所以需要连续多轮工具回合时自动提预算，避免一直卡在 tiny。
- 返回比例应是近似值，前端和提示文案要用“本轮上下文返回阈值约 40%”这种说法，不要把 tiny / medium / full 当成唯一表述。

## 三套体系

### 1. 司管理 / 精确治理

定位：
- 适合重要项目、关键施工、需要保留代码坐标和证据链的任务。
- 由司主导 `context_manage`，做精确收口、替换、保留和清扫。

特征：
- 精度最高。
- 最适合编程、排障、关键决策、重要对账。
- 不追求“少看一点”，而是追求“只保留该保留的东西”。

默认：
- `context_manage` 对司默认展开。
- 其他角色不主动常显，交由 Matrix 分发和模式路由决定。

### 2. 标准任务 / `context_summary + context_compact`

定位：
- 适合标准任务。
- 由模型自管，重点是折叠工具回执、整理旧条目、保持当前任务继续推进。

特征：
- `context_summary` 负责把已知 entry 折叠成小回执。
- `context_compact` 负责粗暴压缩旧上下文。
- 更适合常规对账、普通 coding、稳定执行链路。

模式语义：
- summary 负责“结构化整理”。
- compact 负责“粗暴压缩”。
- 两者配合时，模型不需要一直背着大工具回执走。

### 3. 快速任务 / `Context Vision + context_compact`

定位：
- 适合快速标准任务。
- 更偏执行链路、短查询、状态探测、README / pwd / ls / grep / adb 这类快速动作。

特征：
- `context_vision` 负责动态可见度提示与下一轮返回预算。
- `context_compact` 负责必要时粗压缩。
- 这一路线应比 summary 更轻，不要和 summary 默认并列，避免重复治理。

提示语义：
- 系统应告诉模型“当前上下文基于设置，可见度约 40%”之类的信息。
- 模型可根据任务类型把下一轮预算提高到约 67% 或 100%。
- 百分比只是近似，不要求绝对精确，但要足够稳定。

## 工具分工

- `context_manage`：精确治理，保留关键事实。
- `context_summary`：折叠已知 entries / 工具回执。
- `context_vision`：控制下一轮上下文可见度与返回预算。
- `context_compact`：粗暴压缩。

## 二层抽屉

建议新增系统级抽屉：`context_system`

要求：
- 默认折叠。
- 把 `context_manage / context_summary / context_vision / context_compact` 以及上下文治理设置集中收纳。
- 不让 Matrix 在常规工具箱里反复看到一整排上下文工具。
- 由 Matrix 决定每个角色采用哪种上下文模式。

默认展开策略：
- `context_manage` 默认对司展开。
- 其他角色只展开与当前模式匹配的工具。
- 不同模式之间不要同时把 summary 和 vision 一起暴露，除非明确配置。

## 模式路由

建议把角色上下文模式收敛成三档：

- `advisor_managed`
- `summary_compact`
- `vision_compact`

映射建议：

- `advisor_managed` -> `context_manage`
- `summary_compact` -> `context_summary + context_compact`
- `vision_compact` -> `context_vision + context_compact`

兼容建议：
- 旧的 `self_compact` 可先作为兼容别名保留。
- 对外显示优先用新的三模式名，避免后续语义再打架。

## 动态返回阈值

这里新增的不是“再做一个摘要工具”，而是上下文返回策略。

建议能力：
- 工具调用可携带下一轮上下文回参。
- 回参决定下一轮返回阈值比例，例如 40%、67%、100%。
- 深度任务或连续多轮工具回合时自动抬高预算。
- 当前任务若只是简单确认，不应强制拉满上下文。

建议显示：
- `当前上下文返回阈值约 40%`
- `下一轮建议预算：67%`
- `深度对账建议：100%`

兼容约束：
- `tiny / medium / full` 可保留为内部档位或兼容别名。
- 面向模型和前端的主文案优先用百分比。

## 计划落点

1. 先把上下文模式字段做成系统级路由。
2. 再把 `context_manage / summary / vision / compact` 收进 `context_system`。
3. 然后让 Matrix 在建角色或切模型时只选模式，不直接暴露整堆工具。
4. 最后把动态可见度百分比接到工具回执和下一轮系统提示里。

## 验收标准

- 短任务会自动走低可见度，不再默认全量返回上下文。
- 跨文件对账会自动提高预算到中档。
- 深度任务不会长期卡在 tiny，而是能逐步提额。
- `context_manage` 仍保持司的精确治理定位。
- `context_summary + context_compact` 与 `Context Vision + compact` 不会在默认工具箱里同时泛滥。
- `context_system` 默认折叠，只有被选中的模式才展开对应子工具。

## 实施进展

- 已接入 `context_vision` 工具，支持 `context_percent / percent / mode` 三种入口。
- 已将 `context_percent` 作为通用上下文预算提示接入工具执行链。
- 已把 provider 侧上下文展示改成百分比返回阈值提示，并保留最新轮次。
- 已将 `context_vision` 的主提示语统一为“本轮上下文返回阈值”，明确这是按需返回，不是丢失，避免模型把 30% / 67% / 100% 误解成固定损失档位。
- 已把工具回执与 provider 上下文块同步显示 `context_return_threshold`，前端和模型看到的是同一语义。
- 已让连续工具回合自动抬升可见度，避免深度任务长期卡在低预算。
- 已把 `advisor_managed / summary_compact / vision_compact` 的治理枚举和动态角色工具投影接通。
- 已保留 `self_compact` 兼容别名，不影响旧维护链路。
- 已把 `context_summary / context_vision / context_compact` 在动态角色 runtime 里的 base persona 误填兜底成当前角色上下文，避免 `persona: matrix` 误路由到 Matrix 全局上下文。
- 已把自管维护提示收紧为“不要填 persona，默认作用于自己的角色上下文”。
- 已补充测试覆盖：返回阈值提示、连续工具抬预算、`context_vision` 执行、角色治理投影、动态角色 base persona 误填兜底路由。
- 已完成本轮验证：`cargo test --no-run`、`cargo test tool_manage_can_create_summary_compact_role_and_route_maintenance_to_itself -- --nocapture`、`cargo test context_vision_sets_next_round_visibility_budget -- --nocapture`、`cargo test self_compact_governance_generates_compact_ticket_instead_of_advisor_text -- --nocapture`、`cargo build --release`、`git diff --check`。

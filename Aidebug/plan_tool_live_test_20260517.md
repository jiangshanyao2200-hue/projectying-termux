# Plan Tool Live Test · 2026-05-17

## 目标

继续测试 `update_plan` 在真实工作里的行为，不只看 schema 是否能解析。

本轮重点：

- 三种计划模式是否都能被模型稳定调用：`decision`、`todo/plan`、`blueprint`。
- `decision` 是否只记录决策，不触发复杂任务门禁。
- `todo/plan` 是否能表达当前执行步骤，但不阻断 `persona_manage`、`command`、`apply_patch` 等后续动作。
- `blueprint` 是否适合完整计划，不会让 UI 大幅卡顿或把模型带进“只计划不执行”。
- `update_plan` 工具回执是否默认折叠，避免占用正文空间。
- plan 调用时是否还会造成明显 `ui.tick elapsed_ms` 异常。

## T14 实战任务

让 Matrix 做一个安全、可回滚的小实战：

1. 用 `decision` 模式判断任务范围。
2. 用 `todo/plan` 模式列出执行步骤。
3. 用 `blueprint` 模式给出完整蓝图。
4. 实际检查 `~/snake.html` 是否具备 canvas、开始按钮、方向控制和本地脚本。
5. 生成一份最小报告到 `Aidebug/artifacts/plan_tool_snake_probe.md`。
6. 最终回报每种计划模式是否调用、实际工作是否完成、是否遇到卡顿或门禁。

## 观察点

- `provider.responses.function_call name=update_plan`
- `tool.start/tool.done tool_name=update_plan`
- `ui.tick elapsed_ms > 120`
- `request.completed final_plan_chars`
- `scheduler.lifecycle` 是否从 PASS 退化
- `latest_reply.txt` 是否准确报告实际产物

## 停止条件

- `update_plan` 后无法继续调用实际工具。
- 明显出现长时间 active_request 但无工具推进。
- UI tick 持续异常或请求无法收尾。

# ProjectYing Live Test Plan

日期：2026-05-16

## 目标

针对当前已启动的萤，做一轮集中测试、集中记录、最后集中修复。
这轮只盯健康度更容易出问题的链路，不做分散式修修补补。

## 当前基线

- `overall_state=PASS`
- `overall_score=100`
- `config.governance=PASS`
- `memory.datememory.buffer=PASS`
- `context.manage=PASS`
- `token.budget=PASS`
- `scheduler.lifecycle=PASS`

## 测试顺序

1. datememory 压力链
- 先把 datememory 推到接近阈值的状态
- 看 `memory.datememory.buffer`、`communication.persona_manage`、`memory.datememory.sql` 是否同步给出一致结论
- 重点看是否会出现只报 buffer、不报入库的断层

2. context / token 链
- 连续投递长输入和大工具回执
- 看 `context.manage`、`token.budget`、`config.governance` 是否还能保持可读回执
- 验证 summary / compact / 折叠不会造成残影式上下文污染

3. scheduler 链
- 观察连接中、重试、超时、取消、恢复
- 看 `scheduler.lifecycle` 是否能正确识别“卡住但仍在工作”的假活跃

4. settings / governance 链
- 用 Matrix 执行 `tool_manage settings`
- 复查 `config.governance` 是否仍能单独反映统一 settings 路由
- 顺手测静态 persona 模型切换与工具预算钳制

5. role / UI 链
- 新建、重载、切换动态角色
- 看 `dynamic_role.governance` 和 `ui.contract` 是否退化

6. APK 链路探针
- 让 Matrix 指挥 Coding 产出一个尽量小的独立 APK，只验证构建和安装链路
- 目标是跑通“写代码 -> build -> adb install -> 启动 -> 回执”闭环
- 优先复用现成 Android 构建骨架或最小工程，不把它扩成完整产品
- 重点记录：源码路径、APK 路径、adb 设备、安装结果、启动结果、失败点
- 如果安装成功，再看最小 UI 是否可见和 `logcat` 是否有明显报错

## 记录方式

- 每做完一批测试，立即写入 `Aidebug/round_test_log_20260516.md`
- 发现问题只记事实，不当场扩散修复
- 同类问题合并成一条修复项，避免一边测一边碎修

## 最后修复方式

- 先汇总所有问题，再按影响面排序
- 优先修会污染上下文、误判健康、影响调度的链路
- 修复后统一跑一遍 `cargo check` 和 Aidebug 相关测试
- APK 链路探针完成后，额外记录一次是否影响系统性能或健康度

## 停止条件

- 发现重大 bug
- 某条链路出现 BLOCKED / BROKEN
- 连续测试没有再增加有效信息

## 补充（2026-05-17）

- 已观察到 `update_plan` 会引发一轮明显的整体卡顿，属于需要单独记录的体验问题，不是单纯 UI 小抖动。
- 计划模式建议继续保留三档语义：`decision`、`todo/plan`、`blueprint`，其中 `blueprint` 用于完整计划并按计划执行。
- 下一步实战顺序：先移除本轮创建的测试角色，再在 `~/snake.html` 做一次可跑的本地贪吃蛇实战，观察性能、稳定性和是否还会引入新的健康问题。

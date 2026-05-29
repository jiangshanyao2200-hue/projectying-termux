# Aidebug 链路健康侦测计划

日期：2026-05-15

## 目标

把 Aidebug 从“测试入口”升级成“链路健康侦测器”。它不仅承载调试投递，还要持续暴露并评分 ProjectYing 的核心链路状态，避免重复测试和上下文丢失。

## 核心链路

1. persona 基础链路
- Matrix / Advisor / Coding / Server 的基础状态是否可用
- 动态角色是否具备独立 session、context、memory、tool、UI contract

2. 动态角色治理链路
- `tool_manage role_list / role_reload`
- 角色 enabled / visible / tools / storage / label / topbar contract
- Matrix 新建角色后是否完整纳入治理目录

3. communication 链路
- `persona_manage send / observe / interrupt`
- worker 正文是否只落到 worker 侧
- Matrix 普通聊天是否被污染

4. memory 链路
- `memory_add / memory_check / memory_read`
- `datememorycontext.json` 是待写入 SQL 的临时日记缓冲区，不应长期作为模型上下文堆积
- `memory_add target=datememory clear_context=true` 是否能把缓冲内容按日写入 `datememory.db` 的 `datememory_entries`
- `datememorycontext` 是否可观测，且超限时是否明确触发“SQL 日记入库 + 清空缓冲”维护路径
- `memory/output` 外部化输出协议是否可回收

5. context 链路
- `context_manage`
- Matrix / role / focus context 是否分区清楚
- 压力收口是否闭环

6. token 链路
- `token.budget`
- 当前轮输入 token、会话输入 token、上下文条目数、datememory over-limit 是否共同进入健康判断
- 高压时是否提示优先 `context_manage summary`、退出 focus、避免重复 observe / worker ping

7. tool 链路
- `mcp.rs` 的工具 schema / dispatch / 回执 / 投影
- tool 管控是否仍是单一入口

8. config 链路
- `tool_manage.settings`
- Matrix-only 统一 settings 路由
- 工具预算、系统体验参数、静态 persona 模型切换是否独立成链

9. UI 链路
- 标签页、状态栏、输入状态栏、颜色、标题、badge 是否一致
- `supports_topbar=false` 的动态角色是否不再默认带 topbar

10. 调度链路
- active request / focus / system request / queue / provider phase
- 是否能稳定退出和恢复

## 健康评分

每条链路给出：
- `PASS`
- `DEGRADED`
- `BLOCKED`
- `BROKEN`

建议分值：
- PASS = 100
- DEGRADED = 70
- BLOCKED = 40
- BROKEN = 0

总分按加权平均，以下为权重点：
- persona 基础：15
- 动态角色治理：20
- communication：15
- memory：15
- context：15
- token：10
- tool：10
- config：5
- UI：5
- 调度：5

## 侦测器规则

1. 每次 Aidebug 请求结束时自动记录一条健康快照。
2. 每条链路必须可由一个最小证据点证明，不允许只写结论。
3. 若出现以下任一情况，标记为 `BLOCKED` 或 `BROKEN`：
- worker 正文进入 Matrix 普通聊天
- role reload 后 contract 退化
- `memory_read target=output` 无法按协议读取
- `datememory` 压力字段无法在 observe/status 中读取
- `datememorycontext` 超限但没有生成或执行 SQL 日记入库维护
- focus 无法退出

## 当前链路清单

- Matrix persona
- Advisor persona
- Coding persona
- Server persona
- worker dynamic role
- role governance reload
- persona communication route
- memory read/add/check
- context manage
- token budget
- tool manage
- config governance / tool_manage settings
- UI topbar/status/input status
- output recall
- focus lifecycle
- datememory pressure
- datememory SQL diary writeback

## 本轮已验证结果

- worker dispatch / memory / context / pollution：PASS
- role reload 后 contract 稳定：PASS
- output recall 协议：PASS
- datememory pressure 可观测性：已修复 observe 链路，待重启后回归确认
- token.budget 健康链：已加入 `health.json`，用于评估 context/focus 治理是否带来收益或只增加轮询成本
- config.governance 健康链：已加入 `health.json`，用于单独观测 `tool_manage settings` / Matrix-only 统一 settings 路由
- scheduler.lifecycle 负例：`connecting` 且无工具推进时会明确降级，避免“卡住但看起来在工作”的误判

## 后续执行方式

- 每次测试前先读本计划和 `AI笔记.txt`
- 每次测试后立即追加记录
- 发现重大 bug 直接切修复，不继续扩测
- Aidebug 负责健康观测，Matrix 负责调度和治理

## 下一轮 live 测试顺序

1. datememory buffer 压力回放
- 人为把某个 persona 的 datememory 推到阈值附近
- 验证 `memory.datememory.buffer`、`communication.persona_manage`、`memory.datememory.sql` 是否一起给出一致结论

2. context / token 收口
- 连续投递长输入和大工具回执
- 验证 `context.manage`、`token.budget`、`config.governance` 是否共同维持可读回执

3. scheduler / reconnect
- 观察连接中、重试、超时、取消
- 验证 `scheduler.lifecycle` 是否能从 DEGRADED 回到 PASS

4. settings 治理链
- 用 Matrix 执行 `tool_manage settings`
- 验证 health 里的 `config.governance` 能直接反映矩阵统一治理是否还在

5. 角色治理链
- 新建 / 重载 / 切换动态角色
- 验证 `dynamic_role.governance` 和 `ui.contract` 不退化

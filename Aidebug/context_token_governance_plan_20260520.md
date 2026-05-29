# ProjectYing 上下文阈值与 Token 计量统一计划

日期：2026-05-20

## 目标

把当前 KB-centric 的上下文治理，统一成 token-centric 的主判据，M 只作为人类可读的展示单位。

本轮会同步做对账、文档收口和代码落地，不再停留在只写规划。

## 这次要改的核心

1. 精细化 token 统计
- 统计口径改成 OpenAI 风格的细粒度 token 账本。
- `Per` 只统计“本次启动、本角色”的消耗，不再显示全局 `A` 总账。
- `N` 只显示当前请求/本轮输入输出。
- `Ctx` 和 `Date` 要带百分比、当前值、上限值。
- `Ctx` 的数值要把 system prompt、schema、fastmemory、工具投影和实际注入的上下文一起算进去。

2. 阈值统一
- `context_manage` 走可配置软阈值。
- `context_summary` / `context_vision` 走自管理，不依赖软阈值触发。
- 全局硬兜底保留：`20w input token` 和 `1.0M` 级上下文，任一触达就强制交给司做全量 compact。
- `context_manage` 的默认角色 cap 走 `0.8M / 1.0M`，Matrix 可把单角色改成更小的值，比如 `0.5M`，到点就直接暂停会话让司全量 compact。
- `datememory` 单独保底，固定按 `0.2M` 触发维护，不再作为普通可调参数。

3. 主动治理提示
- `context_manage`、`context_summary`、`context_vision`、`context_compact` 的 schema 描述都要带主动治理提示。
- 工具返回的底部也要带同一条提示，类似：`本上下文由轮询发送，请优先判断是否需要整理上下文。`
- 目标不是强制模型服从，而是把“主动整理上下文”变成默认习惯。

4. 状态栏收口
- 删除旧的 `○ A ↑... ↓...` 总量段。
- 改成类似：

```text
✲ Per 4.7W / 2 ● N ↑0.1m ↓0.0m Ctx 20% 0.2/1.0M Date 50% 0.1/0.2M
```

## 现状对账

当前相关真源：
- `src/main.rs`：状态栏、token 账本、`context_max_size_kb`、`memory_context_limit_kb`
- `Aidebug/aidebug.rs`：`status.json`、`health.json`、token/ctx/datememory 健康链
- `src/roles.rs`：`context_governance_set`、`advisor_managed / summary_compact / vision_compact`
- `Aidebug/context_governance_tools_plan.md`：旧的 KB 阈值蓝图
- `Aidebug/tool_manage_unified_settings_plan.md`：统一设置入口蓝图

当前问题：
- token 和 KB 都在算，但对外展示仍偏 KB。
- `current_round_input_tokens` 还是估算值为主。
- `total_*` 还在前台状态中出现，太吵。
- `context_manage`、`summary`、`vision`、`datememory` 的阈值边界没有统一成一套规则。

## 统一口径

### 主判据

- 主判据是 token，不是 KB。
- `M` 只是展示单位，用来给人看压力，不做唯一决策依据。
- 任何强制 compact / 强制交给司 的判断，最终都要落到 token 级别。
- JSON 字节数只做近似触发，不能和 token 口径混为一谈。

### 统计对象

- `Per`：当前 persona 在本次进程启动后的累计输入/输出。
- `N`：当前轮请求的输入/输出。
- `Ctx`：当前可见上下文的有效装载量，包含 system prompt、schema、fastmemory、工具投影、以及实际注入模型的上下文内容。
- `Date`：当前 `datememorycontext` 缓冲的装载量。
- `Ctx` 和 `Date` 只看压力展示，不要把 JSON 字节数当成主判据。

### 计量原则

- 优先使用真实 provider usage / 真实 token 计数。
- 请求发出前的预估，改成模型感知的 token 估算；只有 tokenizer 缺失时才回退到字节/4。
- `thinking / reasoning` 只要进入 AI 上下文，就算入 `Ctx` 和 `Per`。
- `tool schema`、`prompt schema`、`fastmemory` 不能漏算。

## 阈值矩阵

### 1. `context_manage`

- 这是精密治理路径。
- 默认可配置软阈值，常规默认 `0.8M`，高级调节建议收敛在 `0.2M ~ 0.8M`。
- 硬 cap 固定默认 `1.0M`，状态栏右侧分母显示硬 cap；百分比按软阈值算压力。
- 超过软阈值后，交给司做 `context_manage` 精细收口。
- 这个阈值主要服务 `advisor_managed` 路线。

### 2. `summary_compact` / `vision_compact`

- 这是自管理路线。
- 不建议用软阈值强行打断。
- 角色自己用 `context_summary` / `context_vision` 做局部治理。
- 只有触发全局硬兜底时，才升级给司全量 compact。

### 3. 全局硬兜底

- `20w input token` 封顶。
- `1.0M` 上下文封顶。
- 任一命中，立刻强制交给司做全量 compact。
- 落地动作用现有全量 compact 路由，优先复用现成 `context_manage / context_compact` 能力，不另造平行工具。

### 4. `datememory`

- `datememorycontext` 单独做阈值。
- 固定默认：`0.2M` 触发维护，不再走普通设置页自定义。
- 目标是让司把过渡缓冲整理成 `datememory.db`，而不是继续堆在上下文里。

## 配置入口

统一入口继续走 `tool_manage context_governance_set`，但字段要升级成 token / M 兼容结构：

- `mode`
- `context_soft_limit`
- `context_hard_limit`
- `report_to_matrix`

要求：
- 角色创建时就能一次性写入。
- Matrix 可对静态 persona、dynamic role 统一配置。
- UI 上显示 `M`，内部保存 token 归一值。
- 旧 `manage_threshold_kb / compact_threshold_kb` 只做迁移兼容，不再作为最终主口径。

## 状态栏目标

把现在的：

- `○ A ↑... ↓...`

收掉，改成：

- `Per`：当前 persona 的启动累计
- `N`：本轮请求
- `Ctx`：当前上下文压力
- `Date`：datememory 压力

推荐显示格式：

```text
✲ Per 4.7W / 2 ● N ↑0.1m ↓0.0m Ctx 20% 0.2/1.0M Date 50% 0.1/0.2M
```

## Aidebug 目标

### `status.json`

- 增加 token 级别证据。
- 显示 `Per / N / Ctx / Date` 四块。
- 继续保留 `context_limit_kb`、`datememory_context_limit_kb` 作为过渡兼容字段，但不再作为主展示。

### `health.json`

- `token.budget` 改成 token 主判据。
- `context.manage` 只看可配置软阈值。
- `datememory.buffer` 独立判定。
- 不再让 `total_*` 成为主要告警依据。

## 实施顺序

1. 先把 token 账本做准
- 对齐请求级、会话级、persona 启动级的 token 字段。
- 让 `Ctx` / `Date` 的百分比能稳定算出来。

2. 再改状态展示
- 去掉 `A` 总量段。
- 加 `Per / N / Ctx / Date`。

3. 再统一治理配置
- 把 `context_governance_set` 迁到 token / M 结构。
- 保持旧 KB 字段兼容，后续再清理。

4. 最后打硬兜底
- `20w input token`
- `1.0M` 上下文
- `0.2M datememory`

## 当前执行状态

- [x] `Per / N / Ctx / Date` 的状态栏和 token 账本已经接上。
- [x] `context_manage / context_summary / context_vision / context_compact` 的 schema 和回执底部已经加上主动治理提示。
- [x] `Ctx` 统计已经把 provider 输入和本地 tool schema 计入。
- [x] `Ctx` 状态栏分母改为硬 cap `1.0M`，百分比继续按软触发阈值算。
- [x] `Date` 固定到 `0.2M`，状态和 health 不再读旧的 `0.5M`/自定义值。
- [x] `cargo test -- --test-threads=1` 已通过。
- [ ] 继续把 token 精度从估算推进到更接近真实 provider usage。
- [ ] 统一旧 KB 蓝图文档的引用，避免不同文档给出冲突默认值。

## 验证清单

- `Per` 是否只统计本次启动本角色。
- `N` 是否只统计当前轮。
- `Ctx` 是否包含 prompt、schema、fastmemory、thinking/reasoning。
- `Date` 是否能稳定触发入库，不再只看 KB。
- `summary_compact` / `vision_compact` 是否不被软阈值误打断。
- 硬兜底是否在所有角色上生效。
- 状态栏是否彻底移除 `A` 总量段。

## 回滚原则

- 先保留旧 KB 字段兼容一轮。
- 先隐藏再删除，不直接硬删数据结构。
- 如果 token 统计暂时拿不到真实值，先降级为明确标注的 estimate，不允许静默回退到旧 KB 口径。

## 本轮交付定义

- 这份计划本身先作为执行蓝图。
- 后续实现必须以它为准，逐项对账：
  - token 账本
  - 阈值矩阵
  - 状态栏
  - Aidebug 健康链
  - `context_governance_set` 统一入口

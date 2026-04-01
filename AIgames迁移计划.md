# Companioning 接入 ProjectYing 对账计划

本文件只做一件事：

先把 `AIgames` 接入 `ProjectYing` 的边界、阶段、落点、配置与代码拆分写清楚，作为后续正式植入的施工图。

本轮仍然是对账与方案阶段，不直接开始迁移代码。

---

## 1. 本轮对账结论

- 顶层新标签页英文名定为：`Companion`
- 顶层设置域目录定为：`companioning`
- 顶层新标签页中文名定为：`汐`
- 顶栏最终形态改为三域并列：
  - `Matrix · 萤`
  - `Coding · 绫`
  - `Companion · 汐`
- `ProjectYing` 继续做唯一宿主壳
- `AIgames` 不整体并壳，只迁“游戏域能力”
- `Coding` 与 `Companion` 都要从主城里拆出独立 `rs` 文件
- `main.rs / ui.rs` 继续保留公共壳层，不重写第二套 TUI
- 每个顶层标签页都要有独立生效的设置页
- `Companion` 需要独立上下文目录、独立提示词目录、独立配置域
- `AIgames` 原始数据目录第一阶段继续挂载，不立即物理搬迁

一句话：不是把 `aigamescenter` 塞进 `ProjectYing`，而是让 `ProjectYing` 长出一个 `Companion` 业务域。

---

## 2. 现状核对

### 2.1 ProjectYing 当前现状

当前 `ProjectYing` 已经具备宿主壳能力：

- 顶层 persona tab
- 聊天区与输入区
- 设置页状态机
- provider 主链
- context 工程
- tool / log / queue / topbar

但当前仍有两个结构前提尚未满足：

- 顶层 persona 目前只有 `Matrix / Coding`
- 设置页目前还是一套总设置，不是真正的“按标签页独立生效”

### 2.2 AIgames 当前现状

当前 `AIgames` 不是一个小模块，而是一整套独立宿主：

- 自己的 `main.rs`
- 自己的 settings 状态机
- 自己的 provider/context
- 自己的大厅/游戏页切换
- 自己的游戏状态机

并且它的设置项明显多于 `ProjectYing` 当前 `Matrix / Coding`。

结论很直接：

- 不能把 `AIgames` 主壳整块嵌进来
- 只能迁移“游戏域逻辑 + 游戏域配置 + 游戏域数据接线”

---

## 3. 命名与目录定案

### 3.1 顶层命名

- 英文域名：`Gaming`
- 中文显示名：`弈`
- Header：`Gaming · 弈`
- 内部域标识：`gaming`

### 3.2 代码文件

正式实施后，顶层域模块收口为：

- `src/main.rs`
  - 宿主编排、总状态机、公共入口
- `src/ui.rs`
  - 公共渲染壳
- `src/mcp.rs`
  - 工具城
- `src/departmentrs.rs`
  - 观测政府
- `src/coding.rs`
  - `Coding` 域定义、上下文入口、设置入口、域级行为
- `src/gaming.rs`
  - `Gaming` 域定义、页面状态、配置入口、AIgames 接线

说明：

- `coding.rs / gaming.rs` 不是第二套主程序
- 它们只承接“域模块”
- 主循环、输入总线、聊天壳、公共状态栏仍留在 `main.rs / ui.rs`

### 3.3 上下文目录

正式定案为：

- `context/codex/`
- `context/codexcoding/`
- `context/codexgaming/`

其中 `Gaming` 目录第一阶段至少包含：

- `context/codexgaming/codexprompt.txt`
- `context/codexgaming/fastmemory.json`
- `context/codexgaming/fastcontext.json`
- `context/codexgaming/context.json`
- `context/codexgaming/focuscontext.json`
- `context/codexgaming/contextmeta.json`
- `context/codexgaming/schema/`

### 3.4 Gaming 提示词目录

`Gaming` 不只是一份总 prompt，还要预留游戏专项提示词区。

建议结构：

- `context/codexgaming/codexprompt.txt`
- `context/codexgaming/games/roulette/`
- `context/codexgaming/games/zhajinhua/`
- `context/codexgaming/games/wolfkill/`
- `context/codexgaming/games/lobby/`

第一阶段这些目录可以先只是规划位，不强制立刻搬旧提示词文件。

---

## 4. 宿主与业务域边界

### 4.1 宿主壳负责什么

永远由 `ProjectYing` 负责：

- 顶栏与 persona tab
- 输入框
- 聊天区
- 顶部任务面板
- 系统消息
- tool 卡片
- 主事件循环
- 焦点切换
- 公共设置壳
- provider 总请求节拍

### 4.2 Gaming 域负责什么

由 `Gaming` 域负责：

- 游戏大厅状态
- 游戏选择与二级导航
- AIgames 配置接线
- 游戏运行时状态
- 游戏专属事件
- AIgames provider/context 兼容层
- 旧数据目录挂载

### 4.3 明确不做的事

第一阶段明确不做：

- 不把 `AIgames/aigamescenter` 的主循环一起迁入
- 不保留 AIgames 的第二套顶栏/状态栏/输入框
- 不立刻物理搬迁 `~/AIgames/roulette` 等旧数据目录
- 不一上来就统一所有 provider/context 存储

---

## 5. 设置系统的目标形态

这是本次迁移的真正前置工程。

### 5.1 总原则

每个顶层标签页都要有独立生效的设置页。

不是“能打开不同页面”就算完成，而是：

- `Matrix` 的设置只影响 `Matrix`
- `Coding` 的设置只影响 `Coding`
- `Gaming` 的设置只影响 `Gaming`

### 5.2 推荐结构

宿主保留设置壳，但设置内容按域分仓：

- `config/matrix/`
- `config/coding/`
- `config/gaming/`
- `config/shared/`（如确有必要）

第一阶段为了兼容现有结构，可以先做成：

- `config/api.json`
- `config/system.json`
- `config/context.json`
- `config/theme.json`
- `config/coding/`
- `config/gaming/`

其中：

- `Matrix` 继续吃宿主当前配置
- `Coding` 先拥有独立配置域
- `Gaming` 先拥有独立配置域

### 5.3 Coding 与 Gaming 的设置策略

用户要求是：

- `Coding` 和 `Gaming` 都有独立设置页
- 前期可以沿用 `Matrix` 设置页的壳与交互

因此推荐这样落地：

#### Matrix

- 继续沿用当前完整设置页

#### Coding

- 第一阶段：复用 Matrix 的设置 UI 壳
- 只开放基础 API 相关项
- 读写到 `config/coding/`

#### Gaming

- 第一阶段：同样复用设置 UI 壳
- 先提供基础 API 设置
- 再逐步接入 AIgames 的 `players / user / theme / games`

也就是：

- 先把“设置是独立生效的”这件事做对
- 再把 `Gaming` 的复杂设置项完整迁进来

### 5.4 Gaming 配置拆分定案

`Gaming` 最终推荐拆成：

- `config/gaming/api.json`
- `config/gaming/players.json`
- `config/gaming/user.json`
- `config/gaming/theme.json`
- `config/gaming/games.json`

必要时可补：

- `config/gaming/index.json`

用于版本号、迁移标记、目录总览。

---

## 6. AIgames 数据接入策略

### 6.1 第一阶段：挂载旧目录

第一阶段不搬旧目录，直接继续使用：

- `~/AIgames/Matrix/`
- `~/AIgames/roulette/`
- `~/AIgames/zhajinhua/`
- `~/AIgames/wolfkill/`
- `~/AIgames/config/api.json`

原因：

- 风险最低
- 回滚最简单
- 便于验证 `Gaming` 壳层接线

### 6.2 第二阶段：迁入 ProjectYing

等 `Gaming` 壳、设置页、至少一个游戏完整跑通之后，再考虑把数据迁到：

- `projectying/context/codexgaming/games/...`
- `projectying/config/gaming/...`
- `projectying/log/gaming/...`

这一步不是当前首要任务。

---

## 7. 代码植入的正确顺序

### 阶段 0：备份

正式施工前先备份整个 `projectying`。

目标：

- 备份到 `backup/`
- 保证任何一步接入失控都能秒回滚

### 阶段 1：先把“persona”升级成“域模块”

先做结构，不先碰 AIgames。

本阶段做：

- 为 `Coding` 抽 `src/coding.rs`
- 新建 `src/gaming.rs`
- 在主城里建立统一的域模块入口
- 让 persona 不再只是一组枚举文案，而是一个真正可扩展的域定义

验收：

- `Matrix / Coding` 不回归
- `Gaming` 顶层 tab 可以先空壳显示

### 阶段 2：做“按域独立生效”的设置容器

这是最关键的一步。

本阶段做：

- 给宿主设置页加域路由
- `Matrix / Coding / Gaming` 都能进入各自设置页
- `Coding / Gaming` 可先复用当前设置渲染壳
- 配置写入开始支持分域目录

验收：

- 不同域设置不互相污染
- `Coding / Gaming` 至少有基础 API 配置可生效

### 阶段 3：接入 Gaming 域骨架

本阶段只接宿主层，不接游戏逻辑。

本阶段做：

- 顶层新增 `Gaming · 弈`
- `src/gaming.rs` 内建立页面状态
- 设计 `Gaming` 内部二级导航

推荐二级结构：

- `大厅`
- `游戏`
- `设置`

若后续游戏变多，再扩成：

- `大厅`
- `轮盘`
- `金花`
- `狼人杀`
- `设置`

验收：

- 顶栏切换稳定
- `Gaming` 内部页面切换稳定

### 阶段 4：迁入 AIgames 设置

本阶段开始真正接 AIgames，但先接设置，不接整套运行逻辑。

本阶段做：

- 把 AIgames 原 `api / players / user / theme / games` 设置迁入 `Gaming`
- 做旧配置兼容读取
- 新配置优先写入 `config/gaming/*.json`

验收：

- `Gaming` 的设置页可完整工作
- 不影响 `Matrix / Coding`

### 阶段 5：接入游戏大厅

本阶段做：

- 挂载 `AIgames/Matrix/` 旧大厅数据
- 接入 `Gaming` 的大厅页
- 统一为 `ProjectYing` 的消息流显示

验收：

- `Gaming` 大厅可运行
- 可在任何时刻切回其它顶层标签页

### 阶段 6：先只接一个游戏

先接 `roulette`，不三线并发。

本阶段做：

- 接 `roulette` 配置页
- 接 `roulette` 运行态
- 接 `roulette` 的 provider/context/debug

验收：

- 最小流程能完整跑通一局
- 中途切 tab 不炸状态

### 阶段 7：再接 `zhajinhua / wolfkill`

此时复用已验证过的壳层协议。

本阶段做：

- `zhajinhua` 接入
- `wolfkill` 接入
- 统一游戏事件映射

验收：

- 三个游戏都跑在一套宿主壳中

### 阶段 8：最后才做目录内聚

前面全部稳定后，才考虑：

- 把旧 `AIgames` 数据目录迁入 `ProjectYing`
- 清理兼容桥
- 最终收口

---

## 8. 推荐的 Gaming 内部结构

### 8.1 代码侧

`src/gaming.rs` 建议承接：

- `GamingState`
- `GamingPage`
- `GamingSettingsState`
- `GamingConfigPaths`
- `GamingCompatBridge`
- `GameKind`
- `GameSessionState`

### 8.2 建议的页面枚举

```rust
enum GamingPage {
    Lobby,
    Games,
    Settings,
}
```

后续如果三游戏直接上顶栏内页，再扩：

```rust
enum GamingPage {
    Lobby,
    Roulette,
    Zhajinhua,
    Wolfkill,
    Settings,
}
```

### 8.3 与主城的接口

`main.rs` 不直接理解每个游戏细节，只理解：

- 当前 active domain 是谁
- 当前 domain 需要渲染什么
- 当前 domain 需要什么设置页
- 当前 domain 把什么事件投递给聊天区/状态栏

也就是：

- 游戏规则不要继续塞进主城
- 主城只接 domain 输出

---

## 9. 需要预先规避的风险

### 9.1 双状态机冲突

最大风险始终不是游戏逻辑，而是：

- 双主循环
- 双输入栈
- 双设置栈
- 双焦点状态机

必须坚持“宿主唯一”。

### 9.2 配置污染

如果不先做分域设置容器，后果会很直接：

- `Gaming` 的 provider 覆盖 `Matrix`
- `Gaming` 的玩家配置污染 `Coding`
- `Gaming` 的主题配置影响宿主全局

### 9.3 迁移范围爆炸

不能同一轮同时处理：

- 顶层 tab
- 分域设置
- game lobby
- 三个游戏
- 旧目录迁移
- provider 总收口

必须按阶段压着做。

---

## 10. 本轮之后的正确开工顺序

下一轮正式施工时，建议严格按这个顺序：

1. 备份 `projectying`
2. 抽 `src/coding.rs`
3. 建 `src/gaming.rs`
4. 扩 persona/domain 结构支持 `Gaming`
5. 改造设置页为“按域独立生效”
6. 先让 `Gaming` 空壳跑起来
7. 再迁 AIgames 设置
8. 再接大厅
9. 再接 `roulette`
10. 最后才接另外两个游戏

---

## 11. 一句话定案

`Gaming` 应该作为 `ProjectYing` 的第三个业务域落地，先完成“域模块化 + 独立设置页 + 空壳接入”，再迁 AIgames 的设置与游戏链路；第一阶段不搬旧数据目录，不保留第二套主壳，不并行接三个游戏。

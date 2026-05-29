# ProjectYing Aidebug Plan · SharedBoard Probe

## 范围

- 本轮只处理 `projectying`，不再沿用 `projectling` 的测试结论。
- 以 `Aidebug/artifacts/sharedboard_gating_probe.md` 的失败为入口。

## 发现

- 实测失败点：动态角色 `sharedprobe` 回报看不到 `fastmemory.public / SharedBoard`。
- 源码复核后发现两层原因：
  - `load_matrix_shared_board_items()` 在动态角色上下文中调用 `with_persona_context(Matrix)`，但没有清掉动态角色路由，可能读到角色自己的 `fastmemory.json`，而不是 Matrix 根目录的共享板。
  - `render_shared_board_block()` 在 public 为空时不输出区块，导致模型无法区分“共享板为空”和“共享板不可见”。

## 修复计划

1. 让 SharedBoard 读取固定走 `context/Matrix/fastmemory.json`，绕开当前动态角色路由。
2. 让动态角色和非 Matrix persona 即使在共享板为空时也能看到 `[SharedBoard]` 空态区块。
3. 增加回归测试：角色先创建、共享板后更新时，角色仍能读取 Matrix 最新 public board，而不是自己的旧快照。
4. 通过定向 `cargo test` 后，再更新本轮 Aidebug 结论。

## 健康标准

- 动态角色 provider/system context 中始终可见 `[SharedBoard]`。
- Matrix 写入 `fastmemory.public` 后，已存在的动态角色下一轮也能读取最新内容。
- 空共享板显示为 empty，不再被模型误判为不可见。

## 执行结果

- 已修复：SharedBoard 读取固定走 Matrix 根目录，不再读动态角色旧快照。
- 已修复：空 SharedBoard 也输出 `[SharedBoard]` 区块并显示 `(empty)`。
- 已新增回归：`dynamic_role_shared_board_reads_live_matrix_public_after_role_bootstrap`。
- 验证通过：`cargo test shared_board -- --nocapture`，2 passed / 0 failed。

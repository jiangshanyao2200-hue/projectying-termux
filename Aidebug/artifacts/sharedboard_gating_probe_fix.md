# SharedBoard Gating Probe Fix

## 结论

- 原探针结论：`SharedBoard / fastmemory.public` 对动态角色不可见。
- 修复后代码路径：PASS by regression test。
- 旧运行进程仍需重启，才会加载新 release 二进制。

## 根因

1. 动态角色 provider/system context 构建时，`load_matrix_shared_board_items()` 通过 `with_persona_context(Matrix)` 读取 Matrix 数据，但动态角色路由仍然存在，可能实际读取 `context/Role_*/fastmemory.json`。
2. `render_shared_board_block()` 对空 public board 返回 `None`，使模型无法区分“共享板存在但为空”和“共享板不可见”。

## 修复

- 新增 `matrix_fastmemory_path()`，SharedBoard 读取固定走 `context/Matrix/fastmemory.json`。
- 空 public board 也渲染：
  - `[SharedBoard]`
  - `owner: Matrix`
  - `public:`
  - `(empty)`
- 新增回归：`dynamic_role_shared_board_reads_live_matrix_public_after_role_bootstrap`。

## 验证

- `cargo test shared_board -- --nocapture`
- 结果：2 passed / 0 failed
- `cargo build --release`
- 结果：release build passed

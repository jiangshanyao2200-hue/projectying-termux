# Status / Context / Token Protocol Plan · 20260521

目标：把 `status`、`Aidebug`、`context`、`datememory`、`token` 的显示口径和触发口径统一起来，避免 “看起来压缩了但数值没降” 这类误判。

## 决策

1. `Per` 只表达当前轮输入 token 压力，分母固定按 `20w` hard cap 计算，格式统一为 `Per XX% XX.XW/N`。
2. `● N` 改成 `● TOKEN`，避免把 token 计量和普通序号混淆。
3. `Ctx` 只展示当前上下文体积和 `1.0M` hard cap，百分比按 hard cap 算；`0.8M` 只作为软触发和 self-manage 提示，不占显示分母。
4. `Date` 只展示百分比和当前 `M` 值，不再显示固定分母。
5. `Aidebug / 开发者AI调试` 标题要完整显示，不允许被固定宽度截断。
6. Aidebug health / performance 共享同一组 token 阈值：warn 16w，hard 20w。
7. 硬阈值命中时，直接暂停会话并交给司做全量 compact；软阈值和一般压缩仍由角色自己处理。
8. 只要内容已经进入 AI 上下文，就应当可以被上下文管理工具收口，包含 thinking / tool / summary / vision 等需要被管理的部分。

## 执行清单

- [x] 调整底部 status line 的 `Per / TOKEN / Ctx / Date` 格式。
- [x] 统一 Aidebug token warn / hard 阈值。
- [x] 放宽 Aidebug debug 标题宽度，避免中文尾字被裁掉。
- [ ] 复测 `status.json`、`health.json`、`performance.json` 的阈值与证据字段。
- [ ] 复测 Aidebug 投递后的 status 变化是否按当前 persona 路由。
- [ ] 复测 20w token、0.8M ctx、200KB datememory 的触发分界是否与显示一致。
- [ ] 如果后续发现 `thinking` 没有进入可管理上下文，再单独补上下文链路，不在显示层硬补。

## 对账原则

- 显示层只负责清晰表达，不负责偷偷改阈值。
- 触发层只认 hard cap 和 soft trigger，不依赖 kb 视觉误差。
- `status` 的实时值必须来自当前运行态，`Aidebug` 的快照必须能反映同一份状态源。

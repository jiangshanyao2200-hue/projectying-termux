# Shared FastMemory Board Upgrade

## Goal
- Add a Matrix-maintained shared board for cross-role coordination.
- Make new dynamic roles read Matrix, shared roles, and cooperation state from startup/system prompts.
- Keep shared board data in `fastmemory.public`, editable through `context_manage target=fastmemory section=public`.
- Reduce plan-time friction so `update_plan` does not block ordinary `command` execution.

## Implemented
- `src/main.rs`
  - Added `FastMemorySection::Public`.
  - Added `fastmemory.public` storage and ID normalization.
  - Injected Matrix shared board into provider system sections for non-Matrix/dynamic roles.
  - Rendered public fastmemory in snapshots and maintenance tickets.
- `src/mcp.rs`
  - Extended `context_manage` schema to advertise `public`.
  - Extended `parse_fastmemory_section()` for `public/shared/board` aliases.
  - Allowed `command`, `context_manage`, `context_summary`, `context_compact`, and `context_vision` through the post-plan recovery gate.
- `src/roles.rs`
  - Expanded default dynamic-role prompt to mention Matrix as the mother controller.
  - Mentioned the shared board and cross-role reporting rules.
  - Added `public` to default fastmemory bootstrap JSON.

## Notes
- The shared board is Matrix-owned. Other roles should consume it, not casually rewrite it.
- `public` is still part of the same `fastmemory.json`, but the provider now treats it as a shared coordination surface.
- The plan gate change is intentionally narrow: it removes the `command` / context-governance friction observed in T14 without dropping the rest of the recovery guard.

## Follow-up
- Revisit any prompt templates that still describe roles as isolated single-context actors.
- If shared board growth becomes noisy, add a separate maintenance threshold for `fastmemory.public`.

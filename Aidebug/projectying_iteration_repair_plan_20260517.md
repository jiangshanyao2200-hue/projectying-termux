# ProjectYing Iteration Repair Plan · 2026-05-17

## Scope

- Target: `AItermux/projectying`.
- Source of truth: live Aidebug notes, Matrix/Coding/Advisor pressure logs, and current source review.
- Out of scope: `AITranslatorOverlay` feature repair. The translator APK was only a delegation-chain probe.

## Observed Signals

- `context_vision`, `context_summary`, `context_compact`, SharedBoard, dynamic role routing, and toolmemory externalization already have passing regression notes.
- Live performance shows repeated `ui.tick_slow` at about 220-300 ms, including idle moments after requests complete.
- `write_status()` currently refreshes the full tool projection snapshot during every status write. That snapshot rebuild walks every persona/role toolbox and serializes a large schema, so it is the strongest structural source of periodic UI stalls.
- Dynamic-role idle status can surface as `<role> · ● Ready`, which reads like debug residue rather than useful task state.
- `update_plan` executes quickly, but its UI preview can still render a full plan/blueprint body and amplify redraw cost. The tool should remain a compact coordination marker by default.
- Aidebug health currently marks `dynamic_role.governance` degraded when no dynamic role exists. Empty registry is a valid idle state after role cleanup and should not look like a failure.

## Repair Sequence

1. Stabilize Aidebug cost:
   - Throttle tool projection snapshot refresh independently from `status.json`.
   - Keep status and health live, but stop rebuilding heavy tool projection data every second.
   - Preserve first-write behavior so a missing snapshot is still created immediately.

2. Clean UI signal:
   - Stop prefixing the idle `● Ready` line with dynamic-role names.
   - Keep role prefixes for real status/toast/progress lines so task feedback remains localized.
   - Keep active role identity in the header/resource line instead of duplicating it in idle status.

3. Make plan tool lightweight:
   - Keep `decision`, `plan/todo`, and `blueprint` modes.
   - Clamp plan input preview and rendered detail to a small, structured summary.
   - Ensure `update_plan` remains a planning aid and does not become a large chat block.

4. Reduce false health alarms:
   - Treat zero dynamic roles as a PASS idle state.
   - Keep real broken states for invalid enabled/visible role counts.

5. Verify:
   - Run targeted tests for Aidebug status/projection, dynamic role status, update_plan, and context/tool projection regressions.
   - Run `cargo check` or a broader `cargo test` if targeted tests pass.

## Health Goal

- Idle ProjectYing should not show periodic tick stalls caused by debug snapshot generation.
- Matrix/Coding/Advisor chain state should remain observable through Aidebug without making the UI feel blocked.
- The UI should distinguish real working state from passive identity labels.
- The code path should move toward a clearer "city structure": Aidebug owns debug snapshots, `App` owns runtime orchestration, and chat rendering stays bounded.

## Execution Result

- Added throttling for `tool_projection_snapshot.json` refresh from `write_status()`: status/health remain live, heavy projection rebuild is limited and still runs immediately when the snapshot is missing.
- Dynamic-role idle status now stays `● Ready`; role labels remain on real progress/toast lines.
- `update_plan` keeps `decision` / `plan` / `blueprint`, but large blueprints and long step lists are folded in command/UI previews.
- Empty dynamic-role registry is now treated as healthy idle instead of degraded.
- Focus-gate recovery now exposes `focus_mode`, `command`, and `persona_manage`, so a plan gate does not strand Matrix away from execution tools.
- Updated regression coverage:
  - empty dynamic-role registry health is PASS;
  - dynamic-role idle status has no debug-like role prefix;
  - large blueprint plan preview folds;
  - Coding schema includes the current `context_vision` tool.
- Verification:
  - `cargo test -q`: 530 passed / 0 failed.

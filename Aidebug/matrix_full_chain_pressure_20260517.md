# Matrix Full Chain Pressure · 2026-05-17

## Scope

- Runtime target: ProjectYing / Matrix already running.
- Provider mode observed: `大猫 / gpt-5.5`, user says fast + xhigh enabled.
- Test scope: Matrix, 司/Advisor, Coding, dynamic roles, context governance, memory, tool/output/logging, translator APK crash line.
- Explicitly excluded: Server persona.

## Baseline Snapshot

- `status.json`: Matrix idle, `active_context_kb=148`, `context_limit_kb=200`, `pending_system_requests=0`.
- `health.json`: overall `PASS`, all visible chains PASS.
- Dynamic roles present: `sharedprobe`, `summary_probe`, `vision_probe`.
- Known prior signal in events: `vision_probe` previously saw `[SharedBoard]` but missed a requested marker; this round should retest SharedBoard write/read under pressure.
- Translation app path discovered from previous logs: `/data/data/com.termux/files/home/AITranslatorOverlay`, package `com.projectying.aitranslator`.

## Test Plan

1. Matrix orchestration pressure:
   - Read health.
   - Write SharedBoard public marker.
   - Create or reuse a fast/self-managed role.
   - Send role context_vision/context_summary/context_compact tasks.
   - Coordinate with Coding and Advisor, without touching Server.
2. Advisor governance pressure:
   - Observe Matrix/Coding/dynamic role context state.
   - Validate it can summarize or report governance needs quickly.
   - Validate datememory/fastmemory diary path awareness.
3. Coding real task pressure:
   - Investigate AITranslatorOverlay startup crash using adb/logcat where possible.
   - Build/install/launch if device is reachable.
   - Report crash stack, likely source file, and minimal fix target.
4. Monitor:
   - Poll `status.json`, `health.json`, `performance.json`, and `events.jsonl`.
   - Record any blocked request, repeated retries, slow UI tick/draw, wrong tool routing, missing SharedBoard, tool projection mismatch, context pressure, or translator crash.

## Issues

- Pending. This file is the live issue ledger for this pressure round.

## Results

- Pending.

# Coding Ling Special Test · 2026-05-17

## Scope

- Target persona: Coding · 绫.
- Mother body: Matrix · 萤.
- Advisor/context governance: 司.
- External target app: `/data/data/com.termux/files/home/AITranslatorOverlay`.
- Current reported bug: AI translator app launches or enters translation flow and crashes/records an error.

## Test Intent

This round should test the real delegation path, not a direct Codex-only repair:

1. Matrix receives a task from Aidebug.
2. Matrix delegates the concrete bug repair to Coding.
3. Coding uses its own coding toolchain to inspect, build, and test the app.
4. Matrix observes Coding and summarizes evidence.
5. Advisor/context governance should keep Coding/Matrix context from ballooning.

## Initial Local Observation Before Delegation

- `AITranslatorOverlay` is a standalone folder, not a git repository.
- Existing screenshot artifact `media/screenshot/aito_foreground_main.xml` records this visible error in the app UI:
  - `Can't create handler inside thread Thread[translator-http,5,main] that has not called Looper.prepare()`
- Suspicion: callback or side-effect after `OpenAiTranslatorClient.translate()` runs on `translator-http` thread is touching a UI/Looper-dependent API or otherwise surfacing as an error inside the client wrapper.
- This must be verified by Coding with build/install/logcat if adb is available.

## Issues To Watch

- Whether Matrix truly delegates instead of doing all work itself.
- Whether Coding can see enough shared context from Matrix.
- Whether Coding reports blockers back to Matrix clearly.
- Whether Coding uses command/apply_patch/build/logcat tools in a focused way.
- Whether large logcat/build output is externalized or summarized instead of polluting context.
- Whether Advisor/司 triggers or records context governance if Coding context grows.

## Result Log

- Matrix delegation message submitted:
  - Inbox: `Aidebug/inbox/coding-ling-translator-delegation-20260517.json`
  - Processed as: `Aidebug/processed/1779041048831-coding-ling-translator-delegation-20260517.json`
- Matrix did delegate/observe Coding. Evidence from `events.jsonl`:
  - Matrix used `persona_manage` to send/observe Coding.
  - Coding request count increased from 0 to 5 during this round.
  - Advisor request count increased from 0 to 2, indicating context/memory governance was involved.
- Coding result:
  - Coding identified the same likely root cause: `translator-http` background callback path touching UI/Toast/Handler/Looper-sensitive logic.
  - Coding did not edit code.
  - Coding did not run build/install/logcat.
  - Collaboration issue: Coding gave the right diagnosis but stopped at recommendation, so the repair did not complete through the delegated persona alone.
- Codex intervention:
  - Applied the minimum callback-thread fix in `/data/data/com.termux/files/home/AITranslatorOverlay/src/com/projectying/aitranslator/TranslatorRepository.java`.
  - Final `TranslatorRepository.Callback` delivery is now posted through `Handler(Looper.getMainLooper())`.
  - History writes still happen before delivery, so the UI thread is not used for the whole network/history path.
- Verification:
  - `./build.sh` succeeded.
  - APK produced: `/data/data/com.termux/files/home/AITranslatorOverlay/build/ai-translator-overlay.apk`.
  - `adb install -r build/ai-translator-overlay.apk` succeeded on `emulator-5554`.
  - `adb shell am start -W -n com.projectying.aitranslator/.MainActivity` returned `Status: ok`.
  - `adb shell monkey -p com.projectying.aitranslator -c android.intent.category.LAUNCHER 1` injected one launch event.
  - No `AndroidRuntime`, `FATAL EXCEPTION`, `Looper.prepare`, or `Can't create handler` lines were captured after launch attempts.
- Verification limit:
  - `pidof com.projectying.aitranslator` returned empty after launch attempts.
  - `dumpsys activity` showed multiple `com.projectying.aitranslator` tasks as `visible=false`.
  - Current top activity was not the translator app, so visible foreground UI validation was inconclusive.

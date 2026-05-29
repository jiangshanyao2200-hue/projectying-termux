# ProjectYing External Tools

`tools/external/` is the hot-reload toolbox area managed by Matrix · 萤.

Each enabled tool is described by one manifest:

- `tools/external/<name>.tool.json`
- or `tools/external/<name>/tool.json`

The runtime reloads manifests when Matrix runs `tool_manage list` or `tool_manage reload`, and also when provider schema is built. External tools are visible in persona toolboxes but start closed, so they do not enter provider schema until Matrix opens them for a target persona.

Matrix owns this directory through `tool_manage`. New tools and tool UI changes should normally be made with `tool_manage create/update/remove` for the manifest and `command` / `apply_patch` only for the companion executable scripts. `tool_manage` handles projection; the manifest remains the source of truth for parameters, display text, output policy, and scheduler behavior.

Minimal manifest:

```json
{
  "name": "example_echo",
  "description": "Echo JSON arguments from stdin.",
  "program": "python",
  "args": ["tools/external/example_echo.py"],
  "timeout_secs": 30,
  "parameters": {
    "type": "object",
    "properties": {
      "input": { "type": "string" },
      "brief": { "type": "string" }
    },
    "additionalProperties": false
  }
}
```

Unified manifest fields:

```json
{
  "name": "example_echo",
  "description": "Echo JSON arguments from stdin.",
  "program": "python",
  "args": ["tools/external/example_echo.py"],
  "timeout_secs": 30,
  "parameters": {},
  "display": {
    "title": "Echo",
    "action_label": "Run",
    "brief_template": "Run external echo",
    "collapsed_fields": ["status", "brief", "duration_ms"],
    "expanded_sections": ["input", "summary", "refs"],
    "input_label": "Input",
    "output_label": "Output",
    "max_inline_bytes": 8192,
    "group_key": "external:example_echo",
    "chain_mode": "append",
    "default_collapsed": true
  },
  "output_policy": {
    "max_inline_bytes": 8192,
    "externalize_over_bytes": 16384,
    "preferred_store": "toolmemory"
  },
  "scheduler": {
    "dedupe_key": "external:example_echo",
    "single_flight": true,
    "deadline_ms": 30000,
    "poll_interval_ms": 1500,
    "max_retries": 1,
    "backoff": "none",
    "cancel_scope": "request",
    "cooldown_ms": 60000,
    "payload_policy": "replace_not_stack"
  }
}
```

Runtime contract:

- The tool receives the full JSON arguments on `stdin`.
- `stdout` and `stderr` become the tool receipt.
- `working_dir` defaults to the project root.
- Relative program paths and `working_dir` must stay inside the project root.
- Tool names must match `[A-Za-z_][A-Za-z0-9_]*` and cannot collide with built-in tools.
- Keep new tools closed until a persona intentionally opens them with `tool_manage`.
- New Matrix-authored tools should include `parameters`, `display`, `output_policy`, and `scheduler` when applicable, so provider schema, UI cards, large-output externalization, and retry/deadline behavior stay unified.
- Use `tool_manage create/update/remove` for manifest CRUD. Use `tool_manage open/close/pin/unpin` for persona toolbox projection. Prompt changes belong to `context_manage`, not the external tool manifest.
- If `scheduler.deadline_ms` is set, it is the execution deadline. If `scheduler` is omitted, the runtime falls back to `timeout_secs`. Timeout kills the whole external tool process group, so wrapper scripts must expect abrupt termination.
- Retries must use `payload_policy="replace_not_stack"`; a retry or reload must never append failed payloads to the next request.
- Tool receipts are wrapped into `ToolOutputEnvelope`; large outputs should be returned by reference instead of inline full text.
- Long-running PTY and multiagent logs under `memory/output/` should be inspected with `memory_read target=output` using `latest`, `tail`, `range`, or `since_cursor`; avoid pulling full logs into normal context.

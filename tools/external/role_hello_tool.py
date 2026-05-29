#!/usr/bin/env python3
import json, sys

def main():
    raw = sys.stdin.read().strip()
    data = {}
    if raw:
        try:
            data = json.loads(raw)
        except Exception:
            data = {"raw": raw}
    name = data.get("name") or data.get("raw") or "friend"
    role = data.get("role") or "unknown"
    print(json.dumps({
        "ok": True,
        "tool": "role_hello_tool",
        "message": f"Hello, {name}! from role={role}",
        "input": data
    }, ensure_ascii=False))

if __name__ == '__main__':
    main()

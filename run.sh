#!/data/data/com.termux/files/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

BIN="$ROOT_DIR/target/release/projectying"
SESSION_TITLE="● Project 萤"
rebuild=0
rest=()

# Termux reads the terminal title as the tab/session label. Keep it plain text:
# no SGR styling, no bold, no italic.
printf '\033]0;%s\007' "$SESSION_TITLE"

for arg in "$@"; do
  case "$arg" in
    --rebuild) rebuild=1 ;;
    --help|-h)
      cat <<'EOF'
ProjectYing 启动器

用法：
  run.sh [--rebuild] [-- <projectying args>]
EOF
      exit 0
      ;;
    --)
      shift
      rest+=("$@")
      break
      ;;
    *) rest+=("$arg") ;;
  esac
done

need_build=0
if (( rebuild )) || [[ ! -x "$BIN" ]]; then
  need_build=1
fi

if (( !need_build )) && [[ -f "$ROOT_DIR/Cargo.toml" && "$ROOT_DIR/Cargo.toml" -nt "$BIN" ]]; then
  need_build=1
fi

if (( !need_build )) && [[ -f "$ROOT_DIR/Cargo.lock" && "$ROOT_DIR/Cargo.lock" -nt "$BIN" ]]; then
  need_build=1
fi

if (( !need_build )) && command -v find >/dev/null 2>&1; then
  if find "$ROOT_DIR/src" -type f -name '*.rs' -newer "$BIN" -print -quit 2>/dev/null | grep -q .; then
    need_build=1
  fi
fi

if (( need_build )); then
  echo "[ProjectYing] 检测到源码更新，执行 release 构建..."
  cargo build --release -q
fi

exec "$BIN" "${rest[@]}"

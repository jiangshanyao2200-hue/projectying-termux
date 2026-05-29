#!/data/data/com.termux/files/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

BIN="$ROOT_DIR/target/release/projectying"
BUILD_LOCK_DIR="$ROOT_DIR/target/.projectying-build.lock"
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
  mkdir -p "$ROOT_DIR/target"

  lock_wait_started="$(date +%s 2>/dev/null || echo 0)"
  while ! mkdir "$BUILD_LOCK_DIR" 2>/dev/null; do
    lock_pid="$(cat "$BUILD_LOCK_DIR/pid" 2>/dev/null || true)"
    now="$(date +%s 2>/dev/null || echo 0)"
    if [[ "$lock_pid" =~ ^[0-9]+$ ]] && kill -0 "$lock_pid" 2>/dev/null; then
      if [[ "$now" =~ ^[0-9]+$ && "$lock_wait_started" =~ ^[0-9]+$ ]] && (( now - lock_wait_started > 600 )); then
        echo "[ProjectYing] 等待 release 构建锁超时：pid=$lock_pid" >&2
        exit 1
      fi
      sleep 1
      continue
    fi
    rm -rf "$BUILD_LOCK_DIR" 2>/dev/null || true
  done
  printf '%s\n' "$$" >"$BUILD_LOCK_DIR/pid" 2>/dev/null || true
  cleanup_build_lock() {
    rm -rf "$BUILD_LOCK_DIR" 2>/dev/null || true
  }
  trap cleanup_build_lock EXIT INT TERM

  build_log="$(mktemp "${TMPDIR:-/data/data/com.termux/files/usr/tmp}/projectying-build.XXXXXX.log")"
  if ! cargo build --release -q >"$build_log" 2>&1; then
    if grep -qiE 'Text file busy|os error 26' "$build_log" 2>/dev/null; then
      echo "[ProjectYing] release 构建遇到目标文件占用，切换隔离 target 重试..."
      fallback_target="$ROOT_DIR/target/release-retry-$$"
      rm -rf "$fallback_target" 2>/dev/null || true
      if CARGO_TARGET_DIR="$fallback_target" cargo build --release -q >>"$build_log" 2>&1; then
        tmp_bin="$BIN.tmp.$$"
        cp "$fallback_target/release/projectying" "$tmp_bin"
        chmod 0755 "$tmp_bin" 2>/dev/null || true
        mv -f "$tmp_bin" "$BIN"
        rm -rf "$fallback_target" 2>/dev/null || true
      else
        cat "$build_log" >&2
        rm -f "$build_log" 2>/dev/null || true
        exit 1
      fi
    else
      cat "$build_log" >&2
      rm -f "$build_log" 2>/dev/null || true
      exit 1
    fi
  fi
  rm -f "$build_log" 2>/dev/null || true
  cleanup_build_lock
  trap - EXIT INT TERM
fi

exec "$BIN" "${rest[@]}"

#!/usr/bin/env bash
# 开发态一键启动:Core(14411)+ Vite(14410)+ Tauri 壳。
# 用法: bash scripts/dev.sh          (Ctrl+C 一起退出)
set -euo pipefail
cd "$(dirname "$0")/.."

export TASKASION_DATA_DIR="${TASKASION_DATA_DIR:-$HOME/.taskasion}"
export PYTHONPATH="$PWD/core"

python -m taskasion_core serve &
CORE_PID=$!
trap 'kill $CORE_PID 2>/dev/null || true' EXIT

pnpm tauri dev

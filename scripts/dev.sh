#!/usr/bin/env bash
# 开发态一键启动:Vite(14410)+ Tauri 壳(core 已内置,随壳线程启动,数据默认 ~/.taskasion)。
# 用法: bash scripts/dev.sh          (Ctrl+C 一起退出)
set -euo pipefail
cd "$(dirname "$0")/.."

export TASKASION_DATA_DIR="${TASKASION_DATA_DIR:-$HOME/.taskasion}"

pnpm tauri dev

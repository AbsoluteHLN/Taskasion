#!/usr/bin/env bash
# 启动 Agent 接入的 MCP server(stdio)——由 Claude Code / Codex 配置调用,不要手动交互运行。
# Claude Code: claude mcp add taskasion -- python -m taskasion_core.mcp_server
# 前提: PYTHONPATH 包含本仓库 core/,或已 pip install -e 'core[mcp]'
set -euo pipefail
DIR="$(cd "$(dirname "$0")/.." && pwd)"
export PYTHONPATH="$DIR/core"
# 优先用仓库自带 venv(已装 mcp),否则回落系统 python
PY="$DIR/core/.venv/Scripts/python.exe"
[ -x "$PY" ] || PY="$(command -v python)"
exec "$PY" -m taskasion_core.mcp_server

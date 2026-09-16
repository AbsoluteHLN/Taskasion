"""MCP server stdio 冒烟:initialize → tools/list → task_add → task_plan_today。

用法: python scripts/smoke_mcp.py   (需已安装 mcp 包)
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> None:
    env = {
        **os.environ,
        "PYTHONPATH": str(Path(__file__).resolve().parent.parent / "core"),
        "TASKASION_DATA_DIR": tempfile.mkdtemp(prefix="taskasion-mcp-smoke-"),
    }
    argv = sys.argv[1:]
    cmd = argv if argv else [sys.executable, "-m", "taskasion_core.mcp_server"]
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
        text=True,
        encoding="utf-8",
    )
    assert proc.stdin and proc.stdout

    def send(payload: dict) -> None:
        proc.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
        proc.stdin.flush()

    def recv(want_id: int) -> dict:
        while True:
            line = proc.stdout.readline()
            if not line:
                raise SystemExit("MCP server 意外退出")
            line = line.strip()
            if not line:
                continue
            msg = json.loads(line)
            if msg.get("id") == want_id:
                return msg

    send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
          "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                     "clientInfo": {"name": "smoke", "version": "0"}}})
    init = recv(1)
    server_info = init["result"]["serverInfo"]["name"]
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
    tools = [t["name"] for t in recv(2)["result"]["tools"]]

    send({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
          "params": {"name": "task_add", "arguments": {"title": "MCP 冒烟任务", "priority": "p1"}}})
    added = recv(3)

    send({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
          "params": {"name": "task_plan_today", "arguments": {}}})
    plan = recv(4)

    proc.stdin.close()
    proc.wait(timeout=10)

    print("server:", server_info := init["result"]["serverInfo"])
    print("tools:", tools)
    print("add ->", added["result"]["content"][0]["text"][:120])
    print("plan keys ->", list(json.loads(plan["result"]["content"][0]["text"]).keys()))
    assert len(tools) == 10, f"期望 10 个工具, 实得 {len(tools)}"
    print("MCP_SMOKE_OK")


if __name__ == "__main__":
    main()

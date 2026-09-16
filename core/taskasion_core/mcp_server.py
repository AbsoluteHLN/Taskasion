"""MCP server(stdio)——Agent 管理 Taskasion 任务的标准入口。

接入方式(Claude Code / Codex 等):
    command: python
    args: ["-m", "taskasion_core.mcp_server"]
    env:  TASKASION_DATA_DIR=<数据目录>(与桌面端一致)

设计原则参考 mcp-tasks:工具少而稳,未传字段不动。
"""

from __future__ import annotations

import os
from pathlib import Path

try:
    from mcp.server.fastmcp import FastMCP  # mcp 1.x
except ImportError:  # mcp 2.x 把 FastMCP 改名为 MCPServer,API 兼容本文件的用法
    try:
        from mcp.server.mcpserver import MCPServer as FastMCP
    except ImportError as exc:  # pragma: no cover
        raise SystemExit("缺少依赖:请先 pip install mcp (或 pip install -e 'core[mcp]')") from exc

from .audit import Audit
from .goals import GoalStore
from .store import TaskStore

mcp = FastMCP("taskasion")

_store: TaskStore | None = None
_goals: GoalStore | None = None
_audit: Audit | None = None


def _deps() -> tuple[TaskStore, Audit]:
    global _store, _audit
    if _store is None or _audit is None:
        data_dir = Path(os.environ.get("TASKASION_DATA_DIR") or (Path.home() / ".taskasion"))
        _audit = Audit(data_dir / "audit.jsonl")
        _store = TaskStore(data_dir, audit=_audit)
    return _store, _audit


def _goals() -> GoalStore:
    global _goals
    if _goals is None:
        _, audit = _deps()
        data_dir = Path(os.environ.get("TASKASION_DATA_DIR") or (Path.home() / ".taskasion"))
        _goals = GoalStore(data_dir, audit=audit)
    return _goals


@mcp.tool()
def task_add(title: str, due: str = "", priority: str = "", tags: list[str] | None = None, note: str = "") -> dict:
    """新增待办。due 格式 YYYY-MM-DD(可空);priority 为 p1/p2/p3(可空);tags 为标签列表(可空);note 为备注(可空)。"""
    store, _ = _deps()
    return store.add(
        title,
        due=due or None,
        priority=priority or None,
        tags=tags,
        note=note or None,
        source="agent:mcp",
    ).to_dict()


@mcp.tool()
def task_list(status: str = "todo", tag: str = "") -> list[dict]:
    """列出任务。status: todo/done/all,默认 todo;tag 可选过滤。"""
    store, _ = _deps()
    return [t.to_dict() for t in store.list(status=status, tag=tag or None)]


@mcp.tool()
def task_update(
    task_id: str,
    title: str = "",
    due: str = "",
    priority: str = "",
    tags: list[str] | None = None,
    note: str = "",
) -> dict:
    """更新任务。只传需要修改的字段,未传字段保持不变;清空 due/note 请传 'none'。"""
    store, _ = _deps()
    fields: dict = {}
    if title:
        fields["title"] = title
    if due:
        fields["due"] = None if due == "none" else due
    if priority:
        fields["priority"] = priority
    if tags:
        fields["tags"] = tags
    if note:
        fields["note"] = None if note == "none" else note
    return store.update(task_id, fields, actor="agent:mcp").to_dict()


@mcp.tool()
def task_complete(task_id: str) -> dict:
    """勾选完成任务。"""
    store, _ = _deps()
    return store.set_done(task_id, True, actor="agent:mcp").to_dict()


@mcp.tool()
def task_reopen(task_id: str) -> dict:
    """把已完成任务回退为待办。"""
    store, _ = _deps()
    return store.set_done(task_id, False, actor="agent:mcp").to_dict()


@mcp.tool()
def task_delete(task_id: str) -> dict:
    """删除任务。"""
    store, _ = _deps()
    store.delete(task_id, actor="agent:mcp")
    return {"ok": True}


@mcp.tool()
def task_plan_today() -> dict:
    """今日规划:返回 {today, overdue, today_tasks, next}。"""
    return _deps()[0].plan_today()


@mcp.tool()
def goal_add(title: str) -> dict:
    """新建目标(长期意向)。返回目标 dict(含 progress 进度)。"""
    return _goals().add(title, source="agent:mcp").to_dict()


@mcp.tool()
def goal_list(status: str = "todo") -> list[dict]:
    """列出目标。status: todo/done/all,默认 todo。"""
    return [g.to_dict() for g in _goals().list(status=status)]


@mcp.tool()
def goal_link_task(goal_id: str, task_id: str) -> dict:
    """把任务关联到目标(给任务打 goal:<id> 标签)。"""
    store, _ = _deps()
    task = store.get(task_id)
    tags = list(task.tags) if task else []
    tag = f"goal:{goal_id}"
    if tag not in tags:
        tags.append(tag)
    return store.update(task_id, {"tags": tags}, actor="agent:mcp").to_dict()


def main() -> None:
    mcp.run()  # stdio


if __name__ == "__main__":
    main()

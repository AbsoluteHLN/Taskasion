"""目标存储:goals.md 为真相源,行格式与 todo.md 复用同一套语法。

任务通过标签 goal:<目标id> 关联到目标;目标本身是长期意向,
只有 标题 / 完成态 / created,进度由关联任务统计得出(见 server /api/goals)。
"""

from __future__ import annotations

import os
import threading
from datetime import datetime
from pathlib import Path

from .models import Task, new_id, parse_line, render_line

GOALS_HEADER = "# Taskasion Goals\n\n<!-- 目标真相源;任务用标签 goal:<id> 关联到目标 -->\n\n"


class GoalStore:
    def __init__(self, data_dir: str | Path, audit=None):
        self.data_dir = Path(data_dir)
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.goals_path = self.data_dir / "goals.md"
        self.audit = audit
        self._lock = threading.RLock()
        self._goals: list[Task] = []
        self._stamp: tuple[int, int] | None = None
        with self._lock:
            self._load()

    # ---------- 文件 <-> 内存 ----------

    def _stat(self) -> tuple[int, int] | None:
        try:
            st = self.goals_path.stat()
            return (st.st_mtime_ns, st.st_size)
        except FileNotFoundError:
            return None

    def _load(self) -> None:
        try:
            text = self.goals_path.read_text(encoding="utf-8")
        except FileNotFoundError:
            text = ""
        goals: list[Task] = []
        seen: set[str] = set()
        normalized = False
        for line in text.splitlines():
            goal = parse_line(line)
            if goal is None:
                continue
            if not goal.id or goal.id in seen:
                goal.id = new_id()
                normalized = True
            seen.add(goal.id)
            if not goal.created:
                goal.created = datetime.now().isoformat(timespec="seconds")
                normalized = True
            goals.append(goal)
        self._goals = goals
        self._stamp = self._stat()
        if normalized:
            self._save()

    def reload_if_changed(self) -> bool:
        stamp = self._stat()
        if stamp == self._stamp:
            return False
        with self._lock:
            before = {g.id for g in self._goals}
            self._load()
            if self.audit is not None:
                new_ids = [g.id for g in self._goals if g.id not in before]
                self.audit.append(
                    "external",
                    "external_edit_goals",
                    detail=f"goals={len(self._goals)} new={new_ids}",
                )
        return True

    def _save(self) -> None:
        todo = [render_line(g) for g in self._goals if not g.done]
        done = [render_line(g) for g in self._goals if g.done]
        parts = todo + done
        content = GOALS_HEADER + ("\n".join(parts) + "\n" if parts else "")
        tmp = self.goals_path.with_suffix(".md.tmp")
        tmp.write_text(content, encoding="utf-8")
        os.replace(tmp, self.goals_path)
        self._stamp = self._stat()

    def _log(self, actor: str, action: str, goal: Task | None, detail: str = "") -> None:
        if self.audit is not None:
            self.audit.append(actor, action, task_id=goal.id if goal else None, detail=detail)

    # ---------- 公开操作 ----------

    def list(self, status: str = "all") -> list[Task]:
        with self._lock:
            self.reload_if_changed()
            goals = list(self._goals)
        if status == "todo":
            goals = [g for g in goals if not g.done]
        elif status == "done":
            goals = [g for g in goals if g.done]
        return goals

    def add(self, title: str, source: str = "human") -> Task:
        title = title.strip()
        if not title:
            raise ValueError("title 不能为空")
        goal = Task(
            id=new_id(),
            title=title,
            source=source,
            created=datetime.now().isoformat(timespec="seconds"),
        )
        with self._lock:
            self.reload_if_changed()
            self._goals.insert(0, goal)
            self._save()
        self._log(source, "goal_add", goal, title)
        return goal

    def rename(self, goal_id: str, title: str, actor: str = "human") -> Task:
        title = title.strip()
        if not title:
            raise ValueError("title 不能为空")
        with self._lock:
            self.reload_if_changed()
            goal = self._require(goal_id)
            goal.title = title
            self._save()
        self._log(actor, "goal_rename", goal, title)
        return goal

    def set_done(self, goal_id: str, done: bool, actor: str = "human") -> Task:
        with self._lock:
            self.reload_if_changed()
            goal = self._require(goal_id)
            goal.done = done
            goal.done_at = datetime.now().isoformat(timespec="seconds") if done else None
            self._save()
        self._log(actor, "goal_complete" if done else "goal_reopen", goal, goal.title)
        return goal

    def delete(self, goal_id: str, actor: str = "human") -> bool:
        with self._lock:
            self.reload_if_changed()
            goal = self._require(goal_id)
            self._goals.remove(goal)
            self._save()
        self._log(actor, "goal_delete", goal, goal.title)
        return True

    def _require(self, goal_id: str) -> Task:
        for g in self._goals:
            if g.id == goal_id:
                return g
        raise KeyError(f"目标不存在: {goal_id}")

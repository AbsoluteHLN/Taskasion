"""任务存储:todo.md 是唯一真相源。

- 人、Agent、脚本都可以直接编辑 todo.md;每次公开操作前检测 mtime/size,
  变化则重新加载(外部编辑即时生效,审计记为 actor=external)。
- 保存时原子写(tmp + os.replace),并归一化:补 id、待办在前已完成在后。
"""

from __future__ import annotations

import os
import threading
from datetime import date, datetime
from pathlib import Path

from .models import Task, new_id, parse_line, render_line

HEADER = (
    "# Taskasion\n\n"
    "<!-- 真相源:可直接编辑;行尾 <!-- --> 注释是元数据(Core 会补齐/归一化) -->\n\n"
)

_ALLOWED_UPDATE_FIELDS = {"title", "due", "priority", "tags", "source", "note"}
_CLEARABLE_FIELDS = {"due", "note", "priority"}


class TaskStore:
    def __init__(self, data_dir: str | Path, audit=None):
        self.data_dir = Path(data_dir)
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.todo_path = self.data_dir / "todo.md"
        self.audit = audit
        self._lock = threading.RLock()
        self._tasks: list[Task] = []
        self._stamp: tuple[int, int] | None = None
        with self._lock:
            self._load()

    # ---------- 文件 <-> 内存 ----------

    def _stat(self) -> tuple[int, int] | None:
        try:
            st = self.todo_path.stat()
            return (st.st_mtime_ns, st.st_size)
        except FileNotFoundError:
            return None

    def _load(self) -> None:
        """重新解析 todo.md。调用方须持有锁(或处于构造期)。"""
        try:
            text = self.todo_path.read_text(encoding="utf-8")
        except FileNotFoundError:
            text = ""
        tasks: list[Task] = []
        seen: set[str] = set()
        normalized = False
        for line in text.splitlines():
            task = parse_line(line)
            if task is None:
                continue
            if not task.id or task.id in seen:
                task.id = new_id()  # 无 id(外部手写行)→ 分配并回写,保证 id 稳定
                normalized = True
            seen.add(task.id)
            if not task.created:
                task.created = datetime.now().isoformat(timespec="seconds")
                normalized = True
            tasks.append(task)
        self._tasks = tasks
        self._stamp = self._stat()
        if normalized:
            self._save()

    def reload_if_changed(self) -> bool:
        stamp = self._stat()
        if stamp == self._stamp:
            return False
        with self._lock:
            before = {t.id for t in self._tasks}
            self._load()
            if self.audit is not None:
                new_ids = [t.id for t in self._tasks if t.id not in before]
                self.audit.append(
                    "external",
                    "external_edit",
                    detail=f"tasks={len(self._tasks)} new={new_ids}",
                )
        return True

    def _save(self) -> None:
        todo = [render_line(t) for t in self._tasks if not t.done]
        done = [render_line(t) for t in self._tasks if t.done]
        body = "\n".join(todo + done)
        content = HEADER + (body + "\n" if body else "")
        tmp = self.todo_path.with_suffix(".md.tmp")
        tmp.write_text(content, encoding="utf-8")
        os.replace(tmp, self.todo_path)
        self._stamp = self._stat()

    def _log(self, actor: str, action: str, task: Task | None, detail: str = "") -> None:
        if self.audit is not None:
            self.audit.append(actor, action, task_id=task.id if task else None, detail=detail)

    # ---------- 公开操作 ----------

    def list(self, status: str = "all", tag: str | None = None) -> list[Task]:
        with self._lock:
            self.reload_if_changed()
            tasks = self._tasks
        if status == "todo":
            tasks = [t for t in tasks if not t.done]
        elif status == "done":
            tasks = [t for t in tasks if t.done]
        if tag:
            tasks = [t for t in tasks if tag in t.tags]
        return tasks

    def add(
        self,
        title: str,
        due: str | None = None,
        priority: str | None = None,
        tags: list[str] | None = None,
        source: str = "human",
        note: str | None = None,
    ) -> Task:
        title = title.strip()
        if not title:
            raise ValueError("title 不能为空")
        task = Task(
            id=new_id(),
            title=title,
            due=due or None,
            priority=priority or None,
            tags=list(tags or []),
            source=source,
            created=datetime.now().isoformat(timespec="seconds"),
            note=(note or "").strip() or None,
        )
        with self._lock:
            self.reload_if_changed()
            self._tasks.insert(0, task)
            self._save()
        self._log(source, "add", task, title)
        return task

    def get(self, task_id: str) -> Task | None:
        with self._lock:
            self.reload_if_changed()
            for t in self._tasks:
                if t.id == task_id:
                    return t
        return None

    def update(self, task_id: str, fields: dict, actor: str = "human") -> Task:
        clean = {
            k: (v or None if k in _CLEARABLE_FIELDS else v)
            for k, v in fields.items()
            if k in _ALLOWED_UPDATE_FIELDS and (v is not None or k in _CLEARABLE_FIELDS)
        }
        if not clean:
            raise ValueError("没有可更新字段")
        with self._lock:
            self.reload_if_changed()
            task = self._require(task_id)
            for k, v in clean.items():
                setattr(task, k, v)
            self._save()
        self._log(actor, "update", task, ",".join(clean))
        return task

    def set_done(self, task_id: str, done: bool, actor: str = "human") -> Task:
        with self._lock:
            self.reload_if_changed()
            task = self._require(task_id)
            task.done = done
            task.done_at = datetime.now().isoformat(timespec="seconds") if done else None
            self._save()
        self._log(actor, "complete" if done else "reopen", task, task.title)
        return task

    def delete(self, task_id: str, actor: str = "human") -> bool:
        with self._lock:
            self.reload_if_changed()
            task = self._require(task_id)
            self._tasks.remove(task)
            self._save()
        self._log(actor, "delete", task, task.title)
        return True

    def plan_today(self) -> dict:
        today = date.today().isoformat()
        with self._lock:
            self.reload_if_changed()
            live = [t for t in self._tasks if not t.done]
        overdue = [t.to_dict() for t in live if t.due and t.due < today]
        today_tasks = [t.to_dict() for t in live if t.due == today]
        next_up = [t.to_dict() for t in live if t.due and t.due > today]
        next_up.sort(key=lambda t: (t["due"], t["priority"] or "p9"))
        return {"today": today, "overdue": overdue, "today_tasks": today_tasks, "next": next_up[:5]}

    def _require(self, task_id: str) -> Task:
        for t in self._tasks:
            if t.id == task_id:
                return t
        raise KeyError(f"任务不存在: {task_id}")

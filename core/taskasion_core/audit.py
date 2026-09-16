"""追加式审计日志:所有任务变更(含 Agent 与外部编辑)记入 audit.jsonl。"""

from __future__ import annotations

import json
import threading
from collections import deque
from datetime import datetime
from pathlib import Path


class Audit:
    def __init__(self, path: str | Path):
        self.path = Path(path)
        self._lock = threading.Lock()

    def append(self, actor: str, action: str, task_id: str | None = None, detail: str = "") -> None:
        record = {
            "ts": datetime.now().isoformat(timespec="seconds"),
            "actor": actor,
            "action": action,
            "task_id": task_id,
            "detail": detail,
        }
        line = json.dumps(record, ensure_ascii=False)
        with self._lock:
            with open(self.path, "a", encoding="utf-8") as fh:
                fh.write(line + "\n")

    def tail(self, n: int = 50) -> list[dict]:
        with self._lock:
            try:
                lines = self.path.read_text(encoding="utf-8").splitlines()
            except FileNotFoundError:
                return []
        out = []
        for line in lines[-n:]:
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                continue
        return out
